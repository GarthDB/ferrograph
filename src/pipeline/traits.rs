//! Phase 5: trait and impl mapping (requires rust-analyzer / full tier).
//!
//! Stub: with `ra` feature only creates an `AnalysisHost`; trait/impl extraction is TODO.

use std::path::Path;

use anyhow::Result;

use crate::graph::Store;

/// Map trait implementations to traits (full tier only).
///
/// When the `ra` feature is enabled, uses rust-analyzer's semantic model to discover
/// impl–trait relationships. Currently a stub: no `ImplementsTrait` edges are written.
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
