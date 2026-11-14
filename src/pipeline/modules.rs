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

/// True when every compilation unit that includes `path` is a bin, test, example, bench or build-script unit,
/// so a bare `pub` item in it is invisible outside its own binary. Decided from Cargo's target layout, not
/// from `mod` declarations: `tests/`, `examples/`, `benches/`, `build.rs`, `src/main.rs` and `src/bin/` are
/// always bin-only; any other file of a package is bin-only when the package has no library target
/// (no `src/lib.rs`, no `[lib]` table). Files outside any package (no `Cargo.toml` between `path` and `root`)
/// are never bin-only. Limits: a file shared into a lib through `#[path]`/`include!` from `tests/` is still
/// classified by its own location; and the `mod`s of `src/main.rs` in a package that also has a library
/// count as library files (conservative: they stay roots).
pub(crate) fn is_bin_only_file(root: &Path, path: &Path) -> bool {
    let dir = crate_dir(root, path);
    let manifest_path = dir.join("Cargo.toml");
    let Ok(rel) = path.strip_prefix(&dir) else {
        return false;
    };
    if !manifest_path.is_file() {
        return false;
    }
    let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
    if matches!(first, Some("tests" | "examples" | "benches")) || rel == Path::new("build.rs") {
        return true;
    }
    if rel == Path::new("src/main.rs") || rel.starts_with("src/bin") {
        return true;
    }
    let has_lib = dir.join("src/lib.rs").is_file()
        || std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|t| t.parse::<toml::Value>().ok())
            .is_none_or(|m| m.get("lib").is_some());
    !has_lib
}

/// Crate (directory relative to `root`) and module path within it for a file under `root`
/// (e.g. `crate::utils` for `src/utils.rs`).
fn file_to_module_path(root: &Path, path: &Path) -> Option<(String, String)> {
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    let dir = crate_dir(&root, &path);
    let krate = dir.strip_prefix(&root).ok()?.to_string_lossy().into_owned();
    let rel = path.strip_prefix(&dir).ok()?;
    let rel = custom_root_dir(&dir, rel).map_or(rel, |d| rel.strip_prefix(d).unwrap_or(rel));
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

/// Directory of a crate root declared outside `src/` (`[[bin]] path = "crates/core/main.rs"`) that
/// holds `rel`, a path relative to the crate directory: its module paths start there.
fn custom_root_dir<'a>(dir: &Path, rel: &'a Path) -> Option<&'a Path> {
    if rel.starts_with("src") {
        return None;
    }
    let manifest: toml::Value = std::fs::read_to_string(dir.join("Cargo.toml"))
        .ok()?
        .parse()
        .ok()?;
    let bins = manifest.get("bin").and_then(toml::Value::as_array);
    let root = manifest
        .get("lib")
        .into_iter()
        .chain(bins.into_iter().flatten())
        .filter_map(|t| Path::new(t.get("path")?.as_str()?).parent())
        .filter(|d| !d.as_os_str().is_empty() && !d.starts_with("src"))
        .find(|d| rel.starts_with(d))?;
    rel.ancestors().find(|a| *a == root)
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
/// Each leaf comes with the `line:col` of the innermost function enclosing the `use`, if any.
fn collect_use_paths(content: &str) -> Vec<(String, Option<String>)> {
    let mut parser = Parser::new();
    let Ok(()) = parser.set_language(&LANGUAGE.into()) else {
        return Vec::new();
    };
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    collect_use_paths_rec(&mut cursor, content, None, &mut out);
    out
}

/// `#[path = "x.rs"] mod name;` declarations as (name, path): the file is not at the default location.
fn collect_path_mods(content: &str) -> Vec<(String, String)> {
    let mut parser = Parser::new();
    if !content.contains("#[path") || parser.set_language(&LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
        if node.kind() != "mod_item" || node.child_by_field_name("body").is_some() {
            continue;
        }
        let Some(name) = node
            .child_by_field_name("name")
            .and_then(|n| content.get(n.byte_range()))
        else {
            continue;
        };
        let mut attr = node.prev_sibling();
        while let Some(a) = attr.filter(|a| a.kind() == "attribute_item") {
            let text = content.get(a.byte_range()).unwrap_or("");
            let rel = text
                .strip_prefix("#[path")
                .and_then(|t| t.split('"').nth(1));
            if let Some(rel) = rel {
                out.push((name.to_string(), rel.to_string()));
                break;
            }
            attr = a.prev_sibling();
        }
    }
    out
}

fn collect_use_paths_rec(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    scope: Option<&str>,
    out: &mut Vec<(String, Option<String>)>,
) {
    let node = cursor.node();
    if node.kind() == "use_declaration" {
        if let Some(path) = use_path_from_node(&node, source) {
            out.extend(
                expand_use(&path)
                    .into_iter()
                    .map(|leaf| (leaf, scope.map(str::to_string))),
            );
        }
    }
    let own_scope = (node.kind() == "function_item").then(|| {
        format!(
            "{}:{}",
            node.start_position().row + 1,
            node.start_position().column + 1
        )
    });
    let scope = own_scope.as_deref().or(scope);
    if cursor.goto_first_child() {
        loop {
            collect_use_paths_rec(cursor, source, scope, out);
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
/// Returns (`target_file_id`, `target_node_id_opt`, `names_module`): the node is None when the path
/// names a module (`names_module`) or an item that was not found in its module.
fn resolve_use_path(
    use_path: &str,
    krate: &str,
    current_module: &str,
    maps: &ModuleMaps,
    depth: usize,
) -> Option<(String, Option<String>, bool)> {
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
        return module_file(maps, krate, &base).map(|f| (f.clone(), None, true));
    };
    if let Some(f) = module_file(maps, krate, &join_mod(&base, rest)) {
        return Some((f.clone(), None, true));
    }
    let module = join_mod(&base, mods);
    let file_id = module_file(maps, krate, &module)?.clone();
    let node = find_item_in_file(maps, &file_id, item)
        .or_else(|| follow_reexport(&file_id, item, krate, &module, maps, depth));
    // An unfound CamelCase leaf is a type or const (e.g. re-exported from another crate), never a
    // function, so it says nothing about where a bare call goes.
    if node.is_none() && item.starts_with(char::is_uppercase) {
        return None;
    }
    Some((file_id, node, false))
}

/// Find `item` re-exported by `file_id` (`pub use self::x::item;` or a glob), following re-export chains.
fn follow_reexport(
    file_id: &str,
    item: &str,
    krate: &str,
    module: &str,
    maps: &ModuleMaps,
    depth: usize,
) -> Option<String> {
    if depth >= 4 {
        return None;
    }
    for (leaf, _) in maps.file_uses.get(file_id)? {
        if leaf.ends_with("::*") {
            if let Some((f, None, _)) = resolve_use_path(leaf, krate, module, maps, depth + 1) {
                if let Some(node) = find_item_in_file(maps, &f, item) {
                    return Some(node);
                }
            }
        } else if use_leaves(leaf)
            .iter()
            .any(|(n, o)| n == item && o.is_none())
        {
            if let Some((_, Some(node), _)) = resolve_use_path(leaf, krate, module, maps, depth + 1)
            {
                return Some(node);
            }
        }
    }
    None
}

/// Find a node that is contained in `file_id` (via Contains) and has payload equal to `name` or `pub::{name}`.
fn find_item_in_file(maps: &ModuleMaps, file_id: &str, name: &str) -> Option<String> {
    maps.items
        .get(&(file_id.to_string(), name.to_string()))
        .cloned()
}

/// (file, canonical name) -> first node contained in the file, directly or nested.
fn items_by_file(
    file_ids: &[String],
    payloads: &BTreeMap<String, (String, String)>,
    children: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<(String, String), String> {
    let mut items = BTreeMap::new();
    for file_id in file_ids {
        let mut stack = vec![file_id.as_str()];
        while let Some(id) = stack.pop() {
            for child in children.get(id).into_iter().flatten() {
                stack.push(child);
                if let Some((_, payload)) = payloads.get(child) {
                    let name = payload.strip_prefix("pub::").unwrap_or(payload);
                    items
                        .entry((file_id.clone(), name.to_string()))
                        .or_insert_with(|| child.clone());
                }
            }
        }
    }
    items
}

struct ModuleMaps {
    /// File -> (crate directory, module path within the crate).
    file_id_to_module_path: BTreeMap<String, (String, String)>,
    /// (crate directory, module path) -> file.
    module_path_to_file_id: BTreeMap<(String, String), String>,
    /// (file, item name) -> node contained in the file.
    items: BTreeMap<(String, String), String>,
    /// File -> its `use` leaves, brace groups expanded.
    file_uses: BTreeMap<String, Vec<(String, Option<String>)>>,
    /// (declaring file, module name, file) of `#[path = ".."] mod name;` declarations.
    path_mods: Vec<(String, String, String)>,
}

fn canonical_files<'a>(
    root: &Path,
    file_ids: &'a [String],
) -> BTreeMap<std::path::PathBuf, &'a String> {
    file_ids
        .iter()
        .filter_map(|f| Some((root.join(f).canonicalize().ok()?, f)))
        .collect()
}

/// Re-home files declared with `#[path = ".."] mod name;` under the module that declares them.
fn apply_path_mods(
    root: &Path,
    file_ids: &[String],
    file_to_module: &mut BTreeMap<String, (String, String)>,
    module_to_file: &mut BTreeMap<(String, String), String>,
) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    let by_canonical = canonical_files(root, file_ids);
    for file_id in file_ids {
        let Some((krate, module)) = file_to_module.get(file_id).cloned() else {
            continue;
        };
        let path = root.join(file_id);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, rel) in collect_path_mods(&content) {
            let target = path
                .parent()
                .and_then(|d| d.join(rel).canonicalize().ok())
                .and_then(|c| by_canonical.get(&c).copied());
            if let Some(target) = target {
                let sub = join_mod(&module, &[&name]);
                file_to_module.insert(target.clone(), (krate.clone(), sub.clone()));
                module_to_file.insert((krate.clone(), sub), target.clone());
                found.push((file_id.clone(), name, target.clone()));
            }
        }
    }
    found
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

    let path_mods = apply_path_mods(
        root,
        &file_id_to_path,
        &mut file_id_to_module_path,
        &mut module_path_to_file_id,
    );

    let by_canonical = canonical_files(root, &file_id_to_path);
    let payloads: BTreeMap<String, (String, String)> = nodes
        .rows
        .iter()
        .filter_map(|r| {
            let id = r.first().map(unquote_datavalue)?;
            let ty = r.get(1).map(unquote_datavalue).unwrap_or_default();
            let payload = r.get(2).map(unquote_datavalue).unwrap_or_default();
            Some((id, (ty, payload)))
        })
        .collect();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &edges.rows {
        let from_id = row.first().map(unquote_datavalue).unwrap_or_default();
        let to_id = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != edge_contains {
            continue;
        }
        if let (true, Some((ty, mod_name))) = (
            file_id_to_module_path.contains_key(&from_id),
            payloads.get(&to_id),
        ) {
            if *ty == type_module {
                let (krate, current) = file_id_to_module_path[&from_id].clone();
                let sub_path = join_mod(&current, &[mod_name]);
                let target = resolve_module_to_file(&sub_path, &root.join(&krate))
                    .and_then(|p| p.canonicalize().ok())
                    .and_then(|c| by_canonical.get(&c).copied());
                if let Some(sub_file_id) = target {
                    module_path_to_file_id.insert((krate, sub_path), sub_file_id.clone());
                }
            }
        }
        children.entry(from_id).or_default().push(to_id);
    }
    let items = items_by_file(&file_id_to_path, &payloads, &children);

    ModuleMaps {
        file_id_to_module_path,
        module_path_to_file_id,
        items,
        file_uses,
        path_mods,
    }
}

/// Prefix of `Imports` targets that name an item imported from another crate (`std`, `serde_json`, ..).
pub(crate) const EXTERNAL_IMPORT_PREFIX: &str = "external::";
/// Prefix of `Imports` targets that record a rename: `alias::h=>helper` for `use a::helper as h`,
/// with `=>ID` appended when the renamed item resolved to the node `ID`.
pub(crate) const ALIAS_IMPORT_PREFIX: &str = "alias::";
/// Prefix of `Imports` targets recording a glob import of a project module: `glob::FILE` for
/// `use super::*` (the `Imports` edge starts at the importing file).
pub(crate) const GLOB_IMPORT_PREFIX: &str = "glob::";
/// Prefix of `Imports` targets recording `#[path = ".."] mod name;`: `pathmod::name=>FILE`.
pub(crate) const PATH_MOD_IMPORT_PREFIX: &str = "pathmod::";

/// Whether an `Imports` target is bookkeeping for call resolution rather than a node id.
pub(crate) fn is_import_marker(to: &str) -> bool {
    [
        EXTERNAL_IMPORT_PREFIX,
        ALIAS_IMPORT_PREFIX,
        GLOB_IMPORT_PREFIX,
        PATH_MOD_IMPORT_PREFIX,
    ]
    .iter()
    .any(|p| to.starts_with(p))
}

/// A `use` leaf that did not resolve locally and does not start with `crate`/`self`/`super` or a local module.
fn is_external_use(
    use_path: &str,
    file_id: &str,
    krate: &str,
    current_module: &str,
    maps: &ModuleMaps,
) -> bool {
    let first = use_path
        .trim()
        .trim_start_matches("::")
        .split("::")
        .next()
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("");
    if first.is_empty() || matches!(first, "crate" | "self" | "super") {
        return false;
    }
    let local = |base: &str| module_file(maps, krate, &format!("{base}::{first}")).is_some();
    // An inline `mod first { .. }` of this file is no other crate either.
    !(local("crate") || local(current_module) || find_item_in_file(maps, file_id, first).is_some())
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

/// `use m::f;` where `m` is bound by another `use` of the file (`use crate::ui::m;`): the leaf with
/// that binding spelled out (`crate::ui::m::f`). `None` when no other leaf binds the first segment.
fn expand_use_binding(leaf: &str, leaves: &[(String, Option<String>)]) -> Option<String> {
    let (first, rest) = leaf.trim().split_once("::")?;
    if matches!(first, "crate" | "self" | "super") || first.contains(' ') {
        return None;
    }
    let bound = leaves.iter().map(|(l, _)| l.trim()).find(|l| {
        l.contains("::") && !l.contains(" as ") && l.rsplit("::").next() == Some(first)
    })?;
    Some(format!("{bound}::{rest}"))
}

fn collect_imports_to_add(maps: &ModuleMaps) -> Vec<(NodeId, NodeId, EdgeType)> {
    let mut out = Vec::new();
    for (file, name, target) in &maps.path_mods {
        out.push((
            NodeId::new(file.clone()),
            NodeId::new(format!("{PATH_MOD_IMPORT_PREFIX}{name}=>{target}")),
            EdgeType::Imports,
        ));
    }
    for (file_id, leaves) in &maps.file_uses {
        let Some((krate, module_path)) = maps.file_id_to_module_path.get(file_id) else {
            continue;
        };
        let from = NodeId::new(file_id.clone());
        for (leaf, scope) in leaves {
            // A glob brings no certain item, so it only leaves a marker for bare-call resolution.
            if leaf.ends_with("::*") {
                if let Some((target, None, true)) =
                    resolve_use_path(leaf, krate, module_path, maps, 0)
                {
                    out.push((
                        from.clone(),
                        NodeId::new(format!("{GLOB_IMPORT_PREFIX}{target}")),
                        EdgeType::Imports,
                    ));
                }
                continue;
            }
            let mut external = false;
            let mut item_node = None;
            let resolved = resolve_use_path(leaf, krate, module_path, maps, 0).or_else(|| {
                let expanded = expand_use_binding(leaf, leaves)?;
                resolve_use_path(&expanded, krate, module_path, maps, 0)
            });
            if let Some((target_file_id, target_node_opt, names_module)) = resolved {
                // Importing a module itself brings no function into scope, so it leaves no edge.
                if !names_module {
                    item_node.clone_from(&target_node_opt);
                    let to = NodeId::new(target_node_opt.unwrap_or(target_file_id));
                    out.push((from.clone(), to, EdgeType::Imports));
                }
            } else if is_external_use(leaf, file_id, krate, module_path, maps) {
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
                    let target = item_node
                        .as_ref()
                        .map_or_else(String::new, |n| format!("=>{n}"));
                    // A rename inside a function is only visible there: its edge starts at the function.
                    let from = scope
                        .as_ref()
                        .map_or_else(|| from.clone(), |s| NodeId::new(format!("{file_id}#{s}")));
                    out.push((
                        from,
                        NodeId::new(format!("{ALIAS_IMPORT_PREFIX}{name}=>{orig}{target}")),
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
    let imports_to_add = collect_imports_to_add(&maps);

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
    fn finds_path_attribute_mods() {
        let src =
            "#[path = \"x_tests.rs\"]\n#[cfg(test)]\nmod tests;\nmod plain;\nmod inline { }\n";
        assert_eq!(
            super::collect_path_mods(src),
            [("tests".to_string(), "x_tests.rs".to_string())]
        );
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
    fn bin_only_files_follow_cargo_target_layout() {
        let d = tempfile::tempdir().unwrap();
        let w = |rel: &str, text: &str| {
            let p = d.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let pkg = "[package]\nname = \"p\"\nversion = \"0.1.0\"\n";
        w("app/Cargo.toml", pkg);
        w("app/src/main.rs", "");
        w("app/src/util.rs", "");
        w("lib/Cargo.toml", pkg);
        w("lib/src/lib.rs", "");
        w("lib/src/main.rs", "");
        w("lib/src/util.rs", "");
        w("lib/src/bin/tool.rs", "");
        w("lib/tests/support/mod.rs", "");
        w(
            "custom/Cargo.toml",
            &format!("{pkg}[lib]\npath = \"core/lib.rs\"\n"),
        );
        w("custom/core/x.rs", "");
        w("loose/x.rs", "");
        let bin_only = |rel: &str| is_bin_only_file(d.path(), &d.path().join(rel));
        assert!(bin_only("app/src/main.rs") && bin_only("app/src/util.rs"));
        assert!(!bin_only("lib/src/lib.rs") && !bin_only("lib/src/util.rs"));
        assert!(bin_only("lib/src/main.rs") && bin_only("lib/src/bin/tool.rs"));
        assert!(bin_only("lib/tests/support/mod.rs"));
        assert!(
            !bin_only("custom/core/x.rs"),
            "[lib] table means a library target"
        );
        assert!(
            !bin_only("loose/x.rs"),
            "no manifest: unknown, stay conservative"
        );
    }

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
            "mod utils;\nuse crate::utils::add;\npub fn greet() {}",
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
