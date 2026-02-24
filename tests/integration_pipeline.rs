//! End-to-end pipeline tests using fixture projects.

use std::path::Path;

use ferrograph::graph::Store;
use ferrograph::pipeline::{run_pipeline, PipelineConfig};

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn pipeline_indexes_single_crate() {
    let root = fixture_path("single_crate");
    if !root.exists() {
        return;
    }
    let store = Store::new_memory().unwrap();
    let config = PipelineConfig::default();
    let result = run_pipeline(&store, &root, &config);
    assert!(result.is_ok(), "pipeline failed: {result:?}");
    let rows = ferrograph::graph::Query::all_nodes(&store).unwrap();
    assert!(
        !rows.rows.is_empty(),
        "expected at least one node from single_crate fixture"
    );
}
