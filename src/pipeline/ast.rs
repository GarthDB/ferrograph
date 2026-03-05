//! Phase 2: AST extraction (tree-sitter) into graph nodes and edges.

use std::path::Path;

use anyhow::Result;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::Store;

/// Extract graph nodes and edges from Rust source and write them to the store.
/// Node IDs use paths relative to `root` (e.g. `./src/main.rs`) for portability.
///
/// # Errors
/// Fails if parsing fails, the language cannot be loaded, or store writes fail.
pub fn extract_ast(store: &Store, path: &Path, content: &str, root: &Path) -> Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&LANGUAGE.into())?;
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("parse failed: no tree returned for {}", path.display()))?;
    if tree.root_node().has_error() {
        eprintln!(
            "warning: parse errors in {} (continuing with partial AST)",
            path.display()
        );
    }
    let rel = path.strip_prefix(root).unwrap_or(path);
    let file_id = NodeId::new(format!("./{}", rel.to_string_lossy()));
    let mut nodes = vec![(file_id.clone(), NodeType::File, None)];
    let mut edges = Vec::new();

    let mut cursor = tree.walk();
    let mut stack: Vec<NodeId> = vec![file_id.clone()];
    traverse(
        &mut cursor,
        content,
        &file_id,
        &mut stack,
        &mut nodes,
        &mut edges,
    );

    if !nodes.is_empty() {
        let batch: Vec<_> = nodes
            .iter()
            .map(|(id, typ, payload)| (id.clone(), typ.clone(), payload.as_deref()))
            .collect();
        store.put_nodes_batch(&batch)?;
    }
    if !edges.is_empty() {
        store.put_edges_batch(&edges)?;
    }
    Ok(())
}

fn classify_node(
    kind: &str,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    parent: Option<&NodeId>,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) -> (Option<NodeType>, Option<String>, Option<(NodeId, EdgeType)>) {
    match kind {
        "function_item" => (
            Some(NodeType::Function),
            function_payload(node, source),
            None,
        ),
        "struct_item" => (Some(NodeType::Struct), name_of_node(node, source), None),
        "enum_item" => (Some(NodeType::Enum), name_of_node(node, source), None),
        "trait_item" => (Some(NodeType::Trait), name_of_node(node, source), None),
        "impl_item" => {
            let extra = impl_trait_name(node, source).map(|trait_name| {
                (
                    NodeId::new(format!("{}::{}", file_id.as_str(), trait_name)),
                    EdgeType::ImplementsTrait,
                )
            });
            (Some(NodeType::Impl), impl_type_name(node, source), extra)
        }
        "type_item" => (Some(NodeType::TypeAlias), name_of_node(node, source), None),
        "const_item" => (Some(NodeType::Const), name_of_node(node, source), None),
        "static_item" => (Some(NodeType::Static), name_of_node(node, source), None),
        "macro_definition" => (Some(NodeType::Macro), name_of_node(node, source), None),
        "mod_declaration" | "mod_item" => {
            (Some(NodeType::Module), name_of_node(node, source), None)
        }
        "call_expression" => {
            if let Some(callee_id) = resolve_call_target(node, source, file_id) {
                if let Some(from) = parent {
                    edges.push((from.clone(), callee_id, EdgeType::Calls));
                }
            }
            (None, None, None)
        }
        "method_call_expression" => {
            if let Some(callee_id) = resolve_method_call_target(node, source, file_id) {
                if let Some(from) = parent {
                    edges.push((from.clone(), callee_id, EdgeType::Calls));
                }
            }
            (None, None, None)
        }
        "macro_invocation" => {
            scan_macro_for_calls(node, source, file_id, parent, edges);
            if let (Some(from), Some(macro_name)) = (parent, macro_invocation_name(node, source)) {
                edges.push((
                    from.clone(),
                    NodeId::new(format!("{}::{}", file_id.as_str(), macro_name)),
                    EdgeType::ExpandsTo,
                ));
            }
            (None, None, None)
        }
        "unsafe_block" => {
            if let Some(p) = parent {
                edges.push((p.clone(), p.clone(), EdgeType::UsesUnsafe));
            }
            (None, None, None)
        }
        "type_identifier" | "scoped_type_identifier" => {
            if let (Some(from), Some(name)) = (parent.cloned(), type_name_from_node(node, source)) {
                edges.push((
                    from,
                    NodeId::new(format!("{}::{}", file_id.as_str(), name)),
                    EdgeType::References,
                ));
            }
            (None, None, None)
        }
        _ => (None, None, None),
    }
}

fn traverse(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    file_id: &NodeId,
    stack: &mut Vec<NodeId>,
    nodes: &mut Vec<(NodeId, NodeType, Option<String>)>,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let node = cursor.node();
    let kind = node.kind();
    let parent = stack.last().cloned();
    let (node_type, name_opt, extra_edge) =
        classify_node(kind, &node, source, file_id, parent.as_ref(), edges);

    let added = if let (Some(nt), no, extra) = (node_type, name_opt, extra_edge) {
        let id = NodeId::new(format!(
            "{}#{}:{}",
            file_id.as_str(),
            node.start_position().row + 1,
            node.start_position().column + 1
        ));
        nodes.push((id.clone(), nt, no));
        if let Some(ref p) = parent {
            edges.push((p.clone(), id.clone(), EdgeType::Contains));
        }
        if (kind == "function_item" || kind == "impl_item") && has_unsafe_modifier(&node) {
            edges.push((id.clone(), id.clone(), EdgeType::UsesUnsafe));
        }
        if let Some((to_id, edge_type)) = extra {
            edges.push((id.clone(), to_id, edge_type));
        }
        stack.push(id);
        true
    } else {
        false
    };

    if cursor.goto_first_child() {
        loop {
            traverse(cursor, source, file_id, stack, nodes, edges);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }

    if added {
        stack.pop();
    }
}

fn name_of_node(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let n = cursor.node();
            // Struct, enum, and trait names are type_identifier; some items use identifier.
            if n.kind() == "identifier" || n.kind() == "type_identifier" {
                let r = n.byte_range();
                return source.get(r.start..r.end).map(String::from);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Returns true if any node preceding `node` (in sibling order) is an attribute (e.g. `#[bench]`, `#[test]`)
/// that matches `name`. Walks backwards through all consecutive attribute siblings so multi-attribute
/// items like `#[cfg(test)] #[bench] fn foo()` are detected.
fn has_attribute_on_prev_sibling(node: &tree_sitter::Node, source: &str, name: &str) -> bool {
    let mut current = node.prev_sibling();
    while let Some(prev) = current {
        let kind = prev.kind();
        if kind == "attribute_item" || kind == "outer_attribute_list" {
            if has_attribute_node(&prev, source, name) {
                return true;
            }
            current = prev.prev_sibling();
        } else {
            break;
        }
    }
    false
}

/// Returns true if the node has an attribute containing an identifier with the given name
/// (e.g. `#[test]`, `#[cfg(test)]`, `#[bench]`). Uses structural matching via `attribute_contains_identifier`;
/// if that fails, matches exact attribute text `#[name]` only (avoids false positives like `[bench]` or `#[cfg(bench)]`).
fn has_attribute(node: &tree_sitter::Node, source: &str, name: &str) -> bool {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return false;
    }
    loop {
        if has_attribute_node(&cursor.node(), source, name) {
            return true;
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    false
}

/// Check a single node for attribute match. For `attribute_item`/`outer_attribute_list`, checks
/// identifier content and raw text (does not recurse further). For other node types, recurses
/// into children to find nested attribute nodes.
fn has_attribute_node(n: &tree_sitter::Node, source: &str, name: &str) -> bool {
    let kind = n.kind();
    if kind == "attribute_item" || kind == "outer_attribute_list" {
        if attribute_contains_identifier(n, source, name) {
            return true;
        }
        let r = n.byte_range();
        if let Some(attr_text) = source.get(r.start..r.end) {
            let needle = format!("#[{name}]");
            if attr_text.contains(&needle) {
                return true;
            }
        }
        return false;
    }
    let mut cursor = n.walk();
    if cursor.goto_first_child() {
        loop {
            if has_attribute_node(&cursor.node(), source, name) {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

/// Returns true if any descendant of `node` is an identifier with the given text.
/// Skips `token_tree` (e.g. `(test)` in `#[cfg(test)]`) except when `name == "test"`, so
/// `#[cfg(bench)]` does not match "bench" (not a benchmark entry point).
fn attribute_contains_identifier(node: &tree_sitter::Node, source: &str, name: &str) -> bool {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return false;
    }
    loop {
        let n = cursor.node();
        if n.kind() == "identifier" {
            if source.get(n.byte_range()) == Some(name) {
                return true;
            }
        } else if n.kind() == "token_tree" {
            if name == "test" && attribute_contains_identifier(&n, source, name) {
                return true;
            }
        } else if attribute_contains_identifier(&n, source, name) {
            return true;
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    false
}

/// Payload for a function node: "`pub::name`" if public, "`test::name`" if test, "`bench::name`" if bench, else "name". Used for dead-code entry point detection.
///
/// Note: `test` detection recurses into `#[cfg(test)]` token trees, but `bench`
/// does not recurse into `#[cfg(bench)]` (conditional compilation, not a benchmark).
/// See `attribute_contains_identifier` for the distinction.
fn function_payload(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let name = name_of_node(node, source)?;
    let is_pub = node
        .child(0)
        .is_some_and(|c| c.kind() == "visibility_modifier");
    let is_test =
        has_attribute(node, source, "test") || has_attribute_on_prev_sibling(node, source, "test");
    let is_bench = has_attribute(node, source, "bench")
        || has_attribute_on_prev_sibling(node, source, "bench");
    let prefix = match (is_pub, is_test, is_bench) {
        (true, true, _) => "pub::test::",
        (false, true, _) => "test::",
        (true, false, true) => "pub::bench::",
        (false, false, true) => "bench::",
        (true, false, false) => "pub::",
        (false, false, false) => "",
    };
    Some(format!("{prefix}{name}"))
}

/// Returns true if the node has an `unsafe` modifier (e.g. `unsafe fn` or `unsafe impl`).
/// For `function_item`, `unsafe` appears inside a `function_modifiers` child; for
/// `impl_item`/`trait_item` it is an optional direct child.
fn has_unsafe_modifier(node: &tree_sitter::Node) -> bool {
    let mut i = 0;
    while let Some(child) = node.child(i) {
        let k = child.kind();
        if k == "unsafe" {
            return true;
        }
        if k == "function_modifiers" {
            let mut j = 0;
            while let Some(m) = child.child(j) {
                if m.kind() == "unsafe" {
                    return true;
                }
                j += 1;
            }
        }
        i += 1;
    }
    false
}

/// Payload for an impl block: type name (e.g. "Point" for `impl Point`, or "Draw" for `impl Draw for Point`).
fn impl_type_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let n = cursor.node();
        if n.kind() == "type_identifier" {
            let r = n.byte_range();
            return source.get(r.start..r.end).map(String::from);
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Trait name for trait impls only (`impl Trait for Type`). Returns None for inherent impls.
/// Walks `impl_item` children; when we see "for", the preceding `type_identifier` (or last segment of `scoped_type_identifier`) is the trait.
fn impl_trait_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut i = 0;
    let mut prev_type: Option<tree_sitter::Node> = None;
    while let Some(child) = node.child(i) {
        let k = child.kind();
        if k == "for" {
            let n = prev_type?;
            let r = n.byte_range();
            let text = source.get(r.start..r.end)?;
            return Some(if n.kind() == "scoped_type_identifier" {
                text.rsplit("::").next().unwrap_or(text).to_string()
            } else {
                text.to_string()
            });
        }
        if k == "type_identifier" || k == "scoped_type_identifier" {
            prev_type = Some(child);
        } else if k != "unsafe" && k != "impl" && !k.starts_with("type_parameters") && k != "!" {
            prev_type = None;
        }
        i += 1;
    }
    None
}

/// Type name for a `type_identifier` or `scoped_type_identifier` node (last segment for qualified paths).
fn type_name_from_node(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let r = node.byte_range();
    let text = source.get(r.start..r.end)?;
    Some(if node.kind() == "scoped_type_identifier" {
        text.rsplit("::").next().unwrap_or(text).to_string()
    } else {
        text.to_string()
    })
}

/// Resolve a call expression to a placeholder node id (`file_path::fn_name` or `file_path::path::to::fn`).
///
/// Handles bare identifier calls `foo()` and qualified paths `mod::foo()` (`scoped_identifier`).
/// Method calls (`x.bar()`), and UFCS (`Type::method()`) are handled elsewhere or not yet.
fn resolve_call_target(node: &tree_sitter::Node, source: &str, file_id: &NodeId) -> Option<NodeId> {
    let child = node.child(0)?;
    let path_str = match child.kind() {
        "identifier" | "scoped_identifier" => {
            let r = child.byte_range();
            source.get(r.start..r.end).map(String::from)?
        }
        _ => return None,
    };
    Some(NodeId::new(format!("{}::{}", file_id.as_str(), path_str)))
}

/// Resolve a method call (e.g. `x.foo()`) to a placeholder node id (`file_path::foo`).
fn resolve_method_call_target(
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
) -> Option<NodeId> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let n = cursor.node();
        if n.kind() == "field_identifier" {
            let r = n.byte_range();
            let name = source.get(r.start..r.end).map(String::from)?;
            return Some(NodeId::new(format!("{}::{}", file_id.as_str(), name)));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Macro name from a `macro_invocation` node (first child is `identifier` or `scoped_identifier`; last segment for qualified).
fn macro_invocation_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let child = node.child(0)?;
    let r = child.byte_range();
    let text = source.get(r.start..r.end)?;
    Some(if child.kind() == "scoped_identifier" {
        text.rsplit("::").next().unwrap_or(text).to_string()
    } else {
        text.to_string()
    })
}

/// Returns true if the `token_tree` node's source text starts with `(` (parenthesized group).
fn token_tree_starts_with_paren(node: &tree_sitter::Node, source: &str) -> bool {
    let r = node.byte_range();
    source
        .get(r.start..r.end)
        .is_some_and(|s| s.starts_with('('))
}

/// Scan a macro invocation's body for call-like patterns (identifier followed by `( ... )`).
/// Creates placeholder call edges so the calls phase can resolve them.
fn scan_macro_for_calls(
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    parent: Option<&NodeId>,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let Some(parent_id) = parent else {
        return;
    };
    let mut i = 0;
    while let Some(child) = node.child(i) {
        if child.kind() == "token_tree" && token_tree_starts_with_paren(&child, source) {
            scan_token_tree_for_calls(&child, source, file_id, parent_id, edges);
        }
        i += 1;
    }
}

/// Recursively scan a `token_tree` for identifier followed by parenthesized `token_tree`.
/// Skips method calls (previous sibling `.`) and nested macros (next sibling `!`).
fn scan_token_tree_for_calls(
    tt: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    parent: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let mut i = 0;
    let mut prev_kind: Option<String> = None;
    while let Some(child) = tt.child(i) {
        let kind = child.kind().to_string();
        let next = tt.child(i + 1);
        if kind == "identifier" {
            let prev_is_dot = prev_kind.as_deref() == Some(".");
            if !prev_is_dot {
                if let Some(next_tt) = next {
                    if next_tt.kind() == "token_tree"
                        && token_tree_starts_with_paren(&next_tt, source)
                    {
                        if let Some(name) = source.get(child.byte_range()) {
                            let callee_id = NodeId::new(format!("{}::{}", file_id.as_str(), name));
                            edges.push((parent.clone(), callee_id, EdgeType::Calls));
                        }
                    }
                }
            }
        }
        if kind == "token_tree" {
            scan_token_tree_for_calls(&child, source, file_id, parent, edges);
        }
        prev_kind = Some(kind);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::graph::query::Query;
    use crate::graph::Store;

    use super::extract_ast;

    #[test]
    fn extract_ast_single_function() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = "fn foo() {}";
        extract_ast(&store, path, content, root).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        assert!(rows.rows.len() >= 2, "expected file + function nodes");
        let types: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|r| r.get(1))
            .map(|v| v.to_string().trim_matches('"').to_string())
            .collect();
        assert!(types.contains(&"file".to_string()));
        assert!(types.contains(&"function".to_string()));
    }

    #[test]
    fn extract_ast_invalid_rust_partial_ast() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = "not valid rust {{{";
        // We tolerate parse errors and continue with partial AST instead of bailing.
        let result = extract_ast(&store, path, content, root);
        assert!(result.is_ok(), "partial AST should be accepted: {result:?}");
        let rows = Query::all_nodes(&store).unwrap();
        assert!(!rows.rows.is_empty(), "expected at least file node");
    }

    #[test]
    fn extract_ast_trait_has_name_payload() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = "trait Draw { fn draw(&self); }";
        extract_ast(&store, path, content, root).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        let trait_rows: Vec<_> = rows
            .rows
            .iter()
            .filter(|r| {
                r.get(1)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "trait")
            })
            .collect();
        assert!(
            !trait_rows.is_empty(),
            "expected at least one trait node, got {rows:?}"
        );
        let payload = trait_rows[0].get(2).map(std::string::ToString::to_string);
        assert!(
            payload.as_ref().is_some_and(|p| p.contains("Draw")),
            "trait node should have payload with name Draw, got {payload:?}"
        );
    }

    #[test]
    fn extract_ast_bench_function_has_bench_prefix() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/benches/foo.rs");
        let content = "#[bench] fn my_bench(_: &mut Bencher) {}";
        extract_ast(&store, path, content, root).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        let fn_rows: Vec<_> = rows
            .rows
            .iter()
            .filter(|r| {
                r.get(1)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "function")
            })
            .collect();
        assert!(
            !fn_rows.is_empty(),
            "expected at least one function node, got {rows:?}"
        );
        let payload = fn_rows[0].get(2).map(std::string::ToString::to_string);
        assert!(
            payload
                .as_ref()
                .is_some_and(|p| p.contains("bench::my_bench")),
            "expected bench:: prefix in payload, got {payload:?}"
        );
    }

    #[test]
    fn extract_ast_multi_attr_bench_after_other() {
        // #[bench] is not the immediate prev sibling; we walk back and still detect it.
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/multi.rs");
        let content = "#[allow(dead_code)]\n#[bench]\nfn multi_bench(b: &mut Bencher) {}";
        extract_ast(&store, path, content, root).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        let fn_rows: Vec<_> = rows
            .rows
            .iter()
            .filter(|r| {
                r.get(1)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "function")
            })
            .collect();
        assert!(
            !fn_rows.is_empty(),
            "expected at least one function node, got {rows:?}"
        );
        let payload = fn_rows[0].get(2).map(std::string::ToString::to_string);
        assert!(
            payload
                .as_ref()
                .is_some_and(|p| p.contains("bench::multi_bench")),
            "expected bench:: prefix when #[bench] follows another attribute, got {payload:?}"
        );
    }

    #[test]
    fn extract_ast_cfg_bench_not_bench_entry_point() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/cfg_bench.rs");
        let content = "#[cfg(bench)]\nfn not_a_bench() {}";
        extract_ast(&store, path, content, root).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        let fn_rows: Vec<_> = rows
            .rows
            .iter()
            .filter(|r| {
                r.get(1)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "function")
            })
            .collect();
        assert!(
            !fn_rows.is_empty(),
            "expected at least one function node, got {rows:?}"
        );
        let payload = fn_rows[0].get(2).map(std::string::ToString::to_string);
        assert!(
            payload
                .as_ref()
                .is_some_and(|p| p.contains("not_a_bench") && !p.contains("bench::")),
            "#[cfg(bench)] fn should not get bench:: prefix, got {payload:?}"
        );
    }

    #[test]
    fn extract_ast_uses_unsafe_edges() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r#"
            unsafe fn unsafe_fn() {}
            fn with_unsafe_block() { unsafe { } }
        "#;
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let uses_unsafe: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "uses_unsafe")
            })
            .collect();
        assert_eq!(
            uses_unsafe.len(),
            2,
            "expected exactly two uses_unsafe edges (unsafe fn and fn with unsafe block), got {}",
            uses_unsafe.len()
        );
        for row in &uses_unsafe {
            let from = row
                .first()
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_default();
            let to = row
                .get(1)
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_default();
            assert_eq!(
                from, to,
                "uses_unsafe edge should be self-loop (from == to)"
            );
            assert!(
                from.contains("test.rs#"),
                "uses_unsafe from id should reference test file, got {from}"
            );
        }
    }

    #[test]
    fn extract_ast_call_inside_macro() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = "fn caller() { format!(\"{}\", callee()); }\nfn callee() {}";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let calls: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "calls")
            })
            .collect();
        assert!(
            !calls.is_empty(),
            "expected at least one Calls edge from macro body, got {} edges total",
            edges.rows.len()
        );
        let to_vals: Vec<String> = calls
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            to_vals.iter().any(|t| t.contains("callee")),
            "expected a call edge to callee (placeholder or resolved), got {to_vals:?}"
        );
    }
}
