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
/// Fails if directory traversal or reading a file fails.
pub fn discover_files(root: &Path) -> Result<BTreeMap<std::path::PathBuf, String>> {
    let mut out = BTreeMap::new();
    for result in WalkBuilder::new(root)
        .max_depth(Some(50))
        .follow_links(true)
        .build()
    {
        let entry = result.map_err(|e| anyhow::anyhow!("walk: {e}"))?;
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
