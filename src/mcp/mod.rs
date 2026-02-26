//! MCP server for AI agents.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use glob::Pattern;
use tokio::sync::Mutex;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::serve_server;
use rmcp::transport::stdio;
use rmcp::{handler::server::ServerHandler, service::RequestContext, RoleServer};

fn dead_code_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "file": {
                "type": "string",
                "description": "Glob pattern to filter by file path (e.g. '**/src/**')"
            },
            "node_type": {
                "type": "string",
                "description": "Filter by node type (e.g. 'function')"
            },
            "limit": {
                "type": "integer",
                "description": "Max number of results to return",
                "default": 100
            },
            "offset": {
                "type": "integer",
                "description": "Number of results to skip (pagination)",
                "default": 0
            }
        }
    })
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

fn search_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query (substring match over node payloads)"
            },
            "case_insensitive": {
                "type": "boolean",
                "description": "Match case-insensitively",
                "default": false
            }
        },
        "required": ["query"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn status_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({ "type": "object" })
        .as_object()
        .unwrap()
        .clone()
}

fn trait_implementors_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "trait_node_id": {
                "type": "string",
                "description": "The trait node ID to list implementors of"
            }
        },
        "required": ["trait_node_id"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn module_graph_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "root": {
                "type": "string",
                "description": "Optional node ID to scope the tree (subtree only)"
            }
        }
    })
    .as_object()
    .unwrap()
    .clone()
}

fn node_info_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "node_id": {
                "type": "string",
                "description": "The node ID (e.g. path#line:col or path::name)"
            }
        },
        "required": ["node_id"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn callers_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "node_id": {
                "type": "string",
                "description": "The node ID (e.g. function) to find callers of"
            },
            "depth": {
                "type": "integer",
                "description": "1 = direct callers only; >1 = transitive callers",
                "default": 1
            }
        },
        "required": ["node_id"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn query_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Datalog script to run"
            },
            "limit": {
                "type": "integer",
                "description": "Max rows to return (capped at 10000)",
                "default": 10000
            }
        },
        "required": ["query"]
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
            Tool {
                name: Cow::Borrowed("search"),
                title: None,
                description: Some(Cow::Borrowed(
                    "Text search over node payloads (substring match). Find symbols by name.",
                )),
                input_schema: Arc::new(search_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("status"),
                title: None,
                description: Some(Cow::Borrowed(
                    "Show graph stats: node count, edge count, and database path.",
                )),
                input_schema: Arc::new(status_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("query"),
                title: None,
                description: Some(Cow::Borrowed(
                    "Run a raw Datalog query against the graph. Use :limit N in the script or pass limit.",
                )),
                input_schema: Arc::new(query_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("callers"),
                title: None,
                description: Some(Cow::Borrowed(
                    "List nodes that call the given node (reverse call graph). Depth 1 = direct callers only.",
                )),
                input_schema: Arc::new(callers_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("node_info"),
                title: None,
                description: Some(Cow::Borrowed(
                    "Look up a node by ID: type, payload, and incoming/outgoing edges.",
                )),
                input_schema: Arc::new(node_info_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("module_graph"),
                title: None,
                description: Some(Cow::Borrowed(
                    "Return module containment tree (Contains edges). Optional root scopes to subtree.",
                )),
                input_schema: Arc::new(module_graph_input_schema()),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: Cow::Borrowed("trait_implementors"),
                title: None,
                description: Some(Cow::Borrowed(
                    "List all impl blocks that implement the given trait.",
                )),
                input_schema: Arc::new(trait_implementors_input_schema()),
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
                let file_glob = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("file"))
                    .and_then(serde_json::Value::as_str);
                let node_type_filter = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("node_type"))
                    .and_then(serde_json::Value::as_str);
                let limit = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("limit"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(100)
                    .min(10_000) as usize;
                let offset = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("offset"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;

                let filtered_ids: Vec<String> = if file_glob.is_some() || node_type_filter.is_some()
                {
                    let id_types = crate::graph::Query::node_ids_to_types(&store, &ids)
                        .map_err(|e| {
                            rmcp::ErrorData::internal_error(
                                format!("Dead code (node types) failed: {e}"),
                                None,
                            )
                        })?;
                    let file_pattern = file_glob.and_then(|g| Pattern::new(g).ok());
                    let filtered: Vec<String> = id_types
                        .into_iter()
                        .filter(|(id, ntype)| {
                            if let Some(ref pat) = file_pattern {
                                let file_part = id.split('#').next().unwrap_or(id);
                                if !pat.matches(file_part) {
                                    return false;
                                }
                            }
                            if let Some(nt) = node_type_filter {
                                if ntype != nt {
                                    return false;
                                }
                            }
                            true
                        })
                        .map(|(id, _)| id)
                        .collect();
                    filtered
                } else {
                    ids
                };

                let paginated: Vec<&String> = filtered_ids
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .collect();
                let dead_function_ids: Vec<String> =
                    paginated.into_iter().cloned().collect();
                CallToolResult::structured(serde_json::json!({
                    "dead_function_ids": dead_function_ids,
                    "count": dead_function_ids.len(),
                    "total_filtered": filtered_ids.len(),
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
                let nodes = crate::graph::Query::blast_radius(&store, node_id).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Blast radius query failed: {e}"), None)
                })?;
                let reachable_nodes: Vec<serde_json::Value> = nodes
                    .into_iter()
                    .map(|(nid, ntype, payload)| {
                        serde_json::json!({
                            "node_id": nid,
                            "node_type": ntype,
                            "payload": payload
                        })
                    })
                    .collect();
                CallToolResult::structured(serde_json::json!({
                    "from_node_id": node_id.to_string(),
                    "reachable_nodes": reachable_nodes,
                    "count": reachable_nodes.len()
                }))
            }
            "search" => {
                let query = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("query"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("missing required parameter: query", None)
                    })?;
                let case_insensitive = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("case_insensitive"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let rows = crate::search::text_search(&store, query, case_insensitive).map_err(
                    |e| rmcp::ErrorData::internal_error(format!("Search failed: {e}"), None),
                )?;
                let results: Vec<serde_json::Value> = rows
                    .into_iter()
                    .map(|(node_id, node_type, payload)| {
                        serde_json::json!({
                            "node_id": node_id,
                            "node_type": node_type,
                            "payload": payload
                        })
                    })
                    .collect();
                CallToolResult::structured(serde_json::json!({
                    "results": results,
                    "count": results.len()
                }))
            }
            "status" => {
                let node_count = store.node_count().map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Status (node_count) failed: {e}"), None)
                })?;
                let edge_count = store.edge_count().map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Status (edge_count) failed: {e}"), None)
                })?;
                CallToolResult::structured(serde_json::json!({
                    "db_path": store_path.display().to_string(),
                    "node_count": node_count,
                    "edge_count": edge_count
                }))
            }
            "query" => {
                let query_str = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("query"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("missing required parameter: query", None)
                    })?;
                let limit = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("limit"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(10_000)
                    .min(10_000) as u64;
                let script = if query_str.contains(":limit") {
                    query_str.trim().to_string()
                } else {
                    format!("{}\n:limit {limit}", query_str.trim())
                };
                let params = std::collections::BTreeMap::new();
                let named_rows = store.run_query(&script, params).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Query failed: {e}"), None)
                })?;
                let columns: Vec<String> = named_rows.headers.clone();
                let rows_json: Vec<serde_json::Value> = named_rows
                    .rows
                    .iter()
                    .map(|row| {
                        serde_json::Value::Array(
                            row.iter()
                                .map(|v| {
                                    serde_json::Value::String(crate::graph::unquote_datavalue(v))
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let count = rows_json.len();
                CallToolResult::structured(serde_json::json!({
                    "columns": columns,
                    "rows": rows_json,
                    "count": count
                }))
            }
            "callers" => {
                let node_id = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("node_id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("missing required parameter: node_id", None)
                    })?;
                let depth = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("depth"))
                    .and_then(serde_json::Value::as_u64)
                    .map(|n| n.min(100) as u32)
                    .unwrap_or(1);
                let callers = crate::graph::Query::callers(&store, node_id, depth).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Callers query failed: {e}"), None)
                })?;
                let callers_json: Vec<serde_json::Value> = callers
                    .into_iter()
                    .map(|(node_id, node_type, payload)| {
                        serde_json::json!({
                            "node_id": node_id,
                            "node_type": node_type,
                            "payload": payload
                        })
                    })
                    .collect();
                CallToolResult::structured(serde_json::json!({
                    "node_id": node_id.to_string(),
                    "callers": callers_json,
                    "count": callers_json.len()
                }))
            }
            "node_info" => {
                let node_id = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("node_id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params(
                            "missing required parameter: node_id",
                            None,
                        )
                    })?;
                let info = crate::graph::Query::node_info(&store, node_id).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Node info query failed: {e}"), None)
                })?;
                let found = info.is_some();
                let (node_id_val, node_type, payload, incoming_edges, outgoing_edges) = match info {
                    Some((nid, ntype, payload, inc, out)) => {
                        let inc_json: Vec<serde_json::Value> = inc
                            .into_iter()
                            .map(|(from, edge_type)| {
                                serde_json::json!({ "from": from, "edge_type": edge_type })
                            })
                            .collect();
                        let out_json: Vec<serde_json::Value> = out
                            .into_iter()
                            .map(|(to, edge_type)| {
                                serde_json::json!({ "to": to, "edge_type": edge_type })
                            })
                            .collect();
                        (
                            serde_json::Value::String(nid),
                            serde_json::Value::String(ntype),
                            payload
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                            inc_json,
                            out_json,
                        )
                    }
                    None => (
                        serde_json::Value::Null,
                        serde_json::Value::Null,
                        serde_json::Value::Null,
                        vec![],
                        vec![],
                    ),
                };
                CallToolResult::structured(serde_json::json!({
                    "node_id": node_id_val,
                    "node_type": node_type,
                    "payload": payload,
                    "incoming_edges": incoming_edges,
                    "outgoing_edges": outgoing_edges,
                    "found": found
                }))
            }
            "module_graph" => {
                let root = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("root"))
                    .and_then(serde_json::Value::as_str);
                let modules = crate::graph::Query::module_graph(&store, root).map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Module graph query failed: {e}"),
                        None,
                    )
                })?;
                let modules_json: Vec<serde_json::Value> = modules
                    .into_iter()
                    .map(|(node_id, node_type, payload, children)| {
                        serde_json::json!({
                            "node_id": node_id,
                            "node_type": node_type,
                            "payload": payload,
                            "children": children
                        })
                    })
                    .collect();
                CallToolResult::structured(serde_json::json!({
                    "modules": modules_json
                }))
            }
            "trait_implementors" => {
                let trait_node_id = request
                    .arguments
                    .as_ref()
                    .and_then(|m| m.get("trait_node_id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        rmcp::ErrorData::invalid_params(
                            "missing required parameter: trait_node_id",
                            None,
                        )
                    })?;
                let implementors =
                    crate::graph::Query::trait_implementors(&store, trait_node_id).map_err(|e| {
                        rmcp::ErrorData::internal_error(
                            format!("Trait implementors query failed: {e}"),
                            None,
                        )
                    })?;
                let implementors_json: Vec<serde_json::Value> = implementors
                    .into_iter()
                    .map(|(node_id, node_type, payload)| {
                        serde_json::json!({
                            "node_id": node_id,
                            "node_type": node_type,
                            "payload": payload
                        })
                    })
                    .collect();
                CallToolResult::structured(serde_json::json!({
                    "trait_node_id": trait_node_id.to_string(),
                    "implementors": implementors_json,
                    "count": implementors_json.len()
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
    use super::{
        blast_radius_input_schema, callers_input_schema, dead_code_input_schema,
        module_graph_input_schema, node_info_input_schema, query_input_schema,
        search_input_schema, status_input_schema, trait_implementors_input_schema, FerrographMcp,
    };

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

    #[test]
    fn search_schema_has_required_query() {
        let schema = search_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn status_schema_has_type_object() {
        let schema = status_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
    }

    #[test]
    fn query_schema_has_required_query() {
        let schema = query_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn callers_schema_has_required_node_id() {
        let schema = callers_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("node_id")));
    }

    #[test]
    fn node_info_schema_has_required_node_id() {
        let schema = node_info_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("node_id")));
    }

    #[test]
    fn trait_implementors_schema_has_required_trait_node_id() {
        let schema = trait_implementors_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required
            .iter()
            .any(|v| v.as_str() == Some("trait_node_id")));
    }

    #[test]
    fn module_graph_schema_has_type_object() {
        let schema = module_graph_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
    }
}
