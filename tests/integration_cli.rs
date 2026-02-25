//! CLI output tests: help, index, status, query.

use std::process::Command;

fn ferrograph_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ferrograph"))
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Parse node count from `status` stdout (line containing "nodes: N").
fn parse_node_count(stdout: &str) -> u32 {
    stdout
        .lines()
        .find(|l| l.contains("nodes:"))
        .and_then(|l| {
            l.split(':')
                .nth(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
        .expect("failed to parse node count from status output")
}

#[test]
fn cli_help() {
    let out = ferrograph_cmd().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in ["index", "query", "status", "search", "watch", "mcp"] {
        assert!(
            stdout.contains(sub),
            "help should list subcommand '{sub}', got: {stdout}"
        );
    }
}

#[test]
fn cli_index_help() {
    let out = ferrograph_cmd().args(["index", "--help"]).output().unwrap();
    assert!(out.status.success());
}

#[test]
fn cli_index_and_status_and_query() {
    let fixture = fixture_path("single_crate");
    assert!(
        fixture.exists(),
        "fixture missing: {} (run from repo root)",
        fixture.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(".ferrograph");

    let out = ferrograph_cmd()
        .args([
            "index",
            "--output",
            db_path.to_str().unwrap(),
            fixture.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = ferrograph_cmd()
        .args(["status", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("nodes:"),
        "status should report nodes: {stdout}"
    );
    assert!(
        stdout.contains("edges:"),
        "status should report edges: {stdout}"
    );
    let node_count = parse_node_count(&stdout);
    assert!(
        node_count > 0,
        "expected node count > 0, got {node_count}; stdout: {stdout}"
    );

    let out = ferrograph_cmd()
        .args([
            "query",
            "--db",
            db_path.to_str().unwrap(),
            "?[id, type, payload] := *nodes[id, type, payload]",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "query should return rows: {stdout}"
    );
    assert!(
        stdout.contains("greet") || stdout.contains("main"),
        "query should return expected node payload: {stdout}"
    );
}

#[test]
fn cli_search_after_index() {
    let fixture = fixture_path("single_crate");
    assert!(fixture.exists(), "fixture missing: {}", fixture.display());
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(".ferrograph");
    let out = ferrograph_cmd()
        .args([
            "index",
            "--output",
            db_path.to_str().unwrap(),
            fixture.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = ferrograph_cmd()
        .args(["search", "--db", db_path.to_str().unwrap(), "greet"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("greet"),
        "search for 'greet' should return matching row: {stdout}"
    );
}

#[test]
fn cli_persistent_reopen() {
    let fixture = fixture_path("single_crate");
    assert!(
        fixture.exists(),
        "fixture missing: {} (run from repo root)",
        fixture.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(".ferrograph");

    let out = ferrograph_cmd()
        .args([
            "index",
            "--output",
            db_path.to_str().unwrap(),
            fixture.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out1 = ferrograph_cmd()
        .args(["status", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out1.status.success(), "status (first open) failed");
    let count1 = parse_node_count(&String::from_utf8_lossy(&out1.stdout));

    let out2 = ferrograph_cmd()
        .args(["status", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out2.status.success(), "status (reopen) failed");
    let count2 = parse_node_count(&String::from_utf8_lossy(&out2.stdout));

    assert_eq!(
        count1, count2,
        "node count should be unchanged after reopen"
    );
    assert!(count1 > 0, "expected nodes after index");
}

#[test]
fn cli_index_nonexistent_path_fails() {
    let out = ferrograph_cmd()
        .args(["index", "/nonexistent/path/xyz"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "index of nonexistent path should fail"
    );
}

#[test]
fn cli_query_invalid_datalog_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join(".ferrograph");
    // Create empty db so we open it, then run invalid query
    let store = ferrograph::graph::Store::new_persistent(&db_path).unwrap();
    drop(store);
    let out = ferrograph_cmd()
        .args([
            "query",
            "--db",
            db_path.to_str().unwrap(),
            "?[x] := *nodes[(",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "invalid Datalog should fail");
}
