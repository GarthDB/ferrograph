//! Phase 1: file discovery (walk Rust source files).
//!
//! Uses the `ignore` crate to respect `.gitignore` and limit depth.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use ignore::WalkBuilder;

/// Discover all Rust source files under `root`, returning path -> contents.
/// Respects `.gitignore` and limits depth to 50.
///
/// # Errors
/// Fails if `root` does not exist; unreadable entries below it are skipped with a warning.
pub fn discover_files(root: &Path) -> Result<BTreeMap<std::path::PathBuf, String>> {
    anyhow::ensure!(root.exists(), "path does not exist: {}", root.display());
    let mut out = BTreeMap::new();
    for result in WalkBuilder::new(root)
        .max_depth(Some(50))
        .follow_links(true)
        .build()
    {
        // An unreadable entry (dangling symlink, permission denied) must not abort the whole index (#108).
        let entry = match result {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("warning: skipping unreadable path: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") && path.is_file() {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    out.insert(path.to_path_buf(), s);
                }
                Err(e) => {
                    eprintln!("warning: could not read {}: {e}", path.display());
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn a() {}").unwrap();
        std::os::unix::fs::symlink("missing.gz", dir.path().join("dangling.gz")).unwrap();
        let files = discover_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn discovers_rust_files_only() {
        // Run discovery on this crate's src/ so we know .rs files exist
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(root.exists(), "src/ missing (run from repo root)");
        let files = discover_files(&root).unwrap();
        assert!(
            files
                .keys()
                .any(|p| p.extension().is_some_and(|e| e == "rs")),
            "expected at least one .rs file under src/, got {}",
            files.len()
        );
        assert!(!files
            .keys()
            .any(|p| p.extension().is_some_and(|e| e == "toml")));
    }
}
