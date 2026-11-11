//! Shared operations used by both the CLI and MCP server.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::graph::query::{ModuleEdge, NodeInfo, Query, RESULT_LIMIT};
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

/// Accuracy caveat included in dead code results.
pub const DEAD_CODE_CAVEAT: &str =
    "Note: this is static, name-based analysis, so treat it as a review aid, not proof. \
     It can report live code as dead (false positives), e.g. functions called via dyn Trait \
     (dynamic dispatch), names used only in macro or attribute arguments, attribute-macro \
     impls, and some cross-file paths. It can also miss dead code (false negatives): anything \
     anything `pub` or `pub(crate)`, trait impls, and name collisions keep code alive. \
     See https://github.com/GarthDB/ferrograph/issues/92.";

/// A function that the change under review leaves dead (see [`crate::pr`]).
#[derive(Debug, Serialize)]
pub struct NewlyDeadFunction {
    /// Node id at head.
    pub id: String,
    pub file: String,
    /// Function name without `pub::`/`test::` prefixes.
    pub name: String,
    pub line: u32,
    /// `became_dead` (existed and was live at base) or `added_dead` (new function, never called).
    pub status: &'static str,
}

/// A function containing changed lines, with its own blast radius size.
#[derive(Debug, Serialize)]
pub struct ChangedFunction {
    pub id: String,
    pub file: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub blast_count: usize,
    /// `blast_count` is a lower bound (the blast query hit its internal cap).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blast_truncated: bool,
}

/// Changed lines in one file that fall outside every function (use/struct/const edits).
#[derive(Debug, Serialize)]
pub struct ChangedItems {
    pub file: String,
    /// Inclusive `[start, end]` line ranges on the head side.
    pub lines: Vec<[u32; 2]>,
}

/// Union of the blast radii of all changed functions.
#[derive(Debug, Serialize)]
pub struct PrBlast {
    /// Distinct nodes in the union (always the full count).
    pub count: usize,
    /// Number of nodes listed in `nodes` (capped by `--blast-limit`).
    pub shown: usize,
    /// `count` is a lower bound (some changed function's blast query hit its internal cap).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    pub nodes: Vec<NodeSummary>,
}

/// Files that historically change together with a changed file but are untouched here.
#[derive(Debug, Serialize)]
pub struct CouplingHint {
    pub file: String,
    pub coupled_with: Vec<String>,
}

/// Result of `ferrograph pr`: what a change introduces, for gating CI.
#[derive(Debug, Serialize)]
pub struct PrResult {
    pub base: String,
    pub merge_base: String,
    pub head: String,
    pub changed_files: Vec<String>,
    pub newly_dead: Vec<NewlyDeadFunction>,
    pub newly_dead_count: usize,
    pub changed_functions: Vec<ChangedFunction>,
    pub changed_without_function: Vec<ChangedItems>,
    pub blast: PrBlast,
    /// Present only when the graph has `changes_with` edges (`git` feature).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coupling: Vec<CouplingHint>,
    pub warnings: Vec<String>,
}

/// Result of a blast radius query.
#[derive(Debug, Serialize)]
pub struct BlastRadiusResult {
    pub from_node_id: String,
    pub reachable_nodes: Vec<NodeSummary>,
    pub count: usize,
    /// The graph query hit its internal cap, so `count` is a lower bound.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Result of a callers query.
#[derive(Debug, Serialize)]
pub struct CallersResult {
    pub node_id: String,
    pub callers: Vec<NodeSummary>,
    pub count: usize,
    /// The graph query hit its internal cap, so `count` is a lower bound.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
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

pub(crate) fn path_matches(pat: &glob::Pattern, node_id: &str) -> bool {
    let path = node_id.split('#').next().unwrap_or(node_id);
    pat.matches(path) || pat.matches(path.strip_prefix("./").unwrap_or(path))
}

/// Resolve `entry` patterns to function node ids.
pub(crate) fn resolve_entry_ids(store: &Store, patterns: &[String]) -> Result<Vec<String>> {
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
    let mut nodes = Query::blast_radius_opts(store, node_id, exclude_low_confidence)?;
    let truncated = nodes.len() > RESULT_LIMIT;
    nodes.truncate(RESULT_LIMIT);
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
        truncated,
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
    let mut nodes = Query::callers_opts(store, node_id, depth, exclude_low_confidence)?;
    let truncated = nodes.len() > RESULT_LIMIT;
    nodes.truncate(RESULT_LIMIT);
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
        truncated,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};

    /// `hub` calls `n` leaf functions.
    fn hub_store(n: usize) -> Store {
        let store = Store::new_memory().unwrap();
        let hub = NodeId("hub".to_string());
        let leaves: Vec<NodeId> = (0..n).map(|i| NodeId(format!("leaf{i}"))).collect();
        let mut nodes: Vec<(NodeId, NodeType, Option<&str>)> =
            vec![(hub.clone(), NodeType::Function, None)];
        nodes.extend(leaves.iter().map(|l| (l.clone(), NodeType::Function, None)));
        store.put_nodes_batch(&nodes).unwrap();
        let edges: Vec<_> = leaves
            .iter()
            .map(|l| (hub.clone(), l.clone(), EdgeType::Calls))
            .collect();
        store.put_edges_batch(&edges).unwrap();
        store
    }

    #[test]
    fn blast_count_is_not_capped_at_500() {
        let r = blast_radius(&hub_store(700), "hub", false).unwrap();
        assert_eq!((r.count, r.truncated), (700, false));
        assert!(!serde_json::to_string(&r).unwrap().contains("truncated"));
    }

    #[test]
    fn blast_and_callers_report_truncation_at_the_internal_cap() {
        let store = hub_store(RESULT_LIMIT + 2);
        let r = blast_radius(&store, "hub", false).unwrap();
        assert_eq!((r.count, r.truncated), (RESULT_LIMIT, true));
        assert_eq!(serde_json::to_value(&r).unwrap()["truncated"], true);
        // Callers share the cap logic; here just check the untruncated path.
        let c = callers(&store, "leaf0", 1, false).unwrap();
        assert_eq!((c.count, c.truncated), (1, false));
    }
}
