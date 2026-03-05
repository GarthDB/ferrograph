//! Resolve placeholder `ExpandsTo` edges (invocation site → macro definition) from AST.

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `ExpandsTo` edges (`from_id` → `file::macro_name`) to concrete macro definition node IDs.
/// Uses same-file and import-based resolution like call graph.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_expands_to_edges(store: &Store) -> Result<()> {
    placeholder::resolve_placeholder_edges(store, &EdgeType::ExpandsTo, &[NodeType::Macro])
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::resolve_expands_to_edges;

    #[test]
    fn resolve_expands_to_edges_resolves_same_file_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let macro_id = NodeId::new(format!("{path}#3:1"));
        store
            .put_node(&macro_id, &NodeType::Macro, Some("my_macro"))
            .unwrap();
        let fn_id = NodeId::new(format!("{path}#10:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("caller"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::my_macro"));
        store
            .put_edge(&fn_id, &placeholder, &EdgeType::ExpandsTo)
            .unwrap();
        resolve_expands_to_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert!(
            to_str.contains('#'),
            "edge should point to real macro id (path#line:col), got {to_str}"
        );
        assert_eq!(to_str, format!("{path}#3:1"));
    }
}
