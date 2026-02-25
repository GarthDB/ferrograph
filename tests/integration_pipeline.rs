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
    for want in ["struct", "enum", "trait", "impl"] {
        assert!(
            types.contains(&want.to_string()),
            "expected {want} node from fixture, got: {types:?}"
        );
    }
    let edge_rows = ferrograph::graph::Query::all_edges(&store).unwrap();
    assert!(
        !edge_rows.rows.is_empty(),
        "expected at least one edge (Contains or Calls)"
    );
    let edge_types: Vec<String> = edge_rows
        .rows
        .iter()
        .filter_map(|r| r.get(2))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect();
    assert!(
        edge_types.contains(&"contains".to_string()),
        "expected Contains edges, got: {edge_types:?}"
    );
    assert!(
        edge_types.contains(&"calls".to_string()),
        "expected Calls edges (e.g. main -> bar), got: {edge_types:?}"
    );
    let dead = ferrograph::graph::Query::stored_dead_functions(&store).unwrap();
    assert!(
        !dead.is_empty(),
        "fixture has unused() which should be detected as dead, got: {dead:?}"
    );
    // Test functions are entry points and must not be reported dead.
    let nodes = ferrograph::graph::Query::all_nodes(&store).unwrap();
    let test_fn_ids: Vec<String> = nodes
        .rows
        .iter()
        .filter_map(|r| {
            let payload = r
                .get(2)
                .map(|v| v.to_string().trim_matches('"').to_string())?;
            if payload.starts_with("test::") {
                r.first()
                    .map(|v| v.to_string().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect();
    for id in &test_fn_ids {
        assert!(
            !dead.contains(id),
            "test function {id} must not be in dead list, dead: {dead:?}"
        );
    }
    // Mod/use resolution: main.rs has "use crate::greet" and calls greet(); expect a Calls edge to greet (in lib).
    let edges = ferrograph::graph::Query::all_edges(&store).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(&store).unwrap();
    let greet_id = nodes.rows.iter().find_map(|r| {
        let id = r
            .get(0)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let payload = r
            .get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        if (payload == "greet" || payload == "pub::greet") && id.contains("lib.rs") {
            Some(id)
        } else {
            None
        }
    });
    let has_call_to_greet = greet_id.map_or(false, |gid| {
        edges.rows.iter().any(|r| {
            r.get(2)
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some("calls")
                && r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .as_deref()
                    == Some(&gid)
        })
    });
    assert!(
        has_call_to_greet,
        "expected a Calls edge to greet (from main.rs use crate::greet; greet();), edges: {:?}",
        edges.rows.len()
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
        rows.rows.len() >= 4,
        "workspace should have nodes from multiple crates, got {}",
        rows.rows.len()
    );
    let payloads: Vec<String> = rows
        .rows
        .iter()
        .filter_map(|r| r.get(2))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect();
    assert!(
        payloads.iter().any(|p| p == "a" || p.starts_with("pub::a")),
        "expected node from crate_a (e.g. 'a'), got payloads: {payloads:?}"
    );
    assert!(
        payloads.iter().any(|p| p == "b" || p.starts_with("pub::b")),
        "expected node from crate_b (e.g. 'b'), got payloads: {payloads:?}"
    );
}
