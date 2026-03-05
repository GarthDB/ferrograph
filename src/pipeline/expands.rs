//! Resolve placeholder `ExpandsTo` edges (invocation site → macro definition) from AST.

use std::collections::HashMap;

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{query::Query, unquote_datavalue, Store};

/// Resolve placeholder `ExpandsTo` edges (`from_id` → `file::macro_name`) to concrete macro definition node IDs.
/// Uses same-file and import-based resolution like call graph.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_expands_to_edges(store: &Store) -> Result<()> {
    let macro_type = NodeType::Macro.to_string();
    let edge_type = EdgeType::ExpandsTo;
    let edge_type_str = edge_type.to_string();

    let nodes = Query::all_nodes(store)?;
    let mut local: HashMap<(String, String), Vec<NodeId>> = HashMap::new();
    let mut global_by_name: HashMap<String, Vec<NodeId>> = HashMap::new();
    for row in &nodes.rows {
        let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if type_val != macro_type {
            continue;
        }
        let id_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let payload = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if payload.is_empty() {
            continue;
        }
        let node_id = NodeId(id_str.clone());
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
        if et != EdgeType::Imports.to_string() {
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
        let (path_part, macro_name) = match to_str.split_once("::") {
            Some((pp, name)) => (pp.to_string(), name.to_string()),
            None => continue,
        };
        let from_file = from_str.split('#').next().unwrap_or(&from_str);
        let resolved = resolve_macro_placeholder(
            &path_part,
            &macro_name,
            from_file,
            &local,
            &imports_map,
            &global_by_name,
        );
        let from_id = NodeId(from_str.clone());
        let placeholder_to = NodeId(to_str.clone());
        store.remove_edge(&from_id, &placeholder_to, &edge_type)?;
        if let Some(macro_id) = resolved {
            store.put_edge(&from_id, &macro_id, &edge_type)?;
        }
    }
    Ok(())
}

fn resolve_macro_placeholder(
    path_part: &str,
    macro_name: &str,
    from_file: &str,
    local: &HashMap<(String, String), Vec<NodeId>>,
    imports_map: &HashMap<String, Vec<String>>,
    global_by_name: &HashMap<String, Vec<NodeId>>,
) -> Option<NodeId> {
    if path_part.contains("::") {
        let file_only = path_part.split("::").next().unwrap_or(path_part);
        let key = (file_only.to_string(), macro_name.to_string());
        if let Some(candidates) = local.get(&key) {
            if let [one] = candidates.as_slice() {
                return Some(one.clone());
            }
        }
        return None;
    }
    let key = (path_part.to_string(), macro_name.to_string());
    if let Some(candidates) = local.get(&key) {
        if let [one] = candidates.as_slice() {
            return Some(one.clone());
        }
        return None;
    }
    if let Some(imported) = imports_map.get(from_file) {
        for to_id in imported {
            let in_file: Vec<NodeId> = global_by_name
                .get(macro_name)
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
    global_by_name.get(macro_name).and_then(|v| {
        if v.len() == 1 {
            v.first().cloned()
        } else {
            None
        }
    })
}
