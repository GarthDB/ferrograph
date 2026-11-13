//! `ferrograph pr` tests against real git repositories created in temp dirs.

use std::path::Path;
use std::process::{Command, Output};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

const BASE: &str = "fn main() {\n    used();\n    helper();\n}\nfn used() {}\nfn helper() {}\nfn already_dead() {}\n";

/// Repo with `main` holding BASE and a `feature` branch checked out; returns None without git.
fn repo() -> Option<tempfile::TempDir> {
    if !git_available() {
        eprintln!("skipping: git not found");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), "src/main.rs", BASE);
    write(dir.path(), "README.md", "x\n");
    commit(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    Some(dir)
}

fn pr(dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ferrograph"))
        .current_dir(dir)
        .args(["pr", "--base", "main", "--index-head"])
        .args(extra)
        .output()
        .unwrap()
}

fn json(out: &Output) -> serde_json::Value {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn names(v: &serde_json::Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect()
}

#[test]
fn removing_only_caller_makes_fn_newly_dead() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(
        names(&v["newly_dead"]),
        ["helper"],
        "pre-existing dead fn must not be reported"
    );
    assert_eq!(v["newly_dead"][0]["status"], "became_dead");
    assert_eq!(names(&v["changed_functions"]), ["main"]);
    // helper (callee of the changed main) is not in blast anymore; `used` is.
    let blast: Vec<_> = v["blast"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["payload"].as_str())
        .collect();
    assert!(blast.contains(&"used"), "{blast:?}");
    assert_eq!(v["blast"]["count"], v["blast"]["shown"]);
}

#[test]
fn fail_on_new_dead_exit_codes() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    let out = pr(d.path(), &["--fail-on-new-dead"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("threshold exceeded"));
    assert!(String::from_utf8_lossy(&out.stdout).contains("helper"));
    assert_eq!(
        pr(d.path(), &["--max-new-dead", "1"]).status.code(),
        Some(0)
    );
    assert_eq!(pr(d.path(), &[]).status.code(), Some(0));
    assert_eq!(pr(d.path(), &["--max-blast", "0"]).status.code(), Some(1));
}

#[test]
fn line_shifts_are_not_newly_dead() {
    let Some(d) = repo() else { return };
    write(d.path(), "src/main.rs", &format!("// a\n// b\n\n{BASE}"));
    commit(d.path(), "shift");
    let out = pr(d.path(), &["--fail-on-new-dead", "--json"]);
    let v = json(&out);
    assert_eq!(v["newly_dead_count"], 0);
    assert_eq!(names(&v["changed_functions"]).len(), 0);
    assert_eq!(
        v["changed_without_function"][0]["lines"][0],
        serde_json::json!([1, 3])
    );
}

#[test]
fn changed_fn_callers_appear_in_blast() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used() {}", "fn used() {\n    let _ = 1;\n}"),
    );
    commit(d.path(), "edit used");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(names(&v["changed_functions"]), ["used"]);
    let payloads: Vec<_> = v["blast"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["payload"].as_str())
        .collect();
    assert!(payloads.contains(&"main"), "{payloads:?}");
    assert_eq!(v["newly_dead_count"], 0);
}

#[test]
fn blast_limit_caps_list_not_count() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used() {}", "fn used() {\n    let _ = 1;\n}"),
    );
    commit(d.path(), "edit used");
    let v = json(&pr(d.path(), &["--json", "--blast-limit", "0"]));
    assert_eq!(v["blast"]["shown"], 0);
    assert!(v["blast"]["count"].as_u64().unwrap() > 0);
}

#[test]
fn bad_base_ref_exits_2() {
    let Some(d) = repo() else { return };
    let out = Command::new(env!("CARGO_BIN_EXE_ferrograph"))
        .current_dir(d.path())
        .args(["pr", "--base", "nope", "--index-head"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}

#[test]
fn no_rust_changes_and_base_equals_head() {
    let Some(d) = repo() else { return };
    write(d.path(), "README.md", "y\n");
    commit(d.path(), "docs only");
    let v = json(&pr(d.path(), &["--json", "--fail-on-new-dead"]));
    assert_eq!(v["changed_files"].as_array().unwrap().len(), 0);
    assert_eq!(v["blast"]["count"], 0);
    git(d.path(), &["checkout", "-q", "main"]);
    assert_eq!(pr(d.path(), &["--fail-on-new-dead"]).status.code(), Some(0));
}

#[test]
fn renamed_file_does_not_create_newly_dead() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/extra.rs",
        "fn extra_dead() {}\nfn other_dead() {}\n",
    );
    commit(d.path(), "add extra");
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["merge", "-q", "--ff-only", "feature"]);
    git(d.path(), &["checkout", "-q", "-b", "mv"]);
    git(d.path(), &["mv", "src/extra.rs", "src/moved.rs"]);
    commit(d.path(), "rename");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(v["newly_dead_count"], 0, "{v}");
}

#[test]
fn uses_db_and_base_db_and_reports_stale_head() {
    let Some(d) = repo() else { return };
    let dbs = tempfile::tempdir().unwrap();
    let (base_db, head_db) = (dbs.path().join("base"), dbs.path().join("head"));
    let index = |out: &Path| {
        let o = Command::new(env!("CARGO_BIN_EXE_ferrograph"))
            .current_dir(d.path())
            .args(["index", ".", "--output"])
            .arg(out)
            .output()
            .unwrap();
        assert!(o.status.success());
    };
    index(&base_db);
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_ferrograph"))
            .current_dir(d.path())
            .args(["pr", "--base", "main", "--json", "--db"])
            .arg(&head_db)
            .arg("--base-db")
            .arg(&base_db)
            .output()
            .unwrap()
    };
    let out = run();
    assert_eq!(out.status.code(), Some(2), "missing head db");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--index-head"));
    index(&head_db);
    assert_eq!(names(&json(&run())["newly_dead"]), ["helper"]);
    write(d.path(), "src/extra.rs", "fn z() {}\n");
    let out = run();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("stale"));
}

#[test]
fn config_from_head_tree_applies_to_both_sides() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    write(
        d.path(),
        ".ferrograph.toml",
        "[dead]\nignore = [\"src/**\"]\n",
    );
    commit(d.path(), "drop call + ignore");
    assert_eq!(json(&pr(d.path(), &["--json"]))["newly_dead_count"], 0);
}

#[test]
fn base_worktree_is_removed() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    json(&pr(d.path(), &["--json"]));
    let out = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["worktree", "list"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 1);
}

#[test]
fn dirty_tree_does_not_skew_head_results() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used() {}", "fn used() {\n    let _ = 1;\n}"),
    );
    commit(d.path(), "edit used");
    write(
        d.path(),
        "src/main.rs",
        "// uncommitted\n// shift\n\nfn main() {}\n",
    );
    let out = pr(d.path(), &["--json"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("uncommitted"));
    let v = json(&out);
    assert_eq!(names(&v["changed_functions"]), ["used"]);
    assert_eq!(v["newly_dead_count"], 0);
}

#[test]
fn pr_sarif_and_markdown_formats() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    let sarif = pr(d.path(), &["--format", "sarif", "--max-new-dead", "0"]);
    assert_eq!(sarif.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    let r = &v["runs"][0]["results"];
    assert_eq!(r.as_array().unwrap().len(), 1);
    assert!(r[0]["message"]["text"].as_str().unwrap().contains("helper"));
    assert_eq!(
        r[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "src/main.rs"
    );

    let md = pr(d.path(), &["--format", "markdown"]);
    let md = String::from_utf8(md.stdout).unwrap();
    assert!(md.starts_with("<!-- ferrograph -->\n"));
    assert!(
        md.contains("| helper | src/main.rs:5 | became_dead |"),
        "{md}"
    );
    assert!(md.contains("| main |"), "{md}");
}

#[test]
fn restricted_vis_not_root_applies_to_base_and_head() {
    let Some(d) = repo() else { return };
    let restricted = |s: &str| {
        s.replace("fn used", "pub(crate) fn used")
            .replace("fn helper", "pub(crate) fn helper")
            .replace("fn already_dead", "pub(crate) fn already_dead")
    };
    // Same visibility on both sides; the change only removes the call to `helper`.
    write(d.path(), "src/main.rs", &restricted(BASE));
    commit(d.path(), "restrict");
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["merge", "-q", "--ff-only", "feature"]);
    git(d.path(), &["checkout", "-q", "-b", "feature2"]);
    write(
        d.path(),
        "src/main.rs",
        &restricted(&BASE.replace("    helper();\n", "")),
    );
    commit(d.path(), "drop call");
    let v = json(&pr(d.path(), &["--json"]));
    assert!(
        names(&v["newly_dead"]).is_empty(),
        "default keeps them roots"
    );
    let v = json(&pr(d.path(), &["--json", "--restricted-vis-not-root"]));
    assert_eq!(
        names(&v["newly_dead"]),
        ["helper"],
        "already_dead is pre-existing"
    );
    write(
        d.path(),
        ".ferrograph.toml",
        "[dead]\nrestricted_vis_not_root = true\n",
    );
    commit(d.path(), "config");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(names(&v["newly_dead"]), ["helper"]);
}
