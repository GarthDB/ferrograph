//! Phase 5: trait and impl mapping (requires rust-analyzer / full tier).
//!
//! Tree-sitter: resolves placeholder `ImplementsTrait` edges (impl → trait) emitted by AST.
//! With `ra` feature: stub for rust-analyzer-based extraction.

use std::path::Path;

use anyhow::Result;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::Store;

use super::placeholder;

/// Resolve placeholder `ImplementsTrait` edges (`impl_id` → `file::TraitName`) to concrete trait node IDs.
/// Runs after AST and modules; uses same-file and import-based resolution like call graph.
///
/// # Errors
/// Fails if the store query or update fails.
pub fn resolve_impl_trait_edges(store: &Store) -> Result<()> {
    placeholder::resolve_placeholder_edges(store, &EdgeType::ImplementsTrait, &[NodeType::Trait])
}

/// Map trait implementations to traits (full tier only).
///
/// When the `ra` feature is enabled, uses rust-analyzer's semantic model to discover
/// impl–trait relationships. Tree-sitter placeholder resolution is done by `resolve_impl_trait_edges`.
///
/// # Errors
/// Fails if rust-analyzer or graph update fails.
pub fn map_traits(store: &Store, root: &Path) -> Result<()> {
    #[cfg(feature = "ra")]
    {
        map_traits_ra(store, root)?;
    }
    #[cfg(not(feature = "ra"))]
    {
        let _ = (store, root);
    }
    Ok(())
}

#[cfg(feature = "ra")]
fn map_traits_ra(store: &Store, root: &Path) -> Result<()> {
    let _ = store;
    // TODO: Load project and extract ImplementsTrait edges. For now just create host and validate root.
    // Full project load would use load_cargo (ra_ap_project_model)
    // to populate the database from root; for now we integrate the API and leave
    // trait/impl extraction as future work (requires loading crate graph and HIR).
    let _host = ra_ap_ide::AnalysisHost::default();
    // Ensure root exists so we could load it in a full implementation.
    let _ = root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("root path: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::resolve_impl_trait_edges;

    #[test]
    fn resolve_impl_trait_edges_resolves_same_file_placeholder() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let trait_id = NodeId::new(format!("{path}#10:1"));
        store
            .put_node(&trait_id, &NodeType::Trait, Some("Draw"))
            .unwrap();
        let impl_id = NodeId::new(format!("{path}#15:1"));
        store
            .put_node(&impl_id, &NodeType::Impl, Some("Point"))
            .unwrap();
        let placeholder = NodeId::new(format!("{path}::Draw"));
        store
            .put_edge(&impl_id, &placeholder, &EdgeType::ImplementsTrait)
            .unwrap();
        resolve_impl_trait_edges(&store).unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let to_str = edges.rows[0][1].to_string().trim_matches('"').to_string();
        assert!(
            to_str.contains('#'),
            "edge should point to real trait id (path#line:col), got {to_str}"
        );
        assert_eq!(to_str, format!("{path}#10:1"));
    }
}
