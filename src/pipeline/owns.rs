//! Resolve placeholder `Owns` edges (item → type) from AST.
//!
//! Tracks owned-value relationships (e.g. struct fields by value, function parameters by value).
//! Tree-sitter only; no move/copy or closure analysis. Full ownership tracking would require rust-analyzer.
//!
//! # Limitations
//!
//! - **Primitives**: Resolved to canonical `primitive::{name}` nodes (e.g. `primitive::i32`).
//! - **Generics, external types**: Edges to types that have no node in the graph (e.g. generic `T`,
//!   or `Vec` from std) are removed during resolution. Only struct, enum, trait, `type_alias`, and
//!   primitive nodes in the graph are kept.
//! - **Return types**: Not included; only struct fields and function parameters drive owns edges.
//! - **Tuple struct fields**: Supported via `ordered_field_declaration_list`.

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `Owns` edges (`from_id` → `file::TypeName`) to concrete type node IDs.
/// Target nodes are struct, enum, trait, `type_alias`, and primitive. Uses same-file and import-based resolution.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_owns_edges(store: &Store) -> Result<()> {
    placeholder::resolve_placeholder_edges(
        store,
        &EdgeType::Owns,
        &[
            NodeType::Struct,
            NodeType::Enum,
            NodeType::Trait,
            NodeType::TypeAlias,
            NodeType::Primitive,
        ],
    )
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;
    use crate::pipeline::primitives;

    use super::resolve_owns_edges;

    #[test]
    fn resolve_owns_edges_resolves_same_file_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let struct_id = NodeId::new(format!("{path}#8:1"));
        store
            .put_node(&struct_id, &NodeType::Struct, Some("Point"))
            .unwrap();
        let fn_id = NodeId::new(format!("{path}#12:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("take_point"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Point"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Owns)
            .unwrap();
        resolve_owns_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert!(
            to_str.contains('#'),
            "edge should point to real type id (path#line:col), got {to_str}"
        );
        assert_eq!(to_str, format!("{path}#8:1"));
    }

    #[test]
    fn resolve_owns_edges_resolves_primitive_placeholder() {
        let store = Store::new_memory().unwrap();
        primitives::create_primitive_nodes(&store).unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::i32"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Owns)
            .unwrap();
        resolve_owns_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(
            edges.rows.len(),
            1,
            "owns edge to i32 should resolve to primitive::i32"
        );
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(
            to_str, "primitive::i32",
            "edge should point to primitive::i32, got {to_str}"
        );
    }

    #[test]
    fn resolve_owns_edges_removes_external_type_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Vec"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Owns)
            .unwrap();
        resolve_owns_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(
            edges.rows.len(),
            0,
            "owns edge to external type (e.g. Vec) with no node in graph should be removed"
        );
    }
}
