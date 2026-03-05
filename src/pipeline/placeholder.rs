//! Shared logic for resolving placeholder edges (`from` → `file::Name`) to concrete node IDs.
//!
//! Used by trait impl, type reference, and macro expansion resolvers.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{query::Query, unquote_datavalue, Store};

/// Resolve placeholder edges of the given type to concrete target node IDs.
///
/// Indexes nodes whose type is in `target_node_types`, then for each edge of `edge_type`
/// whose target is a placeholder (no `#` in the target id), resolves it via same-file,
/// import-based, then global name lookup. Unresolved placeholders are removed.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_placeholder_edges(
    store: &Store,
    edge_type: &EdgeType,
    target_node_types: &[NodeType],
) -> Result<()> {
    let type_set: HashSet<String> = target_node_types
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let edge_type_str = edge_type.to_string();
    let imports_type_str = EdgeType::Imports.to_string();

    let nodes = Query::all_nodes(store)?;
    let mut local: HashMap<(String, String), Vec<NodeId>> = HashMap::new();
    let mut global_by_name: HashMap<String, Vec<NodeId>> = HashMap::new();
    for row in &nodes.rows {
        let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if !type_set.contains(&type_val) {
            continue;
        }
        let id_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let payload = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if payload.is_empty() {
            continue;
        }
        let node_id = NodeId::new(id_str.clone());
        let file_path = id_str.split('#').next().unwrap_or(&id_str).to_string();
        local
            .entry((file_path.clone(), payload.clone()))
            .or_default()
            .push(node_id.clone());
        global_by_name.entry(payload).or_default().push(node_id);
    }

    let mut imports_map: HashMap<String, Vec<String>> = HashMap::new();
    let edges = Query::all_edges(store)?;
    for row in &edges.rows {
        let et = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if et != imports_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_str = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let from_file = from_str.split('#').next().unwrap_or(&from_str).to_string();
        imports_map.entry(from_file).or_default().push(to_str);
    }

    for row in &edges.rows {
        let et = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if et != edge_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_str = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if to_str.contains('#') {
            continue;
        }
        let (path_part, name) = match to_str.split_once("::") {
            Some((pp, n)) => (pp.to_string(), n.to_string()),
            None => continue,
        };
        let from_file = from_str.split('#').next().unwrap_or(&from_str);
        let resolved = resolve_placeholder(
            &path_part,
            &name,
            from_file,
            &local,
            &imports_map,
            &global_by_name,
        );
        let from_id = NodeId::new(from_str.clone());
        let placeholder_to = NodeId::new(to_str.clone());
        store.remove_edge(&from_id, &placeholder_to, edge_type)?;
        if let Some(target_id) = resolved {
            store.put_edge(&from_id, &target_id, edge_type)?;
        }
    }
    Ok(())
}

fn resolve_placeholder(
    path_part: &str,
    name: &str,
    from_file: &str,
    local: &HashMap<(String, String), Vec<NodeId>>,
    imports_map: &HashMap<String, Vec<String>>,
    global_by_name: &HashMap<String, Vec<NodeId>>,
) -> Option<NodeId> {
    if path_part.contains("::") {
        let file_only = path_part.split("::").next().unwrap_or(path_part);
        let key = (file_only.to_string(), name.to_string());
        if let Some(candidates) = local.get(&key) {
            if let [one] = candidates.as_slice() {
                return Some(one.clone());
            }
        }
        return None;
    }
    let key = (path_part.to_string(), name.to_string());
    if let Some(candidates) = local.get(&key) {
        if let [one] = candidates.as_slice() {
            return Some(one.clone());
        }
        return None;
    }
    if let Some(imported) = imports_map.get(from_file) {
        for to_id in imported {
            let in_file: Vec<NodeId> = global_by_name
                .get(name)
                .map(|v| {
                    v.iter()
                        .filter(|n| {
                            if to_id.contains('#') {
                                n.as_str() == to_id
                            } else {
                                n.as_str().starts_with(&format!("{to_id}#"))
                            }
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if in_file.len() == 1 {
                return in_file.into_iter().next();
            }
        }
    }
    global_by_name.get(name).and_then(|v| {
        if v.len() == 1 {
            v.first().cloned()
        } else {
            None
        }
    })
}
