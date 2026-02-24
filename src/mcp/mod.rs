//! MCP server for AI agents.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::serve_server;
use rmcp::transport::stdio;
use rmcp::{handler::server::ServerHandler, service::RequestContext, RoleServer};

fn blast_radius_input_schema() -> serde_json::Map<String, serde_json::Value> {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), serde_json::json!("object"));
    let mut props = serde_json::Map::new();
    props.insert(
        "node_id".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "The node ID to compute blast radius for"
        }),
    );
    schema.insert("properties".to_string(), serde_json::Value::Object(props));
    schema.insert("required".to_string(), serde_json::json!(["node_id"]));
    schema
}

fn resolve_store_path() -> Option<PathBuf> {
    std::env::var("FERROGRAPH_DB")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok().map(|p| p.join(".ferrograph")))
}

/// MCP server handler exposing Ferrograph tools.
#[derive(Clone)]
pub struct FerrographMcp;

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
                input_schema: Arc::new(serde_json::Map::new()),
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
        let store = crate::graph::Store::new_persistent(&store_path).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to open graph: {e}"), None)
        })?;
        let result = match name {
            "dead_code" => {
                let ids = crate::graph::Query::dead_functions(&store).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Dead code query failed: {e}"), None)
                })?;
                CallToolResult::structured(serde_json::json!({
                    "dead_function_ids": ids,
                    "count": ids.len()
                }))
            }
            "blast_radius" => {
                let node_id = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("node_id"))
                    .and_then(serde_json::Value::as_str)
                    .map_or_else(|| "main".to_string(), std::string::ToString::to_string);
                let ids = crate::graph::Query::blast_radius(&store, &node_id).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Blast radius query failed: {e}"), None)
                })?;
                CallToolResult::structured(serde_json::json!({
                    "from_node_id": node_id,
                    "reachable_node_ids": ids,
                    "count": ids.len()
                }))
            }
            _ => {
                return Ok(CallToolResult::structured_error(serde_json::json!({
                    "error": "Unknown tool",
                    "name": name
                })));
            }
        };
        Ok(result)
    }
}

/// Run the MCP server over stdio (for use by IDEs and AI agents).
///
/// # Errors
/// Fails if transport or server initialization fails.
pub async fn run_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let handler = FerrographMcp;
    let transport = stdio();
    let service = serve_server(handler, transport).await?;
    service.waiting().await?;
    Ok(())
}
