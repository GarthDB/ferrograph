//! Phase 4: call graph construction (enrichment from AST edges).
//!
//! Resolves placeholder call targets (`file_path::fn_name`) to real function node IDs
//! (`file_path#line:col`) so call edges connect to actual definition nodes.
//!
//! **Known limitation**: Only bare `foo()`-style calls are resolved. Method calls,
//! qualified paths (`mod::fn`), and UFCS are not resolved; those edges are removed
//! when the target cannot be matched to a function in the same file.

use std::collections::HashMap;

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{query::Query, unquote_datavalue, Store};

/// Build or enrich call graph by resolving AST placeholder call targets to real function nodes.
///
/// AST extraction emits call edges with `to_id` of the form `file_path::function_name`.
/// This pass rewrites those edges to point at real node IDs. Unresolvable edges
/// (e.g. cross-file or method calls) are removed.
///
/// # Errors
/// Fails if the graph query or update fails.
pub fn build_call_graph(store: &Store) -> Result<()> {
    let function_type = NodeType::Function.to_string();
    let calls_type = EdgeType::Calls;
    let calls_type_str = calls_type.to_string();

    let nodes = Query::all_nodes(store)?;
    // (file_path, function_name) -> NodeId
    let mut name_to_id: HashMap<(String, String), NodeId> = HashMap::new();
    for row in &nodes.rows {
        let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if type_val != function_type {
            continue;
        }
        let id_trim = row.first().map(unquote_datavalue).unwrap_or_default();
        let payload = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if payload.is_empty() {
            continue;
        }
        let file_path = id_trim.split('#').next().unwrap_or(&id_trim).to_string();
        let node_id = NodeId(id_trim);
        if let Some(prev) = name_to_id.insert((file_path, payload), node_id.clone()) {
            if prev.0 != node_id.0 {
                eprintln!(
                    "warning: duplicate function name in same file, later overwrites: {} vs {}",
                    prev.0, node_id.0
                );
            }
        }
    }

    let edges = Query::all_edges(store)?;
    for row in &edges.rows {
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != calls_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_str = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if !to_str.contains("::") {
            continue;
        }
        let from_id = NodeId(from_str);
        let placeholder_to_id = NodeId(to_str.clone());
        let Some((file_path, fn_name)) = to_str.split_once("::") else {
            continue;
        };
        let key = (file_path.to_string(), fn_name.to_string());
        if let Some(resolved_id) = name_to_id.get(&key) {
            store.remove_edge(&from_id, &placeholder_to_id, &calls_type)?;
            store.put_edge(&from_id, resolved_id, &calls_type)?;
        } else {
            store.remove_edge(&from_id, &placeholder_to_id, &calls_type)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::build_call_graph;

    #[test]
    fn build_call_graph_resolves_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let real_id = NodeId::new(format!("{path}#10:1"));
        store
            .put_node(&real_id, &NodeType::Function, Some("foo"))
            .unwrap();
        let caller_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&caller_id, &NodeType::Function, Some("main"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::foo"));
        store
            .put_edge(&caller_id, &placeholder, &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert!(
            to_str.contains('#'),
            "edge should point to real id (path#line:col), got {to_str}"
        );
    }
}
