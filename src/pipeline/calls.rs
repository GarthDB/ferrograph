//! Phase 4: call graph construction (enrichment from AST edges).
//!
//! Resolves placeholder call targets (`file_path::fn_name`) to real function node IDs
//! (`file_path#line:col`). Tries same-file first, then global `fn_name` lookup for cross-file calls.
//!
//! Method calls (`x.m()`, placeholders ending in `METHOD_SUFFIX`) resolve by name only, among
//! functions defined inside an `impl`/`trait` (never free functions): same-file methods win, else
//! every method with that name in the project. Several candidates become low-confidence edges
//! (1/N); more than `MAX_METHOD_CANDIDATES` are dropped, so std-like names (`len`, `get`, `push`)
//! cannot flood the graph. Qualified paths (`Type::m()`, `mod::fn`) are partially handled (see
//! `resolve_call_target` `scoped_identifier` support).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};

/// Suffix the AST phase puts on call placeholders for paths used as values (see `ast.rs`).
pub(crate) const VALUE_SUFFIX: &str = "@value";
/// Suffix the AST phase puts on method-call placeholders (`x.m()`).
pub(crate) const METHOD_SUFFIX: &str = "@method";
/// A method name with more candidates than this is dropped at the call site (no edges).
const MAX_METHOD_CANDIDATES: usize = 8;
use crate::graph::{query::Query, unquote_datavalue, Store};

/// Strip `pub::`, `test::`, and `bench::` prefixes from payload for canonical function name.
/// While-loop handles composite prefixes (e.g. `pub::test::name`) produced by `function_payload`.
fn canonical_name(payload: &str) -> &str {
    let mut s = payload;
    while let Some(rest) = s
        .strip_prefix("pub::")
        .or_else(|| s.strip_prefix("test::"))
        .or_else(|| s.strip_prefix("bench::"))
    {
        s = rest;
    }
    s
}

/// Lookup tables used to resolve call placeholders.
#[derive(Default)]
struct Index {
    local: HashMap<(String, String), Vec<NodeId>>,
    imports_map: HashMap<String, Vec<String>>,
    node_id_to_payload: HashMap<String, String>,
    global_by_name: HashMap<String, Vec<NodeId>>,
    /// Ids of functions defined directly inside an `impl` or `trait`.
    methods: HashSet<String>,
    /// `contains` edges, child id -> parent id.
    parent: HashMap<String, String>,
    /// Scope-forming nodes by id: (is `impl`/`trait`, payload). `mod` blocks have `false`.
    scopes: HashMap<String, (bool, String)>,
}

impl Index {
    /// Enclosing items of `from`, innermost first, starting with `from` itself.
    fn ancestors<'a>(&'a self, from: &'a str) -> impl Iterator<Item = &'a str> {
        std::iter::successors(Some(from), |n| self.parent.get(*n).map(String::as_str))
    }

    fn parent_of(&self, id: &NodeId) -> Option<&str> {
        self.parent.get(id.as_str()).map(String::as_str)
    }
}

/// Lexical scoping for a bare name: the same-named candidates declared in the nearest enclosing
/// block/fn/mod of `from` (items in `impl`/`trait` bodies are not visible by bare name). `None`
/// when no candidate is in scope.
fn nearest_in_scope(from: &str, cands: &[NodeId], idx: &Index) -> Option<Vec<NodeId>> {
    idx.ancestors(from)
        .filter(|a| !idx.scopes.get(*a).is_some_and(|s| s.0))
        .map(|a| {
            cands
                .iter()
                .filter(|c| idx.parent_of(c) == Some(a))
                .cloned()
                .collect::<Vec<_>>()
        })
        .find(|v| !v.is_empty())
}

/// Pick among same-named candidates for a path call `owner::f()`: `Self` is the enclosing impl
/// (all impls of its type, except `from` itself); otherwise `owner` names the type or module.
fn pick_by_owner(owner: &str, from: &str, cands: &[NodeId], idx: &Index) -> Vec<NodeId> {
    let of_scope = |pred: &dyn Fn(&str, &(bool, String)) -> bool| -> Vec<NodeId> {
        cands
            .iter()
            .filter(|c| {
                idx.parent_of(c)
                    .and_then(|p| idx.scopes.get(p).map(|s| (p, s)))
                    .is_some_and(|(p, s)| pred(p, s))
            })
            .cloned()
            .collect()
    };
    if owner != "Self" {
        return of_scope(&|_, (_, name)| name == owner);
    }
    let Some(ty) = idx
        .ancestors(from)
        .find_map(|a| idx.scopes.get(a).filter(|s| s.0).map(|s| &s.1))
    else {
        return Vec::new();
    };
    if ty.is_empty() || ty == "null" {
        return Vec::new();
    }
    // Every impl of the type (inherent and trait): a trait-impl method calling `Self::f()` must
    // not resolve to itself when an inherent `f` exists.
    of_scope(&|_, (is_impl, name)| *is_impl && name == ty)
        .into_iter()
        .filter(|c| c.as_str() != from)
        .collect()
}

fn resolution_of(mut v: Vec<NodeId>) -> Option<Resolution> {
    match v.len() {
        0 => None,
        1 => Some(Resolution::One(v.remove(0))),
        _ => Some(Resolution::Ambiguous(v)),
    }
}

/// Resolve `x.name()` by name only: same-file methods first, else all methods project-wide.
fn resolve_method(name: &str, from_file: &str, idx: &Index) -> Resolution {
    let all = idx.global_by_name.get(name).map_or(&[][..], Vec::as_slice);
    let prefix = format!("{from_file}#");
    let is_method = |n: &&NodeId| idx.methods.contains(n.as_str());
    let mut cands: Vec<NodeId> = all
        .iter()
        .filter(is_method)
        .filter(|n| n.as_str().starts_with(&prefix))
        .cloned()
        .collect();
    if cands.is_empty() {
        // Cheap cutoff before cloning: a hugely shared name is dropped anyway.
        if all.len() > MAX_METHOD_CANDIDATES
            && all.iter().filter(is_method).count() > MAX_METHOD_CANDIDATES
        {
            return Resolution::None;
        }
        cands = all.iter().filter(is_method).cloned().collect();
    }
    match cands.len() {
        0 => Resolution::None,
        1 => Resolution::One(cands.remove(0)),
        n if n <= MAX_METHOD_CANDIDATES => Resolution::Ambiguous(cands),
        _ => Resolution::None,
    }
}

/// Outcome of resolving a call placeholder.
enum Resolution {
    One(NodeId),
    /// Several candidates and no way to pick (no type info); kept as low-confidence edges.
    Ambiguous(Vec<NodeId>),
    None,
}

/// Resolve a single placeholder (`path_part::fn_name`).
/// For qualified paths (`path_part` contains "::"), first try same-file resolution by file path only.
/// `value` marks a path used as a value (fn pointer): it never uses the global name fallback and
/// never yields ambiguous edges for bare names, since the identifier is often just a local variable.
/// Qualified values (`S::f`) may: the owner must name a real impl type or module, so cfg variants
/// of `S::f` stay linked.
fn resolve_placeholder(
    path_part: &str,
    fn_name: &str,
    from: &str,
    from_file: &str,
    value: bool,
    idx: &Index,
) -> Resolution {
    let Index {
        local,
        imports_map,
        node_id_to_payload,
        global_by_name,
        ..
    } = idx;
    if path_part.contains("::") {
        // TODO: qualified path resolution only uses file-level (first path segment); module path within the file (e.g. mod::submod::foo()) is not considered.
        let file_only = path_part.split("::").next().unwrap_or(path_part);
        let key = (file_only.to_string(), fn_name.to_string());
        if let Some(candidates) = local.get(&key) {
            if let [one] = candidates.as_slice() {
                return Resolution::One(one.clone());
            }
            let owner = path_part.rsplit("::").next().unwrap_or(path_part);
            if let Some(r) = resolution_of(pick_by_owner(owner, from, candidates, idx)) {
                return r;
            }
        }
        // Qualified type/module path (e.g. std::collections::HashMap); do not resolve via global lookup to avoid false positives.
        return Resolution::None;
    }
    let key = (path_part.to_string(), fn_name.to_string());
    if let Some(candidates) = local.get(&key) {
        if candidates.len() > 1 {
            if let Some(r) = nearest_in_scope(from, candidates, idx).and_then(resolution_of) {
                return r;
            }
        }
        return match candidates.as_slice() {
            [one] => Resolution::One(one.clone()),
            many if many.len() > 1 && !value => Resolution::Ambiguous(many.to_vec()),
            _ => Resolution::None,
        };
    }
    if let Some(imported) = imports_map.get(from_file) {
        let mut candidate: Option<NodeId> = None;
        for to_id in imported {
            if to_id.contains('#') {
                if let Some(payload) = node_id_to_payload.get(to_id) {
                    if canonical_name(payload) == fn_name {
                        if candidate.is_some() {
                            return Resolution::None;
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
                        return Resolution::None;
                    }
                    candidate = in_file.into_iter().next();
                }
            }
        }
        return candidate.map_or(Resolution::None, Resolution::One);
    }
    if value {
        return Resolution::None;
    }
    match global_by_name.get(fn_name).map(Vec::as_slice) {
        Some([one]) => Resolution::One(one.clone()),
        Some([]) | None => Resolution::None,
        Some(candidates) => Resolution::Ambiguous(candidates.to_vec()),
    }
}

/// Imports edges grouped by importing file.
fn imports_by_file(edges: &cozo::NamedRows) -> HashMap<String, Vec<String>> {
    let imports = EdgeType::Imports.to_string();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for row in &edges.rows {
        if row.get(2).map(unquote_datavalue).as_deref() != Some(imports.as_str()) {
            continue;
        }
        let from = row.first().map(unquote_datavalue).unwrap_or_default();
        let to = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let file = from.split('#').next().unwrap_or(&from).to_string();
        map.entry(file).or_default().push(to);
    }
    map
}

/// Ids of nodes directly contained in an `impl` or `trait` node.
fn method_ids(nodes: &cozo::NamedRows, edges: &cozo::NamedRows) -> HashSet<String> {
    let containers: HashSet<String> = nodes
        .rows
        .iter()
        .filter(|r| {
            let t = r.get(1).map(unquote_datavalue).unwrap_or_default();
            t == NodeType::Impl.to_string() || t == NodeType::Trait.to_string()
        })
        .filter_map(|r| r.first().map(unquote_datavalue))
        .collect();
    let contains = EdgeType::Contains.to_string();
    edges
        .rows
        .iter()
        .filter(|r| r.get(2).map(unquote_datavalue).as_deref() == Some(contains.as_str()))
        .filter(|r| {
            r.first()
                .is_some_and(|p| containers.contains(&unquote_datavalue(p)))
        })
        .filter_map(|r| r.get(1).map(unquote_datavalue))
        .collect()
}

/// Fill `idx.parent` (from `contains` edges) and `idx.scopes` (impl/trait/mod nodes).
fn fill_scopes(idx: &mut Index, nodes: &cozo::NamedRows, edges: &cozo::NamedRows) {
    let contains = EdgeType::Contains.to_string();
    for row in &edges.rows {
        if row.get(2).map(unquote_datavalue).as_deref() == Some(contains.as_str()) {
            let (Some(p), Some(c)) = (row.first(), row.get(1)) else {
                continue;
            };
            idx.parent
                .insert(unquote_datavalue(c), unquote_datavalue(p));
        }
    }
    for row in &nodes.rows {
        let t = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let is_container = t == NodeType::Impl.to_string() || t == NodeType::Trait.to_string();
        if is_container || t == NodeType::Module.to_string() {
            let id = row.first().map(unquote_datavalue).unwrap_or_default();
            let payload = row.get(2).map(unquote_datavalue).unwrap_or_default();
            idx.scopes.insert(id, (is_container, payload));
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

    let nodes = Query::all_nodes(store)?;
    let mut idx = Index::default();
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
        idx.node_id_to_payload
            .insert(id_trim.clone(), payload.clone());
        let file_path = id_trim.split('#').next().unwrap_or(&id_trim).to_string();
        let node_id = NodeId(id_trim.clone());
        idx.local
            .entry((file_path.clone(), canonical_name(&payload).to_string()))
            .or_default()
            .push(node_id.clone());
        let name = canonical_name(&payload).to_string();
        idx.global_by_name.entry(name).or_default().push(node_id);
    }

    let edges = Query::all_edges(store)?;
    idx.imports_map = imports_by_file(&edges);
    idx.methods = method_ids(&nodes, &edges);
    fill_scopes(&mut idx, &nodes, &edges);

    let mut high: HashSet<(String, String)> = HashSet::new();
    let mut low: Vec<(NodeId, NodeId, f64)> = Vec::new();
    for row in &edges.rows {
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != calls_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let raw_to = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let (to_str, value) = match raw_to.strip_suffix(VALUE_SUFFIX) {
            Some(t) => (t.to_string(), true),
            None => (raw_to.clone(), false),
        };
        let (to_str, method) = match to_str.strip_suffix(METHOD_SUFFIX) {
            Some(t) => (t.to_string(), true),
            None => (to_str, false),
        };
        let from_file = from_str.split('#').next().unwrap_or(&from_str);
        let (path_part, fn_name): (Cow<str>, &str) = if to_str.contains("::") {
            let Some((pp, fn_name_maybe_path)) = to_str.split_once("::") else {
                continue;
            };
            // Qualified call like file::mod::foo: use last segment as fn name for resolution.
            if fn_name_maybe_path.contains("::") {
                fn_name_maybe_path
                    .rsplit_once("::")
                    .map(|(path_suffix, name)| (Cow::Owned(format!("{pp}::{path_suffix}")), name))
                    .unwrap_or((Cow::Borrowed(pp), fn_name_maybe_path))
            } else {
                (Cow::Borrowed(pp), fn_name_maybe_path)
            }
        } else {
            // Unqualified placeholder (bare function name): resolve via same-file then global.
            (Cow::Borrowed(from_file), to_str.as_str())
        };
        let resolution = if method {
            resolve_method(fn_name, from_file, &idx)
        } else {
            resolve_placeholder(
                path_part.as_ref(),
                fn_name,
                &from_str,
                from_file,
                value,
                &idx,
            )
        };
        let from_id = NodeId(from_str.clone());
        store.remove_edge(&from_id, &NodeId(raw_to), &calls_type)?;
        match resolution {
            Resolution::One(id) => {
                high.insert((from_str, id.0.clone()));
                store.put_edge(&from_id, &id, &calls_type)?;
            }
            Resolution::Ambiguous(ids) => {
                #[allow(clippy::cast_precision_loss)] // candidate counts are tiny
                let confidence = 1.0 / ids.len() as f64;
                low.extend(ids.into_iter().map(|id| (from_id.clone(), id, confidence)));
            }
            Resolution::None => {}
        }
    }
    for (from, to, confidence) in low {
        if !high.contains(&(from.0.clone(), to.0.clone())) {
            store.put_low_confidence_edge(&from, &to, &calls_type, confidence)?;
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

    #[test]
    fn build_call_graph_resolves_pub_function_via_local() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let real_id = NodeId::new(format!("{path}#10:1"));
        store
            .put_node(&real_id, &NodeType::Function, Some("pub::foo"))
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
            "same-file call to pub::foo should resolve via local map, got {to_str}"
        );
    }

    #[test]
    fn build_call_graph_resolves_qualified_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let real_id = NodeId::new(format!("{path}#20:1"));
        store
            .put_node(&real_id, &NodeType::Function, Some("bar"))
            .unwrap();
        let caller_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&caller_id, &NodeType::Function, Some("main"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::submod::bar"));
        store
            .put_edge(&caller_id, &placeholder, &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert!(
            to_str.contains('#'),
            "qualified path should resolve to real id, got {to_str}"
        );
    }

    #[test]
    fn build_call_graph_resolves_unqualified_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "./src/lib.rs";
        let target = NodeId::new(format!("{path}#10:1"));
        store
            .put_node(&target, &NodeType::Function, Some("callee"))
            .unwrap();
        let origin = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&origin, &NodeType::Function, Some("caller"))
            .unwrap();
        let placeholder = NodeId::new("callee".to_string());
        store
            .put_edge(&origin, &placeholder, &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(
            to_str,
            format!("{path}#10:1"),
            "unqualified placeholder 'callee' should resolve to same-file callee node"
        );
    }

    fn ambiguous_store(placeholder: &str) -> (Store, NodeId, NodeId, NodeId) {
        let store = Store::new_memory().unwrap();
        let caller = NodeId::new("./a.rs#1:1");
        let c1 = NodeId::new("./b.rs#1:1");
        let c2 = NodeId::new("./c.rs#1:1");
        store
            .put_node(&caller, &NodeType::Function, Some("main"))
            .unwrap();
        store
            .put_node(&c1, &NodeType::Function, Some("text"))
            .unwrap();
        store
            .put_node(&c2, &NodeType::Function, Some("text"))
            .unwrap();
        store
            .put_edge(&caller, &NodeId::new(placeholder), &EdgeType::Calls)
            .unwrap();
        (store, caller, c1, c2)
    }

    #[test]
    fn ambiguous_call_becomes_low_confidence_edges() {
        let (store, caller, c1, c2) = ambiguous_store("./a.rs::text");
        build_call_graph(&store).unwrap();
        let rows = store
            .run_query(
                "?[t, c] := *edge_confidence[f, t, \"calls\", c], f = $f",
                std::collections::BTreeMap::from([(
                    "f".to_string(),
                    cozo::DataValue::from(caller.as_str()),
                )]),
            )
            .unwrap();
        assert_eq!(rows.rows.len(), 2);
        assert!(rows.rows.iter().all(|r| r[1].get_float() == Some(0.5)));
        let edges = Query::all_edges(&store).unwrap();
        let tos: Vec<String> = edges.rows.iter().map(|r| r[1].to_string()).collect();
        assert!(tos.iter().any(|t| t.contains(c1.as_str())));
        assert!(tos.iter().any(|t| t.contains(c2.as_str())));
    }

    #[test]
    fn ambiguous_value_reference_is_dropped() {
        let (store, ..) = ambiguous_store("./a.rs::text@value");
        build_call_graph(&store).unwrap();
        let rows = Query::all_edges(&store).unwrap().rows;
        assert_eq!(rows, Vec::<Vec<cozo::DataValue>>::new());
    }

    fn method_store(n: usize, with_free: bool) -> (Store, NodeId) {
        let store = Store::new_memory().unwrap();
        let caller = NodeId::new("./a.rs#1:1");
        store
            .put_node(&caller, &NodeType::Function, Some("main"))
            .unwrap();
        for i in 0..n {
            let imp = NodeId::new(format!("./m{i}.rs#1:1"));
            let f = NodeId::new(format!("./m{i}.rs#2:1"));
            store.put_node(&imp, &NodeType::Impl, Some("T")).unwrap();
            store
                .put_node(&f, &NodeType::Function, Some("len"))
                .unwrap();
            store.put_edge(&imp, &f, &EdgeType::Contains).unwrap();
        }
        if with_free {
            let free = NodeId::new("./a.rs#9:1");
            store
                .put_node(&free, &NodeType::Function, Some("len"))
                .unwrap();
        }
        store
            .put_edge(
                &caller,
                &NodeId::new("./a.rs::len@method"),
                &EdgeType::Calls,
            )
            .unwrap();
        (store, caller)
    }

    fn call_targets_of(store: &Store) -> Vec<String> {
        Query::all_edges(store)
            .unwrap()
            .rows
            .iter()
            .filter(|r| r[2].to_string().contains("calls"))
            .map(|r| r[1].to_string().trim_matches('"').to_string())
            .collect()
    }

    #[test]
    fn method_call_ignores_free_function_of_same_name() {
        let (store, _) = method_store(2, true);
        build_call_graph(&store).unwrap();
        let tos = call_targets_of(&store);
        assert_eq!(tos.len(), 2, "{tos:?}");
        assert!(tos.iter().all(|t| !t.starts_with("./a.rs")), "{tos:?}");
    }

    #[test]
    fn method_call_prefers_same_file_method() {
        let (store, _) = method_store(2, false);
        let imp = NodeId::new("./a.rs#20:1");
        let f = NodeId::new("./a.rs#21:1");
        store.put_node(&imp, &NodeType::Impl, Some("T")).unwrap();
        store
            .put_node(&f, &NodeType::Function, Some("len"))
            .unwrap();
        store.put_edge(&imp, &f, &EdgeType::Contains).unwrap();
        build_call_graph(&store).unwrap();
        assert_eq!(call_targets_of(&store), vec!["./a.rs#21:1".to_string()]);
    }

    #[test]
    fn method_call_fan_out_is_capped() {
        let (store, _) = method_store(super::MAX_METHOD_CANDIDATES, false);
        build_call_graph(&store).unwrap();
        assert_eq!(call_targets_of(&store).len(), super::MAX_METHOD_CANDIDATES);
        let (store, _) = method_store(super::MAX_METHOD_CANDIDATES + 1, false);
        build_call_graph(&store).unwrap();
        assert_eq!(call_targets_of(&store).len(), 0);
    }

    fn put_fn(store: &Store, id: &str, name: &str, parent: &str) -> NodeId {
        let id = NodeId::new(id);
        store
            .put_node(&id, &NodeType::Function, Some(name))
            .unwrap();
        store
            .put_edge(&NodeId::new(parent), &id, &EdgeType::Contains)
            .unwrap();
        id
    }

    #[test]
    fn bare_name_picks_nearest_enclosing_scope() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId::new("./a.rs"), &NodeType::File, None)
            .unwrap();
        let outer = put_fn(&store, "./a.rs#1:1", "view", "./a.rs");
        let a = put_fn(&store, "./a.rs#2:1", "a", "./a.rs");
        let b = put_fn(&store, "./a.rs#3:1", "b", "./a.rs");
        let inner = put_fn(&store, "./a.rs#3:20", "view", "./a.rs#3:1");
        for from in [&a, &b] {
            let ph = NodeId::new("./a.rs::view@value");
            store.put_edge(from, &ph, &EdgeType::Calls).unwrap();
        }
        build_call_graph(&store).unwrap();
        let edges: Vec<(String, String)> = Query::all_edges(&store)
            .unwrap()
            .rows
            .iter()
            .filter(|r| r[2].to_string().contains("calls"))
            .map(|r| {
                (
                    r[0].to_string().trim_matches('"').to_string(),
                    r[1].to_string().trim_matches('"').to_string(),
                )
            })
            .collect();
        assert_eq!(edges.len(), 2, "{edges:?}");
        assert!(edges.contains(&(a.0, outer.0)));
        assert!(edges.contains(&(b.0, inner.0)));
    }

    #[test]
    fn self_path_prefers_own_impl_and_keeps_cfg_variants() {
        let store = Store::new_memory().unwrap();
        let imp = NodeId::new("./a.rs#1:1");
        store.put_node(&imp, &NodeType::Impl, Some("S")).unwrap();
        let f1 = put_fn(&store, "./a.rs#2:1", "f", "./a.rs#1:1");
        let f2 = put_fn(&store, "./a.rs#3:1", "f", "./a.rs#1:1");
        let api = put_fn(&store, "./a.rs#4:1", "api", "./a.rs#1:1");
        let other = NodeId::new("./a.rs#9:1");
        store.put_node(&other, &NodeType::Impl, Some("T")).unwrap();
        put_fn(&store, "./a.rs#10:1", "f", "./a.rs#9:1");
        store
            .put_edge(&api, &NodeId::new("./a.rs::Self::f"), &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let mut tos = call_targets_of(&store);
        tos.sort();
        assert_eq!(tos, vec![f1.0, f2.0]);
    }

    #[test]
    fn module_path_picks_the_named_module() {
        let store = Store::new_memory().unwrap();
        for (m, f) in [("ma", "./a.rs#1:1"), ("mb", "./a.rs#5:1")] {
            store
                .put_node(&NodeId::new(f), &NodeType::Module, Some(m))
                .unwrap();
            put_fn(&store, &format!("{f}0"), "f", f);
        }
        let caller = NodeId::new("./a.rs#9:1");
        store
            .put_node(&caller, &NodeType::Function, Some("caller"))
            .unwrap();
        store
            .put_edge(&caller, &NodeId::new("./a.rs::ma::f"), &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        assert_eq!(call_targets_of(&store), vec!["./a.rs#1:10".to_string()]);
    }

    #[test]
    fn self_path_in_impl_without_type_payload_is_not_matched() {
        let store = Store::new_memory().unwrap();
        for (i, id) in ["./a.rs#1:1", "./a.rs#5:1"].iter().enumerate() {
            store
                .put_node(&NodeId::new(*id), &NodeType::Impl, None)
                .unwrap();
            put_fn(&store, &format!("./a.rs#{}:1", i * 5 + 2), "f", id);
        }
        let api = put_fn(&store, "./a.rs#3:1", "api", "./a.rs#1:1");
        store
            .put_edge(&api, &NodeId::new("./a.rs::Self::f"), &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        assert_eq!(call_targets_of(&store), Vec::<String>::new());
    }
}
