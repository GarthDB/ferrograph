//! Phase 4: call graph construction (enrichment from AST edges).
//!
//! Resolves placeholder call targets (`file_path::fn_name`) to real function node IDs
//! (`file_path#line:col`). Tries same-file first, then global `fn_name` lookup for cross-file calls.
//!
//! **Known limitation**: Method calls and UFCS are not resolved. Qualified paths (`mod::fn`) are
//! partially handled (see `resolve_call_target` `scoped_identifier` support).

use std::collections::HashMap;

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{query::Query, unquote_datavalue, Store};

/// Strip `pub::`, `test::`, and `bench::` prefixes from payload for canonical function name.
fn canonical_name(payload: &str) -> &str {
    let mut s = payload;
    loop {
        let prev = s;
        for prefix in &["pub::", "test::", "bench::"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                break;
            }
        }
        if s == prev {
            break;
        }
    }
    s
}

/// Resolve a single placeholder (`path_part::fn_name`) to a `NodeId`, or None if ambiguous/unresolved.
/// For qualified paths (`path_part` contains "::"), first try same-file resolution by file path only.
fn resolve_placeholder(
    path_part: &str,
    fn_name: &str,
    from_file: &str,
    local: &HashMap<(String, String), Vec<NodeId>>,
    imports_map: &HashMap<String, Vec<String>>,
    node_id_to_payload: &HashMap<String, String>,
    global_by_name: &HashMap<String, Vec<NodeId>>,
) -> Option<NodeId> {
    if path_part.contains("::") {
        let file_only = path_part.split("::").next().unwrap_or(path_part);
        let key = (file_only.to_string(), fn_name.to_string());
        if let Some(candidates) = local.get(&key) {
            if let [one] = candidates.as_slice() {
                return Some(one.clone());
            }
        }
    }
    let key = (path_part.to_string(), fn_name.to_string());
    if let Some(candidates) = local.get(&key) {
        if let [one] = candidates.as_slice() {
            return Some(one.clone());
        }
        if candidates.len() > 1 {
            eprintln!(
                "warning: duplicate function name in same file '{fn_name}', dropping call edge (ambiguous)"
            );
        }
        return None;
    }
    if let Some(imported) = imports_map.get(from_file) {
        let mut candidate: Option<NodeId> = None;
        for to_id in imported {
            if to_id.contains('#') {
                if let Some(payload) = node_id_to_payload.get(to_id) {
                    if canonical_name(payload) == fn_name {
                        if candidate.is_some() {
                            return None;
                        }
                        candidate = Some(NodeId(to_id.clone()));
                    }
                }
            } else {
                let in_file: Vec<_> = global_by_name
                    .get(fn_name)
                    .map(|v| {
                        v.iter()
                            .filter(|n| n.as_str().starts_with(&format!("{to_id}#")))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if in_file.len() == 1 {
                    if candidate.is_some() {
                        return None;
                    }
                    candidate = in_file.into_iter().next();
                }
            }
        }
        return candidate;
    }
    match global_by_name.get(fn_name).map(Vec::as_slice) {
        Some([one]) => Some(one.clone()),
        Some([]) | None => None,
        Some(candidates) => {
            eprintln!(
                "warning: cross-file call to '{}' is ambiguous ({} candidates), dropping edge",
                fn_name,
                candidates.len()
            );
            None
        }
    }
}

/// Build or enrich call graph by resolving AST placeholder call targets to real function nodes.
///
/// Same-file placeholders are resolved first; then Imports edges (from `resolve_modules`) are used
/// to prefer imported targets; then a global `fn_name` -> candidates map for cross-file resolution.
///
/// # Errors
/// Fails if the graph query or update fails.
pub fn build_call_graph(store: &Store) -> Result<()> {
    let function_type = NodeType::Function.to_string();
    let calls_type = EdgeType::Calls;
    let calls_type_str = calls_type.to_string();
    let imports_type_str = EdgeType::Imports.to_string();

    let nodes = Query::all_nodes(store)?;
    let mut local: HashMap<(String, String), Vec<NodeId>> = HashMap::new();
    let mut global_by_name: HashMap<String, Vec<NodeId>> = HashMap::new();
    let mut node_id_to_payload: HashMap<String, String> = HashMap::new();

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
        node_id_to_payload.insert(id_trim.clone(), payload.clone());
        let file_path = id_trim.split('#').next().unwrap_or(&id_trim).to_string();
        let node_id = NodeId(id_trim.clone());
        local
            .entry((file_path.clone(), payload.clone()))
            .or_default()
            .push(node_id.clone());
        let name = canonical_name(&payload).to_string();
        global_by_name.entry(name).or_default().push(node_id);
    }

    let mut imports_map: HashMap<String, Vec<String>> = HashMap::new();
    let edges = Query::all_edges(store)?;
    for row in &edges.rows {
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != imports_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_str = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let from_file = from_str.split('#').next().unwrap_or(&from_str).to_string();
        imports_map.entry(from_file).or_default().push(to_str);
    }

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
        let Some((path_part, fn_name_maybe_path)) = to_str.split_once("::") else {
            continue;
        };
        // Qualified call like file::mod::foo: use last segment as fn name for resolution.
        let (path_part, fn_name) = if fn_name_maybe_path.contains("::") {
            fn_name_maybe_path
                .rsplit_once("::")
                .map(|(path_suffix, name)| (format!("{path_part}::{path_suffix}"), name))
                .unwrap_or((path_part.to_string(), fn_name_maybe_path))
        } else {
            (path_part.to_string(), fn_name_maybe_path)
        };
        let from_file = from_str.split('#').next().unwrap_or(&from_str);
        let resolved_id = resolve_placeholder(
            &path_part,
            fn_name,
            from_file,
            &local,
            &imports_map,
            &node_id_to_payload,
            &global_by_name,
        );
        let from_id = NodeId(from_str.clone());
        let placeholder_to_id = NodeId(to_str.clone());
        store.remove_edge(&from_id, &placeholder_to_id, &calls_type)?;
        if let Some(id) = resolved_id {
            store.put_edge(&from_id, &id, &calls_type)?;
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
