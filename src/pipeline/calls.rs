//! Phase 4: call graph construction (enrichment from AST edges).
//!
//! Resolves placeholder call targets (`file_path::fn_name`) to real function node IDs
//! (`file_path#line:col`) so call edges connect to actual definition nodes.

use std::collections::HashMap;

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{cozo_str, query::Query, Store};

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
        let type_val = row.get(1).map(cozo_str).unwrap_or_default();
        if type_val != function_type {
            continue;
        }
        let id_trim = row.first().map(cozo_str).unwrap_or_default();
        let payload = row.get(2).map(cozo_str).unwrap_or_default();
        if payload.is_empty() {
            continue;
        }
        let file_path = id_trim.split('#').next().unwrap_or(&id_trim).to_string();
        name_to_id.insert((file_path, payload), NodeId(id_trim));
    }

    let edges = Query::all_edges(store)?;
    for row in &edges.rows {
        let edge_type = row.get(2).map(cozo_str).unwrap_or_default();
        if edge_type != calls_type_str {
            continue;
        }
        let from_str = row.first().map(cozo_str).unwrap_or_default();
        let to_str = row.get(1).map(cozo_str).unwrap_or_default();
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
