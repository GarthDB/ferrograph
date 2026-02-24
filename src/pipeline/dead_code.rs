//! Phase 9: dead code detection via reachability on the call graph.

use anyhow::Result;

use crate::graph::query::Query;
use crate::graph::Store;

/// Compute and record dead (unreachable) functions. Uses reachability from
/// entry points (functions with no incoming call edge).
///
/// # Errors
/// Fails if reachability analysis or graph update fails.
pub fn detect_dead_code(store: &Store) -> Result<()> {
    let _dead = Query::dead_function_ids(store)?;
    // TODO: persist _dead to a dead_functions relation for MCP/CLI queries
    Ok(())
}
