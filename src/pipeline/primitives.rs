//! Canonical graph nodes for Rust primitive types.
//!
//! Creates `primitive::{name}` nodes so that placeholder edges (e.g. `file::i32`, `file::str`)
//! emitted during AST extraction can resolve to a concrete node. Without these nodes, edges to
//! primitives are removed during resolution.

use anyhow::Result;

use crate::graph::schema::{NodeId, NodeType};
use crate::graph::Store;

/// Rust primitive type names (matches tree-sitter-rust's `primitive_type`).
const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64",
];

/// Create canonical nodes for all Rust primitive types. Call once at pipeline start so that
/// owns/borrows/references resolution can resolve placeholders like `path::i32` to `primitive::i32`.
///
/// # Errors
/// Fails if the store batch insert fails.
pub fn create_primitive_nodes(store: &Store) -> Result<()> {
    let nodes: Vec<_> = PRIMITIVES
        .iter()
        .map(|name| {
            (
                NodeId::new(format!("primitive::{name}")),
                NodeType::Primitive,
                Some(*name),
            )
        })
        .collect();
    store.put_nodes_batch(&nodes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;

    use super::create_primitive_nodes;

    #[test]
    fn primitive_nodes_created_and_queryable() {
        let store = crate::graph::Store::new_memory().unwrap();
        create_primitive_nodes(&store).unwrap();
        let nodes = Query::all_nodes(&store).unwrap();
        let primitives: Vec<_> = nodes
            .rows
            .iter()
            .filter(|r| {
                r.get(1).map(|v| v.to_string().trim_matches('"').to_string())
                    == Some("primitive".to_string())
            })
            .collect();
        assert_eq!(
            primitives.len(),
            17,
            "expected 17 primitive nodes, got {}",
            primitives.len()
        );
        let ids: Vec<String> = primitives
            .iter()
            .filter_map(|r| {
                r.first()
                    .map(|v| v.to_string().trim_matches('"').to_string())
            })
            .collect();
        assert!(
            ids.iter().any(|id| id == "primitive::i32"),
            "expected primitive::i32, got {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id == "primitive::str"),
            "expected primitive::str, got {ids:?}"
        );
    }
}
