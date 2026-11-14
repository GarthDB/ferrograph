//! Phase 4: call graph construction (enrichment from AST edges).
//!
//! Resolves placeholder call targets (`file_path::fn_name`) to real function node IDs
//! (`file_path#line:col`). Tries same-file first, then global `fn_name` lookup for cross-file calls.
//!
//! Method calls (`x.m()`, placeholders ending in `METHOD_SUFFIX`) first use the receiver type when
//! the AST phase could tell it (`RECEIVER_MARK`: typed parameters and locals, `self.field`, call
//! results; only project structs/enums count): the methods named `m` in that type's impls. Otherwise
//! they resolve by name only, among
//! functions defined inside an `impl`/`trait` (never free functions): same-file methods win, else
//! every method with that name in the project. Several candidates become low-confidence edges
//! (1/N); more than `MAX_METHOD_CANDIDATES` are dropped, so std-like names (`len`, `get`, `push`)
//! cannot flood the graph. A bare name (`f()`) only falls back to free functions. Qualified paths
//! (`Type::m()`, `mod::fn`) are partially handled (see `resolve_call_target` `scoped_identifier`
//! support).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeId, NodeType};

use super::modules::{
    ALIAS_IMPORT_PREFIX, EXTERNAL_IMPORT_PREFIX, GLOB_IMPORT_PREFIX, PATH_MOD_IMPORT_PREFIX,
};

/// Suffix the AST phase puts on call placeholders for paths used as values (see `ast.rs`).
pub(crate) const VALUE_SUFFIX: &str = "@value";
/// Suffix the AST phase puts on method-call placeholders (`x.m()`).
pub(crate) const METHOD_SUFFIX: &str = "@method";
/// Suffix for method calls whose receiver is `self` (`self.m()`): resolved against the enclosing impl first.
pub(crate) const SELF_METHOD_SUFFIX: &str = "@selfmethod";
/// Separator before the receiver-type hint the AST phase appends to a method placeholder:
/// `name@method@on:HINT`, where `HINT` is a recipe for the receiver's type: `Type`, `Self` (the enclosing
/// impl's type), `HINT.field`, `HINT.method()` or `=path::to::f()` (the result of a call). Further steps:
/// `.[]` (element), `.?` (payload), `.#i` (tuple element), `.()` (call a `fn` pointer) and `.m(HINT)` (an
/// `Option` adapter whose closure returns `HINT`); further starts: `@tuple(H0;H1)`,
/// `@some(H)`, `@vec(H)` (a `Vec` of `H`), `@and_then(H)`.
pub(crate) const RECEIVER_MARK: &str = "@on:";
/// Separator in the temporary struct-field placeholder `field@field:Type` (a `Calls` edge from the
/// struct), consumed here to learn field types.
pub(crate) const FIELD_MARK: &str = "@field:";
/// Separator in the temporary return-type placeholder `@ret:Type` (a `Calls` edge from a function).
pub(crate) const RET_MARK: &str = "@ret:";
/// A method name with more candidates than this is dropped at the call site (no edges).
const MAX_METHOD_CANDIDATES: usize = 8;
use crate::graph::{query::Query, unquote_datavalue, Store};

/// True for a `SCREAMING_CASE` name (`MAX`, `BUILTINS`, `ID_2`), the shape of a const/static use.
pub(crate) fn is_const_like(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

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

/// A `use .. as ..` rename seen from outside: (importing file, original name, resolved item id).
type AliasTarget = (String, String, Option<String>);

/// Lookup tables used to resolve call placeholders.
#[derive(Default)]
struct Index {
    local: HashMap<(String, String), Vec<NodeId>>,
    imports_map: HashMap<String, Vec<String>>,
    /// (file, name) pairs imported from other crates (`use serde_json::from_str`).
    external_imports: HashSet<(String, String)>,
    /// (file, alias) -> original name of `use .. as ..` renames.
    aliases: HashMap<(String, String), String>,
    /// (function id, alias) -> original name of renames declared inside a function body.
    local_aliases: HashMap<(String, String), String>,
    /// (function id, alias) pairs renamed to different items in one body: they name none, and hide
    /// a file-level rename of the same name.
    local_conflicts: HashSet<(String, String)>,
    /// alias -> (file, original name, resolved item) of the same renames, for qualified calls.
    aliases_by_name: HashMap<String, Vec<AliasTarget>>,
    /// File -> files of the project modules it glob-imports (`use super::*`).
    globs: HashMap<String, Vec<String>>,
    /// Module name -> files declared with `#[path = ".."] mod name;`.
    path_mods: HashMap<String, Vec<String>>,
    node_id_to_payload: HashMap<String, String>,
    global_by_name: HashMap<String, Vec<NodeId>>,
    /// Ids of functions defined directly inside an `impl` or `trait`.
    methods: HashSet<String>,
    /// `contains` edges, child id -> parent id.
    parent: HashMap<String, String>,
    /// Consts and statics by (file, name).
    consts: HashMap<(String, String), Vec<NodeId>>,
    consts_by_name: HashMap<String, Vec<NodeId>>,
    /// Scope-forming nodes by id: (is `impl`/`trait`, payload). `mod` blocks have `false`.
    scopes: HashMap<String, (bool, String)>,
    /// Names of the project's structs and enums, the only receiver types we trust.
    type_names: HashSet<String>,
    /// Struct/enum id -> name, to attribute field placeholders.
    type_by_id: HashMap<String, String>,
    /// (struct name, field) -> plain types declared for it (several if same-named structs differ).
    fields: HashMap<(String, String), HashSet<String>>,
    /// Function id -> its plain return type (`Self` for `-> Self`).
    rets: HashMap<String, String>,
}

impl Index {
    /// Enclosing items of `from`, innermost first, starting with `from` itself.
    fn ancestors<'a>(&'a self, from: &'a str) -> impl Iterator<Item = &'a str> {
        std::iter::successors(Some(from), |n| self.parent.get(*n).map(String::as_str))
    }

    /// The original name `name` was renamed from, seen from `from`: renames inside the enclosing
    /// functions win over file-level ones.
    fn alias_of(&self, file: &str, from: &str, name: &str) -> Option<&String> {
        let key = |a: &str| (a.to_string(), name.to_string());
        if self
            .ancestors(from)
            .any(|a| self.local_conflicts.contains(&key(a)))
        {
            return None;
        }
        self.ancestors(from)
            .find_map(|a| self.local_aliases.get(&key(a)))
            .or_else(|| self.aliases.get(&(file.to_string(), name.to_string())))
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
/// `self.m()`: the methods named `m` of the impls/trait enclosing the caller, i.e. of its `Self` type
/// (every impl block of that type, in any file). `None` when the type is unknown or has no such
/// method, so the caller falls back to the name-based lookup (`Deref`, trait methods, ..).
/// Same-named types in different files: a single match in the caller's own file wins.
fn resolve_self_method(name: &str, from: &str, idx: &Index) -> Option<Resolution> {
    let all = methods_of_type(name, enclosing_impl_type(from, idx)?, idx);
    let prefix = format!("{}#", from.split('#').next().unwrap_or(from));
    let mut own = all.iter().filter(|c| c.as_str().starts_with(&prefix));
    if let (Some(c), None) = (own.next(), own.next()) {
        return Some(Resolution::One(c.clone()));
    }
    resolution_of(all)
}

/// The `Self` type of the impl/trait enclosing `from`.
fn enclosing_impl_type<'a>(from: &str, idx: &'a Index) -> Option<&'a str> {
    let ty = idx
        .ancestors(from)
        .find_map(|a| idx.scopes.get(a).filter(|s| s.0).map(|s| s.1.as_str()))?;
    (!ty.is_empty() && ty != "null").then_some(ty)
}

/// The methods named `name` in any impl block of the type `ty`.
fn methods_of_type(name: &str, ty: &str, idx: &Index) -> Vec<NodeId> {
    let all = idx.global_by_name.get(name).map_or(&[][..], Vec::as_slice);
    all.iter()
        .filter(|c| idx.methods.contains(c.as_str()))
        .filter(|c| {
            idx.parent_of(c)
                .and_then(|p| idx.scopes.get(p))
                .is_some_and(|s| s.0 && s.1 == ty)
        })
        .cloned()
        .collect()
}

/// Type named by a hint (see `RECEIVER_MARK`), following fields and the return types of calls.
/// `None` when a step is not a project struct/enum, has no (or several different) such field or
/// method, or the callee cannot be told: generics, trait objects, std types stay unknown.
fn eval_hint(hint: &str, from: &str, from_file: &str, idx: &Index) -> Option<String> {
    let mut parts = split_top(hint, '.');
    let first = parts.next()?;
    let mut ty = if let Some(call) = first.strip_prefix('=') {
        call_return_type(call.strip_suffix("()")?, from, from_file, idx)?
    } else if let Some(elems) = first.strip_prefix("@tuple(") {
        // Elements that cannot be typed stay `_` so the positions line up.
        let elems: Vec<String> = split_top(elems.strip_suffix(')')?, ';')
            .map(|e| {
                Some(e)
                    .filter(|e| !e.is_empty())
                    .and_then(|e| eval_hint(e, from, from_file, idx))
                    .unwrap_or_else(|| "_".to_string())
            })
            .collect();
        format!("({})", elems.join(","))
    } else if let Some(inner) = first.strip_prefix("@some(") {
        format!(
            "Option<{}>",
            eval_hint(inner.strip_suffix(')')?, from, from_file, idx)?
        )
    } else if let Some(inner) = first.strip_prefix("@vec(") {
        format!(
            "Vec<{}>",
            eval_hint(inner.strip_suffix(')')?, from, from_file, idx)?
        )
    } else if let Some(inner) = first.strip_prefix("@and_then(") {
        let ty = eval_hint(inner.strip_suffix(')')?, from, from_file, idx)?;
        split_generic(&ty).filter(|(name, _)| matches!(*name, "Option" | "Result"))?;
        ty
    } else if first == "Self" {
        enclosing_impl_type(from, idx)?.to_string()
    } else if let Some(variant) = first.strip_prefix('%') {
        // `%Variant#i`: payload of a tuple variant or struct named by a bare path.
        // Several same-named variants in the file (`Regex(..)` in two enums): the enclosing impl's own enum.
        // shortcut: wrong if the impl matches a different enum (`use Other::*`) that shares the variant name; carry the scrutinee type to fix.
        field_type(from_file, variant, idx)
            .or_else(|| field_type(enclosing_impl_type(from, idx)?, variant, idx))
            .or_else(|| field_type("", variant, idx))?
    } else {
        first.to_string()
    };
    for part in parts {
        // `m(F)`: an `Option`/`Result` adapter whose closure returns `F`; a project type's own
        // method of that name is a plain method step.
        if let Some((m, closure)) = part
            .split_once('(')
            .filter(|(_, c)| !c.starts_with(')'))
            .and_then(|(m, c)| Some((m, c.strip_suffix(')')?)))
        {
            // A payload that is not a kept type leaves a bare `Option`/`Result`.
            let ty_ref = auto_deref(&ty, idx);
            let name = split_generic(ty_ref).map_or(ty_ref, |(name, _)| name);
            if matches!(name, "Option" | "Result") {
                let name = name.to_string();
                let result = eval_hint(closure, from, from_file, idx)?;
                ty = if m == "map" {
                    format!("{name}<{result}>")
                } else {
                    result
                };
                continue;
            }
            ty = step_type(&ty, &format!("{m}()"), idx)?;
            continue;
        }
        ty = step_type(&ty, part, idx)?;
    }
    Some(ty)
}

/// Split a hint on `sep` outside parentheses.
fn split_top(s: &str, sep: char) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    let mut start = 0;
    let mut cuts = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                cuts.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    cuts.push((start, s.len()));
    cuts.into_iter().map(move |(a, b)| &s[a..b])
}

/// Std locks and guards; a project type of the same name is not one of them.
/// The guard `lock()`/`borrow()` yield: not a valid identifier, so no project type is named like it.
const SYNTH_GUARD: &str = "$Guard";
const GUARDS: [&str; 6] = [
    SYNTH_GUARD,
    "MutexGuard",
    "RwLockReadGuard",
    "RwLockWriteGuard",
    "Ref",
    "RefMut",
];
const LOCKS: [&str; 3] = ["Mutex", "RwLock", "RefCell"];

/// Strip `Box`/`Rc`/`Arc` and lock guards, which method calls see through. A project type named like
/// a lock or guard is returned bare, without its generics.
fn auto_deref<'a>(mut ty: &'a str, idx: &Index) -> &'a str {
    while let Some((name, inner)) = split_generic(ty) {
        if GUARDS.contains(&name) {
            if idx.type_names.contains(name) {
                return name;
            }
        } else if LOCKS.contains(&name) {
            return if idx.type_names.contains(name) {
                name
            } else {
                ty
            };
        } else if !matches!(name, "Box" | "Rc" | "Arc") {
            break;
        }
        ty = inner;
    }
    ty
}

/// `Name<Inner>` into (`Name`, `Inner`).
fn split_generic(ty: &str) -> Option<(&str, &str)> {
    let (name, rest) = ty.split_once('<')?;
    Some((name, rest.strip_suffix('>')?))
}

fn is_std_guard(ty: &str, idx: &Index) -> bool {
    split_generic(ty).is_some_and(|(n, _)| GUARDS.contains(&n) && !idx.type_names.contains(n))
}

/// One step of a hint: `[]` (element), `?` (`Option`/`Result` payload), `m()` (method) or a field.
fn step_type(ty: &str, part: &str, idx: &Index) -> Option<String> {
    // `lock().unwrap()`: the `Result` of `lock()` is not modelled, so unwrapping keeps the guard.
    if matches!(
        part,
        "unwrap()" | "expect()" | "unwrap_or_else()" | "unwrap_or_default()"
    ) && is_std_guard(ty, idx)
    {
        return Some(ty.to_string());
    }
    // tokio's `lock().await` is the guard; the future of anything else is not modelled.
    if part == "await" {
        return is_std_guard(ty, idx).then(|| ty.to_string());
    }
    let ty = auto_deref(ty, idx);
    if ty.starts_with('(') {
        return tuple_elem(ty, part.strip_prefix('#')?.parse().ok()?);
    }
    match part {
        // Iterating a map yields `(K, V)` pairs; the key type is not kept.
        "[]" => split_generic(ty).map(|(name, inner)| {
            if name.ends_with("Map") {
                format!("(_,{inner})")
            } else {
                inner.to_string()
            }
        }),
        // Calling a `fn` pointer (`fn<R>`).
        "()" => split_generic(ty)
            .filter(|(name, _)| *name == "fn")
            .map(|(_, ret)| ret.to_string()),
        "[i]" => split_generic(ty).map(|(_, inner)| inner.to_string()),
        "?" => split_generic(ty)
            .filter(|(name, _)| matches!(*name, "Option" | "Result"))
            .map(|(_, inner)| inner.to_string()),
        _ => {
            let is_project_type = idx.type_names.contains(ty);
            match part.strip_suffix("()") {
                Some(m) if is_project_type => {
                    let fns = methods_of_type(m, ty, idx);
                    if fns.is_empty() && m == "clone" {
                        return Some(ty.to_string());
                    }
                    return_type_of(&fns, ty, idx)
                }
                Some(m) => container_method(ty, m),
                None if is_project_type => field_type(ty, part, idx),
                None => None,
            }
        }
    }
}

/// Element `i` of the tuple type `(A,B,..)` (`None` for `_`).
fn tuple_elem(ty: &str, i: usize) -> Option<String> {
    let inner = ty.strip_prefix('(')?.strip_suffix(')')?;
    let (mut depth, mut start, mut n) = (0usize, 0, 0);
    for (k, c) in inner.char_indices().chain([(inner.len(), ',')]) {
        match c {
            '(' | '<' => depth += 1,
            ')' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if n == i {
                    return Some(inner[start..k].to_string()).filter(|t| t != "_");
                }
                start = k + 1;
                n += 1;
            }
            _ => {}
        }
    }
    None
}

/// Result type of the std method `m` on the container type `ty`: adapters keep the container, element
/// accessors yield `Option<Elem>`, `unwrap` and friends yield the payload.
fn container_method(ty: &str, m: &str) -> Option<String> {
    let (name, inner) = split_generic(ty)?;
    if name.ends_with("Map") {
        return match m {
            "iter" | "iter_mut" | "into_iter" | "drain" => Some(format!("Vec<(_,{inner})>")),
            "values" | "values_mut" | "into_values" => Some(format!("Vec<{inner}>")),
            "get" | "get_mut" => Some(format!("Option<{inner}>")),
            "clone" | "as_ref" | "as_mut" | "borrow" | "borrow_mut" => Some(ty.to_string()),
            _ => None,
        };
    }
    // `lock()` is typed as the guard itself, so it covers `parking_lot` as well as `.lock().unwrap()`.
    match (name, m) {
        ("Mutex" | "RwLock", "lock" | "read" | "write") => {
            return Some(format!("{SYNTH_GUARD}<{inner}>"))
        }
        // `parking_lot` returns `Option<Guard>` and std `Result<Guard>`; `?`, `unwrap` and `Some(g)`/`Ok(g)`
        // patterns treat the two alike.
        ("Mutex", "try_lock") | ("RwLock", "try_read" | "try_write") => {
            return Some(format!("Option<{SYNTH_GUARD}<{inner}>>"))
        }
        ("RefCell", "borrow" | "borrow_mut") => return Some(format!("{SYNTH_GUARD}<{inner}>")),
        // `get_mut` borrows the cell directly: `&mut T`, or `LockResult<&mut T>`.
        ("RefCell", "get_mut") => return Some(inner.to_string()),
        ("Mutex" | "RwLock", "get_mut") => return Some(format!("Result<{inner}>")),
        _ => {}
    }
    let wrapper = matches!(name, "Option" | "Result");
    match m {
        "iter" | "iter_mut" | "into_iter" | "as_ref" | "as_mut" | "as_deref" | "as_deref_mut"
        | "as_slice" | "as_mut_slice" | "drain" | "rev" | "filter" | "skip" | "take"
        | "step_by" | "cloned" | "copied" | "by_ref" | "peekable" | "skip_while" | "take_while"
        | "chain" | "clone" | "to_vec" | "values" | "values_mut" | "into_values" | "borrow"
        | "borrow_mut" => Some(ty.to_string()),
        "first" | "last" | "get" | "get_mut" | "first_mut" | "last_mut" | "pop" | "pop_front"
        | "pop_back" | "front" | "back" | "front_mut" | "back_mut" | "next" | "next_back"
        | "find" | "peek" | "nth" | "min" | "max" | "min_by_key" | "max_by_key" | "min_by"
        | "max_by"
            if !wrapper =>
        {
            Some(format!("Option<{inner}>"))
        }
        "unwrap" | "expect" | "unwrap_or" | "unwrap_or_else" | "unwrap_or_default" if wrapper => {
            Some(inner.to_string())
        }
        _ => None,
    }
}

/// The one plain type declared for field `field` of the struct `ty`. For payloads (`Variant#i`) `ty`
/// may also be `""` or a file, meaning every variant so named in the project or in that file.
fn field_type(ty: &str, field: &str, idx: &Index) -> Option<String> {
    let mut tys = idx.fields.get(&(ty.to_string(), field.to_string()))?.iter();
    match (tys.next(), tys.next()) {
        (Some(t), None) => Some(t.clone()),
        _ => None,
    }
}

/// The one return type shared by `fns` (`Self` stands for `self_ty`).
fn return_type_of(fns: &[NodeId], self_ty: &str, idx: &Index) -> Option<String> {
    let mut tys = fns
        .iter()
        .map(|f| idx.rets.get(f.as_str()).map(String::as_str));
    let first = tys.next()??;
    tys.all(|t| t == Some(first)).then(|| {
        if first == "Self" {
            self_ty.to_string()
        } else {
            first.replace("<Self>", &format!("<{self_ty}>"))
        }
    })
}

/// Return type of the call `path()` (`f`, `Type::f`, `Self::f`, `module::f`).
fn call_return_type(path: &str, from: &str, from_file: &str, idx: &Index) -> Option<String> {
    let mut segs = path.rsplit("::");
    let name = segs.next()?;
    let owner = match segs.next() {
        Some("Self") => Some(enclosing_impl_type(from, idx)?),
        o => o,
    };
    if let Some(ty) = owner.filter(|o| idx.type_names.contains(*o)) {
        let mut fns = methods_of_type(name, ty, idx);
        // Types of one name in several modules (`input::keycaps::Overlay`, `overlay::Overlay`): the
        // module in the path picks the one meant.
        if let Some(module) = segs
            .next()
            .filter(|m| !matches!(*m, "crate" | "self" | "super"))
        {
            let named: Vec<NodeId> = fns
                .iter()
                .filter(|c| file_is_module(c.as_str().split('#').next().unwrap_or(""), module))
                .cloned()
                .collect();
            if !named.is_empty() {
                fns = named;
            }
        }
        return return_type_of(&fns, ty, idx);
    }
    // `Vec::new()`, `fs::read()`, an imported `args()`: library paths must not pick a project fn
    // that merely shares the name.
    let by_module = |c: &NodeId| {
        owner.is_none_or(|o| {
            matches!(o, "crate" | "self" | "super")
                || file_is_module(c.as_str().split('#').next().unwrap_or(""), o)
                || idx
                    .parent_of(c)
                    .and_then(|p| idx.scopes.get(p))
                    .is_some_and(|s| !s.0 && s.1 == o)
        })
    };
    if owner.is_none()
        && idx
            .external_imports
            .contains(&(from_file.to_string(), name.to_string()))
    {
        return None;
    }
    let mut free: Vec<NodeId> = idx
        .global_by_name
        .get(name)?
        .iter()
        .filter(|c| !idx.methods.contains(c.as_str()) && by_module(c))
        .cloned()
        .collect();
    let prefix = format!("{from_file}#");
    let key = crate_key(from_file);
    if free.iter().any(|c| c.as_str().starts_with(&prefix)) {
        free.retain(|c| c.as_str().starts_with(&prefix));
    } else if free.iter().any(|c| crate_key(c.as_str()) == key) {
        free.retain(|c| crate_key(c.as_str()) == key);
    }
    return_type_of(&free, "", idx).filter(|t| !t.is_empty())
}

/// `recv.name()` whose receiver type the AST phase could tell (`hint`, see `RECEIVER_MARK`). Only
/// project structs/enums count. `None` (type unknown, or it has no such method: `Deref`, trait
/// default methods) falls back to the name lookup.
fn resolve_typed_method(
    name: &str,
    hint: &str,
    from: &str,
    from_file: &str,
    idx: &Index,
) -> Option<Resolution> {
    let ty = eval_hint(hint, from, from_file, idx)?;
    // `Rc::clone`/`Arc::clone` are not the inner type's `clone`.
    if name == "clone" && (ty.starts_with("Rc<") || ty.starts_with("Arc<")) {
        return None;
    }
    let ty = auto_deref(&ty, idx);
    if !idx.type_names.contains(ty) {
        return None;
    }
    let mut cands = methods_of_type(name, ty, idx);
    // Several types may share a name (one per example or test file): the one declared next to the
    // call is the likeliest.
    let prefix = format!("{from_file}#");
    if cands.iter().any(|c| c.as_str().starts_with(&prefix)) {
        cands.retain(|c| c.as_str().starts_with(&prefix));
    }
    let key = crate_key(from_file);
    if cands.iter().any(|c| crate_key(c.as_str()) == key) {
        cands.retain(|c| crate_key(c.as_str()) == key);
    }
    resolution_of(cands)
}

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

/// Resolve a const/static use (`NAME`, `Type::NAME`). Same file first, with an exact scope or
/// owner match (`u32::MAX` must not link to a local `const MAX`). A bare name that is not in the
/// file links to the project's only const/static of that name: `SCREAMING_CASE` names are never
/// locals, and this is how `use`d consts from other files are found.
fn resolve_const(path_part: &str, name: &str, from: &str, idx: &Index) -> Option<NodeId> {
    let qualified = path_part.contains("::");
    let file = path_part.split("::").next().unwrap_or(path_part);
    if let Some(cands) = idx.consts.get(&(file.to_string(), name.to_string())) {
        let picked = if qualified {
            let owner = path_part.rsplit("::").next().unwrap_or(path_part);
            pick_by_owner(owner, from, cands, idx)
        } else {
            nearest_in_scope(from, cands, idx).unwrap_or_default()
        };
        if let [one] = picked.as_slice() {
            return Some(one.clone());
        }
    }
    match idx.consts_by_name.get(name).map(Vec::as_slice) {
        Some([one]) if !qualified => Some(one.clone()),
        _ => None,
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
        local, imports_map, ..
    } = idx;
    if !path_part.contains("::") {
        if let Some(orig) = idx
            .alias_of(from_file, from, fn_name)
            .filter(|o| idx.alias_of(from_file, from, o).is_none())
        {
            return resolve_placeholder(path_part, orig, from, from_file, value, idx);
        }
    }
    if path_part.contains("::") {
        // TODO: qualified path resolution only uses file-level (first path segment); module path within the file (e.g. mod::submod::foo()) is not considered.
        let file_only = path_part.split("::").next().unwrap_or(path_part);
        let key = (file_only.to_string(), fn_name.to_string());
        if let Some(candidates) = local.get(&key) {
            let owner = path_part.rsplit("::").next().unwrap_or(path_part);
            // A lone same-file candidate only counts if the owner is relative (`crate`/`self`/`super`)
            // or names it: `HaystackBuilder::new()` must not bind to an unrelated `new` in this file.
            if let ([one], true) = (
                candidates.as_slice(),
                matches!(owner, "crate" | "self" | "super"),
            ) {
                return Resolution::One(one.clone());
            }
            let mut picked = pick_by_owner(owner, from, candidates, idx);
            // `use orig as owner;` renames a module or type declared in this file.
            if let Some(orig) = idx.alias_of(from_file, from, owner) {
                picked.extend(pick_by_owner(orig, from, candidates, idx));
            }
            // `#[cfg(..)] #[path = ".."] mod owner;` declared once per platform next to an inline `mod owner`.
            if !picked.is_empty() {
                picked.extend(path_mod_fns(owner, fn_name, from_file, idx));
            }
            if let Some(r) = resolution_of(picked) {
                return r;
            }
        }
        // `m::alias()` where module `m` re-exports `orig as alias` (#117).
        if let Some(r) = resolve_module_alias(path_part, fn_name, from, value, idx) {
            return r;
        }
        // Not in this file: the owner may be a type or module defined elsewhere in the project (#106).
        // Paths into external crates (`std::collections::HashMap`) find no owner and stay unresolved.
        let segs: Vec<&str> = path_part.split("::").skip(1).collect();
        return resolve_qualified_elsewhere(&segs, fn_name, from, from_file, idx);
    }
    let key = (path_part.to_string(), fn_name.to_string());
    // A bare name never calls a method of the same file (`helper()` is not `S::helper`).
    let free_here: Vec<NodeId> = local
        .get(&key)
        .into_iter()
        .flatten()
        .filter(|n| !idx.methods.contains(n.as_str()))
        .cloned()
        .collect();
    if !free_here.is_empty() {
        let candidates = &free_here;
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
    if let Some(r) = imports_map
        .get(from_file)
        .and_then(|imported| resolve_via_named_imports(imported, fn_name, idx))
    {
        return r;
    }
    if let Some(r) = resolve_via_globs(fn_name, from_file, value, idx) {
        return r;
    }
    // The `use` paths did not lead to the callee (re-exports, nested modules): fall back to the
    // project-wide free-function lookup rather than dropping the call. The callee may also
    // be an external function, so even a unique match is only a low-confidence edge. Imports of
    // a resolved item (const, fn, type) cannot bring the callee into scope, only a module-level import
    // (glob, or an item that was not found in its module) can.
    let module_import = imports_map
        .get(from_file)
        .is_some_and(|imported| imported.iter().any(|t| !t.contains('#')));
    resolve_bare_globally(fn_name, from_file, value, module_import, idx)
}

/// The callee of a bare call among the items `use` statements import into the file: `One` when exactly
/// one imported function has the name (or exactly one function of that name lives in an imported
/// module file), `None` (the resolution) when two do, `None` (no answer) when none does.
fn resolve_via_named_imports(
    imported: &[String],
    fn_name: &str,
    idx: &Index,
) -> Option<Resolution> {
    let mut candidate: Option<NodeId> = None;
    for to_id in imported {
        let found = if to_id.contains('#') {
            idx.node_id_to_payload
                .get(to_id)
                .filter(|payload| canonical_name(payload) == fn_name)
                .map(|_| NodeId(to_id.clone()))
        } else {
            let in_file: Vec<_> = idx
                .global_by_name
                .get(fn_name)
                .into_iter()
                .flatten()
                .filter(|n| n.as_str().starts_with(&format!("{to_id}#")))
                .collect();
            match in_file.as_slice() {
                [one] => Some((*one).clone()),
                _ => None,
            }
        };
        if let Some(found) = found {
            if candidate.is_some() {
                return Some(Resolution::None);
            }
            candidate = Some(found);
        }
    }
    candidate.map(Resolution::One)
}

/// A bare call that no `use` item and no local definition explains, while the file glob-imports
/// project modules (`use super::*`): the callee is the one free function of that name declared at
/// the top level of those modules. `None` unless exactly one such function exists.
fn resolve_via_globs(
    fn_name: &str,
    from_file: &str,
    value: bool,
    idx: &Index,
) -> Option<Resolution> {
    let files = idx.globs.get(from_file).filter(|_| !value)?;
    if idx
        .external_imports
        .contains(&(from_file.to_string(), fn_name.to_string()))
    {
        return None;
    }
    let mut found: Vec<&NodeId> = idx
        .global_by_name
        .get(fn_name)?
        .iter()
        .filter(|n| !idx.methods.contains(n.as_str()))
        .filter(|n| {
            let file = n.as_str().split('#').next().unwrap_or("");
            files.iter().any(|f| f == file) && idx.parent_of(n) == Some(file)
        })
        .collect();
    found.dedup();
    match found.as_slice() {
        [one] => Some(Resolution::One((*one).clone())),
        _ => None,
    }
}

/// `owner::alias()` where the file of module `owner` has an `alias::alias=>orig` import: resolve
/// `orig` through that file's own imports. `None` unless exactly one such module file exists.
/// The edge is certain only when the path's leading segments (`a::b::owner`) also appear in that
/// file's path; otherwise `somecrate::util::alias()` could be an external call, so it is a guess.
fn resolve_module_alias(
    path_part: &str,
    alias: &str,
    from: &str,
    value: bool,
    idx: &Index,
) -> Option<Resolution> {
    let mut segs = path_part.split("::").skip(1).collect::<Vec<_>>();
    let owner = segs.pop()?;
    if matches!(owner, "crate" | "self" | "super") {
        return None;
    }
    let mut hits = idx
        .aliases_by_name
        .get(alias)?
        .iter()
        .filter(|(file, ..)| is_module_file(file, owner));
    let ((file, orig, target), None) = (hits.next()?, hits.next()) else {
        return None;
    };
    let confirmed = segs.iter().all(|s| {
        matches!(*s, "crate" | "self" | "super")
            || std::path::Path::new(file)
                .components()
                .any(|c| c.as_os_str().to_str().map(|c| c.trim_end_matches(".rs")) == Some(s))
    });
    let resolved = match target {
        Some(item) => Resolution::One(NodeId::new(item.clone())),
        None => resolve_placeholder(file, orig, from, file, value, idx),
    };
    match resolved {
        Resolution::None => None,
        Resolution::One(n) if !confirmed => Some(Resolution::Ambiguous(vec![n])),
        r => Some(r),
    }
}

/// `file` is exactly the file of module `name`: `name.rs` or `name/mod.rs`.
fn is_module_file(file: &str, name: &str) -> bool {
    let path = std::path::Path::new(file);
    let stem = path.file_stem().and_then(|s| s.to_str());
    stem == Some(name)
        || (stem == Some("mod")
            && path
                .parent()
                .and_then(|d| d.file_name())
                .is_some_and(|d| d == name))
}

fn resolve_bare_globally(
    fn_name: &str,
    from_file: &str,
    value: bool,
    imports_unresolved: bool,
    idx: &Index,
) -> Resolution {
    let Index { global_by_name, .. } = idx;
    // Imported from another crate: not a local function, whatever local functions share the name (#112).
    if idx
        .external_imports
        .contains(&(from_file.to_string(), fn_name.to_string()))
    {
        return Resolution::None;
    }
    if value {
        return Resolution::None;
    }
    // A bare name cannot call a method (`Type::new` keeps its path), so only free functions count.
    let free: Vec<&NodeId> = global_by_name
        .get(fn_name)
        .into_iter()
        .flatten()
        .filter(|n| !idx.methods.contains(n.as_str()))
        .collect();
    let key = crate_key(from_file);
    let same_crate: Vec<&NodeId> = free
        .iter()
        .filter(|n| key.is_none() || crate_key(n.as_str()) == key)
        .copied()
        .collect();
    let free = if same_crate.is_empty() {
        free
    } else {
        same_crate
    };
    match free.as_slice() {
        [] => Resolution::None,
        [one] if !imports_unresolved => Resolution::One((*one).clone()),
        many => Resolution::Ambiguous(many.iter().map(|n| (*n).clone()).collect()),
    }
}

/// Source root of a file: the path through its `/src/`, so `crate::`/`super::` paths stay inside one
/// crate. `None` for files outside any `src/` (tests, examples), which are then unrestricted.
fn crate_key(file: &str) -> Option<&str> {
    file.find("/src/").map(|i| &file[..i + 5])
}

/// Whether `file` belongs to the module `name`: it is `name.rs`, `name/mod.rs`, or sits inside a
/// `name/` directory (items re-exported through `pub use`, e.g. `derives::f` defined in `derives/x.rs`).
fn file_is_module(file: &str, name: &str) -> bool {
    let path = std::path::Path::new(file);
    let stem = path.file_stem().and_then(|s| s.to_str());
    stem == Some(name)
        || path
            .parent()
            .is_some_and(|d| d.components().any(|c| c.as_os_str() == name))
}

/// Number of leading path components `a` and `b` share.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

/// Resolve a qualified call whose owner is not in the calling file (`Type::f`, `module::f`,
/// `crate::f`, `super::f`, `self::f`) among the whole project. The owner (last path segment) picks
/// an impl/trait/`mod` block by name or a module file by name; `crate`/`super`/`self` pick free
/// functions of the same crate. A unique match is a certain edge, several are low-confidence.
fn resolve_qualified_elsewhere(
    segs: &[&str],
    fn_name: &str,
    from: &str,
    from_file: &str,
    idx: &Index,
) -> Resolution {
    let (Some(all), Some(&owner)) = (idx.global_by_name.get(fn_name), segs.last()) else {
        return Resolution::None;
    };
    // `use Orig as owner;` names the type or module `Orig`.
    let owner = idx
        .alias_of(from_file, from, owner)
        .map_or(owner, String::as_str);
    let relative = matches!(owner, "crate" | "super" | "self");
    // `m::f` where the calling file declares `mod m;`: its file, not any inline `mod m` elsewhere.
    if !relative {
        let own = child_module_fns(owner, from_file, all, idx);
        if !own.is_empty() {
            return resolution_of(own).unwrap_or(Resolution::None);
        }
    }
    let mut module_scoped = false;
    let mut cands: Vec<NodeId> = if relative {
        all.iter()
            .filter(|c| !idx.methods.contains(c.as_str()))
            .cloned()
            .collect()
    } else {
        let mut by_scope = pick_by_owner(owner, from, all, idx);
        // `a::Type::f`: of several same-named types, the one defined in module `a`.
        if let [.., module, _] = segs {
            let in_module: Vec<NodeId> = by_scope
                .iter()
                .filter(|c| file_is_module(file_of(c), module))
                .cloned()
                .collect();
            if !matches!(*module, "crate" | "super" | "self") && in_module.len() == 1 {
                by_scope = in_module;
            }
        }
        if by_scope.is_empty() {
            module_scoped = true;
            all.iter()
                .filter(|c| !idx.methods.contains(c.as_str()))
                .filter(|c| {
                    file_is_module(file_of(c), owner) || is_path_mod_file(owner, file_of(c), idx)
                })
                .cloned()
                .collect()
        } else {
            by_scope
        }
    };
    let key = crate_key(from_file);
    let same_crate: Vec<NodeId> = cands
        .iter()
        .filter(|c| key.is_none() || crate_key(c.as_str()) == key)
        .cloned()
        .collect();
    if !same_crate.is_empty() {
        cands = same_crate;
    }
    if module_scoped && cands.len() > 1 {
        cands = narrow_to_module_file(owner, fn_name, cands, key, idx);
    }
    if relative && cands.len() > 1 {
        let near = |c: &NodeId| common_prefix_len(c.as_str(), from_file);
        let best = cands.iter().map(near).max().unwrap_or(0);
        cands.retain(|c| near(c) == best);
    }
    if cands.is_empty() && !relative && owner != "Self" {
        // Unknown owner (a `use .. as` alias, a re-exported type): fall back to the same-named
        // items of the crate, as low-confidence edges, unless the name is too common to mean anything.
        cands = all
            .iter()
            .filter(|c| key.is_none() || crate_key(c.as_str()) == key)
            .cloned()
            .collect();
        if cands.len() > MAX_METHOD_CANDIDATES {
            return Resolution::None;
        }
        return match cands.len() {
            0 => Resolution::None,
            _ => Resolution::Ambiguous(cands),
        };
    }
    resolution_of(cands).unwrap_or(Resolution::None)
}

/// Among `cands`, the top-level free functions of the file of `from_file`'s child module `owner`
/// (`dir/owner.rs` or `dir/owner/mod.rs`, `dir` being where the module's children live).
fn child_module_fns(owner: &str, from_file: &str, cands: &[NodeId], idx: &Index) -> Vec<NodeId> {
    let path = std::path::Path::new(from_file);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = if matches!(stem, "mod" | "lib" | "main") {
        path.parent().map(std::path::Path::to_path_buf)
    } else {
        path.parent().map(|d| d.join(stem))
    };
    let Some(dir) = dir else {
        return Vec::new();
    };
    let files = [
        dir.join(format!("{owner}.rs")),
        dir.join(owner).join("mod.rs"),
    ];
    cands
        .iter()
        .filter(|c| !idx.methods.contains(c.as_str()))
        .filter(|c| crate_key(from_file).is_none() || crate_key(file_of(c)) == crate_key(from_file))
        .filter(|c| {
            let file = file_of(c);
            idx.parent_of(c) == Some(file) && files.iter().any(|f| f.to_str() == Some(file))
        })
        .cloned()
        .collect()
}

/// Top-level free functions named `fn_name` in the files `#[path = ".."] mod name;` declares, within
/// the crate of `from_file`.
fn path_mod_fns(name: &str, fn_name: &str, from_file: &str, idx: &Index) -> Vec<NodeId> {
    let key = crate_key(from_file);
    idx.global_by_name
        .get(fn_name)
        .into_iter()
        .flatten()
        .filter(|c| !idx.methods.contains(c.as_str()))
        .filter(|c| is_path_mod_file(name, file_of(c), idx) && idx.parent_of(c) == Some(file_of(c)))
        .filter(|c| key.is_none() || crate_key(file_of(c)) == key)
        .cloned()
        .collect()
}

fn file_of(id: &NodeId) -> &str {
    id.as_str().split('#').next().unwrap_or("")
}

/// `file` is declared as module `name` by `#[path = ".."] mod name;`.
fn is_path_mod_file(name: &str, file: &str, idx: &Index) -> bool {
    idx.path_mods
        .get(name)
        .is_some_and(|files| files.iter().any(|f| f == file))
}

fn is_file_of_module(name: &str, file: &str, idx: &Index) -> bool {
    is_module_file(file, name) || is_path_mod_file(name, file, idx)
}

/// `module::f` with candidates spread over the files below a module directory (platform variants,
/// re-exported items) and nested `mod` blocks: `f` is the free function the module's own file
/// declares at its top level, else the one that file re-exports. Unchanged when neither is unique.
fn narrow_to_module_file(
    module: &str,
    fn_name: &str,
    cands: Vec<NodeId>,
    key: Option<&str>,
    idx: &Index,
) -> Vec<NodeId> {
    let own: Vec<NodeId> = cands
        .iter()
        .filter(|c| {
            let file = file_of(c);
            is_file_of_module(module, file, idx) && idx.parent_of(c) == Some(file)
        })
        .cloned()
        .collect();
    if !own.is_empty() {
        return own;
    }
    let mut reexported: Vec<NodeId> = idx
        .imports_map
        .iter()
        .filter(|(file, _)| is_file_of_module(module, file, idx))
        .filter(|(file, _)| key.is_none() || crate_key(file) == key)
        .flat_map(|(_, targets)| targets)
        .filter(|t| {
            idx.node_id_to_payload
                .get(t.as_str())
                .is_some_and(|p| canonical_name(p) == fn_name)
        })
        .map(|t| NodeId::new(t.clone()))
        .collect();
    reexported.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    reexported.dedup();
    if reexported.len() == 1 {
        return reexported;
    }
    cands
}

/// Imports edges grouped by importing file.
/// Imports edges grouped by importing file, plus the (file, name) pairs imported from other crates.
#[derive(Default)]
struct ImportMaps {
    by_file: HashMap<String, Vec<String>>,
    external: HashSet<(String, String)>,
    /// (file, alias) -> original name, for `use a::f as g`.
    aliases: HashMap<(String, String), String>,
    local_aliases: HashMap<(String, String), String>,
    local_conflicts: HashSet<(String, String)>,
    aliases_by_name: HashMap<String, Vec<AliasTarget>>,
    globs: HashMap<String, Vec<String>>,
    path_mods: HashMap<String, Vec<String>>,
}

fn fill_imports(idx: &mut Index, edges: &cozo::NamedRows) {
    let imports = imports_by_file(edges);
    idx.imports_map = imports.by_file;
    idx.external_imports = imports.external;
    idx.aliases = imports.aliases;
    idx.local_aliases = imports.local_aliases;
    idx.local_conflicts = imports.local_conflicts;
    idx.aliases_by_name = imports.aliases_by_name;
    idx.globs = imports.globs;
    idx.path_mods = imports.path_mods;
}

fn imports_by_file(edges: &cozo::NamedRows) -> ImportMaps {
    let imports = EdgeType::Imports.to_string();
    let mut maps = ImportMaps::default();
    let mut conflicts = HashSet::new();
    let mut local_conflicts = HashSet::new();
    for row in &edges.rows {
        if row.get(2).map(unquote_datavalue).as_deref() != Some(imports.as_str()) {
            continue;
        }
        let from = row.first().map(unquote_datavalue).unwrap_or_default();
        let to = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let file = from.split('#').next().unwrap_or(&from).to_string();
        if let Some(name) = to.strip_prefix(EXTERNAL_IMPORT_PREFIX) {
            maps.external.insert((file, name.to_string()));
        } else if let Some(target) = to.strip_prefix(GLOB_IMPORT_PREFIX) {
            maps.globs.entry(file).or_default().push(target.to_string());
        } else if let Some((name, target)) = to
            .strip_prefix(PATH_MOD_IMPORT_PREFIX)
            .and_then(|r| r.split_once("=>"))
        {
            maps.path_mods
                .entry(name.to_string())
                .or_default()
                .push(target.to_string());
        } else if let Some((alias, orig)) = to
            .strip_prefix(ALIAS_IMPORT_PREFIX)
            .and_then(|r| r.split_once("=>"))
        {
            let (orig, target) = orig
                .split_once("=>")
                .map_or((orig, None), |(o, t)| (o, Some(t.to_string())));
            if from.contains('#') {
                let key = (from, alias.to_string());
                if maps.local_aliases.get(&key).is_some_and(|o| o != orig) {
                    local_conflicts.insert(key.clone());
                }
                maps.local_aliases.insert(key, orig.to_string());
                continue;
            }
            maps.aliases_by_name
                .entry(alias.to_string())
                .or_default()
                .push((file.clone(), orig.to_string(), target));
            // A name renamed to different items in one file (`#[cfg]` variants) names none of them.
            let key = (file, alias.to_string());
            if maps.aliases.get(&key).is_some_and(|o| o != orig) {
                conflicts.insert(key.clone());
            }
            maps.aliases.insert(key, orig.to_string());
        } else {
            maps.by_file.entry(file).or_default().push(to);
        }
    }
    maps.aliases.retain(|k, _| !conflicts.contains(k));
    maps.local_aliases
        .retain(|k, _| !local_conflicts.contains(k));
    maps.local_conflicts = local_conflicts;
    maps
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

/// Split a placeholder target (`file::path::name`, or a bare name) into (path part, fn name).
fn split_placeholder<'a>(to_str: &'a str, from_file: &'a str) -> (Cow<'a, str>, &'a str) {
    let Some((pp, rest)) = to_str.split_once("::") else {
        // Unqualified placeholder (bare function name): resolve via same-file then global.
        return (Cow::Borrowed(from_file), to_str);
    };
    // Qualified call like file::mod::foo: use last segment as fn name for resolution.
    match rest.rsplit_once("::") {
        Some((path_suffix, name)) => (Cow::Owned(format!("{pp}::{path_suffix}")), name),
        None => (Cow::Borrowed(pp), rest),
    }
}

/// Fill `idx.type_names`/`idx.type_by_id` from struct and enum nodes, then `idx.fields` from the
/// temporary field-type placeholders (`FIELD_MARK`) and `idx.rets` from return-type ones (`RET_MARK`);
/// both kinds are removed from the store.
fn fill_fields(
    store: &Store,
    idx: &mut Index,
    nodes: &cozo::NamedRows,
    edges: &cozo::NamedRows,
) -> Result<()> {
    let kinds = [NodeType::Struct.to_string(), NodeType::Enum.to_string()];
    for row in &nodes.rows {
        let t = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let name = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if kinds.contains(&t) && !name.is_empty() {
            idx.type_names.insert(name.clone());
            idx.type_by_id
                .insert(row.first().map(unquote_datavalue).unwrap_or_default(), name);
        }
    }
    let calls = EdgeType::Calls;
    for row in &edges.rows {
        let from = row.first().map(unquote_datavalue).unwrap_or_default();
        let to = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if let Some((_, ty)) = to.rsplit_once(RET_MARK) {
            idx.rets.insert(from.clone(), ty.to_string());
            store.remove_edge(&NodeId(from), &NodeId(to), &calls)?;
            continue;
        }
        let Some((path, ty)) = to.rsplit_once(FIELD_MARK) else {
            continue;
        };
        if let (Some(owner), Some(field)) = (idx.type_by_id.get(&from), path.rsplit("::").next()) {
            // Tuple payloads (`Variant#i`) are also found by variant name alone, for bare patterns:
            // first among the variants of the same file, then of the whole project.
            if field.contains('#') {
                let file = from.split('#').next().unwrap_or_default();
                for key in [String::new(), file.to_string()] {
                    idx.fields
                        .entry((key, field.to_string()))
                        .or_default()
                        .insert(ty.to_string());
                }
            }
            idx.fields
                .entry((owner.clone(), field.to_string()))
                .or_default()
                .insert(ty.to_string());
        }
        store.remove_edge(&NodeId(from), &NodeId(to), &calls)?;
    }
    Ok(())
}

/// Fill `idx.consts` from const and static nodes.
fn fill_consts(idx: &mut Index, nodes: &cozo::NamedRows) {
    let kinds = [NodeType::Const.to_string(), NodeType::Static.to_string()];
    for row in &nodes.rows {
        let t = row.get(1).map(unquote_datavalue).unwrap_or_default();
        let id = row.first().map(unquote_datavalue).unwrap_or_default();
        let name = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if kinds.contains(&t) && !name.is_empty() {
            let file = id.split('#').next().unwrap_or(&id).to_string();
            let node_id = NodeId(id);
            idx.consts_by_name
                .entry(name.clone())
                .or_default()
                .push(node_id.clone());
            idx.consts.entry((file, name)).or_default().push(node_id);
        }
    }
}

/// Split a call placeholder into (path and name, is `VALUE_SUFFIX`, is `self.m()`, is any method
/// call, receiver-type hint).
fn parse_target(raw_to: &str) -> (String, bool, bool, bool, Option<&str>) {
    let (core, hint) = match raw_to.rsplit_once(RECEIVER_MARK) {
        Some((c, h)) => (c, Some(h)),
        None => (raw_to, None),
    };
    let (core, value) = core
        .strip_suffix(VALUE_SUFFIX)
        .map_or((core, false), |t| (t, true));
    let (core, self_method) = core
        .strip_suffix(SELF_METHOD_SUFFIX)
        .map_or((core, false), |t| (t, true));
    let (core, method) = core
        .strip_suffix(METHOD_SUFFIX)
        .map_or((core, self_method), |t| (t, true));
    (core.to_string(), value, self_method, method, hint)
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
    fill_imports(&mut idx, &edges);
    idx.methods = method_ids(&nodes, &edges);
    fill_scopes(&mut idx, &nodes, &edges);
    fill_consts(&mut idx, &nodes);
    fill_fields(store, &mut idx, &nodes, &edges)?;

    let mut high: HashSet<(String, String)> = HashSet::new();
    let mut low: Vec<(NodeId, NodeId, f64)> = Vec::new();
    for row in &edges.rows {
        let edge_type = row.get(2).map(unquote_datavalue).unwrap_or_default();
        if edge_type != calls_type_str {
            continue;
        }
        let from_str = row.first().map(unquote_datavalue).unwrap_or_default();
        let raw_to = row.get(1).map(unquote_datavalue).unwrap_or_default();
        if raw_to.contains(FIELD_MARK) || raw_to.contains(RET_MARK) {
            continue;
        }
        let (to_str, value, self_method, method, hint) = parse_target(&raw_to);
        let from_file = from_str.split('#').next().unwrap_or(&from_str);
        let (path_part, fn_name) = split_placeholder(&to_str, from_file);
        let resolution = if method {
            let own = if self_method {
                resolve_self_method(fn_name, &from_str, &idx)
            } else {
                hint.and_then(|h| resolve_typed_method(fn_name, h, &from_str, from_file, &idx))
            };
            own.unwrap_or_else(|| resolve_method(fn_name, from_file, &idx))
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
        if value && !method && matches!(resolution, Resolution::None) && is_const_like(fn_name) {
            // A const/static use keeps its initialiser's callees alive (reachability follows `references`).
            if let Some(id) = resolve_const(path_part.as_ref(), fn_name, &from_str, &idx) {
                store.put_edge(&from_id, &id, &EdgeType::References)?;
            }
            continue;
        }
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

    use super::{build_call_graph, is_const_like};

    #[test]
    fn const_like_names_are_screaming_case_only() {
        for yes in ["MAX", "ID_2", "BUILTINS", "N"] {
            assert!(is_const_like(yes), "{yes}");
        }
        for no in ["", "Max", "max", "_X", "2X", "Self", "f"] {
            assert!(!is_const_like(no), "{no}");
        }
    }

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
    fn bare_call_not_resolved_by_imports_is_only_a_low_confidence_guess() {
        let store = Store::new_memory().unwrap();
        let local = put_fn(&store, "./lib.rs#1:1", "from_str", "./lib.rs");
        put_fn(&store, "./util.rs#1:1", "helper", "./util.rs");
        let caller = put_fn(&store, "./a.rs#1:1", "run", "./a.rs");
        // `a.rs` imports the module `util.rs` (no item resolved) and `from_str` from an external crate (not recorded).
        store
            .put_edge(
                &NodeId::new("./a.rs"),
                &NodeId::new("./util.rs"),
                &EdgeType::Imports,
            )
            .unwrap();
        store
            .put_edge(&caller, &NodeId::new("./a.rs::from_str"), &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let low = store
            .run_query(
                "?[t] := *edge_confidence[_, t, \"calls\", _]",
                std::collections::BTreeMap::new(),
            )
            .unwrap();
        let low: Vec<String> = low
            .rows
            .iter()
            .map(|r| r[0].to_string().trim_matches('"').to_string())
            .collect();
        assert_eq!(low, vec![local.as_str().to_string()]);
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

    #[test]
    fn bare_name_global_fallback_ignores_methods() {
        let store = Store::new_memory().unwrap();
        let caller = NodeId::new("./a.rs#1:1");
        store
            .put_node(&caller, &NodeType::Function, Some("main"))
            .unwrap();
        let free = NodeId::new("./free.rs#1:1");
        store
            .put_node(&free, &NodeType::Function, Some("find"))
            .unwrap();
        for i in 0..3 {
            let imp = NodeId::new(format!("./m{i}.rs#1:1"));
            let method = NodeId::new(format!("./m{i}.rs#2:5"));
            store.put_node(&imp, &NodeType::Impl, Some("S")).unwrap();
            store
                .put_node(&method, &NodeType::Function, Some("find"))
                .unwrap();
            store.put_edge(&imp, &method, &EdgeType::Contains).unwrap();
        }
        store
            .put_edge(&caller, &NodeId::new("./a.rs::find"), &EdgeType::Calls)
            .unwrap();
        build_call_graph(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let calls: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| r[2].to_string().contains("calls"))
            .map(|r| r[1].to_string().trim_matches('"').to_string())
            .collect();
        assert_eq!(calls, ["./free.rs#1:1"]);
        let low = store
            .run_query(
                "?[t] := *edge_confidence[_, t, \"calls\", _]",
                std::collections::BTreeMap::new(),
            )
            .unwrap();
        assert!(
            low.rows.is_empty(),
            "unique free fn is a high-confidence edge"
        );
    }
}
