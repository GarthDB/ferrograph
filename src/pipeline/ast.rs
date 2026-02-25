//! Phase 2: AST extraction (tree-sitter) into graph nodes and edges.

use std::path::Path;

use anyhow::Result;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE;

use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::Store;

/// Extract graph nodes and edges from Rust source and write them to the store.
///
/// # Errors
/// Fails if parsing fails, the language cannot be loaded, or store writes fail.
pub fn extract_ast(store: &Store, path: &Path, content: &str) -> Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&LANGUAGE.into())?;
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("parse failed"))?;
    if tree.root_node().has_error() {
        anyhow::bail!("parse failed");
    }
    let file_id = NodeId::new(path.to_string_lossy().to_string());
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

    let (node_type, name_opt) = match kind {
        "function_item" => (Some(NodeType::Function), name_of_node(&node, source)),
        "struct_item" => (Some(NodeType::Struct), name_of_node(&node, source)),
        "enum_item" => (Some(NodeType::Enum), name_of_node(&node, source)),
        "trait_item" => (Some(NodeType::Trait), name_of_node(&node, source)),
        "impl_item" => (Some(NodeType::Impl), None),
        "type_item" => (Some(NodeType::TypeAlias), name_of_node(&node, source)),
        "const_item" => (Some(NodeType::Const), name_of_node(&node, source)),
        "static_item" => (Some(NodeType::Static), name_of_node(&node, source)),
        "macro_definition" => (Some(NodeType::Macro), name_of_node(&node, source)),
        "call_expression" => {
            if let Some(callee_id) = resolve_call_target(&node, source, file_id) {
                if let Some(ref from) = parent {
                    edges.push((from.clone(), callee_id, EdgeType::Calls));
                }
            }
            (None, None)
        }
        _ => (None, None),
    };

    let added = if let (Some(nt), no) = (node_type, name_opt) {
        let id = NodeId::new(format!(
            "{}#{}:{}",
            file_id.0,
            node.start_position().row + 1,
            node.start_position().column + 1
        ));
        nodes.push((id.clone(), nt, no));
        if let Some(ref p) = parent {
            edges.push((p.clone(), id.clone(), EdgeType::Contains));
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
            if n.kind() == "identifier" {
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

/// Resolve a call expression to a placeholder node id (`file_path::fn_name`).
///
/// **Limitation**: Only bare identifier calls like `foo()` are handled. Method calls
/// (`self.foo()`, `x.bar()`), qualified paths (`mod::foo()`), and UFCS (`Type::method()`)
/// are not resolved and do not produce call edges.
fn resolve_call_target(node: &tree_sitter::Node, source: &str, file_id: &NodeId) -> Option<NodeId> {
    let child = node.child(0)?;
    let name = if child.kind() == "identifier" {
        let r = child.byte_range();
        source.get(r.start..r.end).map(String::from)
    } else {
        None
    };
    name.map(|n| NodeId::new(format!("{}::{}", file_id.0, n)))
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
        let path = Path::new("/test.rs");
        let content = "fn foo() {}";
        extract_ast(&store, path, content).unwrap();
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
    fn extract_ast_empty_file_fails_parse() {
        let store = Store::new_memory().unwrap();
        let path = Path::new("/test.rs");
        let content = "not valid rust {{{";
        let result = extract_ast(&store, path, content);
        assert!(result.is_err());
    }
}
