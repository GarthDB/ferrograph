//! Phase 3: module and import resolution.
//!
//! Builds a module tree from `mod` declarations, resolves `use` paths,
//! and creates Imports edges for module-aware call resolution.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use cozo::NamedRows;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::{unquote_datavalue, Store};

/// Compute the canonical module path for a file under `root` (e.g. `crate::utils` for `src/utils.rs`).
fn file_to_module_path(root: &Path, path: &Path) -> Option<String> {
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    let rel = path.strip_prefix(&root).ok()?;
    let s = rel.to_string_lossy();
    let s = s
        .strip_prefix("src/")
        .or_else(|| s.strip_prefix("src\\"))
        .unwrap_or(&s);
    let s = s.strip_suffix(".rs").unwrap_or(s);
    if s.is_empty() || s == "lib" || s == "main" {
        return Some("crate".to_string());
    }
    let s = s.replace(std::path::MAIN_SEPARATOR, "::");
    let s = s.strip_suffix("::mod").unwrap_or(&s);
    Some(format!("crate::{s}"))
}

/// Extract use path text from a `use_declaration` node (e.g. `crate::utils::add`).
fn use_path_from_node(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let r = node.byte_range();
    let full = source.get(r.start..r.end)?;
    let s = full
        .strip_prefix("use ")
        .unwrap_or(full)
        .trim_end_matches(';')
        .trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Collect all use path strings from Rust source.
fn collect_use_paths(content: &str) -> Vec<String> {
    let mut parser = Parser::new();
    let Ok(()) = parser.set_language(&LANGUAGE.into()) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    collect_use_paths_rec(&mut cursor, content, &mut out);
    out
}

fn collect_use_paths_rec(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    out: &mut Vec<String>,
) {
    let node = cursor.node();
    if node.kind() == "use_declaration" {
        if let Some(path) = use_path_from_node(&node, source) {
            out.push(path);
        }
    }
    if cursor.goto_first_child() {
        loop {
            collect_use_paths_rec(cursor, source, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Resolve `use_path` (e.g. `crate::utils::add`) relative to `current_module_path`.
/// Returns (`target_file_id`, `target_node_id_opt`). If the path refers to a module only, `node_id` is None.
fn resolve_use_path(
    use_path: &str,
    current_module_path: &str,
    module_path_to_file: &BTreeMap<String, String>,
    store: &Store,
) -> Option<(String, Option<String>)> {
    let path = use_path.trim();
    if path.is_empty() {
        return None;
    }
    let segments: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }

    let (module_path, item_name) = if segments[0] == "crate" {
        if segments.len() == 1 {
            ("crate".to_string(), None)
        } else if segments.len() == 2 {
            (format!("crate::{}", segments[1]), None)
        } else {
            let mod_path = segments[..segments.len() - 1].join("::");
            (
                format!("crate::{mod_path}"),
                Some(segments[segments.len() - 1]),
            )
        }
    } else if segments[0] == "self" {
        if segments.len() == 1 {
            (current_module_path.to_string(), None)
        } else if segments.len() == 2 {
            let full = if current_module_path == "crate" {
                format!("crate::{}", segments[1])
            } else {
                format!("{current_module_path}::{}", segments[1])
            };
            (full, None)
        } else {
            let mod_path = segments[..segments.len() - 1].join("::");
            let full_mod = if current_module_path == "crate" {
                format!("crate::{mod_path}")
            } else {
                format!("{current_module_path}::{mod_path}")
            };
            (full_mod, Some(segments[segments.len() - 1]))
        }
    } else if segments[0] == "super" {
        let parent = current_module_path
            .rsplit_once("::")
            .map_or_else(|| "crate".to_string(), |(p, _)| p.to_string());
        if segments.len() == 1 {
            (parent, None)
        } else if segments.len() == 2 {
            let full = if parent == "crate" {
                format!("crate::{}", segments[1])
            } else {
                format!("{parent}::{}", segments[1])
            };
            (full, None)
        } else {
            let mod_path = segments[1..segments.len() - 1].join("::");
            let full_mod = if parent == "crate" {
                format!("crate::{mod_path}")
            } else {
                format!("{parent}::{mod_path}")
            };
            (full_mod, Some(segments[segments.len() - 1]))
        }
    } else {
        return None;
    };

    let file_id = module_path_to_file.get(&module_path)?.clone();

    if let Some(name) = item_name {
        if let Some(node_id) = find_item_in_file(store, &file_id, name) {
            return Some((file_id, Some(node_id)));
        }
    }
    Some((file_id, None))
}

/// Find a node that is contained in `file_id` (via Contains) and has payload equal to `name` or `pub::{name}`.
fn find_item_in_file(store: &Store, file_id: &str, name: &str) -> Option<String> {
    let edge_contains = EdgeType::Contains.to_string();
    let mut params = BTreeMap::new();
    params.insert("file_id".to_string(), cozo::DataValue::from(file_id));
    let script = format!(
        r#"
        reachable[to_id] := *edges[from_id, to_id, edge_type], from_id = $file_id, edge_type = "{edge_contains}"
        reachable[to_id] := reachable[from_id], *edges[from_id, to_id, edge_type], edge_type = "{edge_contains}"
        ?[id, payload] := reachable[id], *nodes[id, type, payload], id != $file_id
        "#
    );
    let rows = store.run_query(script.trim(), params).ok()?;
    for row in &rows.rows {
        let id = row.first().map(unquote_datavalue)?;
        let payload = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let canonical = payload.strip_prefix("pub::").unwrap_or(&payload);
        if canonical == name {
            return Some(id);
        }
    }
    None
}

struct ModuleMaps {
    file_id_to_path: Vec<String>,
    file_id_to_module_path: BTreeMap<String, String>,
    module_path_to_file_id: BTreeMap<String, String>,
}

fn build_module_maps(root: &Path, nodes: &NamedRows, edges: &NamedRows) -> ModuleMaps {
    let type_file = NodeType::File.to_string();
    let type_module = NodeType::Module.to_string();
    let edge_contains = EdgeType::Contains.to_string();

    let file_id_to_path: Vec<String> = nodes
        .rows
        .iter()
        .filter_map(|row| {
            let ty = row.get(1).map(unquote_datavalue).unwrap_or_default();
            if ty == type_file {
                row.first().map(unquote_datavalue)
            } else {
                None
            }
        })
        .collect();

    let mut file_id_to_module_path = BTreeMap::new();
    let mut module_path_to_file_id = BTreeMap::new();

    for file_id in &file_id_to_path {
        let path = root.join(file_id.as_str());
        if let Some(module_path) = file_to_module_path(root, &path) {
            file_id_to_module_path.insert(file_id.clone(), module_path.clone());
            let prefer = module_path == "crate"
                && module_path_to_file_id
                    .get(&module_path)
                    .is_none_or(|existing: &String| {
                        !existing.contains("lib.rs") && file_id.contains("lib.rs")
                    });
            if prefer || module_path != "crate" {
                module_path_to_file_id.insert(module_path, file_id.clone());
            }
        }
    }

    for row in &edges.rows {
        let from_id = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_id = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != edge_contains || !file_id_to_module_path.contains_key(&from_id) {
            continue;
        }
        let to_type = nodes
            .rows
            .iter()
            .find(|r| r.first().map(unquote_datavalue).as_deref() == Some(to_id.as_str()))
            .and_then(|r| r.get(1).map(unquote_datavalue));
        if to_type.as_deref() != Some(type_module.as_str()) {
            continue;
        }
        let to_payload = nodes
            .rows
            .iter()
            .find(|r| r.first().map(unquote_datavalue).as_deref() == Some(to_id.as_str()))
            .and_then(|r| r.get(2).map(unquote_datavalue));
        if let Some(ref mod_name) = to_payload {
            let current = file_id_to_module_path
                .get(&from_id)
                .cloned()
                .unwrap_or_default();
            let sub_path = if current == "crate" {
                format!("crate::{mod_name}")
            } else {
                format!("{current}::{mod_name}")
            };
            if let Some(target_path) = resolve_module_to_file(&sub_path, root) {
                let target_canon = target_path.canonicalize().ok();
                if let Some(sub_file_id) = file_id_to_path
                    .iter()
                    .find(|fid| root.join(fid.as_str()).canonicalize().ok() == target_canon)
                {
                    module_path_to_file_id.insert(sub_path, sub_file_id.clone());
                }
            }
        }
    }

    ModuleMaps {
        file_id_to_path,
        file_id_to_module_path,
        module_path_to_file_id,
    }
}

fn collect_imports_to_add(
    maps: &ModuleMaps,
    store: &Store,
    root: &Path,
) -> Vec<(NodeId, NodeId, EdgeType)> {
    let mut out = Vec::new();
    for file_id in &maps.file_id_to_path {
        let Some(module_path) = maps.file_id_to_module_path.get(file_id) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(root.join(file_id.as_str())) else {
            continue;
        };
        for use_path in collect_use_paths(&content) {
            if let Some((target_file_id, target_node_opt)) =
                resolve_use_path(&use_path, module_path, &maps.module_path_to_file_id, store)
            {
                let from = NodeId::new(file_id.clone());
                let to = match &target_node_opt {
                    Some(node_id) => NodeId::new(node_id.clone()),
                    None => NodeId::new(target_file_id),
                };
                out.push((from, to, EdgeType::Imports));
            }
        }
    }
    out
}

/// Resolve `mod` and `use` statements into graph edges.
///
/// Builds a module path -> file map from File and Module nodes, then for each file
/// parses source to find `use` declarations and creates Imports edges to resolved targets.
///
/// # Errors
/// Fails if module resolution or store writes fail.
pub fn resolve_modules(store: &Store, root: &Path) -> Result<()> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let nodes = store.run_query(
        "?[id, type, payload] := *nodes[id, type, payload]",
        BTreeMap::new(),
    )?;
    let edges = store.run_query(
        "?[from_id, to_id, edge_type] := *edges[from_id, to_id, edge_type]",
        BTreeMap::new(),
    )?;

    let maps = build_module_maps(&root, &nodes, &edges);
    let imports_to_add = collect_imports_to_add(&maps, store, &root);

    for (from, to, et) in imports_to_add {
        store.put_edge(&from, &to, &et)?;
    }

    Ok(())
}

fn resolve_module_to_file(module_path: &str, root: &Path) -> Option<std::path::PathBuf> {
    let rest = module_path.strip_prefix("crate::").unwrap_or(module_path);
    if rest.is_empty() {
        let lib = root.join("src").join("lib.rs");
        let main = root.join("src").join("main.rs");
        if lib.exists() {
            return Some(lib);
        }
        if main.exists() {
            return Some(main);
        }
        return None;
    }
    let rel = rest.replace("::", std::path::MAIN_SEPARATOR_STR);
    let rs = root.join("src").join(format!("{rel}.rs"));
    if rs.exists() {
        return Some(rs);
    }
    let mod_rs = root.join("src").join(rel).join("mod.rs");
    if mod_rs.exists() {
        return Some(mod_rs);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_modules_empty_store_succeeds() {
        let store = Store::new_memory().unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(resolve_modules(&store, root).is_ok());
    }

    #[test]
    fn resolve_modules_creates_imports_edge() {
        let store = Store::new_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "mod utils;\nuse crate::utils;\npub fn greet() {}",
        )
        .unwrap();
        std::fs::write(src.join("utils.rs"), "pub fn add() {}").unwrap();
        let files = crate::pipeline::discovery::discover_files(root).unwrap();
        for (path, content) in &files {
            crate::pipeline::ast::extract_ast(&store, path, content, root).unwrap();
        }
        resolve_modules(&store, root).unwrap();
        let edges = crate::graph::Query::all_edges(&store).unwrap();
        let imports_count = edges
            .rows
            .iter()
            .filter(|r| r.get(2).map(crate::graph::unquote_datavalue).as_deref() == Some("imports"))
            .count();
        assert!(
            imports_count >= 1,
            "expected at least one Imports edge, got {imports_count}"
        );
    }
}
