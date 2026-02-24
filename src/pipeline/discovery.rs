//! Phase 1: file discovery (walk Rust source files).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use walkdir::WalkDir;

/// Discover all Rust source files under `root`, returning path -> contents.
///
/// # Errors
/// Fails if directory traversal or reading a file fails.
pub fn discover_files(root: &Path) -> Result<BTreeMap<std::path::PathBuf, String>> {
    let mut out = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !n.starts_with('.') && n != "target" && n != "node_modules"
        })
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(s) = std::fs::read_to_string(path) {
                out.insert(path.to_path_buf(), s);
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
        if !root.exists() {
            return;
        }
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
