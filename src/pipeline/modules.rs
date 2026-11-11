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

/// Directory of the crate owning `path`: the nearest ancestor (within `root`) holding a `Cargo.toml`.
fn crate_dir(root: &Path, path: &Path) -> std::path::PathBuf {
    path.ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root))
        .find(|d| d.join("Cargo.toml").is_file())
        .unwrap_or(root)
        .to_path_buf()
}

/// Crate (directory relative to `root`) and module path within it for a file under `root`
/// (e.g. `crate::utils` for `src/utils.rs`).
fn file_to_module_path(root: &Path, path: &Path) -> Option<(String, String)> {
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    let dir = crate_dir(&root, &path);
    let krate = dir.strip_prefix(&root).ok()?.to_string_lossy().into_owned();
    let rel = path.strip_prefix(&dir).ok()?;
    let s = rel.to_string_lossy();
    let s = s
        .strip_prefix("src/")
        .or_else(|| s.strip_prefix("src\\"))
        .unwrap_or(&s);
    let s = s.strip_suffix(".rs").unwrap_or(s);
    if s.is_empty() || s == "lib" || s == "main" {
        return Some((krate, "crate".to_string()));
    }
    let s = s.replace(std::path::MAIN_SEPARATOR, "::");
    let s = s.strip_suffix("::mod").unwrap_or(&s);
    Some((krate, format!("crate::{s}")))
}

/// Extract the use tree text from a `use_declaration` node, without `pub`/`pub(..)` and `use`.
fn use_path_from_node(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let arg = node.child_by_field_name("argument")?;
    let s = source.get(arg.byte_range())?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Expand brace groups into one path per leaf: `a::{b, c::d}` -> `a::b`, `a::c::d`.
/// A `self` leaf stays as `a::self`; renames and globs are kept (`a::b as c`, `a::*`).
fn expand_use(path: &str) -> Vec<String> {
    let path = path.trim();
    let Some(open) = path.find('{') else {
        return vec![path.to_string()];
    };
    let prefix = path[..open].trim_end_matches("::");
    let close = path.rfind('}').unwrap_or(path.len());
    let inner = &path[open + 1..close.max(open + 1)];
    let (mut depth, mut start) = (0usize, 0usize);
    let mut parts = Vec::new();
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .flat_map(expand_use)
        .map(|leaf| {
            if prefix.is_empty() {
                leaf
            } else {
                format!("{prefix}::{leaf}")
            }
        })
        .collect()
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
            out.extend(expand_use(&path));
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

fn module_file<'a>(maps: &'a ModuleMaps, krate: &str, module: &str) -> Option<&'a String> {
    maps.module_path_to_file_id
        .get(&(krate.to_string(), module.to_string()))
}

fn join_mod(base: &str, segments: &[&str]) -> String {
    segments
        .iter()
        .fold(base.to_string(), |acc, s| acc + "::" + *s)
}

/// Resolve one `use` leaf (e.g. `crate::utils::add`) relative to `current_module` of `krate`.
/// Returns (`target_file_id`, `target_node_id_opt`). If the path refers to a module only, `node_id` is None.
fn resolve_use_path(
    use_path: &str,
    krate: &str,
    current_module: &str,
    maps: &ModuleMaps,
    store: &Store,
    depth: usize,
) -> Option<(String, Option<String>)> {
    let path = use_path.split(" as ").next()?.trim();
    let path = path.strip_prefix("::").unwrap_or(path);
    let path = path
        .strip_suffix("::*")
        .or_else(|| path.strip_suffix("::self"))
        .unwrap_or(path);
    let segments: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    let parent = |m: &str| {
        m.rsplit_once("::")
            .map_or_else(|| "crate".to_string(), |(p, _)| p.to_string())
    };
    let (base, rest): (String, &[&str]) = match *segments.first()? {
        "crate" => ("crate".to_string(), &segments[1..]),
        "self" => (current_module.to_string(), &segments[1..]),
        "super" => {
            let n = segments.iter().take_while(|s| **s == "super").count();
            let base = (0..n).fold(current_module.to_string(), |m, _| parent(&m));
            (base, &segments[n..])
        }
        first if module_file(maps, krate, &join_mod(current_module, &[first])).is_some() => {
            (current_module.to_string(), &segments[..])
        }
        _ => return None,
    };
    let Some((item, mods)) = rest.split_last() else {
        return module_file(maps, krate, &base).map(|f| (f.clone(), None));
    };
    if let Some(f) = module_file(maps, krate, &join_mod(&base, rest)) {
        return Some((f.clone(), None));
    }
    let module = join_mod(&base, mods);
    let file_id = module_file(maps, krate, &module)?.clone();
    let node = find_item_in_file(store, &file_id, item)
        .or_else(|| follow_reexport(&file_id, item, krate, &module, maps, store, depth));
    Some((file_id, node))
}

/// Find `item` re-exported by `file_id` (`pub use self::x::item;` or a glob), following re-export chains.
fn follow_reexport(
    file_id: &str,
    item: &str,
    krate: &str,
    module: &str,
    maps: &ModuleMaps,
    store: &Store,
    depth: usize,
) -> Option<String> {
    if depth >= 4 {
        return None;
    }
    for leaf in maps.file_uses.get(file_id)? {
        if leaf.ends_with("::*") {
            if let Some((f, None)) = resolve_use_path(leaf, krate, module, maps, store, depth + 1) {
                if let Some(node) = find_item_in_file(store, &f, item) {
                    return Some(node);
                }
            }
        } else if use_leaves(leaf)
            .iter()
            .any(|(n, o)| n == item && o.is_none())
        {
            if let Some((_, Some(node))) =
                resolve_use_path(leaf, krate, module, maps, store, depth + 1)
            {
                return Some(node);
            }
        }
    }
    None
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
    /// File -> (crate directory, module path within the crate).
    file_id_to_module_path: BTreeMap<String, (String, String)>,
    /// (crate directory, module path) -> file.
    module_path_to_file_id: BTreeMap<(String, String), String>,
    /// File -> its `use` leaves, brace groups expanded.
    file_uses: BTreeMap<String, Vec<String>>,
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
    let mut file_uses = BTreeMap::new();

    for file_id in &file_id_to_path {
        let path = root.join(file_id.as_str());
        if let Some((krate, module_path)) = file_to_module_path(root, &path) {
            file_id_to_module_path.insert(file_id.clone(), (krate.clone(), module_path.clone()));
            if let Ok(content) = std::fs::read_to_string(&path) {
                file_uses.insert(file_id.clone(), collect_use_paths(&content));
            }
            let key = (krate, module_path);
            let prefer = key.1 == "crate"
                && module_path_to_file_id
                    .get(&key)
                    .is_none_or(|existing: &String| {
                        !existing.contains("lib.rs") && file_id.contains("lib.rs")
                    });
            if prefer || key.1 != "crate" {
                module_path_to_file_id.insert(key, file_id.clone());
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
            let (krate, current) = file_id_to_module_path
                .get(&from_id)
                .cloned()
                .unwrap_or_default();
            let sub_path = if current == "crate" {
                format!("crate::{mod_name}")
            } else {
                format!("{current}::{mod_name}")
            };
            if let Some(target_path) = resolve_module_to_file(&sub_path, &root.join(&krate)) {
                let target_canon = target_path.canonicalize().ok();
                if let Some(sub_file_id) = file_id_to_path
                    .iter()
                    .find(|fid| root.join(fid.as_str()).canonicalize().ok() == target_canon)
                {
                    module_path_to_file_id.insert((krate, sub_path), sub_file_id.clone());
                }
            }
        }
    }

    ModuleMaps {
        file_id_to_module_path,
        module_path_to_file_id,
        file_uses,
    }
}

/// Prefix of `Imports` targets that name an item imported from another crate (`std`, `serde_json`, ..).
pub(crate) const EXTERNAL_IMPORT_PREFIX: &str = "external::";
/// Prefix of `Imports` targets that record a rename: `alias::h=>helper` for `use a::helper as h`.
pub(crate) const ALIAS_IMPORT_PREFIX: &str = "alias::";

/// A `use` leaf that did not resolve locally and does not start with `crate`/`self`/`super` or a local module.
fn is_external_use(use_path: &str, krate: &str, current_module: &str, maps: &ModuleMaps) -> bool {
    let first = use_path
        .trim()
        .trim_start_matches("::")
        .split("::")
        .next()
        .unwrap_or("");
    if first.is_empty() || matches!(first, "crate" | "self" | "super") {
        return false;
    }
    let local = |base: &str| module_file(maps, krate, &format!("{base}::{first}")).is_some();
    !(local("crate") || local(current_module))
}

/// Names a `use` path brings into scope, as (name, original name when renamed with `as`): the last
/// segment of each leaf, with brace groups expanded. Globs and `self` leaves are skipped.
fn use_leaves(path: &str) -> Vec<(String, Option<String>)> {
    let path = path.trim();
    if let Some(open) = path.find('{') {
        let close = path.rfind('}').unwrap_or(path.len());
        let inner = &path[open + 1..close.max(open + 1)];
        let (mut depth, mut start, mut out) = (0usize, 0usize, Vec::new());
        for (i, c) in inner.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    out.extend(use_leaves(&inner[start..i]));
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.extend(use_leaves(&inner[start..]));
        return out;
    }
    let last = |p: &str| p.rsplit("::").next().unwrap_or(p).trim().to_string();
    let (name, orig) = match path.rsplit_once(" as ") {
        Some((before, alias)) => (alias.trim().to_string(), Some(last(before))),
        None => (last(path), None),
    };
    if name.is_empty() || name == "*" || name == "self" || name == "_" {
        Vec::new()
    } else {
        vec![(name, orig)]
    }
}

fn collect_imports_to_add(maps: &ModuleMaps, store: &Store) -> Vec<(NodeId, NodeId, EdgeType)> {
    let mut out = Vec::new();
    for (file_id, leaves) in &maps.file_uses {
        let Some((krate, module_path)) = maps.file_id_to_module_path.get(file_id) else {
            continue;
        };
        let from = NodeId::new(file_id.clone());
        for leaf in leaves {
            // Glob imports stay unresolved here (they only feed `follow_reexport`).
            if leaf.ends_with("::*") {
                continue;
            }
            let mut external = false;
            if let Some((target_file_id, target_node_opt)) =
                resolve_use_path(leaf, krate, module_path, maps, store, 0)
            {
                let to = NodeId::new(target_node_opt.unwrap_or(target_file_id));
                out.push((from.clone(), to, EdgeType::Imports));
            } else if is_external_use(leaf, krate, module_path, maps) {
                external = true;
            }
            for (name, orig) in use_leaves(leaf) {
                if external {
                    // Remember which names come from other crates so bare calls to them are not
                    // matched against same-named local functions (#112).
                    out.push((
                        from.clone(),
                        NodeId::new(format!("{EXTERNAL_IMPORT_PREFIX}{name}")),
                        EdgeType::Imports,
                    ));
                } else if let Some(orig) = orig {
                    out.push((
                        from.clone(),
                        NodeId::new(format!("{ALIAS_IMPORT_PREFIX}{name}=>{orig}")),
                        EdgeType::Imports,
                    ));
                }
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
    let imports_to_add = collect_imports_to_add(&maps, store);

    for (from, to, et) in imports_to_add {
        store.put_edge(&from, &to, &et)?;
    }

    Ok(())
}

fn resolve_module_to_file(module_path: &str, crate_dir: &Path) -> Option<std::path::PathBuf> {
    let rest = module_path.strip_prefix("crate::").unwrap_or(module_path);
    let src = crate_dir.join("src");
    if rest.is_empty() {
        return ["lib.rs", "main.rs"]
            .iter()
            .map(|f| src.join(f))
            .find(|p| p.exists());
    }
    let rel = rest.replace("::", std::path::MAIN_SEPARATOR_STR);
    let rs = src.join(format!("{rel}.rs"));
    if rs.exists() {
        return Some(rs);
    }
    let mod_rs = src.join(rel).join("mod.rs");
    mod_rs.exists().then_some(mod_rs)
}

#[cfg(test)]
mod leaf_tests {
    use super::use_leaves;

    #[test]
    fn expands_brace_groups() {
        assert_eq!(
            super::expand_use("a::{b, c::{d as e, self}, *}"),
            ["a::b", "a::c::d as e", "a::c::self", "a::*"]
        );
        assert_eq!(super::expand_use("{x::y, z}"), ["x::y", "z"]);
        assert_eq!(super::expand_use("a::b"), ["a::b"]);
    }

    #[test]
    fn leaves_of_use_paths() {
        let names = |p: &str| -> Vec<String> { use_leaves(p).into_iter().map(|l| l.0).collect() };
        assert_eq!(names("serde_json::from_str"), ["from_str"]);
        assert_eq!(
            names("std::{fs::read, io::{self, Write}, path::*}"),
            ["read", "Write"]
        );
        assert_eq!(
            use_leaves("a::{b as c, d}"),
            [
                ("c".to_string(), Some("b".to_string())),
                ("d".to_string(), None)
            ]
        );
    }
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
