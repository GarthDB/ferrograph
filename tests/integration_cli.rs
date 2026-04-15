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

/// Parse node count from `status` stdout (line containing "Nodes (N total)").
fn parse_node_count(stdout: &str) -> u32 {
    stdout
        .lines()
        .find(|l| l.contains("Nodes ("))
        .and_then(|l| {
            // Extract number from "  Nodes (42 total):"
            let start = l.find('(')? + 1;
            let end = l.find(" total")?;
            l[start..end].trim().parse::<u32>().ok()
        })
        .expect("failed to parse node count from status output")
}

#[test]
fn cli_help() {
    let out = ferrograph_cmd().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in [
        "index", "query", "status", "search", "watch", "claude", "mcp",
    ] {
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
        stdout.contains("Nodes ("),
        "status should report nodes: {stdout}"
    );
    assert!(
        stdout.contains("Edges ("),
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

/// Index the `single_crate` fixture into a temp DB and return `(db_path, tempdir)`.
/// The tempdir is returned so the caller keeps it alive for the test's duration.
fn index_fixture() -> (std::path::PathBuf, tempfile::TempDir) {
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
    (db_path, dir)
}

#[test]
fn cli_dead() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args(["dead", "--db", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dead failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("private_unused"),
        "dead should include private_unused: {stdout}"
    );
    assert!(
        stdout.contains("dead nodes found"),
        "dead should print summary line: {stdout}"
    );
    assert!(
        stdout.contains("dynamic dispatch"),
        "dead should print dynamic dispatch caveat: {stdout}"
    );
}

#[test]
fn cli_dead_json() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args(["--json", "dead", "--db", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dead --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("dead --json output is not valid JSON: {e}\n{stdout}"));
    assert!(
        parsed["count"].as_u64().unwrap_or(0) > 0,
        "dead --json should report count > 0: {stdout}"
    );
    assert!(
        parsed["dead_nodes"].is_array(),
        "dead --json should have dead_nodes array: {stdout}"
    );
    assert!(
        parsed["caveat"].is_string(),
        "dead --json should include caveat: {stdout}"
    );
}

#[test]
fn cli_blast() {
    let (db_path, _dir) = index_fixture();
    // use_add calls utils::add, so blast radius from use_add should include something
    let out = ferrograph_cmd()
        .args(["search", "--db", db_path.to_str().unwrap(), "pub::use_add"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let node_id = stdout.lines().next().unwrap().split('\t').next().unwrap();

    let out = ferrograph_cmd()
        .args(["blast", "--db", db_path.to_str().unwrap(), node_id])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "blast failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("blast radius"),
        "blast should print summary: {stdout}"
    );
}

#[test]
fn cli_info() {
    let (db_path, _dir) = index_fixture();
    // Search for greet function to get a node ID
    let out = ferrograph_cmd()
        .args(["search", "--db", db_path.to_str().unwrap(), "pub::greet"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let node_id = stdout.lines().next().unwrap().split('\t').next().unwrap();

    let out = ferrograph_cmd()
        .args(["info", "--db", db_path.to_str().unwrap(), node_id])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "info failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("greet"),
        "info should show node payload: {stdout}"
    );
}

#[test]
fn cli_info_not_found() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args(["info", "--db", db_path.to_str().unwrap(), "nonexistent#0:0"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("not found"),
        "info for missing node should say not found: {stdout}"
    );
}

#[test]
fn cli_modules() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args(["modules", "--db", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "modules failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("containment edges"),
        "modules should print summary: {stdout}"
    );
}

#[test]
fn cli_traits() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args(["traits", "--db", db_path.to_str().unwrap(), "Draw"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "traits failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("implementors of"),
        "traits should print summary: {stdout}"
    );
}

#[test]
fn cli_callers() {
    let (db_path, _dir) = index_fixture();
    // Find the add function (called by use_add)
    let out = ferrograph_cmd()
        .args(["search", "--db", db_path.to_str().unwrap(), "pub::add"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let node_id = stdout.lines().next().unwrap().split('\t').next().unwrap();

    let out = ferrograph_cmd()
        .args(["callers", "--db", db_path.to_str().unwrap(), node_id])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "callers failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("callers of"),
        "callers should print summary: {stdout}"
    );
}

#[test]
fn cli_status_json() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args([
            "--json",
            "status",
            db_path.parent().unwrap().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "status --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("status --json output is not valid JSON: {e}\n{stdout}"));
    assert!(
        parsed["node_count"].as_u64().unwrap_or(0) > 0,
        "status --json should report node_count > 0: {stdout}"
    );
    assert!(
        parsed["nodes_by_type"].is_array(),
        "status --json should have nodes_by_type: {stdout}"
    );
    assert!(
        parsed["edges_by_type"].is_array(),
        "status --json should have edges_by_type: {stdout}"
    );
}

#[test]
fn cli_query_json() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args([
            "--json",
            "query",
            "--db",
            db_path.to_str().unwrap(),
            "?[n] := n in [1, 2, 3]",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "query --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("query --json output is not valid JSON: {e}\n{stdout}"));
    let rows = parsed["rows"].as_array().expect("rows should be array");
    assert_eq!(rows.len(), 3, "should have 3 rows");
    // Values should be numeric, not stringified
    assert!(
        rows[0][0].is_number(),
        "query --json should preserve numeric types, got: {}",
        rows[0][0]
    );
}

#[test]
fn cli_search_json() {
    let (db_path, _dir) = index_fixture();
    let out = ferrograph_cmd()
        .args([
            "--json",
            "search",
            "--db",
            db_path.to_str().unwrap(),
            "greet",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "search --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("search --json output is not valid JSON: {e}\n{stdout}"));
    assert!(
        parsed["count"].as_u64().unwrap_or(0) > 0,
        "search --json should find results: {stdout}"
    );
    assert!(
        parsed["results"].is_array(),
        "search --json should have results array: {stdout}"
    );
}

#[test]
fn cli_help_lists_new_subcommands() {
    let out = ferrograph_cmd().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in ["dead", "blast", "callers", "info", "modules", "traits"] {
        assert!(
            stdout.contains(sub),
            "help should list new subcommand '{sub}', got: {stdout}"
        );
    }
    assert!(
        stdout.contains("--json"),
        "help should list --json flag: {stdout}"
    );
}

#[test]
fn cli_claude_status() {
    let out = ferrograph_cmd()
        .args(["claude", "status"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "claude status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Skill file:"),
        "claude status should report skill file: {stdout}"
    );
}

#[test]
fn cli_claude_status_json() {
    let out = ferrograph_cmd()
        .args(["--json", "claude", "status"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "claude status --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("claude status --json not valid JSON: {e}\n{stdout}"));
    assert!(
        parsed["skill_file"].is_boolean(),
        "should have skill_file boolean: {stdout}"
    );
    assert!(
        parsed["up_to_date"].is_boolean(),
        "should have up_to_date boolean: {stdout}"
    );
}

#[test]
fn cli_claude_help() {
    let out = ferrograph_cmd()
        .args(["claude", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in ["install", "uninstall", "status"] {
        assert!(
            stdout.contains(sub),
            "claude help should list '{sub}': {stdout}"
        );
    }
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
