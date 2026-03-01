//! MCP server for AI agents.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use rmcp::model::{
    Annotated, CallToolRequestParams, CallToolResult, Implementation, ListResourcesResult,
    ListToolsResult, RawResource, ReadResourceRequestParams, ReadResourceResult, ResourceContents,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::serve_server;
use rmcp::transport::stdio;
use rmcp::{handler::server::ServerHandler, service::RequestContext, RoleServer};

/// Valid JSON Schema for `dead_code` (optional filters).
fn dead_code_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "file": {
                "type": "string",
                "description": "Glob pattern to filter node IDs by path (e.g. src/**/*.rs, **/utils/*)"
            },
            "node_type": {
                "type": "string",
                "description": "Filter by node type (e.g. function, struct)"
            },
            "limit": {
                "type": "integer",
                "description": "Max number of IDs to return (default 100)",
                "default": 100
            },
            "offset": {
                "type": "integer",
                "description": "Number of IDs to skip for pagination (default 0)",
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
            },
            "limit": {
                "type": "integer",
                "description": "Max number of results to return (default 100)",
                "default": 100
            },
            "offset": {
                "type": "integer",
                "description": "Number of results to skip for pagination (default 0)",
                "default": 0
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

fn query_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "script": {
                "type": "string",
                "description": "Datalog script to run (read-only)"
            },
            "limit": {
                "type": "integer",
                "description": "Max rows to return (default 100)",
                "default": 100
            }
        },
        "required": ["script"]
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
                "description": "Max hops backward (1 = direct callers only, default 1)",
                "default": 1
            }
        },
        "required": ["node_id"]
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
                "description": "Node ID (e.g. ./apps/ferris-cli/src/main.rs#82:5) to get type, payload, and edges for"
            }
        },
        "required": ["node_id"]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn trait_implementors_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "trait_name": {
                "type": "string",
                "description": "Trait name (substring match) to find impl blocks for"
            }
        },
        "required": ["trait_name"]
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
                "description": "Optional path prefix to filter module graph (e.g. src/)"
            }
        }
    })
    .as_object()
    .unwrap()
    .clone()
}

fn reindex_input_schema() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Project root to index (default: current working directory)"
            }
        }
    })
    .as_object()
    .unwrap()
    .clone()
}

fn get_str_arg<'a>(request: &'a CallToolRequestParams, key: &str) -> Option<&'a str> {
    request
        .arguments
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(serde_json::Value::as_str)
}

fn tool(
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Map<String, serde_json::Value>,
) -> Tool {
    Tool {
        name: Cow::Borrowed(name),
        title: None,
        description: Some(Cow::Borrowed(description)),
        input_schema: Arc::new(input_schema),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

fn all_tools() -> Vec<Tool> {
    vec![
        tool(
            "dead_code",
            "List node ids that are not reachable from any entry point (e.g. dead functions).",
            dead_code_input_schema(),
        ),
        tool(
            "blast_radius",
            "List nodes reachable from a given node (what breaks if this changes).",
            blast_radius_input_schema(),
        ),
        tool(
            "search",
            "Text search over node payloads (substring match). Find symbols by name without scanning files.",
            search_input_schema(),
        ),
        tool(
            "status",
            "Graph stats: node count, edge count, db path. Verify the graph is indexed and healthy.",
            status_input_schema(),
        ),
        tool(
            "query",
            "Run a raw Datalog query against the graph (read-only). A row limit (default 100, max 10000) is always applied; an explicit :limit in the script is overridden by this cap.",
            query_input_schema(),
        ),
        tool(
            "callers",
            "Reverse call graph: list nodes that call the given node (who calls this function?).",
            callers_input_schema(),
        ),
        tool(
            "node_info",
            "Given a node ID, return its type, payload, containing context, and all incoming/outgoing edges.",
            node_info_input_schema(),
        ),
        tool(
            "trait_implementors",
            "Given a trait name, list all impl blocks that implement it (uses ImplementsTrait edges). Note: results depend on the index pipeline having populated trait edges; currently a stub.",
            trait_implementors_input_schema(),
        ),
        tool(
            "module_graph",
            "Return the module containment tree (Contains edges between file/module/crate_root). Optional path prefix filter.",
            module_graph_input_schema(),
        ),
        tool(
            "reindex",
            "Re-run the index pipeline on the current graph database (clears and repopulates). Optional project root path; defaults to current working directory.",
            reindex_input_schema(),
        ),
    ]
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
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(all_tools()))
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        let resource = Annotated::new(
            RawResource {
                uri: "ferrograph://status".to_string(),
                name: "status".to_string(),
                title: Some("Graph status".to_string()),
                description: Some("Node count, edge count, and db path".to_string()),
                mime_type: Some("application/json".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            None,
        );
        Ok(ListResourcesResult::with_all_items(vec![resource]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        const STATUS_URI: &str = "ferrograph://status";
        if request.uri != STATUS_URI {
            return Err(rmcp::ErrorData::method_not_found::<
                rmcp::model::ReadResourceRequestMethod,
            >());
        }
        let Some(store_path) = resolve_store_path() else {
            let empty = serde_json::json!({
                "error": "Could not resolve graph path",
                "node_count": null,
                "edge_count": null,
                "db_path": null
            });
            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(empty.to_string(), STATUS_URI)],
            });
        };
        if !store_path.exists() {
            let no_db = serde_json::json!({
                "error": "No graph database found",
                "hint": "Run 'ferrograph index --output .ferrograph' or set FERROGRAPH_DB",
                "node_count": null,
                "edge_count": null,
                "db_path": store_path.display().to_string()
            });
            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(no_db.to_string(), STATUS_URI)],
            });
        }
        let store = self.get_or_open_store(&store_path).await?;
        let node_count = store.node_count().map_err(|e| {
            rmcp::ErrorData::internal_error(format!("node_count failed: {e}"), None)
        })?;
        let edge_count = store.edge_count().map_err(|e| {
            rmcp::ErrorData::internal_error(format!("edge_count failed: {e}"), None)
        })?;
        let indexed_at = std::fs::metadata(&store_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let status = serde_json::json!({
            "node_count": node_count,
            "edge_count": edge_count,
            "db_path": store_path.display().to_string(),
            "indexed_at": indexed_at
        });
        Ok(ReadResourceResult {
            contents: vec![ResourceContents::text(status.to_string(), STATUS_URI)],
        })
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
            "dead_code" => Self::handle_dead_code(&request, &store)?,
            "blast_radius" => Self::handle_blast_radius(&request, &store)?,
            "search" => Self::handle_search(&request, &store)?,
            "status" => Self::handle_status(&store, &store_path)?,
            "query" => Self::handle_query(&request, &store)?,
            "callers" => Self::handle_callers(&request, &store)?,
            "node_info" => Self::handle_node_info(&request, &store)?,
            "trait_implementors" => Self::handle_trait_implementors(&request, &store)?,
            "module_graph" => Self::handle_module_graph(&request, &store)?,
            "reindex" => Self::handle_reindex(&request, &store, &store_path)?,
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

fn parse_limit(request: &CallToolRequestParams, default: u64, cap: u64) -> usize {
    let n = request
        .arguments
        .as_ref()
        .and_then(|m| m.get("limit"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(default)
        .min(cap);
    usize::try_from(n).unwrap_or(usize::MAX)
}

fn parse_offset(request: &CallToolRequestParams) -> usize {
    let n = request
        .arguments
        .as_ref()
        .and_then(|m| m.get("offset"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    usize::try_from(n).unwrap_or(0)
}

fn parse_depth(request: &CallToolRequestParams) -> u32 {
    let n = request
        .arguments
        .as_ref()
        .and_then(|m| m.get("depth"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        .min(100);
    u32::try_from(n).unwrap_or(1).min(100)
}

impl FerrographMcp {
    fn handle_dead_code(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut ids = crate::graph::Query::stored_dead_functions(store).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Dead code query failed: {e}"), None)
        })?;
        let source = if ids.is_empty() {
            ids = crate::graph::Query::compute_dead_functions(store).map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Dead code (live) query failed: {e}"), None)
            })?;
            "computed"
        } else {
            "stored"
        };
        if let Some(pattern) = get_str_arg(request, "file") {
            let pat = glob::Pattern::new(pattern).map_err(|e| {
                rmcp::ErrorData::invalid_params(format!("Invalid glob pattern: {e}"), None)
            })?;
            ids.retain(|id| pat.matches(id));
        }
        if let Some(nt) = get_str_arg(request, "node_type") {
            let id_list: Vec<cozo::DataValue> = ids
                .iter()
                .map(|s| cozo::DataValue::from(s.as_str()))
                .collect();
            let mut params = std::collections::BTreeMap::new();
            params.insert("ids".to_string(), cozo::DataValue::List(id_list));
            params.insert("node_type".to_string(), cozo::DataValue::from(nt));
            let rows = store
                .run_query(
                    "?[id] := *nodes[id, type, _], id in $ids, type = $node_type",
                    params,
                )
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Dead code filter (node_type) failed: {e}"),
                        None,
                    )
                })?;
            let type_ok: std::collections::HashSet<String> = rows
                .rows
                .iter()
                .filter_map(|r| r.first())
                .map(crate::graph::unquote_datavalue)
                .collect();
            ids.retain(|id| type_ok.contains(id));
        }
        let total_before_pagination = ids.len();
        let offset = parse_offset(request);
        let limit = parse_limit(request, 100, 10_000);
        let page: Vec<String> = ids.into_iter().skip(offset).take(limit).collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "dead_node_ids": page,
            "count": page.len(),
            "total_filtered": total_before_pagination,
            "offset": offset,
            "limit": limit,
            "source": source
        })))
    }

    fn handle_blast_radius(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let node_id = get_str_arg(request, "node_id").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: node_id", None)
        })?;
        let nodes = crate::graph::Query::blast_radius(store, node_id).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Blast radius query failed: {e}"), None)
        })?;
        let reachable: Vec<serde_json::Value> = nodes
            .into_iter()
            .map(|(id, node_type, payload)| {
                serde_json::json!({
                    "id": id,
                    "type": node_type,
                    "payload": payload
                })
            })
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "from_node_id": node_id.to_string(),
            "reachable_nodes": reachable,
            "count": reachable.len()
        })))
    }

    fn handle_search(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let query = get_str_arg(request, "query").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: query", None)
        })?;
        let case_insensitive = request
            .arguments
            .as_ref()
            .and_then(|m| m.get("case_insensitive"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let rows = crate::search::text_search(store, query, case_insensitive)
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Search failed: {e}"), None))?;
        let total = rows.len();
        let offset = parse_offset(request);
        let limit = parse_limit(request, 100, 10_000);
        let page: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();
        let results: Vec<serde_json::Value> = page
            .into_iter()
            .map(|(id, node_type, payload)| {
                serde_json::json!({
                    "id": id,
                    "type": node_type,
                    "payload": payload
                })
            })
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "results": results,
            "count": results.len(),
            "total": total,
            "offset": offset,
            "limit": limit
        })))
    }

    fn handle_status(
        store: &crate::graph::Store,
        store_path: &Path,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let node_count = store.node_count().map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Status (node_count) failed: {e}"), None)
        })?;
        let edge_count = store.edge_count().map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Status (edge_count) failed: {e}"), None)
        })?;
        let indexed_at = std::fs::metadata(store_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Ok(CallToolResult::structured(serde_json::json!({
            "node_count": node_count,
            "edge_count": edge_count,
            "db_path": store_path.display().to_string(),
            "indexed_at": indexed_at
        })))
    }

    fn handle_query(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        const MUTATION_DIRECTIVES: &[&str] = &[":put", ":rm", ":create", ":replace", ":remove"];
        let script = get_str_arg(request, "script").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: script", None)
        })?;
        let script_lower = script.to_lowercase();
        for directive in MUTATION_DIRECTIVES {
            if script_lower.contains(directive) {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Script may not contain mutation directive {directive}"),
                    None,
                ));
            }
        }
        let limit = parse_limit(request, 100, 10_000);
        let script_with_limit = format!("{}\n:limit {limit}", script.trim());
        let params = std::collections::BTreeMap::new();
        let result = store
            .run_query(&script_with_limit, params)
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Query failed: {e}"), None))?;
        let rows: Vec<Vec<serde_json::Value>> = result
            .rows
            .iter()
            .map(|row| row.iter().map(crate::graph::datavalue_to_json).collect())
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "headers": result.headers,
            "rows": rows,
            "count": rows.len()
        })))
    }

    fn handle_callers(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let node_id = get_str_arg(request, "node_id").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: node_id", None)
        })?;
        let depth = parse_depth(request);
        let callers = crate::graph::Query::callers(store, node_id, depth).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Callers query failed: {e}"), None)
        })?;
        let results: Vec<serde_json::Value> = callers
            .into_iter()
            .map(|(id, node_type, payload)| {
                serde_json::json!({
                    "id": id,
                    "type": node_type,
                    "payload": payload
                })
            })
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "node_id": node_id,
            "callers": results,
            "count": results.len()
        })))
    }

    fn handle_node_info(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let node_id = get_str_arg(request, "node_id").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: node_id", None)
        })?;
        let info = crate::graph::Query::node_info(store, node_id).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Node info query failed: {e}"), None)
        })?;
        let result = match info {
            Some(n) => serde_json::to_value(&n).unwrap_or(serde_json::Value::Null),
            None => serde_json::json!({
                "error": "Node not found",
                "node_id": node_id
            }),
        };
        Ok(CallToolResult::structured(result))
    }

    fn handle_trait_implementors(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let trait_name = get_str_arg(request, "trait_name").ok_or_else(|| {
            rmcp::ErrorData::invalid_params("missing required parameter: trait_name", None)
        })?;
        let impls = crate::graph::Query::trait_implementors(store, trait_name).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Trait implementors query failed: {e}"), None)
        })?;
        let results: Vec<serde_json::Value> = impls
            .into_iter()
            .map(|(id, node_type, payload)| {
                serde_json::json!({
                    "id": id,
                    "type": node_type,
                    "payload": payload
                })
            })
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "trait_name": trait_name,
            "implementors": results,
            "count": results.len()
        })))
    }

    fn handle_module_graph(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let root = get_str_arg(request, "root");
        let edges = crate::graph::Query::module_graph(store, root).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Module graph query failed: {e}"), None)
        })?;
        let results: Vec<serde_json::Value> = edges
            .into_iter()
            .map(|(from_id, to_id, from_type, to_type)| {
                serde_json::json!({
                    "from_id": from_id,
                    "to_id": to_id,
                    "from_type": from_type,
                    "to_type": to_type
                })
            })
            .collect();
        Ok(CallToolResult::structured(serde_json::json!({
            "edges": results,
            "count": results.len()
        })))
    }

    fn handle_reindex(
        request: &CallToolRequestParams,
        store: &crate::graph::Store,
        store_path: &Path,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let root = get_str_arg(request, "path").map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| store_path.to_path_buf()),
            PathBuf::from,
        );
        if !root.exists() {
            return Ok(CallToolResult::structured_error(serde_json::json!({
                "error": "Project path does not exist",
                "path": root.display().to_string()
            })));
        }
        store.clear().map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Reindex (clear) failed: {e}"), None)
        })?;
        let config = crate::pipeline::PipelineConfig::default();
        crate::pipeline::run_pipeline(store, &root, &config).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Reindex (pipeline) failed: {e}"), None)
        })?;
        Ok(CallToolResult::structured(serde_json::json!({
            "ok": true,
            "message": "Reindex complete",
            "path": root.display().to_string(),
            "db_path": store_path.display().to_string()
        })))
    }

    async fn get_or_open_store(
        &self,
        store_path: &Path,
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
    use std::borrow::Cow;

    use rmcp::model::{CallToolRequestParams, CallToolResult};

    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::{
        blast_radius_input_schema, callers_input_schema, dead_code_input_schema,
        module_graph_input_schema, node_info_input_schema, query_input_schema,
        reindex_input_schema, search_input_schema, status_input_schema,
        trait_implementors_input_schema, FerrographMcp,
    };

    fn tool_request(
        name: &'static str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> CallToolRequestParams {
        CallToolRequestParams {
            meta: None,
            name: Cow::Borrowed(name),
            arguments,
            task: None,
        }
    }

    #[test]
    fn mcp_default_constructs() {
        let _ = FerrographMcp::default();
    }

    #[test]
    fn dead_code_schema_has_type_object_and_optional_filters() {
        let schema = dead_code_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(props.contains_key("file"));
        assert!(props.contains_key("node_type"));
        assert!(props.contains_key("limit"));
        assert!(props.contains_key("offset"));
    }

    #[test]
    fn blast_radius_schema_has_required_node_id() {
        let schema = blast_radius_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("node_id")));
    }

    #[test]
    fn search_schema_has_required_query_and_pagination() {
        let schema = search_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(props.contains_key("case_insensitive"));
        assert!(props.contains_key("limit"));
        assert!(props.contains_key("offset"));
    }

    #[test]
    fn status_schema_is_empty_object() {
        let schema = status_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
    }

    #[test]
    fn query_schema_has_required_script() {
        let schema = query_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("script")));
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
    fn trait_implementors_schema_has_required_trait_name() {
        let schema = trait_implementors_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("trait_name")));
    }

    #[test]
    fn module_graph_schema_has_optional_root() {
        let schema = module_graph_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap()
            .contains_key("root"));
    }

    #[test]
    fn reindex_schema_has_optional_path() {
        let schema = reindex_input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap()
            .contains_key("path"));
    }

    fn result_json(result: CallToolResult) -> serde_json::Value {
        result.structured_content.unwrap()
    }

    #[test]
    fn handle_dead_code_returns_structure_with_dead_node_ids() {
        let store = Store::new_memory().unwrap();
        let request = tool_request("dead_code", None);
        let result = FerrographMcp::handle_dead_code(&request, &store).unwrap();
        let json = result_json(result);
        assert!(json.get("dead_node_ids").unwrap().is_array());
        assert!(json.get("count").unwrap().is_number());
        assert!(json.get("total_filtered").unwrap().is_number());
        assert_eq!(json.get("source").unwrap().as_str().unwrap(), "computed");
    }

    #[test]
    fn handle_dead_code_file_glob_filters_by_pattern() {
        let store = Store::new_memory().unwrap();
        store.put_dead_function("src/lib.rs#10:1").unwrap();
        store.put_dead_function("src/foo.rs#5:1").unwrap();
        store.put_dead_function("tests/helper.rs#1:1").unwrap();
        let mut args = serde_json::Map::new();
        args.insert("file".to_string(), serde_json::json!("src/**"));
        let request = tool_request("dead_code", Some(args));
        let result = FerrographMcp::handle_dead_code(&request, &store).unwrap();
        let json = result_json(result);
        let ids = json.get("dead_node_ids").unwrap().as_array().unwrap();
        let ids: Vec<&str> = ids.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            ids.iter().all(|id| id.starts_with("src/")),
            "all returned ids should match src/**"
        );
        assert!(
            !ids.iter().any(|id| id.starts_with("tests/")),
            "tests/ helper should be filtered out"
        );
    }

    #[test]
    fn handle_blast_radius_returns_reachable_nodes() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("a".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_node(&NodeId("b".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_edge(
                &NodeId("a".to_string()),
                &NodeId("b".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let mut args = serde_json::Map::new();
        args.insert("node_id".to_string(), serde_json::json!("a"));
        let request = tool_request("blast_radius", Some(args));
        let result = FerrographMcp::handle_blast_radius(&request, &store).unwrap();
        let json = result_json(result);
        let reachable = json.get("reachable_nodes").unwrap().as_array().unwrap();
        assert_eq!(reachable.len(), 1);
        assert_eq!(reachable[0].get("id").unwrap().as_str().unwrap(), "b");
    }

    #[test]
    fn handle_query_accepts_read_only_script() {
        let store = Store::new_memory().unwrap();
        let mut args = serde_json::Map::new();
        args.insert(
            "script".to_string(),
            serde_json::json!("?[id, type, payload] := *nodes[id, type, payload]"),
        );
        let request = tool_request("query", Some(args));
        let result = FerrographMcp::handle_query(&request, &store).unwrap();
        let json = result_json(result);
        assert!(json.get("headers").unwrap().is_array());
        assert!(json.get("rows").unwrap().is_array());
    }

    #[test]
    fn handle_query_rejects_mutation_directive() {
        let store = Store::new_memory().unwrap();
        let mut args = serde_json::Map::new();
        args.insert(
            "script".to_string(),
            serde_json::json!(":put x[a] := a = 1"),
        );
        let request = tool_request("query", Some(args));
        let err = FerrographMcp::handle_query(&request, &store).unwrap_err();
        assert!(err.to_string().contains(":put"));
    }

    #[test]
    fn handle_node_info_returns_node_when_found() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("n1".to_string()), &NodeType::Function, Some("foo"))
            .unwrap();
        let mut args = serde_json::Map::new();
        args.insert("node_id".to_string(), serde_json::json!("n1"));
        let request = tool_request("node_info", Some(args));
        let result = FerrographMcp::handle_node_info(&request, &store).unwrap();
        let json = result_json(result);
        assert_eq!(json.get("id").unwrap().as_str().unwrap(), "n1");
        assert_eq!(json.get("node_type").unwrap().as_str().unwrap(), "function");
    }

    #[test]
    fn handle_node_info_returns_error_when_not_found() {
        let store = Store::new_memory().unwrap();
        let mut args = serde_json::Map::new();
        args.insert("node_id".to_string(), serde_json::json!("nonexistent"));
        let request = tool_request("node_info", Some(args));
        let result = FerrographMcp::handle_node_info(&request, &store).unwrap();
        let json = result_json(result);
        assert_eq!(
            json.get("error").unwrap().as_str().unwrap(),
            "Node not found"
        );
    }

    #[test]
    fn handle_callers_depth1_returns_direct_callers() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("caller".to_string()),
                &NodeType::Function,
                Some("caller"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("callee".to_string()),
                &NodeType::Function,
                Some("callee"),
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("caller".to_string()),
                &NodeId("callee".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let mut args = serde_json::Map::new();
        args.insert("node_id".to_string(), serde_json::json!("callee"));
        args.insert("depth".to_string(), serde_json::json!(1));
        let request = tool_request("callers", Some(args));
        let result = FerrographMcp::handle_callers(&request, &store).unwrap();
        let json = result_json(result);
        let callers = json.get("callers").unwrap().as_array().unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].get("id").unwrap().as_str().unwrap(), "caller");
    }
}
