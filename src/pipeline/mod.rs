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
/// # Errors
/// Fails if the pipeline fails or the database file cannot be written or renamed.
pub fn index_to_path(db_path: &Path, root: &Path, config: &PipelineConfig) -> Result<()> {
    let graph = Store::new_memory()?;
    run_pipeline(&graph, root, config)?;
    let mut tmp_name = db_path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = db_path.with_file_name(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let out = Store::new_persistent(&tmp)?;
        out.copy_from(&graph)
    })();
    if let Err(e) = written.and_then(|()| Ok(std::fs::rename(&tmp, db_path)?)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// What the graph of a project root was built from: a hash of the Rust source paths and
/// the newest modification time among them. A changed hash means files were added,
/// removed or renamed; a newer time means a file was edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources {
    pub paths_hash: u64,
    pub newest: Option<std::time::SystemTime>,
}

/// Scan `root` for the Rust files [`discover_files`] would index (same `.gitignore` rules)
/// without reading them.
#[must_use]
pub fn scan_sources(root: &Path) -> Sources {
    use std::hash::{Hash, Hasher};
    let mut paths = Vec::new();
    let mut newest = None;
    for entry in ignore::WalkBuilder::new(root)
        .max_depth(Some(50))
        .follow_links(true)
        .build()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") && path.is_file() {
            if let Some(m) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
                newest = newest.max(Some(m));
            }
            paths.push(path.to_path_buf());
        }
    }
    paths.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    paths.hash(&mut h);
    Sources {
        paths_hash: h.finish(),
        newest,
    }
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
    fn scan_sources_tracks_paths_and_the_newest_edit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let one = super::scan_sources(dir.path());
        assert!(one.newest.is_some());
        assert_eq!(one, super::scan_sources(dir.path()), "deterministic");
        std::fs::write(dir.path().join("b.rs"), "fn b() {}").unwrap();
        assert_ne!(one.paths_hash, super::scan_sources(dir.path()).paths_hash);
    }
}
