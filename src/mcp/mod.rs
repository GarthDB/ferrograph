//! MCP server for AI agents.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::serve_server;
use rmcp::transport::stdio;
use rmcp::{handler::server::ServerHandler, service::RequestContext, RoleServer};

/// Valid JSON Schema for tools with no parameters.
fn dead_code_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({ "type": "object" })
        .as_object()
        .unwrap()
        .clone()
}

fn blast_radius_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "node_id": {
                "type": "string",
                "description": "The node ID to compute blast radius for"
            }
        },
        "required": ["node_id"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn resolve_store_path() -> Option<PathBuf> {
    std::env::var("FERROGRAPH_DB")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(|p| p.join(".ferrograph")))
}

/// Cache: path and store handle so we do not reopen the database on every tool call.
type StoreCache = Arc<Mutex<Option<(PathBuf, Arc<crate::graph::Store>)>>>;

/// MCP server handler exposing Ferrograph tools.
#[derive(Clone)]
pub struct FerrographMcp {
    cached: StoreCache,
}

impl Default for FerrographMcp {
    fn default() -> Self {
        Self {
            cached: Arc::new(Mutex::new(None)),
        }
    }
}

impl ServerHandler for FerrographMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "ferrograph".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            Tool {
                name: Cow::Borrowed("dead_code"),
                title: None,
                description: Some(Cow::Borrowed(
                    "List function node ids that are not reachable from any entry point.",
                )),
                input_schema: Arc::new(dead_code_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("blast_radius"),
                title: None,
                description: Some(Cow::Borrowed(
                    "List nodes reachable from a given node (what breaks if this changes).",
                )),
                input_schema: Arc::new(blast_radius_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let name = request.name.as_ref();
        let store_path = resolve_store_path().ok_or_else(|| {
            rmcp::ErrorData::invalid_params(
                "Could not resolve graph path (current dir or FERROGRAPH_DB)",
                None,
            )
        })?;
        if !store_path.exists() {
            return Ok(CallToolResult::structured_error(serde_json::json!({
                "error": "No graph database found",
                "hint": "Run 'ferrograph index --output .ferrograph' in the project root, or set FERROGRAPH_DB to the graph path."
            })));
        }
        let store = self.get_or_open_store(&store_path).await?;
        let result = match name {
            "dead_code" => {
                let mut ids = crate::graph::Query::stored_dead_functions(&store).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Dead code query failed: {e}"), None)
                })?;
                let source = if ids.is_empty() {
                    ids = crate::graph::Query::compute_dead_functions(&store).map_err(|e| {
                        rmcp::ErrorData::internal_error(
                            format!("Dead code (live) query failed: {e}"),
                            None,
                        )
                    })?;
                    "computed"
                } else {
                    "stored"
                };
                CallToolResult::structured(serde_json::json!({
                    "dead_function_ids": ids,
                    "count": ids.len(),
                    "source": source
                }))
            }
            "blast_radius" => {
                let node_id = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("node_id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("missing required parameter: node_id", None)
                    })?;
                let ids = crate::graph::Query::blast_radius(&store, node_id).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Blast radius query failed: {e}"), None)
                })?;
                CallToolResult::structured(serde_json::json!({
                    "from_node_id": node_id.to_string(),
                    "reachable_node_ids": ids,
                    "count": ids.len()
                }))
            }
            _ => {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Unknown tool: {name}"),
                    None,
                ));
            }
        };
        Ok(result)
    }
}

impl FerrographMcp {
    async fn get_or_open_store(
        &self,
        store_path: &std::path::Path,
    ) -> Result<Arc<crate::graph::Store>, rmcp::ErrorData> {
        let path_buf = store_path.to_path_buf();
        {
            let mut guard = self.cached.lock().await;
            if let Some((ref cached_path, ref store)) = *guard {
                if *cached_path == path_buf {
                    return Ok(Arc::clone(store));
                }
            }
            let store = crate::graph::Store::new_persistent(&path_buf).map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to open graph: {e}"), None)
            })?;
            let store = Arc::new(store);
            *guard = Some((path_buf, Arc::clone(&store)));
            Ok(store)
        }
    }
}

/// Run the MCP server over stdio (for use by IDEs and AI agents).
///
/// # Errors
/// Fails if transport or server initialization fails.
pub async fn run_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let handler = FerrographMcp::default();
    let transport = stdio();
    let service = serve_server(handler, transport).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{blast_radius_input_schema, dead_code_input_schema, FerrographMcp};

    #[test]
    fn mcp_default_constructs() {
        let _ = FerrographMcp::default();
    }

    #[test]
    fn dead_code_schema_has_type_object() {
        let schema = dead_code_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
    }

    #[test]
    fn blast_radius_schema_has_required_node_id() {
        let schema = blast_radius_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("node_id")));
    }
}
