//! Resolve placeholder `Borrows` edges (item → type) from AST.
//!
//! Tracks borrow relationships (`&T`, `&mut T`) in struct fields and function parameters.
//! Tree-sitter only; no lifetime or validity analysis. Full borrow tracking would require rust-analyzer.
//!
//! # Limitations
//!
//! - **Primitives, generics, external types**: Edges to types that have no node in the graph
//!   (e.g. `str`, `i32`, generic `T`, or `Vec` from std) are removed during resolution. Only
//!   struct, enum, trait, and `type_alias` nodes in the indexed crate (or imported) are kept.
//! - **Return types**: Not included; only struct fields and function parameters drive borrows edges.
//! - **`&` vs `&mut`**: Not distinguished; both produce the same `Borrows` edge.

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `Borrows` edges (`from_id` → `file::TypeName`) to concrete type node IDs.
/// Target nodes are struct, enum, trait, `type_alias`. Uses same-file and import-based resolution.
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
        ],
    )
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::resolve_borrows_edges;

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
    fn resolve_borrows_edges_removes_unresolved_placeholder() {
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
            0,
            "unresolved placeholder (e.g. primitive str) should be removed; no edge remains"
        );
    }

    #[test]
    fn resolve_borrows_edges_removes_external_type_placeholder() {
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
            0,
            "borrows edge to external type (e.g. HashMap) with no node in graph should be removed"
        );
    }
}
