//! Shared operations used by both the CLI and MCP server.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::graph::query::{ModuleEdge, NodeInfo, Query};
use crate::graph::Store;

/// Result of a status query.
#[derive(Debug, Serialize)]
pub struct StatusResult {
    pub db_path: String,
    pub node_count: usize,
    pub edge_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<u64>,
    /// Project root the graph was built from (absent for graphs from 1.5.1 or earlier).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub nodes_by_type: Vec<(String, usize)>,
    pub edges_by_type: Vec<(String, usize)>,
}

/// A single search result row.
#[derive(Debug, Serialize)]
pub struct SearchResultItem {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
}

/// Result of a search query.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub results: Vec<SearchResultItem>,
    pub count: usize,
    pub total: usize,
}

/// Result of a raw Datalog query.
#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub count: usize,
}

/// Return graph status information including per-type breakdowns.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn status(store: &Store, store_path: &Path) -> Result<StatusResult> {
    let node_count = store.node_count()?;
    let edge_count = store.edge_count()?;
    let indexed_at = std::fs::metadata(store_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let nodes_by_type = Query::count_nodes_by_type(store)?;
    let edges_by_type = Query::count_edges_by_type(store)?;
    Ok(StatusResult {
        db_path: store_path.display().to_string(),
        node_count,
        edge_count,
        indexed_at,
        root: store.get_meta("root")?,
        nodes_by_type,
        edges_by_type,
    })
}

/// Run a text search.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn search(
    store: &Store,
    query: &str,
    case_insensitive: bool,
    limit: usize,
    offset: usize,
) -> Result<SearchResult> {
    let (rows, total) = crate::search::text_search(store, query, case_insensitive, limit, offset)?;
    let results: Vec<SearchResultItem> = rows
        .into_iter()
        .map(|(id, node_type, payload)| SearchResultItem {
            id,
            node_type,
            payload,
        })
        .collect();
    let count = results.len();
    Ok(SearchResult {
        results,
        count,
        total,
    })
}

/// A node summary (id, type, payload) used across multiple results.
#[derive(Debug, Serialize)]
pub struct NodeSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
}

/// Result of a dead code analysis.
#[derive(Debug, Serialize)]
pub struct DeadCodeResult {
    pub dead_nodes: Vec<NodeSummary>,
    pub count: usize,
    pub source: String,
    pub caveat: &'static str,
}

/// Caveat about dynamic dispatch included in dead code results.
pub const DEAD_CODE_CAVEAT: &str =
    "Note: functions called via dyn Trait (dynamic dispatch) may appear as dead \
     because ferrograph uses static analysis only.";

/// Result of a blast radius query.
#[derive(Debug, Serialize)]
pub struct BlastRadiusResult {
    pub from_node_id: String,
    pub reachable_nodes: Vec<NodeSummary>,
    pub count: usize,
}

/// Result of a callers query.
#[derive(Debug, Serialize)]
pub struct CallersResult {
    pub node_id: String,
    pub callers: Vec<NodeSummary>,
    pub count: usize,
}

/// Result of a trait implementors query.
#[derive(Debug, Serialize)]
pub struct TraitImplementorsResult {
    pub trait_name: String,
    pub implementors: Vec<NodeSummary>,
    pub count: usize,
}

/// Result of a module graph query.
#[derive(Debug, Serialize)]
pub struct ModuleGraphResult {
    pub edges: Vec<ModuleEdge>,
    pub count: usize,
}

/// Options for [`dead_code`].
#[derive(Debug, Default)]
pub struct DeadOptions {
    /// Ignore ambiguous (low-confidence) call edges when deciding liveness.
    pub strict: bool,
    /// Extra entry points: file globs (`examples/**`) or attributes (`#[no_mangle]`).
    pub entry: Vec<String>,
    /// File globs whose functions are never reported.
    pub ignore: Vec<String>,
}

fn path_matches(pat: &glob::Pattern, node_id: &str) -> bool {
    let path = node_id.split('#').next().unwrap_or(node_id);
    pat.matches(path) || pat.matches(path.strip_prefix("./").unwrap_or(path))
}

/// Resolve `entry` patterns to function node ids.
fn resolve_entry_ids(store: &Store, patterns: &[String]) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let mut globs = Vec::new();
    for pat in patterns {
        if let Some(attr) = pat.strip_prefix("#[").and_then(|p| p.strip_suffix(']')) {
            let mut params = std::collections::BTreeMap::new();
            params.insert(
                "t".to_string(),
                cozo::DataValue::from(format!("attr::{attr}").as_str()),
            );
            let rows = store.run_query("?[id] := *edges[id, $t, \"has_attribute\"]", params)?;
            ids.extend(
                rows.rows
                    .iter()
                    .filter_map(|r| r.first())
                    .map(crate::graph::unquote_datavalue),
            );
        } else {
            globs.push(glob::Pattern::new(pat)?);
        }
    }
    if !globs.is_empty() {
        let rows = store.run_query(
            "?[id] := *nodes[id, \"function\", _]",
            std::collections::BTreeMap::new(),
        )?;
        ids.extend(
            rows.rows
                .iter()
                .filter_map(|r| r.first())
                .map(crate::graph::unquote_datavalue)
                .filter(|id| globs.iter().any(|g| path_matches(g, id))),
        );
    }
    Ok(ids)
}

/// Find dead (unreachable) code with symbol names.
///
/// # Errors
/// Returns an error if the store query or a glob pattern is invalid.
pub fn dead_code(
    store: &Store,
    file_glob: Option<&str>,
    opts: &DeadOptions,
) -> Result<DeadCodeResult> {
    // The stored relation reflects index-time defaults only, so any option forces a recompute.
    let custom = opts.strict || !opts.entry.is_empty();
    let mut triples = if custom {
        Vec::new()
    } else {
        Query::stored_dead_functions_with_payload(store)?
    };
    let source = if triples.is_empty() {
        let entries = resolve_entry_ids(store, &opts.entry)?;
        triples = Query::compute_dead_functions_with_payload(store, opts.strict, &entries)?;
        "computed"
    } else {
        "stored"
    };
    for pattern in &opts.ignore {
        let pat = glob::Pattern::new(pattern)?;
        triples.retain(|(id, _, _)| !path_matches(&pat, id));
    }
    if let Some(pattern) = file_glob {
        let pat = glob::Pattern::new(pattern)?;
        triples.retain(|(id, _, _)| pat.matches(id));
    }
    let dead_nodes: Vec<NodeSummary> = triples
        .into_iter()
        .map(|(id, node_type, payload)| NodeSummary {
            id,
            node_type,
            payload,
        })
        .collect();
    let count = dead_nodes.len();
    Ok(DeadCodeResult {
        dead_nodes,
        count,
        source: source.to_string(),
        caveat: DEAD_CODE_CAVEAT,
    })
}

/// Compute the blast radius from a given node.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn blast_radius(
    store: &Store,
    node_id: &str,
    exclude_low_confidence: bool,
) -> Result<BlastRadiusResult> {
    let nodes = Query::blast_radius_opts(store, node_id, exclude_low_confidence)?;
    let reachable_nodes: Vec<NodeSummary> = nodes
        .into_iter()
        .map(|(id, node_type, payload)| NodeSummary {
            id,
            node_type,
            payload,
        })
        .collect();
    let count = reachable_nodes.len();
    Ok(BlastRadiusResult {
        from_node_id: node_id.to_string(),
        reachable_nodes,
        count,
    })
}

/// Find callers of a node up to a given depth.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn callers(
    store: &Store,
    node_id: &str,
    depth: u32,
    exclude_low_confidence: bool,
) -> Result<CallersResult> {
    let nodes = Query::callers_opts(store, node_id, depth, exclude_low_confidence)?;
    let callers: Vec<NodeSummary> = nodes
        .into_iter()
        .map(|(id, node_type, payload)| NodeSummary {
            id,
            node_type,
            payload,
        })
        .collect();
    let count = callers.len();
    Ok(CallersResult {
        node_id: node_id.to_string(),
        callers,
        count,
    })
}

/// Get full info for a node.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn node_info(store: &Store, node_id: &str) -> Result<Option<NodeInfo>> {
    Query::node_info(store, node_id)
}

/// Find implementors of a trait.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn trait_implementors(store: &Store, trait_name: &str) -> Result<TraitImplementorsResult> {
    let nodes = Query::trait_implementors(store, trait_name)?;
    let implementors: Vec<NodeSummary> = nodes
        .into_iter()
        .map(|(id, node_type, payload)| NodeSummary {
            id,
            node_type,
            payload,
        })
        .collect();
    let count = implementors.len();
    Ok(TraitImplementorsResult {
        trait_name: trait_name.to_string(),
        implementors,
        count,
    })
}

/// Get the module containment graph.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn module_graph(store: &Store, root: Option<&str>) -> Result<ModuleGraphResult> {
    let edges = Query::module_graph(store, root)?;
    let count = edges.len();
    Ok(ModuleGraphResult { edges, count })
}

/// Run a raw Datalog query.
///
/// # Errors
/// Returns an error if the store query fails.
pub fn query(store: &Store, script: &str) -> Result<QueryResult> {
    let params = std::collections::BTreeMap::new();
    let script = if script.contains(":limit") {
        script.trim().to_string()
    } else {
        format!("{}\n:limit 10000", script.trim())
    };
    let named_rows = store.run_query(&script, params)?;
    let headers = named_rows.headers.clone();
    let rows: Vec<Vec<serde_json::Value>> = named_rows
        .rows
        .iter()
        .map(|row| row.iter().map(crate::graph::datavalue_to_json).collect())
        .collect();
    let count = rows.len();
    Ok(QueryResult {
        headers,
        rows,
        count,
    })
}
