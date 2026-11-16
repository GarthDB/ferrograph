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
    assert_eq!(
        names(&v["newly_dead"]),
        ["helper"],
        "already_dead is pre-existing"
    );
    let v = json(&pr(d.path(), &["--json", "--no-restricted-vis-not-root"]));
    assert!(
        names(&v["newly_dead"]).is_empty(),
        "opt-out keeps them roots"
    );
    write(
        d.path(),
        ".ferrograph.toml",
        "[dead]\nrestricted_vis_not_root = false\n",
    );
    commit(d.path(), "config");
    let v = json(&pr(d.path(), &["--json"]));
    assert!(names(&v["newly_dead"]).is_empty(), "config opt-out");
}

#[test]
fn bin_pub_not_root_applies_to_base_and_head() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    let public = |s: &str| {
        s.replace("fn used", "pub fn used")
            .replace("fn helper", "pub fn helper")
            .replace("fn already_dead", "pub fn already_dead")
    };
    write(d.path(), "src/main.rs", &public(BASE));
    commit(d.path(), "pub");
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["merge", "-q", "--ff-only", "feature"]);
    git(d.path(), &["checkout", "-q", "-b", "feature2"]);
    write(
        d.path(),
        "src/main.rs",
        &public(&BASE.replace("    helper();\n", "")),
    );
    commit(d.path(), "drop call");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(
        names(&v["newly_dead"]),
        ["helper"],
        "already_dead is pre-existing"
    );
    let v = json(&pr(d.path(), &["--json", "--no-bin-pub-not-root"]));
    assert!(
        names(&v["newly_dead"]).is_empty(),
        "opt-out keeps them roots"
    );
    write(
        d.path(),
        ".ferrograph.toml",
        "[dead]\nbin_pub_not_root = false\n",
    );
    commit(d.path(), "config");
    let v = json(&pr(d.path(), &["--json"]));
    assert!(names(&v["newly_dead"]).is_empty(), "config opt-out");
}

#[test]
fn pr_strict_keeps_the_visibility_options_off_unless_asked() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    let public = |s: &str| {
        s.replace("fn used", "pub fn used")
            .replace("fn helper", "pub fn helper")
            .replace("fn already_dead", "pub fn already_dead")
    };
    write(d.path(), "src/main.rs", &public(BASE));
    commit(d.path(), "pub");
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["merge", "-q", "--ff-only", "feature"]);
    git(d.path(), &["checkout", "-q", "-b", "feature2"]);
    write(
        d.path(),
        "src/main.rs",
        &public(&BASE.replace("    helper();\n", "")),
    );
    commit(d.path(), "drop call");
    let v = json(&pr(d.path(), &["--json", "--strict"]));
    assert!(
        names(&v["newly_dead"]).is_empty(),
        "strict: pub stays a root"
    );
    let v = json(&pr(d.path(), &["--json", "--strict", "--bin-pub-not-root"]));
    assert_eq!(names(&v["newly_dead"]), ["helper"], "explicit flag wins");
}

#[test]
fn bin_pub_not_root_warns_when_only_one_side_has_the_marker() {
    let Some(d) = repo() else { return };
    // Base has no Cargo.toml (no bin-only files, so no markers); head adds it and marks `main.rs` bin-only.
    write(
        d.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used", "pub fn used"),
    );
    commit(d.path(), "manifest");
    let warned = |extra: &[&str]| {
        json(&pr(d.path(), extra))["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("pub_bin_only"))
    };
    assert!(
        !warned(&["--json", "--no-bin-pub-not-root"]),
        "option off: nothing to warn about"
    );
    assert!(warned(&["--json"]));
}

#[test]
fn pr_out_writes_several_formats_from_one_run() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("    helper();\n", ""),
    );
    commit(d.path(), "drop call");
    let out = pr(
        d.path(),
        &[
            "--out",
            "json=r.json",
            "--out",
            "markdown=r.md",
            "--out",
            "sarif=r.sarif",
            "--max-new-dead",
            "0",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "threshold exit code is kept");
    assert!(out.stdout.is_empty(), "only --out: nothing on stdout");
    let read = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&read("r.json")).unwrap();
    assert_eq!(names(&v["newly_dead"]), ["helper"]);
    let md = read("r.md");
    assert!(md.starts_with("<!-- ferrograph -->\n"), "{md}");
    assert!(
        md.contains(&format!("{} newly dead", v["newly_dead_count"])),
        "{md}"
    );
    let sarif: serde_json::Value = serde_json::from_str(&read("r.sarif")).unwrap();
    assert_eq!(
        sarif["runs"][0]["results"].as_array().unwrap().len(),
        v["newly_dead"].as_array().unwrap().len()
    );
}

#[test]
fn pr_out_keeps_stdout_when_a_format_is_given() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used", "fn used2"),
    );
    commit(d.path(), "rename");
    let out = pr(d.path(), &["--json", "--out", "markdown=r.md"]);
    let v = json(&out);
    let md = std::fs::read_to_string(d.path().join("r.md")).unwrap();
    assert!(md.contains(&format!("{} newly dead", v["newly_dead_count"])));
}

#[test]
fn pr_out_rejects_a_bad_spec() {
    let Some(d) = repo() else { return };
    for bad in ["json", "json=", "yaml=r.yml"] {
        let out = pr(d.path(), &["--out", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
    }
}

#[test]
fn pr_out_keeps_stdout_with_an_explicit_format_and_creates_directories() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used", "fn used2"),
    );
    commit(d.path(), "rename");
    let out = pr(
        d.path(),
        &[
            "--format",
            "markdown",
            "--out",
            "json=reports/nested/r.json",
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("<!-- ferrograph -->"));
    assert!(d.path().join("reports/nested/r.json").is_file());
}

#[test]
fn pr_out_write_failure_is_an_error_but_stdout_is_kept() {
    let Some(d) = repo() else { return };
    write(
        d.path(),
        "src/main.rs",
        &BASE.replace("fn used", "fn used2"),
    );
    commit(d.path(), "rename");
    // The target path is an existing directory, so the write fails.
    let out = pr(d.path(), &["--json", "--out", "json=src"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Failed to write src"));
    serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap();
}

const IMPACT_BASE: &str = "fn main() {\n    caller();\n}\nfn caller() {\n    leaf();\n}\nfn leaf() {}\n#[test]\nfn leaf_works() {\n    leaf();\n}\n";

#[test]
fn reports_transitive_callers_and_tests_changed() {
    let Some(d) = repo() else { return };
    write(d.path(), "src/main.rs", IMPACT_BASE);
    commit(d.path(), "impact base");
    git(d.path(), &["checkout", "-q", "-B", "main"]);
    git(d.path(), &["checkout", "-q", "-b", "f2"]);
    write(
        d.path(),
        "src/main.rs",
        &IMPACT_BASE
            .replace("fn leaf() {}", "fn leaf() {\n    let _ = 1;\n}")
            .replace("fn leaf_works() {\n", "fn leaf_works() {\n    let _ = 2;\n"),
    );
    commit(d.path(), "edit leaf and its test");
    let v = json(&pr(d.path(), &["--json"]));
    let f = |n: &str| {
        v["changed_functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == n)
            .unwrap()
            .clone()
    };
    let leaf = f("leaf");
    assert_eq!(leaf["kind"], "fn");
    // leaf <- caller <- main (2 hops), leaf <- leaf_works
    assert_eq!(leaf["impacted"]["count"], 3, "{leaf}");
    assert_eq!(leaf["impacted"]["tests"], 1);
    assert_eq!(v["impacted"]["library"]["count"], 2);
    // #170: a changed test has no blast and no impacted set, and adds nothing to the union.
    let t = f("leaf_works");
    assert_eq!(
        (t["kind"].as_str(), t["blast_count"].as_u64()),
        (Some("test"), Some(0))
    );
    assert!(t.get("impacted").is_none());
    let depth1 = json(&pr(d.path(), &["--json", "--impact-depth", "1"]));
    assert_eq!(depth1["impacted"]["library"]["count"], 1);
    let md = String::from_utf8_lossy(&pr(d.path(), &["--format", "markdown"]).stdout).into_owned();
    assert!(
        md.contains("impacted (0 tests, 0 examples, 2 library)\n"),
        "{md}"
    );
    assert!(
        md.contains("Impacted by this change")
            && md.contains("Tests, test helpers, benches and examples changed"),
        "{md}"
    );
}

#[test]
fn ignore_filters_changed_lists() {
    let Some(d) = repo() else { return };
    write(d.path(), "src/generated.rs", "fn gen() {}\n");
    write(d.path(), "src/main.rs", &format!("{BASE}// c\n"));
    commit(d.path(), "edit both");
    let v = json(&pr(d.path(), &["--json"]));
    assert_eq!(v["changed_without_function"].as_array().unwrap().len(), 1);
    assert_eq!(names(&v["changed_functions"]), ["gen"]);
    let v = json(&pr(
        d.path(),
        &[
            "--json",
            "--ignore",
            "**/main.rs",
            "--ignore",
            "**/generated.rs",
        ],
    ));
    assert_eq!(v["changed_without_function"].as_array().unwrap().len(), 0);
    assert_eq!(names(&v["changed_functions"]).len(), 0);
}

#[test]
fn test_helper_keeps_impacted_but_has_no_blast() {
    let Some(d) = repo() else { return };
    let base =
        "fn view() {\n    inner();\n}\nfn inner() {}\n#[test]\nfn uses_view() {\n    view();\n}\n";
    write(d.path(), "tests/t.rs", base);
    commit(d.path(), "add tests/t.rs");
    git(d.path(), &["checkout", "-q", "-B", "main"]);
    git(d.path(), &["checkout", "-q", "-b", "f3"]);
    write(
        d.path(),
        "tests/t.rs",
        &base.replace("    inner();", "    inner();\n    inner();"),
    );
    commit(d.path(), "edit helper");
    let v = json(&pr(d.path(), &["--json"]));
    let f = &v["changed_functions"][0];
    assert_eq!(
        (f["name"].as_str(), f["kind"].as_str()),
        (Some("view"), Some("helper"))
    );
    assert_eq!(f["blast_count"], 0);
    assert_eq!(f["impacted"]["tests"], 1, "{f}");
    assert_eq!(v["impacted"]["tests"]["count"], 1);
    assert_eq!(
        pr(d.path(), &["--impact-depth", "0"]).status.code(),
        Some(2)
    );
}

#[test]
fn example_fn_has_no_blast_and_max_impacted_gates() {
    let Some(d) = repo() else { return };
    let base = "fn story() {\n    inner();\n}\nfn inner() {}\n";
    write(
        d.path(),
        "examples/g.rs",
        &format!("{base}fn main() {{\n    story();\n}}\n"),
    );
    commit(d.path(), "add example");
    git(d.path(), &["checkout", "-q", "-B", "main"]);
    git(d.path(), &["checkout", "-q", "-b", "f4"]);
    write(
        d.path(),
        "examples/g.rs",
        &format!("{base}fn main() {{\n    story();\n}}\n")
            .replace("    inner();", "    inner();\n    inner();"),
    );
    commit(d.path(), "edit story");
    let v = json(&pr(d.path(), &["--json"]));
    let f = &v["changed_functions"][0];
    assert_eq!(
        (f["kind"].as_str(), f["blast_count"].as_u64()),
        (Some("example"), Some(0))
    );
    assert_eq!(f["impacted"]["count"], 1, "{f}");
    assert_eq!(
        pr(d.path(), &["--max-impacted", "1"]).status.code(),
        Some(0)
    );
    assert_eq!(
        pr(d.path(), &["--max-impacted", "0"]).status.code(),
        Some(1)
    );
}

/// #187: a call through a trait bound resolves to every `impl`, so all edges are low-confidence and
/// `--exclude-low-confidence` drops the real callers. Documents why the flag is not the default.
#[test]
fn exclude_low_confidence_drops_trait_dispatch_callers() {
    let Some(d) = repo() else { return };
    let src = "trait Draw { fn draw(&self) -> u32; }\nstruct A;\nstruct B;\nimpl Draw for A { fn draw(&self) -> u32 { 1 } }\nimpl Draw for B { fn draw(&self) -> u32 { 2 } }\nfn render<T: Draw>(t: &T) -> u32 { t.draw() }\nfn main() { render(&A); }\n";
    write(d.path(), "src/main.rs", src);
    commit(d.path(), "trait base");
    git(d.path(), &["checkout", "-q", "-B", "main"]);
    git(d.path(), &["checkout", "-q", "-b", "f3"]);
    write(d.path(), "src/main.rs", &src.replace("{ 1 }", "{ 11 }"));
    commit(d.path(), "change one draw");
    let all = json(&pr(d.path(), &["--json"]));
    let mut got = names(&all["impacted"]["nodes"]);
    got.sort_unstable();
    assert_eq!(got, ["main", "render"]);
    let x = json(&pr(d.path(), &["--json", "--exclude-low-confidence"]));
    assert!(names(&x["impacted"]["nodes"]).is_empty(), "{x}");
}
