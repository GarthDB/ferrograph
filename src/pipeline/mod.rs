//! Analysis pipeline: file discovery, AST, modules, call graph, and downstream phases.

mod ast;
mod calls;
mod dead_code;
mod discovery;
mod git_coupling;
mod modules;
mod traits;

pub use ast::extract_ast;
pub use calls::build_call_graph;
pub use dead_code::detect_dead_code;
pub use discovery::discover_files;
pub use git_coupling::analyze_git_coupling;
pub use modules::resolve_modules;
pub use traits::map_traits;

use anyhow::Result;
use std::path::Path;

use crate::graph::Store;

/// Pipeline configuration (tier: fast, balanced, full).
#[derive(Clone, Debug, Default)]
pub struct PipelineConfig {
    /// If true, run rust-analyzer-based phases (full tier).
    pub full_semantic: bool,
}

/// Run the full indexing pipeline on a project root.
///
/// # Errors
/// Fails if any pipeline phase fails (discovery, AST, store writes, etc.).
pub fn run_pipeline(store: &Store, root: &Path, config: &PipelineConfig) -> Result<()> {
    let files = discover_files(root)?;
    for (path, content) in &files {
        let nodes_edges = extract_ast(path, content)?;
        for (id, typ, payload) in nodes_edges.nodes {
            store.put_node(&id, &typ, payload.as_deref())?;
        }
        for (from, to, edge_type) in nodes_edges.edges {
            store.put_edge(&from, &to, &edge_type)?;
        }
    }
    resolve_modules(store, root)?;
    build_call_graph(store)?;
    if config.full_semantic {
        map_traits(store, root)?;
    }
    detect_dead_code(store)?;
    if config.full_semantic {
        analyze_git_coupling(store, root)?;
    }
    Ok(())
}
