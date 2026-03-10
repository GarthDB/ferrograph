//! Resolve placeholder `Borrows` edges (item → type) from AST.
//!
//! Tracks borrow relationships (`&T`, `&mut T`) in struct fields and function parameters.
//! Tree-sitter only; no lifetime or validity analysis. Full borrow tracking would require rust-analyzer.
//!
//! # Limitations
//!
//! - **Primitives**: Resolved to canonical `primitive::{name}` nodes (e.g. `primitive::str`).
//! - **Generics, external types**: Edges to types with no node in the graph are re-pointed to
//!   synthetic `ExternalType` nodes (e.g. `external::Vec`) so the dependency is retained.
//! - **Return types**: Included; functions that return by-reference (`&T`, `&mut T`) get a borrows edge to that type.
//! - **`&` vs `&mut`**: Distinguished via `Borrows` (shared) and `BorrowsMut` (exclusive) edge types.

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `Borrows` edges (`from_id` → `file::TypeName`) to concrete type node IDs.
/// Target nodes are struct, enum, trait, `type_alias`, and primitive. Uses same-file and import-based resolution.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_borrows_edges(store: &Store) -> Result<()> {
    placeholder::resolve_placeholder_edges(
        store,
        &EdgeType::Borrows,
        &[
            NodeType::Struct,
            NodeType::Enum,
            NodeType::Trait,
            NodeType::TypeAlias,
            NodeType::Primitive,
        ],
    )
}

/// Resolve placeholder `BorrowsMut` edges (`from_id` → `file::TypeName`) to concrete type node IDs.
/// Same resolution rules as `resolve_borrows_edges`.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_borrows_mut_edges(store: &Store) -> Result<()> {
    placeholder::resolve_placeholder_edges(
        store,
        &EdgeType::BorrowsMut,
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

    use super::{resolve_borrows_edges, resolve_borrows_mut_edges};

    #[test]
    fn resolve_borrows_edges_resolves_same_file_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let struct_id = NodeId::new(format!("{path}#8:1"));
        store
            .put_node(&struct_id, &NodeType::Struct, Some("Point"))
            .unwrap();
        let fn_id = NodeId::new(format!("{path}#12:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("borrow_point"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Point"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Borrows)
            .unwrap();
        resolve_borrows_edges(&store).unwrap();
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
    fn resolve_borrows_edges_resolves_primitive_placeholder() {
        let store = Store::new_memory().unwrap();
        primitives::create_primitive_nodes(&store).unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::str"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Borrows)
            .unwrap();
        resolve_borrows_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(
            edges.rows.len(),
            1,
            "borrows edge to str should resolve to primitive::str"
        );
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(
            to_str, "primitive::str",
            "edge should point to primitive::str, got {to_str}"
        );
    }

    #[test]
    fn resolve_borrows_edges_retains_unresolved_as_external_type() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::str"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Borrows)
            .unwrap();
        resolve_borrows_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(
            edges.rows.len(),
            1,
            "edge should point to external_type node when primitives not created"
        );
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(to_str, "external::str");
    }

    #[test]
    fn resolve_borrows_edges_retains_external_type_as_synthetic_node() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::HashMap"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::Borrows)
            .unwrap();
        resolve_borrows_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(
            edges.rows.len(),
            1,
            "edge should point to external_type node"
        );
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(to_str, "external::HashMap");
    }

    #[test]
    fn resolve_borrows_mut_edges_resolves_same_file_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let struct_id = NodeId::new(format!("{path}#8:1"));
        store
            .put_node(&struct_id, &NodeType::Struct, Some("Point"))
            .unwrap();
        let fn_id = NodeId::new(format!("{path}#12:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("borrow_mut_point"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Point"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::BorrowsMut)
            .unwrap();
        resolve_borrows_mut_edges(&store).unwrap();
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
    fn resolve_borrows_mut_edges_retains_unresolved_as_external_type() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("f"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Vec"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::BorrowsMut)
            .unwrap();
        resolve_borrows_mut_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert_eq!(to_str, "external::Vec");
    }
}
