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
