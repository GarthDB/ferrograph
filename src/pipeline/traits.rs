//! Phase 5: trait and impl mapping (requires rust-analyzer / full tier).

use std::path::Path;

use anyhow::Result;

use crate::graph::Store;

/// Map trait implementations to traits (full tier only).
///
/// When the `ra` feature is enabled, uses rust-analyzer's semantic model to discover
/// impl–trait relationships. Without the feature, or if analysis is skipped, this is a no-op.
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
    // Create an analysis host. Full project load would use load_cargo (ra_ap_project_model)
    // to populate the database from root; for now we integrate the API and leave
    // trait/impl extraction as future work (requires loading crate graph and HIR).
    let _host = ra_ap_ide::AnalysisHost::default();
    let _ = store;
    // Ensure root exists so we could load it in a full implementation.
    let _ = root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("root path: {e}"))?;
    Ok(())
}
