//! Helpers for tests that need the ferrograph binary and the synthetic corpus.

use crate::corpus::Corpus;
use std::path::Path;

/// `$FERROGRAPH`, else `target/{debug,release}/ferrograph`. Tests skip when there is none, except
/// under `$CI`, where a missing binary is an error (a silently skipped self-test is no test).
pub fn binary() -> Option<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let found = std::env::var("FERROGRAPH")
        .ok()
        .into_iter()
        .chain(["debug", "release"].map(|p| {
            root.join("target")
                .join(p)
                .join("ferrograph")
                .display()
                .to_string()
        }))
        .find(|c| Path::new(c).is_file());
    if found.is_none() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI: no ferrograph binary (set $FERROGRAPH or run `cargo build`)"
        );
        eprintln!("skipped: no ferrograph binary");
    }
    found
}

pub fn synthetic() -> Corpus {
    Corpus {
        name: "synthetic".into(),
        url: None,
        git_ref: None,
        path: Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scripts/accuracy/synthetic")
                .display()
                .to_string(),
        ),
        index_path: None,
        group: None,
        heldout: false,
        cohort: None,
        cargo_args: None,
    }
}

/// A git repo (`file://` url corpus) with three commits on top of the synthetic corpus: base, one that
/// deletes the only call to `cleanup`, and a comment-only one.
pub fn synthetic_history(dir: &Path) -> Corpus {
    let origin = dir.join("origin");
    crate::corpus::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accuracy/synthetic"),
        &origin,
    )
    .unwrap();
    let git = |args: &[&str]| {
        let mut v = vec![
            "git",
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ];
        v.extend(args);
        crate::run::run(
            &v,
            Some(&origin),
            &[],
            std::time::Duration::from_secs(60),
            true,
        )
        .unwrap();
    };
    let main = origin.join("bin_app/src/main.rs");
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    let src = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, src.replace("    cleanup();\n", "\n")).unwrap();
    git(&["commit", "-qam", "drop cleanup call"]);
    std::fs::write(
        &main,
        std::fs::read_to_string(&main)
            .unwrap()
            .replace("fn teardown() {", "// note\nfn teardown() {"),
    )
    .unwrap();
    git(&["commit", "-qam", "comment"]);
    Corpus {
        name: "hist".into(),
        url: Some(format!("file://{}", origin.display())),
        path: None,
        ..synthetic()
    }
}
