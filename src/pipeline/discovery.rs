//! Phase 1: file discovery (walk Rust source files).
//!
//! Uses the `ignore` crate to respect `.gitignore` and limit depth.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

/// Discover all Rust source files under `root`, returning path -> contents.
/// Respects `.gitignore` and limits depth to 50. A file reachable by several paths (a symlinked
/// directory) is kept once, under the path that does not go through a symlink (#239); if every path
/// goes through a symlink, all are kept.
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
    drop_symlink_duplicates(root, &mut out);
    Ok(out)
}

/// Of the paths that name the same file, keep the one below `root` without a symlinked component;
/// when there is none, keep them all.
fn drop_symlink_duplicates(root: &Path, files: &mut BTreeMap<PathBuf, String>) {
    let Ok(canon_root) = root.canonicalize() else {
        return;
    };
    let mut by_target: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for path in files.keys() {
        if let Ok(target) = path.canonicalize() {
            by_target.entry(target).or_default().push(path.clone());
        }
    }
    for (target, paths) in by_target {
        if paths.len() < 2 {
            continue;
        }
        let real = paths.iter().find(|p| {
            p.strip_prefix(root)
                .is_ok_and(|rel| canon_root.join(rel) == target)
        });
        // Without a symlink-free path (a directory shared from outside the root by several
        // crates) every path stays: each side's `mod`/`use` resolution needs its own copy.
        let Some(keep) = real else { continue };
        for p in &paths {
            if p != keep {
                files.remove(p);
            }
        }
    }
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

    #[cfg(unix)]
    #[test]
    fn symlinked_directory_is_indexed_once_under_its_real_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("real/sub")).unwrap();
        std::fs::write(dir.path().join("real/sub/a.rs"), "pub fn a() {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn b() {}").unwrap();
        // `alias` sorts before `real`, so a first-seen rule would keep the symlinked copy.
        std::os::unix::fs::symlink("real", dir.path().join("alias")).unwrap();
        let files = discover_files(dir.path()).unwrap();
        let mut names: Vec<String> = files
            .keys()
            .map(|p| p.strip_prefix(dir.path()).unwrap().display().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["b.rs", "real/sub/a.rs"]);
    }

    #[cfg(unix)]
    #[test]
    fn file_symlink_to_a_file_in_the_tree_is_indexed_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z_real.rs"), "pub fn a() {}").unwrap();
        std::os::unix::fs::symlink("z_real.rs", dir.path().join("a_link.rs")).unwrap();
        let files = discover_files(dir.path()).unwrap();
        let names: Vec<_> = files
            .keys()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["z_real.rs"]);
    }

    #[cfg(unix)]
    #[test]
    fn directory_shared_from_outside_the_root_keeps_every_path() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("shared.rs"), "pub fn s() {}").unwrap();
        let root = tempfile::tempdir().unwrap();
        for krate in ["one", "two"] {
            std::fs::create_dir(root.path().join(krate)).unwrap();
            std::os::unix::fs::symlink(outside.path(), root.path().join(krate).join("shared"))
                .unwrap();
        }
        let files = discover_files(root.path()).unwrap();
        assert_eq!(files.len(), 2, "{:?}", files.keys().collect::<Vec<_>>());
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
