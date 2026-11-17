//! Marks bare `pub` free functions, consts and statics that a library crate does not export (`attr::pub_unexported`).
//!
//! A function is exported when its module is reachable from the crate root through bare `pub mod`
//! declarations, `pub use` globs and `pub use` of modules, or when a `pub use` in a reachable
//! module names it (following re-export chains). Limits (all err towards "exported"): methods and
//! types are not marked (nor are associated consts); a `pub use` path that does not resolve exports every function of that
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
    fns: Vec<Func>,
    types: Vec<TypeDecl>,
    methods: Vec<Method>,
    /// `impl Trait for Ty`: the type names the whole impl mentions (associated types, `Deref::Target`, ...).
    trait_impls: Vec<(String, Vec<String>)>,
    pub_uses: Vec<Leaf>,
}

/// A bare `pub` free function, const or static. `id` is set when it may be demoted.
struct Func {
    name: String,
    id: Option<NodeId>,
    /// Type names in the signature (params, return type, bounds): they are nameable through this item.
    sig: Vec<String>,
}

struct TypeDecl {
    name: String,
    is_pub: bool,
    /// Every type name inside the declaration (over-approximates: private fields count).
    exposes: Vec<String>,
}

/// A bare `pub` method of an inherent `impl`.
struct Method {
    ty: String,
    func: Func,
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

fn field_text(node: &Node, field: &str, source: &str) -> String {
    node.child_by_field_name(field)
        .and_then(|n| source.get(n.byte_range()))
        .unwrap_or_default()
        .to_string()
}

fn node_id(node: &Node, file_id: &str) -> NodeId {
    NodeId::new(format!(
        "{file_id}#{}:{}",
        node.start_position().row + 1,
        node.start_position().column + 1
    ))
}

/// Every `type_identifier` under `node` (including the last segment of scoped paths).
fn type_idents(node: &Node, source: &str, out: &mut Vec<String>) {
    if node.kind() == "type_identifier" {
        out.extend(source.get(node.byte_range()).map(str::to_string));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        type_idents(&child, source, out);
    }
}

/// `Ty`, `Ty<T>` or `path::Ty` as the self type of an `impl`; anything else (`&Ty`, tuples) is not tracked.
fn self_type_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => source.get(node.byte_range()).map(str::to_string),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|t| self_type_name(&t, source)),
        "scoped_type_identifier" => node
            .child_by_field_name("name")
            .and_then(|t| self_type_name(&t, source)),
        _ => None,
    }
}

fn func(node: &Node, source: &str, file_id: &str, demotable: bool) -> Func {
    let mut sig = Vec::new();
    let body = node.child_by_field_name("body").map(|b| b.id());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if Some(child.id()) != body {
            type_idents(&child, source, &mut sig);
        }
    }
    Func {
        name: field_text(node, "name", source),
        id: demotable.then(|| node_id(node, file_id)),
        sig,
    }
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
                "function_item" if is_bare_pub(&node, source) => {
                    let f = func(
                        &node,
                        source,
                        file_id,
                        super::ast::is_unexported_candidate(&node, source),
                    );
                    self.mods[idx].fns.push(f);
                }
                "const_item" | "static_item" if is_bare_pub(&node, source) => {
                    let name = field_text(&node, "name", source);
                    let id = Some(node_id(&node, file_id));
                    // The declared type is nameable through the item; the initializer is not.
                    let mut sig = Vec::new();
                    if let Some(ty) = node.child_by_field_name("type") {
                        type_idents(&ty, source, &mut sig);
                    }
                    self.mods[idx].fns.push(Func { name, id, sig });
                }
                "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                    let mut exposes = Vec::new();
                    type_idents(&node, source, &mut exposes);
                    self.mods[idx].types.push(TypeDecl {
                        name: field_text(&node, "name", source),
                        is_pub: is_bare_pub(&node, source),
                        exposes,
                    });
                }
                "impl_item" => self.impl_block(idx, &node, source, file_id),
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

    fn impl_block(&mut self, idx: usize, node: &Node, source: &str, file_id: &str) {
        let Some(ty) = node
            .child_by_field_name("type")
            .and_then(|t| self_type_name(&t, source))
        else {
            return;
        };
        if node.child_by_field_name("trait").is_some() {
            let mut names = Vec::new();
            type_idents(node, source, &mut names);
            self.mods[idx].trait_impls.push((ty, names));
            return;
        }
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let mut cursor = body.walk();
        for f in body.children(&mut cursor) {
            if f.kind() == "function_item" && is_bare_pub(&f, source) {
                let demotable = super::ast::is_unexported_candidate(&f, source)
                    && !super::ast::is_attribute_macro_impl_method(&f, source);
                let func = func(&f, source, file_id, demotable);
                self.mods[idx].methods.push(Method {
                    ty: ty.clone(),
                    func,
                });
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

impl State {
    fn is_exported(&self, m: usize, name: &str) -> bool {
        self.reach.contains(&m)
            || self.names.contains(&(m, name.to_string()))
            || self.unresolved.contains(name)
    }
}

impl Crate<'_> {
    /// Names of the types a user of the crate can name or obtain: declared bare `pub` in a reachable
    /// module, named by a `pub use`, or mentioned in the signature of an exported function or method, in an
    /// exported type, or in a trait impl of one (name-based, over-approximate).
    fn exported_types(&self, st: &State) -> HashSet<String> {
        let mut exposed: HashSet<String> = HashSet::new();
        let mut out: HashSet<String> = HashSet::new();
        loop {
            let before = (exposed.len(), out.len());
            for (i, m) in self.mods.iter().enumerate() {
                for f in &m.fns {
                    if st.is_exported(i, &f.name) {
                        exposed.extend(f.sig.iter().cloned());
                    }
                }
                for t in &m.types {
                    let public = (st.reach.contains(&i) && t.is_pub)
                        || st.names.contains(&(i, t.name.clone()))
                        || st.unresolved.contains(&t.name);
                    if public || exposed.contains(&t.name) {
                        out.insert(t.name.clone());
                        exposed.extend(t.exposes.iter().cloned());
                    }
                }
                for f in &m.methods {
                    if out.contains(&f.ty) {
                        exposed.extend(f.func.sig.iter().cloned());
                    }
                }
                for (ty, names) in &m.trait_impls {
                    if out.contains(ty) {
                        exposed.extend(names.iter().cloned());
                    }
                }
            }
            if before == (exposed.len(), out.len()) {
                return out;
            }
        }
    }

    fn exported(&self) -> State {
        let mut st = State::default();
        st.reach.insert(0);
        loop {
            let before = (st.reach.len(), st.names.len(), st.unresolved.len());
            for (i, m) in self.mods.iter().enumerate() {
                if i == 0 || (m.is_pub && m.parent.is_some_and(|p| st.reach.contains(&p))) {
                    st.reach.insert(i);
                }
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
        let exported_types = krate.exported_types(&st);
        let known: HashSet<&str> = krate
            .mods
            .iter()
            .flat_map(|m| m.types.iter().map(|t| t.name.as_str()))
            .collect();
        let demote = |id: &NodeId| {
            (
                id.clone(),
                NodeId::new(format!("attr::{PUB_UNEXPORTED_ATTR}")),
                EdgeType::HasAttribute,
            )
        };
        for (i, m) in krate.mods.iter().enumerate() {
            if !st.reach.contains(&i) {
                for f in &m.fns {
                    if let (Some(id), false) = (&f.id, st.is_exported(i, &f.name)) {
                        edges.push(demote(id));
                    }
                }
            }
            // A method is demoted only when its type is declared in this crate and no declaration of that name is exported.
            for f in &m.methods {
                let hidden = known.contains(f.ty.as_str()) && !exported_types.contains(&f.ty);
                if let (Some(id), true) = (&f.func.id, hidden) {
                    edges.push(demote(id));
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
