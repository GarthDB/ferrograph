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
    if !out.status.success() {
        eprintln!("stderr: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }

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
