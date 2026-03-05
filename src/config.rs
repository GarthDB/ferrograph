//! Configuration and CLI definition.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// Graph-powered Rust code intelligence.
#[derive(Debug, Parser)]
#[command(name = "ferrograph", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Index a Rust project into the graph.
    Index {
        /// Root path of the Rust project (directory containing Cargo.toml or src/).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output path for the graph database.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run a Datalog or named query against the graph.
    Query {
        /// Path to the graph database (default: .ferrograph in project root).
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Query to run (Datalog script or named query).
        query: String,
    },
    /// Semantic search over the codebase.
    Search {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Search query.
        query: String,
        /// Match case-insensitively.
        #[arg(short, long)]
        case_insensitive: bool,
    },
    /// Show index status and stats.
    Status {
        /// Path to the project or graph database.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Watch for file changes and re-index.
    Watch {
        /// Root path of the Rust project.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output path for the graph database.
        #[arg(short, long, required = true)]
        output: Option<PathBuf>,
    },
    /// Run the MCP server over stdio (for AI agents and IDEs).
    Mcp,
}

/// Run the CLI command.
///
/// # Errors
/// Returns an error if the selected command fails (e.g. I/O or graph errors).
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Index { path, output } => run_index(&path, output.as_ref()),
        Command::Query { db, query } => run_query(db.as_ref(), &query),
        Command::Search {
            db,
            query,
            case_insensitive,
        } => run_search(db.as_ref(), &query, case_insensitive),
        Command::Status { path } => run_status(&path),
        Command::Watch { path, output } => run_watch(&path, output.as_ref()),
        Command::Mcp => run_mcp(),
    }
}

fn run_mcp() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(crate::mcp::run_stdio())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn default_db_path() -> Option<PathBuf> {
    std::env::current_dir().ok().map(|p| p.join(".ferrograph"))
}

fn resolve_db_path(db: Option<&PathBuf>) -> Result<PathBuf> {
    db.cloned()
        .or_else(default_db_path)
        .context("No graph database path (use --db or run from a directory with .ferrograph)")
}

fn run_index(path: &Path, output: Option<&PathBuf>) -> Result<()> {
    let store = if let Some(out) = output {
        crate::graph::Store::new_persistent(out)
            .with_context(|| format!("Failed to create persistent store at {}", out.display()))?
    } else {
        crate::graph::Store::new_memory()?
    };
    let config = crate::pipeline::PipelineConfig::default();
    crate::pipeline::run_pipeline(&store, path, &config)?;
    if let Some(out) = output {
        println!("Indexed {} into {}", path.display(), out.display());
    } else {
        let nodes = store.node_count()?;
        let edges = store.edge_count()?;
        println!(
            "Indexed {} (in-memory: {nodes} nodes, {edges} edges; use --output to persist)",
            path.display()
        );
    }
    Ok(())
}

fn run_query(db: Option<&PathBuf>, query: &str) -> Result<()> {
    let db_path = resolve_db_path(db)?;
    if !db_path.exists() {
        anyhow::bail!(
            "Graph database not found at {}. Run 'ferrograph index --output {}' first.",
            db_path.display(),
            db_path.display()
        );
    }
    let store = crate::graph::Store::new_persistent(&db_path)
        .with_context(|| format!("Failed to open graph at {}", db_path.display()))?;
    let params = std::collections::BTreeMap::new();
    let script = if query.contains(":limit") {
        query.trim().to_string()
    } else {
        format!("{}\n:limit 10000", query.trim())
    };
    let rows = store
        .run_query(&script, params)
        .context("Query execution failed")?;
    for row in &rows.rows {
        let line: Vec<String> = row.iter().map(std::string::ToString::to_string).collect();
        println!("{}", line.join("\t"));
    }
    Ok(())
}

fn run_search(db: Option<&PathBuf>, query: &str, case_insensitive: bool) -> Result<()> {
    let db_path = resolve_db_path(db)?;
    if !db_path.exists() {
        anyhow::bail!(
            "Graph database not found at {}. Run 'ferrograph index --output {}' first.",
            db_path.display(),
            db_path.display()
        );
    }
    let store = crate::graph::Store::new_persistent(&db_path)
        .with_context(|| format!("Failed to open graph at {}", db_path.display()))?;
    let (rows, _total) = crate::search::text_search(&store, query, case_insensitive, 10_000, 0)?;
    for (id, node_type, payload) in rows {
        let payload_display = payload.as_deref().unwrap_or("—");
        println!("{id}\t{node_type}\t{payload_display}");
    }
    Ok(())
}

fn run_watch(path: &Path, output: Option<&PathBuf>) -> Result<()> {
    let out = output
        .ok_or_else(|| anyhow::anyhow!("Watch requires --output (path to graph database)"))?;
    let store = crate::graph::Store::new_persistent(out)
        .with_context(|| format!("Failed to open graph at {}", out.display()))?;
    let config = crate::pipeline::PipelineConfig::default();
    crate::watch::watch_and_reindex(&store, path, &config)
}

fn run_status(path: &Path) -> Result<()> {
    let db_path = if path.is_dir() {
        path.join(".ferrograph")
    } else {
        path.to_path_buf()
    };
    if !db_path.exists() {
        println!(
            "No graph at {}. Run 'ferrograph index --output {}' first.",
            path.display(),
            db_path.display()
        );
        return Ok(());
    }
    let store = crate::graph::Store::new_persistent(&db_path)
        .with_context(|| format!("Failed to open graph at {}", db_path.display()))?;
    let node_count = store.node_count()?;
    let edge_count = store.edge_count()?;
    println!("Graph: {}", db_path.display());
    println!("  nodes: {node_count}");
    println!("  edges: {edge_count}");
    Ok(())
}
