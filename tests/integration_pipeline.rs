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

fn run_fixture(name: &str) -> Store {
    let root = fixture_path(name);
    assert!(
        root.exists(),
        "fixture missing: {} (run from repo root)",
        root.display()
    );
    let store = Store::new_memory().unwrap();
    let config = PipelineConfig::default();
    run_pipeline(&store, &root, &config).expect("pipeline failed");
    store
}

fn node_types(store: &Store) -> Vec<String> {
    ferrograph::graph::Query::all_nodes(store)
        .unwrap()
        .rows
        .iter()
        .filter_map(|r| r.get(1))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect()
}

fn edge_types(store: &Store) -> Vec<String> {
    ferrograph::graph::Query::all_edges(store)
        .unwrap()
        .rows
        .iter()
        .filter_map(|r| r.get(2))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect()
}

fn assert_test_fns_not_dead(store: &Store) {
    let dead = ferrograph::graph::Query::stored_dead_functions(store).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
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
}

/// Find a node ID by payload (substring match) and optional type. Prefers lib.rs for `single_crate`.
fn find_node_id(
    store: &Store,
    payload_contains: &str,
    node_type_filter: Option<&str>,
) -> Option<String> {
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    for row in &nodes.rows {
        let id = row
            .first()
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let typ = row
            .get(1)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let payload = row
            .get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        if payload.contains(payload_contains)
            && node_type_filter.is_none_or(|t| typ == t)
            && id.contains("lib.rs")
        {
            return Some(id);
        }
    }
    None
}

/// Assert that an edge exists from a node (by payload) to another (by payload) with the given edge type.
fn assert_has_edge(
    store: &Store,
    from_payload: &str,
    from_type: &str,
    to_payload: &str,
    to_type: &str,
    edge_type: &str,
) {
    let from_id = find_node_id(store, from_payload, Some(from_type))
        .unwrap_or_else(|| panic!("node not found: {from_payload} ({from_type})"));
    let to_id = find_node_id(store, to_payload, Some(to_type))
        .unwrap_or_else(|| panic!("node not found: {to_payload} ({to_type})"));
    let edges = ferrograph::graph::Query::all_edges(store).unwrap();
    let has = edges.rows.iter().any(|r| {
        r.get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .as_deref()
            == Some(edge_type)
            && r.first()
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some(from_id.as_str())
            && r.get(1)
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some(to_id.as_str())
    });
    assert!(
        has,
        "expected {edge_type} edge from {from_payload} to {to_payload}, from_id={from_id}, to_id={to_id}"
    );
}

fn assert_call_to_greet(store: &Store) {
    let edges = ferrograph::graph::Query::all_edges(store).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    let greet_id = nodes.rows.iter().find_map(|r| {
        let id = r
            .first()
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
    let has_call = greet_id.is_some_and(|gid| {
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
        has_call,
        "expected a Calls edge to greet (from main.rs use crate::greet; greet();)"
    );
}

#[test]
fn pipeline_indexes_single_crate() {
    let store = run_fixture("single_crate");
    let types = node_types(&store);
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
    let etypes = edge_types(&store);
    assert!(
        etypes.contains(&"contains".to_string()),
        "expected Contains edges, got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"calls".to_string()),
        "expected Calls edges (e.g. main -> bar), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"owns".to_string()),
        "expected Owns edges (e.g. Container->Point, take_ownership->Point), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"borrows".to_string()),
        "expected Borrows edges (e.g. borrow_ref->Point), got: {etypes:?}"
    );
    assert_has_edge(&store, "Container", "struct", "Point", "struct", "owns");
    assert_has_edge(
        &store,
        "borrow_ref",
        "function",
        "Point",
        "struct",
        "borrows",
    );
    assert_has_edge(&store, "Pair", "struct", "Point", "struct", "owns");
    let dead = ferrograph::graph::Query::stored_dead_functions(&store).unwrap();
    assert!(
        !dead.is_empty(),
        "fixture has unused() which should be detected as dead, got: {dead:?}"
    );
    assert_test_fns_not_dead(&store);
    assert_call_to_greet(&store);
}

#[test]
fn pipeline_indexes_workspace() {
    let store = run_fixture("workspace");
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
