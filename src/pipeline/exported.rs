//! Marks bare `pub` free functions that a library crate does not export (`attr::pub_unexported`).
//!
//! A function is exported when its module is reachable from the crate root through bare `pub mod`
//! declarations, `pub use` globs and `pub use` of modules, or when a `pub use` in a reachable
//! module names it (following re-export chains). Limits (all err towards "exported"): methods and
//! types are not marked; a `pub use` path that does not resolve exports every function of that
//! name; macro-generated items are not seen; proc-macro crates are skipped.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use tree_sitter::{Node, Parser};
use tree_sitter_rust::LANGUAGE;

use super::modules::{crate_dir, expand_use, use_path_from_node};
use crate::graph::schema::{EdgeType, NodeId};
use crate::graph::Store;

/// Marker target, kept in sync with the entry rules in `graph::query`.
pub(super) const PUB_UNEXPORTED_ATTR: &str = "pub_unexported";
/// Written whenever the phase ran, on the first indexed file. `pub_unexported` itself is sparse (a library
/// with nothing to demote has none), so `pr` checks this one to tell an older graph from an empty result.
const INDEXED_ATTR: &str = "lib_exported_indexed";

enum LeafKind {
    Name,
    Glob,
    SelfMod,
}

struct Leaf {
    segs: Vec<String>,
    alias: Option<String>,
    kind: LeafKind,
}

#[derive(Default)]
struct Mod {
    parent: Option<usize>,
    children: BTreeMap<String, usize>,
    /// Bare `pub mod` (the root counts as public).
    is_pub: bool,
    fns: Vec<(String, NodeId)>,
    pub_uses: Vec<Leaf>,
}

struct Crate<'a> {
    files: &'a BTreeMap<PathBuf, String>,
    root: &'a Path,
    mods: Vec<Mod>,
}

fn is_bare_pub(node: &Node, source: &str) -> bool {
    node.child(0).is_some_and(|c| {
        c.kind() == "visibility_modifier" && source.get(c.byte_range()) == Some("pub")
    })
}

fn parse_leaf(text: &str) -> Leaf {
    let (path, alias) = match text.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim().to_string())),
        None => (text.trim(), None),
    };
    let mut segs: Vec<String> = path
        .trim_start_matches("::")
        .split("::")
        .map(str::to_string)
        .collect();
    let kind = match segs.last().map(String::as_str) {
        Some("*") => LeafKind::Glob,
        Some("self") => LeafKind::SelfMod,
        _ => LeafKind::Name,
    };
    if !matches!(kind, LeafKind::Name) {
        segs.pop();
    }
    Leaf { segs, alias, kind }
}

impl Crate<'_> {
    /// Parse module `idx` from `file`; `dir` is where its child `mod x;` files live.
    fn load(&mut self, idx: usize, file: &Path, dir: &Path) {
        let Some(source) = self.files.get(file) else {
            return;
        };
        let mut parser = Parser::new();
        if parser.set_language(&LANGUAGE.into()).is_err() {
            return;
        }
        let Some(tree) = parser.parse(source, None) else {
            return;
        };
        let file_id = format!(
            "./{}",
            file.strip_prefix(self.root)
                .unwrap_or(file)
                .to_string_lossy()
        );
        self.items(idx, tree.root_node(), source, file, dir, &file_id);
    }

    fn items(
        &mut self,
        idx: usize,
        body: Node,
        source: &str,
        file: &Path,
        dir: &Path,
        file_id: &str,
    ) {
        let mut cursor = body.walk();
        for node in body.children(&mut cursor) {
            match node.kind() {
                "function_item" if super::ast::is_unexported_candidate(&node, source) => {
                    let name = node
                        .child_by_field_name("name")
                        .and_then(|n| source.get(n.byte_range()))
                        .unwrap_or_default();
                    let id = NodeId::new(format!(
                        "{file_id}#{}:{}",
                        node.start_position().row + 1,
                        node.start_position().column + 1
                    ));
                    self.mods[idx].fns.push((name.to_string(), id));
                }
                "use_declaration" if is_bare_pub(&node, source) => {
                    if let Some(path) = use_path_from_node(&node, source) {
                        let leaves = expand_use(&path).into_iter().map(|l| parse_leaf(&l));
                        self.mods[idx].pub_uses.extend(leaves);
                    }
                }
                "mod_item" => self.module(idx, node, source, file, dir, file_id),
                _ => {}
            }
        }
    }

    fn module(
        &mut self,
        parent: usize,
        node: Node,
        source: &str,
        file: &Path,
        dir: &Path,
        file_id: &str,
    ) {
        let Some(name) = node
            .child_by_field_name("name")
            .and_then(|n| source.get(n.byte_range()))
        else {
            return;
        };
        let child = *self.mods[parent]
            .children
            .entry(name.to_string())
            .or_insert(usize::MAX);
        let child = if child == usize::MAX {
            self.mods.push(Mod {
                parent: Some(parent),
                ..Mod::default()
            });
            let i = self.mods.len() - 1;
            self.mods[parent].children.insert(name.to_string(), i);
            i
        } else {
            child
        };
        self.mods[child].is_pub |= is_bare_pub(&node, source);
        if let Some(body) = node.child_by_field_name("body") {
            self.items(child, body, source, file, &dir.join(name), file_id);
            return;
        }
        let path_attr = std::iter::successors(node.prev_sibling(), Node::prev_sibling)
            .take_while(|s| s.kind() == "attribute_item")
            .find_map(|a| {
                let t = source.get(a.byte_range())?;
                t.strip_prefix("#[path")?
                    .split('"')
                    .nth(1)
                    .map(str::to_string)
            });
        let candidates = match path_attr {
            Some(p) => vec![(file.parent().unwrap_or(dir).join(p), None)],
            None => vec![
                (dir.join(format!("{name}.rs")), Some(dir.join(name))),
                (dir.join(name).join("mod.rs"), Some(dir.join(name))),
            ],
        };
        for (path, child_dir) in candidates {
            if self.files.contains_key(&path) {
                let child_dir =
                    child_dir.unwrap_or_else(|| path.parent().unwrap_or(dir).to_path_buf());
                self.load(child, &path, &child_dir);
                return;
            }
        }
    }

    fn resolve(&self, cur: usize, segs: &[String]) -> Option<usize> {
        let mut m = cur;
        for (i, s) in segs.iter().enumerate() {
            m = match s.as_str() {
                "crate" if i == 0 => 0,
                "self" if i == 0 => cur,
                "super" => self.mods[m].parent?,
                name => *self.mods[m].children.get(name)?,
            };
        }
        Some(m)
    }

    /// Mark `(module, name)` exported, following `pub use` re-exports of that name inside `module`.
    fn export(&self, m: usize, name: &str, out: &mut State) {
        if !out.names.insert((m, name.to_string())) {
            return;
        }
        for leaf in &self.mods[m].pub_uses {
            match leaf.kind {
                LeafKind::Glob => {
                    if let Some(t) = self.resolve(m, &leaf.segs) {
                        self.export(t, name, out);
                    }
                }
                LeafKind::Name => {
                    let last = leaf.segs.last().map_or("", String::as_str);
                    if leaf.alias.as_deref().unwrap_or(last) == name {
                        self.bind(m, leaf, out);
                    }
                }
                LeafKind::SelfMod => {}
            }
        }
    }

    /// Effect of one `pub use` leaf of an exported module: reach modules, export names.
    fn bind(&self, m: usize, leaf: &Leaf, out: &mut State) {
        let (LeafKind::Name, Some((last, parent))) = (&leaf.kind, leaf.segs.split_last()) else {
            if let Some(t) = self.resolve(m, &leaf.segs) {
                out.reach.insert(t);
            }
            return;
        };
        let Some(t) = (if parent.is_empty() {
            Some(m)
        } else {
            self.resolve(m, parent)
        }) else {
            out.unresolved.insert(last.clone());
            return;
        };
        if let Some(&child) = self.mods[t].children.get(last) {
            out.reach.insert(child);
        }
        self.export(t, last, out);
    }
}

#[derive(Default)]
struct State {
    reach: HashSet<usize>,
    names: HashSet<(usize, String)>,
    unresolved: HashSet<String>,
}

impl Crate<'_> {
    fn exported(&self) -> State {
        let mut st = State::default();
        st.reach.insert(0);
        loop {
            let before = (st.reach.len(), st.names.len(), st.unresolved.len());
            for i in 0..self.mods.len() {
                if i != 0
                    && !(self.mods[i].is_pub
                        && self.mods[i].parent.is_some_and(|p| st.reach.contains(&p)))
                {
                    continue;
                }
                st.reach.insert(i);
            }
            let reached: Vec<usize> = st.reach.iter().copied().collect();
            for m in reached {
                for leaf in &self.mods[m].pub_uses {
                    self.bind(m, leaf, &mut st);
                }
            }
            if before == (st.reach.len(), st.names.len(), st.unresolved.len()) {
                return st;
            }
        }
    }
}

/// Library root file of the package at `dir`, unless it is a proc-macro crate.
fn lib_root(dir: &Path) -> Option<PathBuf> {
    let manifest: toml::Value = std::fs::read_to_string(dir.join("Cargo.toml"))
        .ok()?
        .parse()
        .ok()?;
    let lib = manifest.get("lib");
    if lib
        .and_then(|l| l.get("proc-macro"))
        .and_then(toml::Value::as_bool)
        == Some(true)
    {
        return None;
    }
    let path = lib
        .and_then(|l| l.get("path"))
        .and_then(toml::Value::as_str)
        .map_or_else(|| dir.join("src/lib.rs"), |p| dir.join(p));
    path.is_file().then_some(path)
}

/// Add the [`PUB_UNEXPORTED_ATTR`] marker to every bare `pub` free function of a library crate that
/// is not reachable from its public API.
///
/// # Errors
/// Fails if the store write fails.
pub(super) fn mark_pub_unexported(
    store: &Store,
    root: &Path,
    files: &BTreeMap<PathBuf, String>,
) -> Result<()> {
    let dirs: HashSet<PathBuf> = files
        .keys()
        .map(|f| crate_dir(root, f))
        .filter(|d| d.join("Cargo.toml").is_file())
        .collect();
    let mut edges = Vec::new();
    for dir in dirs {
        let Some(lib) = lib_root(&dir) else { continue };
        let mut krate = Crate {
            files,
            root,
            mods: vec![Mod {
                is_pub: true,
                ..Mod::default()
            }],
        };
        krate.load(0, &lib, lib.parent().unwrap_or(&dir));
        let st = krate.exported();
        for (i, m) in krate.mods.iter().enumerate() {
            if st.reach.contains(&i) {
                continue;
            }
            for (name, id) in &m.fns {
                if !st.names.contains(&(i, name.clone())) && !st.unresolved.contains(name) {
                    edges.push((
                        id.clone(),
                        NodeId::new(format!("attr::{PUB_UNEXPORTED_ATTR}")),
                        EdgeType::HasAttribute,
                    ));
                }
            }
        }
    }
    if let Some(first) = files.keys().next() {
        let rel = first.strip_prefix(root).unwrap_or(first);
        edges.push((
            NodeId::new(format!("./{}", rel.to_string_lossy())),
            NodeId::new(format!("attr::{INDEXED_ATTR}")),
            EdgeType::HasAttribute,
        ));
    }
    if !edges.is_empty() {
        store.put_edges_batch(&edges)?;
    }
    Ok(())
}
