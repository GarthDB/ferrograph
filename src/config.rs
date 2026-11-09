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
        /// Do not treat ambiguous (low-confidence) call edges as keeping a function alive.
        #[arg(long)]
        strict: bool,
        /// Extra entry point: file glob (`examples/**`) or attribute (`#[no_mangle]`). Repeatable.
        #[arg(long)]
        entry: Vec<String>,
        /// Never report functions in files matching this glob. Repeatable.
        #[arg(long)]
        ignore: Vec<String>,
    },
    /// Show blast radius (transitive impact) of a node.
    Blast {
        /// Path to the graph database.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to compute blast radius from.
        node_id: String,
        /// Ignore ambiguous (low-confidence) call edges.
        #[arg(long)]
        exclude_low_confidence: bool,
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
        /// Ignore ambiguous (low-confidence) call edges.
        #[arg(long)]
        exclude_low_confidence: bool,
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
    /// Manage the Claude Code skill (install/uninstall/status).
    Claude {
        #[command(subcommand)]
        action: ClaudeAction,
    },
    /// Run the MCP server over stdio (for AI agents and IDEs).
    Mcp,
}

/// Actions for the `ferrograph claude` subcommand.
#[derive(Debug, Subcommand)]
pub enum ClaudeAction {
    /// Install the ferrograph skill into Claude Code.
    Install,
    /// Remove the ferrograph skill from Claude Code.
    Uninstall,
    /// Check whether the skill is installed and up to date.
    Status,
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
        Command::Dead {
            db,
            file,
            strict,
            entry,
            ignore,
        } => run_dead(
            db.as_ref(),
            file.as_deref(),
            crate::ops::DeadOptions {
                strict,
                entry,
                ignore,
            },
            json,
        ),
        Command::Blast {
            db,
            node_id,
            exclude_low_confidence,
        } => run_blast(db.as_ref(), &node_id, exclude_low_confidence, json),
        Command::Callers {
            db,
            node_id,
            depth,
            exclude_low_confidence,
        } => run_callers(db.as_ref(), &node_id, depth, exclude_low_confidence, json),
        Command::Info { db, node_id } => run_info(db.as_ref(), &node_id, json),
        Command::Modules { db, root } => run_modules(db.as_ref(), root.as_deref(), json),
        Command::Traits { db, trait_name } => run_traits(db.as_ref(), &trait_name, json),
        Command::Claude { ref action } => crate::claude::run(action, json),
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
    let config = crate::pipeline::PipelineConfig::default();
    if let Some(out) = output {
        // Replaces the database file whole, so a running MCP server is not blocked and no
        // graph from an earlier run is left behind.
        crate::pipeline::index_to_path(out, path, &config)
            .with_context(|| format!("Failed to write graph to {}", out.display()))?;
        println!("Indexed {} into {}", path.display(), out.display());
    } else {
        let store = crate::graph::Store::new_memory()?;
        crate::pipeline::run_pipeline(&store, path, &config)?;
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
            let cols: Vec<String> = row
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            println!("{}", cols.join("\t"));
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
    let config = crate::pipeline::PipelineConfig::default();
    crate::watch::watch_and_reindex(out, path, &config)
}

/// Project config read from `.ferrograph.toml` next to the graph database (or in the cwd).
#[derive(Debug, Default, serde::Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub dead: DeadConfig,
}

/// `[dead]` section of `.ferrograph.toml`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadConfig {
    /// Extra entry points: file globs or attributes such as `#[no_mangle]`.
    #[serde(default)]
    pub entry: Vec<String>,
    /// File globs whose functions are never reported.
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Load `.ferrograph.toml` from the database's directory, falling back to the cwd.
///
/// # Errors
/// Fails if the file exists but cannot be read or parsed.
pub fn load_file_config(db_path: &Path) -> Result<FileConfig> {
    let candidates = [
        db_path.parent().map(|p| p.join(".ferrograph.toml")),
        Some(PathBuf::from(".ferrograph.toml")),
    ];
    for path in candidates.into_iter().flatten() {
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            return toml::from_str(&text)
                .with_context(|| format!("Invalid config {}", path.display()));
        }
    }
    Ok(FileConfig::default())
}

fn run_dead(
    db: Option<&PathBuf>,
    file: Option<&str>,
    mut opts: crate::ops::DeadOptions,
    json: bool,
) -> Result<()> {
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
    let cfg = load_file_config(&db_path)?;
    opts.entry.extend(cfg.dead.entry);
    opts.ignore.extend(cfg.dead.ignore);
    let result = crate::ops::dead_code(&store, file, &opts)?;
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

fn run_blast(db: Option<&PathBuf>, node_id: &str, exclude_low: bool, json: bool) -> Result<()> {
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
    let result = crate::ops::blast_radius(&store, node_id, exclude_low)?;
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

fn run_callers(
    db: Option<&PathBuf>,
    node_id: &str,
    depth: u32,
    exclude_low: bool,
    json: bool,
) -> Result<()> {
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
    let result = crate::ops::callers(&store, node_id, depth, exclude_low)?;
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
        if let Some(root) = &result.root {
            println!("  root: {root}");
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
