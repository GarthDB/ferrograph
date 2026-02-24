//! Phase 2: AST extraction (tree-sitter) into graph nodes and edges.

use std::path::Path;

use anyhow::Result;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE;

use crate::graph::schema::{EdgeType, NodeId, NodeType};

/// Result of AST extraction: nodes and edges to add to the graph.
pub struct AstResult {
    pub nodes: Vec<(NodeId, NodeType, Option<String>)>,
    pub edges: Vec<(NodeId, NodeId, EdgeType)>,
}

/// Extract graph nodes and edges from Rust source.
///
/// # Errors
/// Fails if parsing fails or the language cannot be loaded.
pub fn extract_ast(path: &Path, content: &str) -> Result<AstResult> {
    let mut parser = Parser::new();
    parser.set_language(&LANGUAGE.into())?;
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("parse failed"))?;
    let file_id = NodeId(path.to_string_lossy().to_string());
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

    Ok(AstResult { nodes, edges })
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
        let id = NodeId(format!(
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
                return Some(source[r.start..r.end].to_string());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn resolve_call_target(node: &tree_sitter::Node, source: &str, file_id: &NodeId) -> Option<NodeId> {
    // Simplified: we don't resolve names to definitions here; we use a placeholder.
    // Full resolution would require module/type context.
    let child = node.child(0)?;
    let name = if child.kind() == "identifier" {
        let r = child.byte_range();
        source.get(r.start..r.end).map(String::from)
    } else {
        None
    };
    name.map(|n| NodeId(format!("{}::{}", file_id.0, n)))
}
