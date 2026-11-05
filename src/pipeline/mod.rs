//! Analysis pipeline: file discovery, AST, modules, call graph, and downstream phases.

mod ast;
mod borrows;
mod calls;
mod dead_code;
mod discovery;
mod expands;
mod git_coupling;
mod lifetimes;
mod modules;
mod owns;
mod placeholder;
mod primitives;
mod references;
mod traits;

pub use ast::extract_ast;
pub use borrows::{resolve_borrows_edges, resolve_borrows_mut_edges};
pub use calls::build_call_graph;
pub use dead_code::detect_dead_code;
pub use discovery::discover_files;
pub use expands::resolve_expands_to_edges;
pub use git_coupling::analyze_git_coupling;
pub use modules::resolve_modules;
pub use owns::resolve_owns_edges;
pub use references::resolve_reference_edges;
pub use traits::{map_traits, resolve_impl_trait_edges};

use anyhow::Result;
use std::path::Path;

use crate::graph::Store;

/// Pipeline configuration (tier: fast, balanced, full).
#[derive(Clone, Debug, Default)]
pub struct PipelineConfig {
    /// If true, run trait/impl mapping (rust-analyzer, requires `ra` feature).
    pub enable_trait_mapping: bool,
    /// If true, run git change coupling (requires `git` feature).
    pub enable_git_coupling: bool,
}

/// Index `root` and replace the graph database at `db_path` in one step.
///
/// The pipeline runs into memory and is written to a temporary file next to `db_path`,
/// which is then renamed over it. Readers that already have the database open (an MCP
/// server) are never blocked and never see a partial graph; they keep the old file until
/// they reopen it. On failure the existing database is left as it was.
///
/// The database records the canonical `root` (meta key `root`) and the
/// [`source_fingerprint`] taken before indexing (meta key `sources`), so a reader can tell
/// which project the graph belongs to and whether its sources changed since.
///
/// # Errors
/// Fails if the pipeline fails or the database file cannot be written or renamed.
pub fn index_to_path(db_path: &Path, root: &Path, config: &PipelineConfig) -> Result<()> {
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = root.canonicalize()?;
    let fingerprint = source_fingerprint(&root);
    let graph = Store::new_memory()?;
    run_pipeline(&graph, &root, config)?;
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp_name = db_path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp-{}-{seq}", std::process::id()));
    let tmp = db_path.with_file_name(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let out = Store::new_persistent(&tmp)?;
        out.copy_from(&graph)?;
        out.put_meta("root", &root.to_string_lossy())?;
        out.put_meta("sources", &fingerprint.to_string())
    })();
    if let Err(e) = written.and_then(|()| Ok(std::fs::rename(&tmp, db_path)?)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// A hash of the Rust files [`discover_files`] would index under `root` (same `.gitignore`
/// rules): each path relative to `root`, its size and modification time. Any added,
/// removed, renamed or edited file changes it. Files are not read.
#[must_use]
pub fn source_fingerprint(root: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .max_depth(Some(50))
        .follow_links(true)
        .build()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") && path.is_file() {
            let meta = entry.metadata().ok();
            files.push((
                path.strip_prefix(root).unwrap_or(path).to_path_buf(),
                meta.as_ref().map(std::fs::Metadata::len),
                meta.and_then(|m| m.modified().ok()),
            ));
        }
    }
    files.sort();
    // DefaultHasher may change between Rust releases; a mismatch only costs one reindex.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut h);
    h.finish()
}

/// Run the full indexing pipeline on a project root.
///
/// # Errors
/// Fails if any pipeline phase fails (discovery, AST, store writes, etc.).
pub fn run_pipeline(store: &Store, root: &Path, config: &PipelineConfig) -> Result<()> {
    let files = discover_files(root)?;
    for (path, content) in &files {
        extract_ast(store, path, content, root)?;
    }
    primitives::create_primitive_nodes(store)?;
    resolve_modules(store, root)?; // Imports edges must exist before call resolution
    build_call_graph(store)?;
    resolve_impl_trait_edges(store)?; // Resolve placeholder impl→trait edges from AST
    resolve_reference_edges(store)?; // Resolve placeholder type reference edges from AST
    resolve_owns_edges(store)?; // Resolve placeholder owns edges (item → type)
    resolve_borrows_edges(store)?; // Resolve placeholder borrows edges (item → type)
    resolve_borrows_mut_edges(store)?; // Resolve placeholder borrows_mut edges (item → type)
    resolve_expands_to_edges(store)?; // Resolve placeholder macro invocation→definition edges from AST
    if config.enable_trait_mapping {
        map_traits(store, root)?;
    }
    detect_dead_code(store)?;
    if config.enable_git_coupling {
        analyze_git_coupling(store, root)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_fingerprint_changes_with_rust_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let fp = || super::source_fingerprint(dir.path());
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        let one = fp();
        assert_eq!(one, fp(), "deterministic");
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert_eq!(one, fp(), "non-Rust files are ignored");
        std::fs::write(dir.path().join("a.rs"), "fn a() { let _ = 1; }").unwrap();
        let edited = fp();
        assert_ne!(one, edited, "edit");
        std::fs::write(dir.path().join("b.rs"), "fn b() {}").unwrap();
        assert_ne!(edited, fp(), "added file");
    }

    #[test]
    fn index_to_path_records_root_and_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        let db = dir.path().join(".ferrograph");
        super::index_to_path(&db, dir.path(), &super::PipelineConfig::default()).unwrap();
        let store = crate::graph::Store::new_persistent(&db).unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(
            store.get_meta("root").unwrap().as_deref(),
            Some(root.to_string_lossy().as_ref())
        );
        assert_eq!(
            store.get_meta("sources").unwrap(),
            Some(super::source_fingerprint(&root).to_string())
        );
    }
}
