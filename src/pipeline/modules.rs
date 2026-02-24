//! Phase 3: module and import resolution.

use std::path::Path;

use anyhow::Result;

use crate::graph::Store;

/// Resolve `mod` and `use` statements into graph edges (no-op for fast tier beyond AST).
///
/// # Errors
/// Fails if module resolution encounters an error.
pub fn resolve_modules(_store: &Store, _root: &Path) -> Result<()> {
    Ok(())
}
