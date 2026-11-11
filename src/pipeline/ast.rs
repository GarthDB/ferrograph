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
        &mut Vec::new(),
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
        "identifier" | "scoped_identifier" => {
            if let (Some(from), true) = (
                parent,
                is_value_position(node) || is_const_use(node, source),
            ) {
                let r = node.byte_range();
                if let Some(path) = source.get(r.start..r.end) {
                    let target = NodeId::new(format!(
                        "{}::{}{}",
                        file_id.as_str(),
                        path,
                        super::calls::VALUE_SUFFIX
                    ));
                    edges.push((from.clone(), target, EdgeType::Calls));
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
        "field_declaration" | "parameter" => {
            push_owns_or_borrows_edge(parent, node, source, file_id, edges);
            if kind == "field_declaration" {
                push_field_type_edge(parent, node, source, file_id, edges);
            }
            (None, None, None)
        }
        "ordered_field_declaration_list" => {
            push_owns_or_borrows_edges_for_tuple_fields(parent, node, source, file_id, edges);
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
    fns: &mut Vec<NodeId>,
    nodes: &mut Vec<(NodeId, NodeType, Option<String>)>,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let node = cursor.node();
    let kind = node.kind();
    let parent = stack.last().cloned();
    if matches!(kind, "attribute_item" | "inner_attribute_item") {
        // The enclosing fn (or the file) anchors the edges: attribute arguments are evaluated by
        // derive/attribute macros whose output we do not see, so the annotated type is not a reliable owner.
        let anchor = fns.last().unwrap_or(file_id);
        scan_attribute_refs(&node, source, file_id, anchor, edges);
    }
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
        if (kind == "function_item"
            || kind == "struct_item"
            || kind == "enum_item"
            || kind == "impl_item"
            || kind == "trait_item")
            && has_lifetime_parameters(&node)
        {
            edges.push((id.clone(), id.clone(), EdgeType::LifetimeScope));
        }
        if kind == "function_item" {
            for name in attribute_names(&node, source) {
                edges.push((
                    id.clone(),
                    NodeId::new(format!("attr::{name}")),
                    EdgeType::HasAttribute,
                ));
            }
            if is_trait_impl_method(&node) {
                edges.push((
                    id.clone(),
                    NodeId::new(format!("attr::{TRAIT_IMPL_ATTR}")),
                    EdgeType::HasAttribute,
                ));
            }
            push_return_type_owns_or_borrows_edge(&id, &node, source, file_id, edges);
        }
        if kind == "const_item" || kind == "static_item" {
            push_const_roots(&id, &node, source, edges);
        }
        if kind == "macro_definition" {
            scan_macro_rules_body(&node, source, file_id, &id, edges);
        }
        if let Some((to_id, edge_type)) = extra {
            edges.push((id.clone(), to_id, edge_type));
        }
        if kind == "function_item" {
            fns.push(id.clone());
        }
        stack.push(id);
        true
    } else {
        false
    };

    if cursor.goto_first_child() {
        loop {
            traverse(cursor, source, file_id, stack, fns, nodes, edges);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }

    if added {
        stack.pop();
        if kind == "function_item" {
            fns.pop();
        }
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
        } else if !is_comment(&prev) {
            break;
        }
        current = prev.prev_sibling();
    }
    false
}

fn is_comment(node: &tree_sitter::Node) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

/// Synthetic attribute marking a method inside `impl Trait for Type` (a dead-code root).
/// Keep in sync with the entry rules in `graph::query`.
const TRAIT_IMPL_ATTR: &str = "trait_impl";

/// True for a function directly inside `impl <Trait> for <Type>` (including generic traits).
fn is_trait_impl_method(node: &tree_sitter::Node) -> bool {
    node.parent()
        .filter(|p| p.kind() == "declaration_list")
        .and_then(|p| p.parent())
        .is_some_and(|i| i.kind() == "impl_item" && i.child_by_field_name("trait").is_some())
}

/// Outer attribute names on a function (`no_mangle`, `export_name`, ...), plus `extern` for
/// `extern "C" fn`. Name is the attribute path, without arguments (`#[tokio::main(x)]` -> `tokio::main`).
fn attribute_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = node.prev_sibling();
    while let Some(prev) = current {
        if is_comment(&prev) {
            current = prev.prev_sibling();
            continue;
        }
        if prev.kind() != "attribute_item" {
            break;
        }
        if let Some(text) = source.get(prev.byte_range()) {
            let inner = text.trim().trim_start_matches("#[").trim_end_matches(']');
            // `#[unsafe(no_mangle)]` -> `no_mangle`
            let inner = inner
                .strip_prefix("unsafe(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(inner);
            let name: String = inner
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
        current = prev.prev_sibling();
    }
    let mut cursor = node.walk();
    let has_extern = node.children(&mut cursor).any(|c| {
        c.kind() == "function_modifiers"
            && source
                .get(c.byte_range())
                .is_some_and(|t| t.split_whitespace().any(|w| w == "extern"))
    });
    if has_extern {
        names.push("extern".to_string());
    }
    names
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
    // `pub(crate)`/`pub(super)` still count as roots: until method and path calls resolve far better,
    // dropping them floods `dead` with false positives (see #91 and the accuracy harness).
    let is_pub = node
        .child(0)
        .is_some_and(|c| c.kind() == "visibility_modifier");
    // `test_case`/`rstest`/`wasm_bindgen_test` expand to test functions (#107).
    let is_test = ["test", "test_case", "rstest", "wasm_bindgen_test"]
        .iter()
        .any(|name| {
            has_attribute(node, source, name) || has_attribute_on_prev_sibling(node, source, name)
        });
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

/// Returns true if the node has explicit lifetime parameters (e.g. `fn foo<'a>`, `struct S<'a>`).
/// Only checks the item's `type_parameters` child, not nested items (avoids false positives when
/// a function body contains a struct with lifetimes).
fn has_lifetime_parameters(node: &tree_sitter::Node) -> bool {
    let mut i = 0;
    while let Some(child) = node.child(i) {
        if child.kind() == "type_parameters" {
            return child_contains_lifetime_parameter(&child);
        }
        i += 1;
    }
    false
}

fn child_contains_lifetime_parameter(node: &tree_sitter::Node) -> bool {
    let mut i = 0;
    while let Some(child) = node.child(i) {
        if child.kind() == "lifetime_parameter" {
            return true;
        }
        i += 1;
    }
    false
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

/// Payload for an impl block: the implemented type (e.g. "Point" for `impl Point` and for
/// `impl Draw for Point`). Generic arguments and path prefixes are dropped; other self types
/// (references, slices, tuples, ...) have no payload.
fn impl_type_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut ty = node.child_by_field_name("type")?;
    if ty.kind() == "generic_type" {
        ty = ty.child_by_field_name("type")?;
    }
    matches!(ty.kind(), "type_identifier" | "scoped_type_identifier")
        .then(|| type_name_from_node(&ty, source))
        .flatten()
}

/// Trait name for trait impls only (`impl Trait for Type`). Returns None for inherent impls.
/// Reads the `trait` field; generic traits (`Trait<T>`) and scoped paths reduce to the last bare name.
fn impl_trait_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut t = node.child_by_field_name("trait")?;
    if t.kind() == "generic_type" {
        t = t.child_by_field_name("type")?;
    }
    let text = source.get(t.byte_range())?;
    Some(text.rsplit("::").next().unwrap_or(text).to_string())
}

/// Ownership classification for a type (owned value, shared borrow, or mutable borrow).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ownership {
    Owns,
    Borrows,
    BorrowsMut,
}

/// Emit one `Owns` or `Borrows` placeholder edge per type in a tuple struct's `ordered_field_declaration_list`.
/// The list may contain a repeat node (e.g. `ordered_field_declaration_list_repeat1`), so we recurse into
/// non-punctuation children to find each field type.
fn push_owns_or_borrows_edges_for_tuple_fields(
    parent: Option<&NodeId>,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let Some(from) = parent else {
        return;
    };
    collect_and_push_tuple_field_types(from, node, source, file_id, edges);
}

fn collect_and_push_tuple_field_types(
    from: &NodeId,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    if let Some((type_name, ownership)) = find_type_name_and_ownership(node, source) {
        let edge_type = match ownership {
            Ownership::Owns => EdgeType::Owns,
            Ownership::Borrows => EdgeType::Borrows,
            Ownership::BorrowsMut => EdgeType::BorrowsMut,
        };
        edges.push((
            from.clone(),
            NodeId::new(format!("{}::{}", file_id.as_str(), type_name)),
            edge_type,
        ));
        return;
    }
    let mut i = 0;
    while let Some(child) = node.child(i) {
        let k = child.kind();
        if k != "(" && k != ")" && k != "," {
            collect_and_push_tuple_field_types(from, &child, source, file_id, edges);
        }
        i += 1;
    }
}

/// Emit `Owns` or `Borrows` placeholder edge from parent to type for `field_declaration` or `parameter` nodes.
fn push_owns_or_borrows_edge(
    parent: Option<&NodeId>,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let Some(from) = parent else {
        return;
    };
    let Some((type_name, ownership)) = type_and_ownership_from_type_node(node, source) else {
        return;
    };
    let edge_type = match ownership {
        Ownership::Owns => EdgeType::Owns,
        Ownership::Borrows => EdgeType::Borrows,
        Ownership::BorrowsMut => EdgeType::BorrowsMut,
    };
    edges.push((
        from.clone(),
        NodeId::new(format!("{}::{}", file_id.as_str(), type_name)),
        edge_type,
    ));
}

/// Emit `Owns` or `Borrows` placeholder edge from function to its return type (for `function_item` nodes).
fn push_return_type_owns_or_borrows_edge(
    function_id: &NodeId,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    // Return type is optional(seq('->', field('return_type', $._type))). Find it by walking
    // the function's children: skip until past the parameters node, then the next node (if not block) is the return type.
    let return_type_node = node.child_by_field_name("return_type").or_else(|| {
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return None;
        }
        // Advance until we're on the parameters node.
        loop {
            if cursor.node().kind() == "parameters" || cursor.node().kind() == "parameter_list" {
                break;
            }
            if !cursor.goto_next_sibling() {
                return None;
            }
        }
        // After parameters come "->" and the return type (e.g. type_identifier or reference_type); then block.
        while cursor.goto_next_sibling() {
            let after_params = cursor.node();
            if after_params.kind() == "block" {
                return None;
            }
            if find_type_name_and_ownership(&after_params, source).is_some() {
                return Some(after_params);
            }
        }
        None
    });
    let Some(return_type_node) = return_type_node else {
        return;
    };
    let Some((type_name, ownership)) = find_type_name_and_ownership(&return_type_node, source)
    else {
        return;
    };
    let edge_type = match ownership {
        Ownership::Owns => EdgeType::Owns,
        Ownership::Borrows => EdgeType::Borrows,
        Ownership::BorrowsMut => EdgeType::BorrowsMut,
    };
    edges.push((
        function_id.clone(),
        NodeId::new(format!("{}::{}", file_id.as_str(), type_name)),
        edge_type,
    ));
}

/// Type and ownership from a `field_declaration` or `parameter` node.
/// Returns (`type_name`, `ownership`). For `reference_type`, distinguishes `&T` (Borrows) from `&mut T` (`BorrowsMut`).
fn type_and_ownership_from_type_node(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<(String, Ownership)> {
    find_type_name_and_ownership(node, source)
}

/// Recursively find first type and ownership. Returns (`type_name`, `ownership`).
/// We check `reference_type` before recursing so that we classify &T / &mut T as Borrows / `BorrowsMut`, not owns.
fn find_type_name_and_ownership(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<(String, Ownership)> {
    if node.kind() == "reference_type" {
        let inner = type_identifier_or_scoped_inside(node, source)?;
        let is_mut = (0..node.child_count())
            .filter_map(|j| node.child(j))
            .any(|c| c.kind() == "mutable_specifier");
        return Some((
            inner,
            if is_mut {
                Ownership::BorrowsMut
            } else {
                Ownership::Borrows
            },
        ));
    }
    if node.kind() == "type_identifier" || node.kind() == "scoped_type_identifier" {
        return type_name_from_node(node, source).map(|n| (n, Ownership::Owns));
    }
    if node.kind() == "primitive_type" {
        let r = node.byte_range();
        return source
            .get(r.start..r.end)
            .map(|s| (s.to_string(), Ownership::Owns));
    }
    let mut i = 0;
    while let Some(child) = node.child(i) {
        let k = child.kind();
        if k == "reference_type" {
            let inner = type_identifier_or_scoped_inside(&child, source)?;
            let is_mut = (0..child.child_count())
                .filter_map(|j| child.child(j))
                .any(|c| c.kind() == "mutable_specifier");
            return Some((
                inner,
                if is_mut {
                    Ownership::BorrowsMut
                } else {
                    Ownership::Borrows
                },
            ));
        }
        if k == "type_identifier" || k == "scoped_type_identifier" {
            return type_name_from_node(&child, source).map(|n| (n, Ownership::Owns));
        }
        if k == "primitive_type" {
            let r = child.byte_range();
            return source
                .get(r.start..r.end)
                .map(|s| (s.to_string(), Ownership::Owns));
        }
        if k != "identifier"
            && k != "field_identifier"
            && k != "visibility_modifier"
            && k != "mutable_specifier"
            && !k.starts_with("_pattern")
            && k != ":"
        {
            if let Some(res) = find_type_name_and_ownership(&child, source) {
                return Some(res);
            }
        }
        i += 1;
    }
    None
}

/// Type name of the referent inside a `reference_type` node (e.g. `str` from `&str`).
/// Handles `type_identifier`, `scoped_type_identifier`, and `primitive_type` (e.g. str, i32).
fn type_identifier_or_scoped_inside(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut i = 0;
    while let Some(child) = node.child(i) {
        let k = child.kind();
        if k == "type_identifier" || k == "scoped_type_identifier" {
            return type_name_from_node(&child, source);
        }
        if k == "primitive_type" {
            let r = child.byte_range();
            return source.get(r.start..r.end).map(String::from);
        }
        if k != "&" && k != "lifetime" && k != "mutable_specifier" {
            if let Some(inner) = type_identifier_or_scoped_inside(&child, source) {
                return Some(inner);
            }
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

/// True when an identifier/path is used as a value (argument, field value, array element, ...)
/// rather than called or declared. Such paths may name a fn item (fn pointer); the calls phase
/// drops the placeholder if it does not resolve to one.
fn is_value_position(node: &tree_sitter::Node) -> bool {
    let mut node = *node;
    // `f::<T>` as a value: the path sits inside a `generic_function`, which takes its place.
    if let Some(g) = node.parent().filter(|g| {
        g.kind() == "generic_function" && g.child_by_field_name("function") == Some(node)
    }) {
        node = g;
    }
    let Some(p) = node.parent() else {
        return false;
    };
    match p.kind() {
        "arguments"
        | "field_initializer"
        | "shorthand_field_initializer"
        | "array_expression"
        | "tuple_expression"
        | "reference_expression"
        | "parenthesized_expression"
        | "return_expression"
        | "type_cast_expression"
        | "block" => true,
        "let_declaration" | "match_arm" | "const_item" | "static_item" => {
            p.child_by_field_name("value") == Some(node)
        }
        "assignment_expression" => p.child_by_field_name("right") == Some(node),
        _ => false,
    }
}

/// True for an outermost path that looks like a use of a const/static (`MAX`, `Self::ID`) outside
/// declarations and `use` items. The calls phase links it to a same-file const/static, else drops it.
fn is_const_use(node: &tree_sitter::Node, source: &str) -> bool {
    let Some(p) = node.parent() else {
        return false;
    };
    if matches!(
        p.kind(),
        "scoped_identifier"
            | "token_tree"
            | "token_repetition"
            | "token_tree_pattern"
            | "use_declaration"
            | "use_list"
            | "scoped_use_list"
            | "use_as_clause"
            | "use_wildcard"
    ) || (matches!(p.kind(), "const_item" | "static_item")
        && p.child_by_field_name("name") == Some(*node))
    {
        return false;
    }
    source
        .get(node.byte_range())
        .is_some_and(|t| super::calls::is_const_like(t.rsplit("::").next().unwrap_or(t)))
}

/// Marks a const/static as a dead-code root (see `graph::query`): `pub`, an item of a trait impl,
/// or a static with an entry attribute (`#[no_mangle]`, `#[used]`, ...).
fn push_const_roots(
    id: &NodeId,
    node: &tree_sitter::Node,
    source: &str,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let is_bare_pub = node.child(0).is_some_and(|c| {
        c.kind() == "visibility_modifier" && source.get(c.byte_range()) == Some("pub")
    });
    let mut roots = Vec::new();
    if is_bare_pub {
        roots.push("pub".to_string());
    }
    if is_trait_impl_method(node) {
        roots.push(TRAIT_IMPL_ATTR.to_string());
    }
    if node.kind() == "static_item" {
        roots.extend(attribute_names(node, source));
    }
    for name in roots {
        edges.push((
            id.clone(),
            NodeId::new(format!("attr::{name}")),
            EdgeType::HasAttribute,
        ));
    }
}

/// Resolve a call expression to a placeholder node id (`file_path::fn_name` or `file_path::path::to::fn`).
///
/// Handles `foo()` and `mod::foo()`, method calls `x.bar()` (a `field_expression` callee; tree-sitter
/// has no method call node) and turbofish calls (`generic_function` wrapping any of those).
/// Method placeholders carry `METHOD_SUFFIX`. Other callee kinds (parenthesized, closures) are ignored.
fn resolve_call_target(node: &tree_sitter::Node, source: &str, file_id: &NodeId) -> Option<NodeId> {
    callee_placeholder(&node.child(0)?, source, file_id)
}

fn callee_placeholder(
    callee: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
) -> Option<NodeId> {
    let text = |n: &tree_sitter::Node| source.get(n.byte_range());
    let name = match callee.kind() {
        "identifier" | "scoped_identifier" => text(callee)?.to_string(),
        "field_expression" => {
            let field = callee.child_by_field_name("field")?;
            if field.kind() != "field_identifier" {
                return None;
            }
            let on_self = callee
                .child_by_field_name("value")
                .is_some_and(|v| v.kind() == "self");
            let suffix = if on_self {
                super::calls::SELF_METHOD_SUFFIX
            } else {
                super::calls::METHOD_SUFFIX
            };
            let hint = callee
                .child_by_field_name("value")
                .filter(|_| !on_self)
                .and_then(|v| receiver_hint(&v, source))
                .map(|h| format!("{}{h}", super::calls::RECEIVER_MARK))
                .unwrap_or_default();
            format!("{}{suffix}{hint}", text(&field)?)
        }
        "generic_function" => {
            return callee_placeholder(&callee.child_by_field_name("function")?, source, file_id)
        }
        _ => return None,
    };
    Some(NodeId::new(format!("{}::{}", file_id.as_str(), name)))
}

/// Name of a plain type: `T`, `a::T`, `T<..>`, `&T`, `&mut T`. Anything else (tuples, slices, `dyn`,
/// `impl`, pointers) is `None`.
fn plain_type_name(ty: &tree_sitter::Node, source: &str) -> Option<String> {
    match ty.kind() {
        "type_identifier" | "scoped_type_identifier" => type_name_from_node(ty, source),
        "generic_type" | "reference_type" => {
            plain_type_name(&ty.child_by_field_name("type")?, source)
        }
        _ => None,
    }
}

/// Temporary `Calls` edge from a struct to `file::field@field:Type`, consumed by the calls phase
/// to resolve `self.field.m()` (see `calls::FIELD_MARK`).
fn push_field_type_edge(
    parent: Option<&NodeId>,
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let (Some(from), Some(name), Some(ty)) = (
        parent,
        node.child_by_field_name("name")
            .and_then(|n| source.get(n.byte_range())),
        node.child_by_field_name("type")
            .and_then(|t| plain_type_name(&t, source)),
    ) else {
        return;
    };
    edges.push((
        from.clone(),
        NodeId::new(format!(
            "{}::{name}{}{ty}",
            file_id.as_str(),
            super::calls::FIELD_MARK
        )),
        EdgeType::Calls,
    ));
}

/// Type of the value of a `let` initialiser when it is evident: `T { .. }`, `T::new(..)` /
/// `T::default()` (optionally behind `&` or `?`).
fn initialiser_type(v: &tree_sitter::Node, source: &str) -> Option<String> {
    match v.kind() {
        "struct_expression" => plain_type_name(&v.child_by_field_name("name")?, source),
        "reference_expression" => initialiser_type(&v.child_by_field_name("value")?, source),
        "try_expression" => initialiser_type(&v.child(0)?, source),
        "call_expression" => {
            let f = v.child_by_field_name("function")?;
            let name = f.child_by_field_name("name")?;
            let path = f.child_by_field_name("path")?;
            let ty = source.get(path.byte_range())?.rsplit("::").next()?;
            (f.kind() == "scoped_identifier"
                && matches!(source.get(name.byte_range())?, "new" | "default")
                && ty.starts_with(|c: char| c.is_ascii_uppercase()))
            .then(|| ty.to_string())
        }
        _ => None,
    }
}

/// Is `n` an identifier that introduces a binding (pattern position)?
fn is_binder(n: &tree_sitter::Node) -> bool {
    let Some(p) = n.parent() else {
        return false;
    };
    let k = p.kind();
    k.ends_with("_pattern")
        || k == "closure_parameters"
        || (matches!(
            k,
            "let_declaration" | "parameter" | "for_expression" | "let_condition"
        ) && p.child_by_field_name("pattern") == Some(*n))
}

/// A plain `let x`/`let mut x`/`fn f(x: ..)` binding of `n`: (scope node, declared or evident type,
/// end of the declaration).
fn simple_binding<'a>(
    n: &tree_sitter::Node<'a>,
    source: &str,
) -> Option<(tree_sitter::Node<'a>, Option<String>, usize)> {
    let mut pat = *n;
    let mut decl = n.parent()?;
    if decl.kind() == "mut_pattern" {
        pat = decl;
        decl = decl.parent()?;
    }
    if decl.child_by_field_name("pattern") != Some(pat) {
        return None;
    }
    let declared = decl
        .child_by_field_name("type")
        .and_then(|t| plain_type_name(&t, source));
    match decl.kind() {
        "let_declaration" => {
            let ty = declared.or_else(|| {
                decl.child_by_field_name("value")
                    .and_then(|v| initialiser_type(&v, source))
            });
            Some((decl.parent()?, ty, decl.end_byte()))
        }
        "parameter" if decl.parent()?.kind() == "parameters" => {
            Some((decl.parent()?.parent()?, declared, decl.end_byte()))
        }
        _ => None,
    }
}

/// Collect the identifiers named `name` below `node`.
fn named_identifiers<'a>(
    node: &tree_sitter::Node<'a>,
    source: &str,
    name: &str,
    out: &mut Vec<tree_sitter::Node<'a>>,
) {
    let mut i = 0;
    while let Some(c) = node.child(i) {
        if matches!(c.kind(), "identifier" | "shorthand_field_identifier")
            && source.get(c.byte_range()) == Some(name)
        {
            out.push(c);
        }
        named_identifiers(&c, source, name, out);
        i += 1;
    }
}

/// Type of the local `x` (an identifier at the call site) from its nearest `let`/parameter binding
/// in the enclosing function. Any other kind of binding of the same name (closure parameter,
/// pattern, `for`, `if let`) makes it unknown: shadowing is not tracked.
fn binding_type(x: &tree_sitter::Node, source: &str) -> Option<String> {
    let name = source.get(x.byte_range())?;
    let mut func = x.parent()?;
    while func.kind() != "function_item" {
        func = func.parent()?;
    }
    let mut ids = Vec::new();
    named_identifiers(&func, source, name, &mut ids);
    let mut best: Option<(usize, Option<String>)> = None;
    for id in ids.iter().filter(|n| is_binder(n)) {
        let (scope, ty, decl_end) = simple_binding(id, source)?;
        let covers = scope.start_byte() <= x.start_byte() && x.end_byte() <= scope.end_byte();
        if covers
            && decl_end <= x.start_byte()
            && best.as_ref().is_none_or(|b| id.start_byte() > b.0)
        {
            best = Some((id.start_byte(), ty));
        }
    }
    best?.1
}

/// Receiver type hint for `recv.m()`: `Type` for a typed local, `Type.field` / `Self.field` for a
/// field of one (`self.field.m()`); see `calls::RECEIVER_MARK`.
fn receiver_hint(recv: &tree_sitter::Node, source: &str) -> Option<String> {
    let base = |n: &tree_sitter::Node| match n.kind() {
        "self" => Some("Self".to_string()),
        "identifier" => binding_type(n, source),
        _ => None,
    };
    match recv.kind() {
        "identifier" => binding_type(recv, source),
        "field_expression" => {
            let field = recv.child_by_field_name("field")?;
            if field.kind() != "field_identifier" {
                return None;
            }
            let ty = base(&recv.child_by_field_name("value")?)?;
            Some(format!("{ty}.{}", source.get(field.byte_range())?))
        }
        _ => None,
    }
}

/// `receiver_hint` for a method call inside a macro token tree, where the method name is token `i`
/// (`x . m (` or `self . f . m (`).
fn token_receiver_hint(tt: &tree_sitter::Node, i: usize, source: &str) -> Option<String> {
    let at = |j: usize| (j < i).then(|| tt.child(i - j)).flatten();
    let recv = at(2).filter(|n| n.kind() == "identifier")?;
    match at(3).map(|n| n.kind()) {
        Some(".") => {
            let field = source.get(recv.byte_range())?;
            let base = at(4)?;
            let ty = match base.kind() {
                "self" => "Self".to_string(),
                "identifier" if at(5).is_none_or(|n| !matches!(n.kind(), "." | "::")) => {
                    binding_type(&base, source)?
                }
                _ => return None,
            };
            Some(format!("{ty}.{field}"))
        }
        Some("::") => None,
        _ => binding_type(&recv, source),
    }
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
        if child.kind() == "token_tree" {
            scan_token_tree_for_calls(&child, source, file_id, parent_id, edges);
        }
        i += 1;
    }
}

/// Scan the transcriber (right-hand side) of each `macro_rules!` rule, attributing calls to the
/// macro node. The matcher side is not scanned: it only holds patterns and fragment names.
fn scan_macro_rules_body(
    node: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    macro_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let mut cursor = node.walk();
    for rule in node.children(&mut cursor) {
        if let Some(body) = rule
            .child_by_field_name("right")
            .filter(|b| b.kind() == "token_tree")
        {
            scan_token_tree_for_calls(&body, source, file_id, macro_id, edges);
        }
    }
}

/// Attribute keys whose string value names a function (`#[serde(skip_serializing_if = "is_default")]`).
const FN_NAMING_ATTR_KEYS: [&str; 4] = [
    "skip_serializing_if",
    "serialize_with",
    "deserialize_with",
    "default",
];

/// Reference edges for functions named in attribute arguments (#89): the value after `=`
/// (`#[arg(value_parser = parse_hex)]`, `#[command(about = about())]`, `.map(f)` inside it) and
/// string values of [`FN_NAMING_ATTR_KEYS`]. Bare flags such as `#[arg(short, long)]` are not references.
fn scan_attribute_refs(
    item: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    anchor: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let mut cursor = item.walk();
    for attr in item.children(&mut cursor) {
        if attr.kind() != "attribute" {
            continue;
        }
        let mut inner = attr.walk();
        for child in attr.children(&mut inner) {
            if child.kind() == "token_tree" {
                scan_attribute_tree(&child, source, file_id, anchor, false, edges);
            }
        }
    }
}

fn scan_attribute_tree(
    tt: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    anchor: &NodeId,
    inherited_expr: bool,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let mut in_expr = inherited_expr;
    let mut key: Option<&str> = None;
    let mut prev_kind = "";
    let mut i = 0;
    while let Some(child) = tt.child(i) {
        let kind = child.kind();
        match kind {
            "=" => in_expr = true,
            "," if !inherited_expr => in_expr = false,
            "identifier" if !in_expr => key = source.get(child.byte_range()),
            "identifier" if prev_kind != "." && tt.child(i + 1).map(|n| n.kind()) != Some("!") => {
                if !is_path_segment(tt, i) {
                    push_attribute_ref(tt, i, &child, source, file_id, anchor, edges);
                }
            }
            "string_literal" if in_expr && prev_kind == "=" => {
                let path = source
                    .get(child.byte_range())
                    .map(|t| t.trim_matches('"'))
                    .filter(|_| key.is_some_and(|k| FN_NAMING_ATTR_KEYS.contains(&k)))
                    .filter(|t| {
                        !t.is_empty() && t.split("::").all(|seg| !seg.is_empty() && is_ident(seg))
                    });
                if let Some(path) = path {
                    edges.push((
                        anchor.clone(),
                        NodeId::new(format!(
                            "{}::{path}{}",
                            file_id.as_str(),
                            super::calls::VALUE_SUFFIX
                        )),
                        EdgeType::Calls,
                    ));
                }
            }
            "token_tree" => scan_attribute_tree(&child, source, file_id, anchor, in_expr, edges),
            _ => {}
        }
        prev_kind = kind;
        i += 1;
    }
}

fn is_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Edge for the path ending at identifier `i` of an attribute token tree: a call if it is followed
/// by `( .. )` (or a turbofish and `( .. )`), otherwise a value reference.
fn push_attribute_ref(
    tt: &tree_sitter::Node,
    i: usize,
    ident: &tree_sitter::Node,
    source: &str,
    file_id: &NodeId,
    anchor: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeType)>,
) {
    let Some(name) = source.get(ident.byte_range()) else {
        return;
    };
    let called = tt
        .child(i + 1)
        .is_some_and(|n| n.kind() == "token_tree" && token_tree_starts_with_paren(&n, source))
        || turbofish_then_paren(tt, i + 1, source);
    let suffix = if called {
        ""
    } else {
        super::calls::VALUE_SUFFIX
    };
    edges.push((
        anchor.clone(),
        NodeId::new(format!(
            "{}::{}{name}{suffix}",
            file_id.as_str(),
            path_prefix(tt, i, source)
        )),
        EdgeType::Calls,
    ));
}

/// True if the tokens from `start` are `::<...>` (angle brackets balanced) followed by `( ... )`.
fn turbofish_then_paren(tt: &tree_sitter::Node, start: usize, source: &str) -> bool {
    if tt.child(start).map(|n| n.kind()) != Some("::") {
        return false;
    }
    let mut depth = 0i32;
    let mut j = start + 1;
    while let Some(n) = tt.child(j) {
        match n.kind() {
            "<" => depth += 1,
            ">" => depth -= 1,
            ">>" => depth -= 2,
            _ if depth <= 0 => return false,
            _ => {}
        }
        j += 1;
        if depth <= 0 {
            return tt.child(j).is_some_and(|p| {
                p.kind() == "token_tree" && token_tree_starts_with_paren(&p, source)
            });
        }
    }
    false
}

/// True if the identifier at child `i` is followed by `::` and another segment (not a turbofish).
fn is_path_segment(tt: &tree_sitter::Node, i: usize) -> bool {
    tt.child(i + 1).is_some_and(|n| n.kind() == "::")
        && tt.child(i + 2).map(|n| n.kind()) != Some("<")
}

/// The `A::B::` path before the identifier at child `i` of a token tree (empty if there is none),
/// so `Type::new(` keeps its path as it does outside macros instead of becoming a bare `new`.
fn path_prefix(tt: &tree_sitter::Node, i: usize, source: &str) -> String {
    let mut segments = Vec::new();
    let mut j = i;
    while j >= 2 && tt.child(j - 1).is_some_and(|n| n.kind() == "::") {
        let Some(seg) = tt
            .child(j - 2)
            .filter(|n| matches!(n.kind(), "identifier" | "crate" | "self" | "super"))
            .and_then(|n| source.get(n.byte_range()))
        else {
            break;
        };
        segments.push(seg);
        j -= 2;
    }
    segments.reverse();
    segments
        .into_iter()
        .fold(String::new(), |acc, s| acc + s + "::")
}

/// Recursively scan a `token_tree` (any delimiter: `()`, `[]`, `{}`) for identifier followed by a
/// parenthesized `token_tree`. Method calls (previous sibling `.`) become method placeholders;
/// nested macros (next sibling `!`) become `ExpandsTo` placeholders. Identifiers that name a new
/// item (`fn f`, `struct S`) or a field/binding (`name:`) are not references.
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
        let next_kind = next.map(|n| n.kind());
        let declares = matches!(
            prev_kind.as_deref(),
            Some("fn" | "struct" | "enum" | "union" | "trait" | "type" | "mod")
        );
        if kind == "identifier" && !declares && next_kind == Some("!") {
            if tt.child(i + 2).is_some_and(|n| n.kind() == "token_tree") {
                if let Some(name) = source.get(child.byte_range()) {
                    edges.push((
                        parent.clone(),
                        NodeId::new(format!("{}::{}", file_id.as_str(), name)),
                        EdgeType::ExpandsTo,
                    ));
                }
            }
        } else if kind == "identifier" && !declares {
            let prev_is_dot = prev_kind.as_deref() == Some(".");
            let called = next.is_some_and(|n| {
                n.kind() == "token_tree" && token_tree_starts_with_paren(&n, source)
            }) || turbofish_then_paren(tt, i + 1, source);
            if called {
                if let Some(name) = source.get(child.byte_range()) {
                    let (path, suffix) = if prev_is_dot {
                        let on_self = i >= 2 && tt.child(i - 2).is_some_and(|n| n.kind() == "self");
                        let hint = if on_self {
                            None
                        } else {
                            token_receiver_hint(tt, i, source)
                        };
                        (
                            String::new(),
                            format!(
                                "{}{}",
                                if on_self {
                                    super::calls::SELF_METHOD_SUFFIX
                                } else {
                                    super::calls::METHOD_SUFFIX
                                },
                                hint.map(|h| format!("{}{h}", super::calls::RECEIVER_MARK))
                                    .unwrap_or_default()
                            ),
                        )
                    } else {
                        (path_prefix(tt, i, source), String::new())
                    };
                    let callee_id =
                        NodeId::new(format!("{}::{}{}{}", file_id.as_str(), path, name, suffix));
                    edges.push((parent.clone(), callee_id, EdgeType::Calls));
                }
            } else if !prev_is_dot
                && !matches!(next_kind, Some("token_tree" | ":"))
                && !is_path_segment(tt, i)
            {
                if let Some(name) = source.get(child.byte_range()) {
                    let target = NodeId::new(format!(
                        "{}::{}{}{}",
                        file_id.as_str(),
                        path_prefix(tt, i, source),
                        name,
                        super::calls::VALUE_SUFFIX
                    ));
                    edges.push((parent.clone(), target, EdgeType::Calls));
                }
            }
        }
        if kind == "token_tree" || kind == "token_repetition" {
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

    fn parses_cleanly(src: &str) -> bool {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        !parser.parse(src, None).unwrap().root_node().has_error()
    }

    fn function_names(src: &str) -> Vec<String> {
        let store = Store::new_memory().unwrap();
        extract_ast(&store, Path::new("/t.rs"), src, Path::new("/")).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        rows.rows
            .iter()
            .filter(|r| r.get(1).is_some_and(|t| t.to_string().contains("function")))
            .filter_map(|r| r.get(2).map(ToString::to_string))
            .collect()
    }

    fn call_targets(src: &str) -> Vec<String> {
        let store = Store::new_memory().unwrap();
        extract_ast(&store, Path::new("/t.rs"), src, Path::new("/")).unwrap();
        let mut v: Vec<String> = Query::all_edges(&store)
            .unwrap()
            .rows
            .iter()
            .filter(|r| r[2].to_string().contains("calls"))
            .map(|r| r[1].to_string().trim_matches('"').to_string())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn method_and_turbofish_calls_become_placeholders() {
        let src = "fn f(x: S) { x.m(1); a.b().c(); g::<u8>(); a::b::<T>(); x.k::<T>(); }";
        let got = call_targets(src);
        let want: Vec<String> = [
            "/t.rs::a::b",
            "/t.rs::b@method",
            "/t.rs::c@method",
            "/t.rs::g",
            "/t.rs::k@method@on:S",
            "/t.rs::m@method@on:S",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(got.len(), want.len(), "{got:?}");
        for w in &want {
            assert!(
                got.iter()
                    .any(|g| g.ends_with(w.trim_start_matches("/t.rs"))),
                "{w} in {got:?}"
            );
        }
    }

    #[test]
    fn method_turbofish_in_macro_args_becomes_placeholder() {
        let got = call_targets(
            r#"fn f(x: S) { println!("{} {}", x.m::<u8>(), x.k::<Vec<Vec<u8>>>()); }"#,
        );
        for n in ["m", "k"] {
            assert!(
                got.iter()
                    .any(|g| g.ends_with(&format!("::{n}@method@on:S"))),
                "{got:?}"
            );
        }
    }

    #[test]
    fn field_access_and_paren_callee_make_no_call_edge() {
        let got = call_targets("fn f(x: S) { let _ = x.field; (x.cb)(1); }");
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn method_calls_in_macro_args_become_placeholders() {
        let got = call_targets(r#"fn f(x: S) { println!("{}", x.m()); }"#);
        assert!(
            got.iter().any(|g| g.ends_with("::m@method@on:S")),
            "{got:?}"
        );
    }

    #[test]
    fn unsafe_extern_block_parses_without_error() {
        let src = r#"unsafe extern "C" { fn ext(); } fn after() {}"#;
        assert!(parses_cleanly(src));
        assert!(function_names(src).iter().any(|n| n.contains("after")));
    }

    #[test]
    fn raw_local_index_parses_without_error() {
        let src = "fn f() { let raw = [1, 2]; let _ = &raw[..]; } fn g() {}";
        assert!(parses_cleanly(src));
        let names = function_names(src);
        for want in ["f", "g"] {
            assert!(
                names.iter().any(|n| n.trim_matches('"') == want),
                "missing {want} in {names:?}"
            );
        }
    }

    // Known gap in tree-sitter-rust 0.24.2 (issue #81): attribute inside a destructuring pattern.
    #[test]
    #[ignore = "tree-sitter-rust 0.24.2 does not parse attributes in struct patterns"]
    fn attribute_in_destructuring_pattern_parses_without_error() {
        let src = "struct S { a: u8, b: u8 } fn f(s: S) { let S { a, #[cfg(debug_assertions)] b: _ } = s; }";
        assert!(parses_cleanly(src));
    }

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

    fn impl_payloads(src: &str) -> Vec<String> {
        let store = Store::new_memory().unwrap();
        extract_ast(&store, Path::new("/t.rs"), src, Path::new("/")).unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        rows.rows
            .iter()
            .filter(|r| r[1].to_string().trim_matches('"') == "impl")
            .map(|r| r[2].to_string().trim_matches('"').to_string())
            .collect()
    }

    #[test]
    fn impl_payload_is_the_implemented_type() {
        let src = "struct P<T>(T); trait Draw {} impl P<u8> {} impl Draw for P<u8> {} impl<T> m::Draw for P<T> {}";
        assert_eq!(impl_payloads(src), vec!["P"; 3]);
        // Unsupported self types get no payload (never the trait name).
        let odd =
            "trait Draw {} impl Draw for &u8 {} impl Draw for [u8] {} impl Draw for (u8, u8) {}";
        assert!(
            impl_payloads(odd).iter().all(|p| !p.contains("Draw")),
            "{:?}",
            impl_payloads(odd)
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
        let content = r"
            unsafe fn unsafe_fn() {}
            fn with_unsafe_block() { unsafe { } }
        ";
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

    #[test]
    fn extract_ast_owns_and_borrows_placeholder_edges() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r"
            pub struct Container {
                owned_data: String,
                borrowed_ref: &'static str,
            }
            pub fn take_ownership(val: String) -> String { val }
            pub fn borrow_ref(val: &str) -> &str { val }
        ";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let owns: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "owns")
            })
            .collect();
        let borrows: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "borrows")
            })
            .collect();
        assert!(
            !owns.is_empty(),
            "expected at least one owns edge (e.g. struct->String, fn param String), got {}",
            owns.len()
        );
        assert!(
            !borrows.is_empty(),
            "expected at least one borrows edge (e.g. &str, &'static str), got {}",
            borrows.len()
        );
        let to_owns: Vec<String> = owns
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        let to_borrows: Vec<String> = borrows
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            to_owns.iter().any(|t| t.contains("String")),
            "owns should include target String, got {to_owns:?}"
        );
        assert!(
            to_borrows.iter().any(|t| t.contains("str")),
            "borrows should include target str, got {to_borrows:?}"
        );
    }

    #[test]
    fn extract_ast_return_type_owns_and_borrows_edges() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r#"
            pub struct Point { pub x: i32, pub y: i32 }
            pub fn create_point() -> Point { Point { x: 0, y: 0 } }
            pub fn get_str() -> &'static str { "hello" }
        "#;
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let owns: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "owns")
            })
            .collect();
        let borrows: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "borrows")
            })
            .collect();
        let to_owns: Vec<String> = owns
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        let to_borrows: Vec<String> = borrows
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            to_owns.iter().any(|t| t.contains("Point")),
            "owns from return type (create_point -> Point) should be present, got {to_owns:?}"
        );
        assert!(
            to_borrows.iter().any(|t| t.contains("str")),
            "borrows from return type (get_str -> str) should be present, got {to_borrows:?}"
        );
    }

    #[test]
    fn extract_ast_borrows_mut_emits_borrows_mut_edge() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r"
            pub struct S { x: i32 }
            pub fn take_mut(r: &mut S) -> &mut i32 { &mut r.x }
        ";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let borrows_mut: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "borrows_mut")
            })
            .collect();
        assert!(
            !borrows_mut.is_empty(),
            "expected at least one borrows_mut edge for &mut S or &mut i32, got {}",
            borrows_mut.len()
        );
        let to_vals: Vec<String> = borrows_mut
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            to_vals.iter().any(|t| t.contains('S') || t.contains("i32")),
            "borrows_mut should include target S or i32, got {to_vals:?}"
        );
    }

    #[test]
    fn extract_ast_lifetime_scope_self_loops() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r"
            pub struct Wrapper<'a> {
                inner: &'a str,
            }
            pub fn with_lifetime<'a>(s: &'a str) -> &'a str { s }
        ";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let lifetime_scope: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "lifetime_scope")
            })
            .collect();
        assert!(
            !lifetime_scope.is_empty(),
            "expected at least one lifetime_scope edge (self-loop on item with lifetime params), got {}",
            lifetime_scope.len()
        );
        for row in &lifetime_scope {
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
                "lifetime_scope edge should be self-loop (from == to)"
            );
        }
    }

    #[test]
    fn extract_ast_no_lifetime_scope_without_own_lifetime() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = r"
            fn outer() {
                struct Inner<'a> { x: &'a str }
            }
        ";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let lifetime_scope: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "lifetime_scope")
            })
            .collect();
        // Only Inner should have lifetime_scope, not outer
        assert_eq!(
            lifetime_scope.len(),
            1,
            "only Inner<'a> should have lifetime_scope, outer() should not; got {lifetime_scope:?}"
        );
    }

    #[test]
    fn extract_ast_tuple_struct_owns_placeholders() {
        let store = Store::new_memory().unwrap();
        let root = Path::new("/");
        let path = Path::new("/test.rs");
        let content = "pub struct Pair(Point, i32);";
        extract_ast(&store, path, content, root).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        let owns: Vec<_> = edges
            .rows
            .iter()
            .filter(|r| {
                r.get(2)
                    .is_some_and(|v| v.to_string().trim_matches('"') == "owns")
            })
            .collect();
        assert!(
            !owns.is_empty(),
            "tuple struct Pair(Point, i32) should emit at least one owns edge, got {}",
            owns.len()
        );
        let to_vals: Vec<String> = owns
            .iter()
            .filter_map(|r| {
                r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            to_vals.iter().any(|t| t.contains("Point")),
            "owns should include Point (tuple field type), got {to_vals:?}"
        );
    }

    #[test]
    fn bracket_and_brace_macros_are_scanned_like_parens() {
        let src = "fn f() { vec![a(), b.m()]; t! { k => v, w(1) }; g!(h()); }";
        let got = call_targets(src);
        for want in ["./t.rs::a", "./t.rs::m@method", "./t.rs::w", "./t.rs::h"] {
            assert!(got.iter().any(|g| g == want), "{want} missing: {got:?}");
        }
        for want in ["./t.rs::v@value", "./t.rs::k@value"] {
            assert!(got.iter().any(|g| g == want), "{want} missing: {got:?}");
        }
    }

    #[test]
    fn macro_paths_keep_their_qualifier() {
        let got = call_targets("fn f() { vec![Type::new(), a::b::c(), crate::d(), p::V] }");
        for want in [
            "./t.rs::Type::new",
            "./t.rs::a::b::c",
            "./t.rs::crate::d",
            "./t.rs::p::V@value",
        ] {
            assert!(got.iter().any(|g| g == want), "{want} missing: {got:?}");
        }
        let got = call_targets("fn f() { assert!(own::<T>(&rt).x); g!(conv::<u8>); }");
        for want in ["./t.rs::own", "./t.rs::conv@value"] {
            assert!(got.iter().any(|g| g == want), "{want} missing: {got:?}");
        }
        let got = call_targets("fn f() { vec![Type::new(), a::b::c(), crate::d(), p::V] }");
        for not in ["./t.rs::new", "./t.rs::Type@value", "./t.rs::a@value"] {
            assert!(!got.iter().any(|g| g == not), "{not} present: {got:?}");
        }
    }

    #[test]
    fn macro_rules_body_calls_belong_to_the_macro_not_the_matcher() {
        let src = "macro_rules! m { (matcher_only(x)) => { real(); fn gen() { inner() } struct S(u8); name: v }; }";
        let got = call_targets(src);
        assert!(got.contains(&"./t.rs::real".to_string()), "{got:?}");
        assert!(got.contains(&"./t.rs::inner".to_string()), "{got:?}");
        for not in ["matcher_only", "gen", "S", "name"] {
            assert!(
                !got.iter().any(|g| g.starts_with(&format!("./t.rs::{not}"))),
                "{not} is not a reference: {got:?}"
            );
        }
    }

    #[test]
    fn const_like_uses_and_turbofish_values_become_placeholders() {
        let src = "const A: u8 = f; fn g() { h(MAX, Self::ID, u8::MAX, x::<u8>); let _ = TABLE[0]; map(conv::<T>); i(conv::<T>()); }";
        let got = call_targets(src);
        for want in [
            "./t.rs::MAX@value",
            "./t.rs::Self::ID@value",
            "./t.rs::TABLE@value",
            "./t.rs::f@value",
            "./t.rs::x@value",
            "./t.rs::conv@value",
        ] {
            assert!(got.iter().any(|g| g == want), "{want} missing: {got:?}");
        }
        // the callee turbofish is a call, never a value
        assert_eq!(got.iter().filter(|g| *g == "./t.rs::conv").count(), 1);
        assert!(!got.iter().any(|g| g == "./t.rs::A@value"));
    }
}
