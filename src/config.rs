//! Configuration and CLI definition.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// Graph-powered Rust code intelligence.
#[derive(Debug, Parser)]
#[command(
    name = "ferrograph",
    version,
    about,
    long_version = concat!(include_str!("../assets/ascii-banner.txt"), "\n  v", env!("CARGO_PKG_VERSION"))
)]
pub struct Cli {
    /// Output JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,

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
    /// Find dead (unreachable) code.
    Dead {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Filter results by file glob pattern.
        #[arg(short, long)]
        file: Option<String>,
    },
    /// Show blast radius (transitive impact) of a node.
    Blast {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to compute blast radius from.
        node_id: String,
    },
    /// Show callers of a node (reverse call graph).
    Callers {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to find callers for.
        node_id: String,
        /// Maximum call depth (default: 1 = direct callers only).
        #[arg(long, default_value = "1")]
        depth: u32,
    },
    /// Show full info for a node (type, payload, edges).
    Info {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to inspect.
        node_id: String,
    },
    /// Show module containment graph.
    Modules {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Filter to modules under this path prefix (e.g. "./src/").
        #[arg(short, long)]
        root: Option<String>,
    },
    /// Show trait implementors.
    Traits {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Trait name to search for.
        trait_name: String,
    },
    /// Run the MCP server over stdio (for AI agents and IDEs).
    Mcp,
}

/// Run the CLI command.
///
/// # Errors
/// Returns an error if the selected command fails (e.g. I/O or graph errors).
pub fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.command {
        Command::Index { path, output } => run_index(&path, output.as_ref()),
        Command::Query { db, query } => run_query(db.as_ref(), &query, json),
        Command::Search {
            db,
            query,
            case_insensitive,
        } => run_search(db.as_ref(), &query, case_insensitive, json),
        Command::Status { path } => run_status(&path, json),
        Command::Watch { path, output } => run_watch(&path, output.as_ref()),
        Command::Dead { db, file } => run_dead(db.as_ref(), file.as_deref(), json),
        Command::Blast { db, node_id } => run_blast(db.as_ref(), &node_id, json),
        Command::Callers { db, node_id, depth } => run_callers(db.as_ref(), &node_id, depth, json),
        Command::Info { db, node_id } => run_info(db.as_ref(), &node_id, json),
        Command::Modules { db, root } => run_modules(db.as_ref(), root.as_deref(), json),
        Command::Traits { db, trait_name } => run_traits(db.as_ref(), &trait_name, json),
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

fn run_query(db: Option<&PathBuf>, query: &str, json: bool) -> Result<()> {
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
    let result = crate::ops::query(&store, query).context("Query execution failed")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for row in &result.rows {
            println!("{}", row.join("\t"));
        }
    }
    Ok(())
}

fn run_search(db: Option<&PathBuf>, query: &str, case_insensitive: bool, json: bool) -> Result<()> {
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
    let result = crate::ops::search(&store, query, case_insensitive, 10_000, 0)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for item in &result.results {
            let payload_display = item.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload_display}", item.id, item.node_type);
        }
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

fn run_dead(db: Option<&PathBuf>, file: Option<&str>, json: bool) -> Result<()> {
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
    let result = crate::ops::dead_code(&store, file)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for node in &result.dead_nodes {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!(
            "\n{} dead nodes found (source: {})",
            result.count, result.source
        );
        println!("\n{}", crate::ops::DEAD_CODE_CAVEAT);
    }
    Ok(())
}

fn run_blast(db: Option<&PathBuf>, node_id: &str, json: bool) -> Result<()> {
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
    let result = crate::ops::blast_radius(&store, node_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for node in &result.reachable_nodes {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!("\n{} nodes in blast radius of {}", result.count, node_id);
    }
    Ok(())
}

fn run_callers(db: Option<&PathBuf>, node_id: &str, depth: u32, json: bool) -> Result<()> {
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
    let result = crate::ops::callers(&store, node_id, depth)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for node in &result.callers {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!("\n{} callers of {}", result.count, node_id);
    }
    Ok(())
}

fn run_info(db: Option<&PathBuf>, node_id: &str, json: bool) -> Result<()> {
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
    let info = crate::ops::node_info(&store, node_id)?;
    match info {
        Some(n) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&n)?);
            } else {
                let payload = n.payload.as_deref().unwrap_or("—");
                println!("{}\t{}\t{payload}", n.id, n.node_type);
                if !n.outgoing_edges.is_empty() {
                    println!("\n  Outgoing edges:");
                    for e in &n.outgoing_edges {
                        let p = e.payload.as_deref().unwrap_or("—");
                        println!("    --[{}]--> {}\t{}\t{p}", e.edge_type, e.id, e.node_type);
                    }
                }
                if !n.incoming_edges.is_empty() {
                    println!("\n  Incoming edges:");
                    for e in &n.incoming_edges {
                        let p = e.payload.as_deref().unwrap_or("—");
                        println!("    <--[{}]-- {}\t{}\t{p}", e.edge_type, e.id, e.node_type);
                    }
                }
            }
        }
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "error": "Node not found",
                        "node_id": node_id
                    }))?
                );
            } else {
                println!("Node not found: {node_id}");
            }
        }
    }
    Ok(())
}

fn run_modules(db: Option<&PathBuf>, root: Option<&str>, json: bool) -> Result<()> {
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
    let result = crate::ops::module_graph(&store, root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for edge in &result.edges {
            println!(
                "{} ({}) --> {} ({})",
                edge.from_id, edge.from_type, edge.to_id, edge.to_type
            );
        }
        println!("\n{} containment edges", result.count);
    }
    Ok(())
}

fn run_traits(db: Option<&PathBuf>, trait_name: &str, json: bool) -> Result<()> {
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
    let result = crate::ops::trait_implementors(&store, trait_name)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for node in &result.implementors {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!("\n{} implementors of \"{}\"", result.count, trait_name);
    }
    Ok(())
}

fn run_status(path: &Path, json: bool) -> Result<()> {
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
    let result = crate::ops::status(&store, &db_path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Graph: {}", result.db_path);
        if let Some(ts) = result.indexed_at {
            println!("  indexed_at: {ts}");
        }
        println!();
        println!("  Nodes ({} total):", result.node_count);
        for (type_name, count) in &result.nodes_by_type {
            println!("    {type_name:<20} {count:>6}");
        }
        println!();
        println!("  Edges ({} total):", result.edge_count);
        for (type_name, count) in &result.edges_by_type {
            println!("    {type_name:<20} {count:>6}");
        }
    }
    Ok(())
}
