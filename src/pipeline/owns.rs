//! Resolve placeholder `Owns` edges (item → type) from AST.
//!
//! Tracks owned-value relationships (e.g. struct fields by value, function parameters by value).
//! Tree-sitter only; no move/copy or closure analysis. Full ownership tracking would require rust-analyzer.

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `Owns` edges (`from_id` → `file::TypeName`) to concrete type node IDs.
/// Target nodes are struct, enum, trait, `type_alias`. Uses same-file and import-based resolution.
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
        ],
    )
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

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
}
