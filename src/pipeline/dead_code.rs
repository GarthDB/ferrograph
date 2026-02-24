//! Phase 9: dead code detection via reachability on the call graph.

use anyhow::Result;

use crate::graph::query::Query;
use crate::graph::Store;

/// Compute and record dead (unreachable) functions. Uses reachability from
/// entry points (functions with no incoming call edge). Persists results to
/// the `dead_functions` relation for MCP/CLI queries.
///
/// # Errors
/// Fails if reachability analysis or graph update fails.
pub fn detect_dead_code(store: &Store) -> Result<()> {
    let dead = Query::dead_function_ids(store)?;
    store.clear_dead_functions()?;
    for id in &dead {
        store.put_dead_function(id)?;
    }
    Ok(())
}
