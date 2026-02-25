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
    assert!(
        root.exists(),
        "fixture missing: {} (run from repo root)",
        root.display()
    );
    let store = Store::new_memory().unwrap();
    let config = PipelineConfig::default();
    run_pipeline(&store, &root, &config).expect("pipeline failed");
    let rows = ferrograph::graph::Query::all_nodes(&store).unwrap();
    assert!(
        !rows.rows.is_empty(),
        "expected at least one node from single_crate fixture"
    );
    let types: Vec<String> = rows
        .rows
        .iter()
        .filter_map(|r| r.get(1))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect();
    assert!(
        types.contains(&"function".to_string()),
        "expected at least one function node, got: {types:?}"
    );
    assert!(
        types.contains(&"file".to_string()),
        "expected file node, got: {types:?}"
    );
    let edge_rows = ferrograph::graph::Query::all_edges(&store).unwrap();
    assert!(
        !edge_rows.rows.is_empty(),
        "expected at least one edge (Contains or Calls)"
    );
}

#[test]
fn pipeline_indexes_workspace() {
    let root = fixture_path("workspace");
    assert!(
        root.exists(),
        "fixture missing: {} (run from repo root)",
        root.display()
    );
    let store = Store::new_memory().unwrap();
    let config = PipelineConfig::default();
    run_pipeline(&store, &root, &config).expect("pipeline failed");
    let rows = ferrograph::graph::Query::all_nodes(&store).unwrap();
    assert!(
        rows.rows.len() >= 2,
        "workspace should have nodes from multiple crates, got {}",
        rows.rows.len()
    );
}
