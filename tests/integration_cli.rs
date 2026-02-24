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

#[test]
fn cli_help() {
    let out = ferrograph_cmd().arg("--help").output().unwrap();
    assert!(out.status.success());
}

#[test]
fn cli_index_help() {
    let out = ferrograph_cmd().args(["index", "--help"]).output().unwrap();
    assert!(out.status.success());
}

#[test]
fn cli_index_and_status_and_query() {
    let fixture = fixture_path("single_crate");
    if !fixture.exists() {
        return;
    }
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
    let node_count = stdout
        .lines()
        .find(|l| l.contains("nodes:"))
        .and_then(|l| {
            l.split(':')
                .nth(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
        .unwrap_or(0);
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
}

#[test]
fn cli_persistent_reopen() {
    let fixture = fixture_path("single_crate");
    if !fixture.exists() {
        return;
    }
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
    let stdout1 = String::from_utf8_lossy(&out1.stdout);
    let count1 = stdout1
        .lines()
        .find(|l| l.contains("nodes:"))
        .and_then(|l| {
            l.split(':')
                .nth(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
        .unwrap_or(0);

    let out2 = ferrograph_cmd()
        .args(["status", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out2.status.success(), "status (reopen) failed");
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    let count2 = stdout2
        .lines()
        .find(|l| l.contains("nodes:"))
        .and_then(|l| {
            l.split(':')
                .nth(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
        .unwrap_or(0);

    assert_eq!(
        count1, count2,
        "node count should be unchanged after reopen"
    );
    assert!(count1 > 0, "expected nodes after index");
}
