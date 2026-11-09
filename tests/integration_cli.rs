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
        parsed["root"].is_string(),
        "status --json should report the project root: {stdout}"
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

/// Re-indexing into an existing database replaces the graph: nodes of a deleted file go,
/// and the database is replaced even while another handle has it open.
#[test]
fn cli_reindex_replaces_the_previous_graph() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("src")).unwrap();
    std::fs::write(
        src.path().join("src/main.rs"),
        "fn main() { keep(); }\nfn keep() {}\n",
    )
    .unwrap();
    std::fs::write(src.path().join("src/gone.rs"), "pub fn gone_fn() {}\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join(".ferrograph");
    let index = || {
        let out = ferrograph_cmd()
            .args([
                "index",
                "--output",
                db.to_str().unwrap(),
                src.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "index failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let search = || {
        let out = ferrograph_cmd()
            .args(["search", "--db", db.to_str().unwrap(), "gone_fn"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    index();
    assert!(search().contains("gone_fn"));
    // A reader holding the old file must not block the rewrite.
    let reader = ferrograph::graph::Store::new_persistent(&db).unwrap();
    std::fs::remove_file(src.path().join("src/gone.rs")).unwrap();
    index();
    assert!(!search().contains("gone_fn"), "{}", search());
    drop(reader);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty());
}

/// Index a throwaway project and run `dead` with extra args; returns stdout.
fn dead_in_temp_project(config: Option<&str>, extra: &[&str]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("examples")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {}\n#[no_mangle]\nfn exported_c() {}\nextern \"C\" fn callback_c() {}\nfn plain_unused() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("examples/demo.rs"), "fn demo_helper() {}\n").unwrap();
    std::fs::write(root.join("src/generated.rs"), "fn table_entry() {}\n").unwrap();
    if let Some(c) = config {
        std::fs::write(root.join(".ferrograph.toml"), c).unwrap();
    }
    let db = root.join(".ferrograph");
    let out = ferrograph_cmd()
        .args([
            "index",
            "--output",
            db.to_str().unwrap(),
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = ferrograph_cmd()
        .args(["dead", "--db", db.to_str().unwrap()])
        .args(extra)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn cli_dead_default_reports_everything() {
    let out = dead_in_temp_project(None, &[]);
    for name in [
        "exported_c",
        "callback_c",
        "plain_unused",
        "demo_helper",
        "table_entry",
    ] {
        assert!(
            out.contains(name),
            "{name} should be dead by default: {out}"
        );
    }
}

#[test]
fn cli_dead_entry_and_ignore_flags() {
    let out = dead_in_temp_project(
        None,
        &[
            "--entry",
            "examples/**",
            "--entry",
            "#[no_mangle]",
            "--entry",
            "#[extern]",
            "--ignore",
            "src/generated.rs",
        ],
    );
    for name in ["exported_c", "demo_helper", "table_entry", "callback_c"] {
        assert!(!out.contains(name), "{name} should not be reported: {out}");
    }
    assert!(out.contains("plain_unused"), "still dead: {out}");
}

#[test]
fn cli_dead_reads_config_file() {
    let out = dead_in_temp_project(
        Some(
            "[dead]\nentry = [\"examples/*\", \"#[no_mangle]\"]\nignore = [\"src/generated.rs\"]\n",
        ),
        &[],
    );
    assert!(
        !out.contains("demo_helper") && !out.contains("exported_c"),
        "{out}"
    );
    assert!(!out.contains("table_entry"), "{out}");
    assert!(
        out.contains("callback_c") && out.contains("plain_unused"),
        "{out}"
    );
}

fn run(args: &[&str]) -> std::process::Output {
    ferrograph_cmd().args(args).output().unwrap()
}

fn s(p: &std::path::Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn cli_status_db_forms() {
    let (db, dir) = index_fixture();
    for target in [s(&db), s(dir.path())] {
        for flag in ["--db", "-d"] {
            let out = run(&["status", flag, target]);
            assert!(out.status.success());
            assert!(String::from_utf8_lossy(&out.stdout).contains("Nodes ("));
            assert_eq!(out.stderr, b"", "no deprecation note expected");
            let out = run(&["status", "--json", flag, target]);
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap();
        }
    }
}

#[test]
fn cli_status_no_args_uses_cwd() {
    let (_db, dir) = index_fixture();
    let out = ferrograph_cmd()
        .arg("status")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Nodes ("));
    assert_eq!(out.stderr, b"");
}

#[test]
fn cli_status_legacy_positional_warns_on_stderr() {
    let (db, dir) = index_fixture();
    for target in [s(&db), s(dir.path())] {
        let out = run(&["status", target]);
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("Nodes ("));
        assert!(String::from_utf8_lossy(&out.stderr).contains("deprecated"));
        assert!(!String::from_utf8_lossy(&out.stdout).contains("deprecated"));
        let out = run(&["status", "--json", target]);
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap();
    }
}

#[test]
fn cli_status_path_conflicts_with_db() {
    let (db, dir) = index_fixture();
    let out = run(&["status", s(dir.path()), "--db", s(&db)]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn cli_all_subcommands_accept_project_dir_as_db() {
    let (_db, dir) = index_fixture();
    let d = s(dir.path());
    let cases: [&[&str]; 8] = [
        &["query", "?[id] := *dead_functions[id]"],
        &["search", "main"],
        &["dead"],
        &["blast", "./src/lib.rs"],
        &["callers", "./src/lib.rs"],
        &["info", "./src/lib.rs"],
        &["modules"],
        &["traits", "Greeter"],
    ];
    for case in cases {
        let mut args = vec![case[0], "--db", d];
        args.extend_from_slice(&case[1..]);
        let out = run(&args);
        assert!(
            out.status.success(),
            "{case:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn cli_index_db_alias_of_output() {
    let fixture = fixture_path("single_crate");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("g.db");
    let out = run(&["index", "--db", s(&db), s(&fixture)]);
    assert!(out.status.success());
    assert!(db.exists());
    let other = dir.path().join("other.db");
    let out = run(&["index", "-o", s(&db), "-d", s(&other), s(&fixture)]);
    assert!(!out.status.success());
    assert!(!other.exists());
}

#[test]
fn cli_watch_accepts_db_alias() {
    let out = run(&["watch", "--help"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("--db"));
    // Missing both must be rejected by clap without starting a watcher.
    assert_eq!(run(&["watch", "."]).status.code(), Some(2));
}

fn code(out: &std::process::Output) -> Option<i32> {
    out.status.code()
}

#[test]
fn cli_dead_exit_codes() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let default = run(&["dead", "--db", d]);
    assert_eq!(code(&default), Some(0));
    assert!(String::from_utf8_lossy(&default.stdout).contains("private_unused"));

    let fail = run(&["dead", "--db", d, "--fail-on-any"]);
    assert_eq!(code(&fail), Some(1));
    assert_eq!(
        fail.stdout, default.stdout,
        "findings still printed unchanged"
    );
    assert!(String::from_utf8_lossy(&fail.stderr).contains("threshold exceeded:"));

    assert_eq!(code(&run(&["dead", "--db", d, "--max", "1000"])), Some(0));
    let zero = run(&["dead", "--db", d, "--max", "0"]);
    assert_eq!(code(&zero), Some(1));
    assert_eq!(zero.stdout, fail.stdout);
    assert_eq!(zero.stderr, fail.stderr);

    let both = run(&["dead", "--db", d, "--fail-on-any", "--max", "5"]);
    assert_eq!(code(&both), Some(2));
}

#[test]
fn cli_dead_json_unchanged_when_threshold_exceeded() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let plain = run(&["--json", "dead", "--db", d]);
    let out = run(&["--json", "dead", "--db", d, "--fail-on-any"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(out.stdout, plain.stdout);
    serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap();
}

#[test]
fn cli_blast_max() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let search = run(&["search", "--db", d, "pub::use_add"]);
    let stdout = String::from_utf8_lossy(&search.stdout).into_owned();
    let node_id = stdout.lines().next().unwrap().split('\t').next().unwrap();
    let json = run(&["--json", "blast", "--db", d, node_id]);
    let count = serde_json::from_slice::<serde_json::Value>(&json.stdout).unwrap()["count"]
        .as_u64()
        .unwrap();
    assert!(count > 0);
    let over = (count - 1).to_string();
    let at = count.to_string();
    assert_eq!(
        code(&run(&["blast", "--db", d, node_id, "--max", &over])),
        Some(1)
    );
    assert_eq!(
        code(&run(&["blast", "--db", d, node_id, "--max", &at])),
        Some(0)
    );
}

#[test]
fn cli_runtime_error_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let out = run(&["dead", "--db", s(&missing)]);
    assert_eq!(code(&out), Some(2));
}

#[test]
fn cli_format_flag_and_json_alias() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let alias = run(&["dead", "--db", d, "--json"]);
    let flag = run(&["dead", "--db", d, "--format", "json"]);
    assert_eq!(alias.stdout, flag.stdout);
    let both_same = run(&["dead", "--db", d, "--json", "--format", "json"]);
    assert_eq!(both_same.stdout, flag.stdout);
    let text = run(&["dead", "--db", d, "--format", "text"]);
    assert_eq!(text.stdout, run(&["dead", "--db", d]).stdout);
    let conflict = run(&["dead", "--db", d, "--json", "--format", "sarif"]);
    assert_eq!(code(&conflict), Some(2));
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflicts"));
}

#[test]
fn cli_dead_sarif_and_markdown() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let out = run(&["dead", "--db", d, "--format", "sarif", "--fail-on-any"]);
    assert_eq!(code(&out), Some(1), "threshold applies in every format");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is pure SARIF");
    assert_eq!(v["version"], "2.1.0");
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert_ne!(results.len(), 0);
    for r in results {
        let p = &r["locations"][0]["physicalLocation"];
        let uri = p["artifactLocation"]["uri"].as_str().unwrap();
        assert!(!uri.starts_with("./"), "{uri}");
        assert!(p["region"]["startLine"].as_u64().unwrap() >= 1);
        assert!(r["partialFingerprints"].is_object());
    }
    assert!(String::from_utf8_lossy(&out.stderr).contains("threshold exceeded:"));

    let md = run(&[
        "dead",
        "--db",
        d,
        "--format",
        "markdown",
        "--markdown-limit",
        "1",
    ]);
    let md = String::from_utf8(md.stdout).unwrap();
    assert!(md.starts_with("<!-- ferrograph -->\n"));
    assert!(md.contains("| Function | Location |"));
    assert!(md.contains("more"), "{md}");
}

#[test]
fn cli_blast_and_callers_formats() {
    let (db, _dir) = index_fixture();
    let d = s(&db);
    let search = run(&["search", "--db", d, "pub::add"]);
    let stdout = String::from_utf8_lossy(&search.stdout).into_owned();
    let id = stdout.lines().next().unwrap().split('\t').next().unwrap();
    for sub in ["blast", "callers"] {
        let sarif = run(&[sub, "--db", d, id, "--format", "sarif"]);
        assert_eq!(code(&sarif), Some(2));
        assert!(String::from_utf8_lossy(&sarif.stderr).contains("sarif not supported"));
        let md = run(&[sub, "--db", d, id, "--format", "markdown"]);
        assert_eq!(code(&md), Some(0));
        assert!(String::from_utf8_lossy(&md.stdout).starts_with("<!-- ferrograph -->"));
    }
}
