//! Phase 4: call graph construction (enrichment from AST edges).

use anyhow::Result;

use crate::graph::Store;

/// Build or enrich call graph (AST already added Calls edges; this can add transitive/typed edges).
///
/// # Errors
/// Fails if the graph query or update fails.
pub fn build_call_graph(_store: &Store) -> Result<()> {
    Ok(())
}
