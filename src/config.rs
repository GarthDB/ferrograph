//! Configuration and CLI definition.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::format::{self, Format};

/// Output selection shared by `dead`, `pr`, `blast` and `callers`.
#[derive(Debug, Args)]
pub struct FormatArgs {
    /// Output format (`--json` is an alias for `--format json`).
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// Maximum table rows in Markdown output.
    #[arg(long, default_value_t = format::DEFAULT_MARKDOWN_LIMIT)]
    pub markdown_limit: usize,
}

impl FormatArgs {
    fn resolve(&self, json: bool) -> Result<(Format, usize)> {
        let format = match (json, self.format) {
            (true, Some(f)) if f != Format::Json => {
                anyhow::bail!("--json conflicts with --format {f:?}: pass only one")
            }
            (true, _) => Format::Json,
            (false, f) => f.unwrap_or(Format::Text),
        };
        Ok((format, self.markdown_limit))
    }
}

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
        /// Alias of --output (same convention as the other subcommands).
        #[arg(short, long)]
        db: Option<PathBuf>,
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
        /// Path to the graph database, or a project directory containing .ferrograph.
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
        /// Path to the graph database, or a project directory containing .ferrograph.
        #[arg(short, long, conflicts_with = "path")]
        db: Option<PathBuf>,
        /// Deprecated: use --db. Project directory or graph database file.
        path: Option<PathBuf>,
    },
    /// Watch for file changes and re-index.
    Watch {
        /// Root path of the Rust project.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output path for the graph database.
        #[arg(short, long, required_unless_present = "db")]
        output: Option<PathBuf>,
        /// Alias of --output.
        #[arg(short, long)]
        db: Option<PathBuf>,
    },
    /// Find dead (unreachable) code.
    ///
    /// Static, name-based analysis: may have false positives and false negatives; a review aid, not proof.
    Dead {
        /// Path to the graph database, or a project directory containing .ferrograph.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Filter results by file glob pattern.
        #[arg(short, long)]
        file: Option<String>,
        /// Do not treat ambiguous (low-confidence) call edges as keeping a function alive.
        #[arg(long)]
        strict: bool,
        /// Do not treat `pub(crate)`/`pub(super)`/`pub(in ..)` functions as entry points (bare `pub` still is). On by default, off with `--strict` unless passed.
        #[arg(long, conflicts_with = "no_restricted_vis_not_root")]
        restricted_vis_not_root: bool,
        /// Keep `pub(crate)`/`pub(super)`/`pub(in ..)` functions as entry points (overrides the config file).
        #[arg(long)]
        no_restricted_vis_not_root: bool,
        /// Do not treat bare `pub` functions in files compiled only by bin/test/example/bench targets as entry points. On by default, off with `--strict` unless passed.
        #[arg(long, conflicts_with = "no_bin_pub_not_root")]
        bin_pub_not_root: bool,
        /// Keep bare `pub` functions in bin/test/example/bench-only files as entry points (overrides the config file).
        #[arg(long)]
        no_bin_pub_not_root: bool,
        /// Do not treat bare `pub` free functions that a library crate does not export (private module, no `pub use` path) as entry points. Off by default.
        #[arg(long, conflicts_with = "no_lib_exported_only")]
        lib_exported_only: bool,
        /// Keep unexported library `pub` functions as entry points (overrides the config file).
        #[arg(long)]
        no_lib_exported_only: bool,
        /// Extra entry point: file glob (`examples/**`) or attribute (`#[no_mangle]`). Repeatable.
        #[arg(long)]
        entry: Vec<String>,
        /// Never report items in files matching this glob. Repeatable.
        #[arg(long)]
        ignore: Vec<String>,
        /// What to report: dead functions (default), consts or statics. A const is live when a live function or a root item (bare `pub`, trait impl, `const _`) references it.
        #[arg(long = "type", value_enum, default_value_t)]
        item: crate::ops::DeadItem,
        /// Exit with code 1 if any dead item is reported (same as `--max 0`).
        #[arg(long, conflicts_with = "max")]
        fail_on_any: bool,
        /// Exit with code 1 if more than N dead items are reported.
        #[arg(long)]
        max: Option<usize>,
        #[command(flatten)]
        fmt: FormatArgs,
    },
    /// Report what a change introduces relative to a base ref: newly dead functions, impacted callers and blast radius (what the changed code depends on).
    ///
    /// Accuracy is still being measured; try without thresholds first.
    Pr {
        /// Graph of HEAD (database file or project directory containing .ferrograph).
        #[arg(short, long, conflicts_with = "index_head")]
        db: Option<PathBuf>,
        /// Base ref to compare against (diffed from the merge base with HEAD).
        #[arg(long)]
        base: String,
        /// Reuse an already indexed base graph instead of indexing the base in a temp worktree.
        #[arg(long)]
        base_db: Option<PathBuf>,
        /// Index the HEAD commit into memory instead of reading --db.
        #[arg(long)]
        index_head: bool,
        /// Do not treat ambiguous (low-confidence) call edges as keeping a function alive.
        #[arg(long)]
        strict: bool,
        /// Do not treat `pub(crate)`/`pub(super)`/`pub(in ..)` functions as entry points (bare `pub` still is). On by default, off with `--strict` unless passed.
        #[arg(long, conflicts_with = "no_restricted_vis_not_root")]
        restricted_vis_not_root: bool,
        /// Keep `pub(crate)`/`pub(super)`/`pub(in ..)` functions as entry points (overrides the config file).
        #[arg(long)]
        no_restricted_vis_not_root: bool,
        /// Do not treat bare `pub` functions in files compiled only by bin/test/example/bench targets as entry points. On by default, off with `--strict` unless passed.
        #[arg(long, conflicts_with = "no_bin_pub_not_root")]
        bin_pub_not_root: bool,
        /// Keep bare `pub` functions in bin/test/example/bench-only files as entry points (overrides the config file).
        #[arg(long)]
        no_bin_pub_not_root: bool,
        /// Do not treat bare `pub` free functions that a library crate does not export (private module, no `pub use` path) as entry points. Off by default.
        #[arg(long, conflicts_with = "no_lib_exported_only")]
        lib_exported_only: bool,
        /// Keep unexported library `pub` functions as entry points (overrides the config file).
        #[arg(long)]
        no_lib_exported_only: bool,
        /// Extra entry point: file glob or attribute (`#[no_mangle]`). Repeatable.
        #[arg(long)]
        entry: Vec<String>,
        /// Never report functions in files matching this glob, also in the changed and impacted lists (like `[dead] ignore`). Repeatable.
        #[arg(long)]
        ignore: Vec<String>,
        /// Ignore ambiguous (low-confidence) call edges in the impacted set and blast radius.
        #[arg(long)]
        exclude_low_confidence: bool,
        /// Also report consts and statics the change leaves dead (`newly_dead_items`); they count toward `--fail-on-new-dead` and `--max-new-dead`.
        #[arg(long)]
        include_consts: bool,
        /// Maximum impacted and blast radius nodes listed (the full counts are always reported).
        #[arg(long, default_value = "200")]
        blast_limit: usize,
        /// Caller hops followed to find what a change impacts (1-100). A changed function's count includes other changed functions; the union does not.
        #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..=100))]
        impact_depth: u32,
        /// Exit with code 1 if any function is newly dead.
        #[arg(long, conflicts_with = "max_new_dead")]
        fail_on_new_dead: bool,
        /// Exit with code 1 if more than N functions are newly dead.
        #[arg(long)]
        max_new_dead: Option<usize>,
        /// Exit with code 1 if any function gained `unsafe` (an `unsafe fn` or an `unsafe` block; per function, not per block).
        #[arg(long, conflicts_with = "max_new_unsafe")]
        fail_on_new_unsafe: bool,
        /// Exit with code 1 if more than N functions gained `unsafe`.
        #[arg(long)]
        max_new_unsafe: Option<usize>,
        /// Exit with code 1 if the combined blast radius (what the change depends on, not the impacted callers) has more than N nodes.
        #[arg(long)]
        max_blast: Option<usize>,
        /// Exit with code 1 if more than N functions are impacted (transitive callers of the changed functions; name-based, so it can over-count).
        #[arg(long)]
        max_impacted: Option<usize>,
        /// Also write a report to a file: `FORMAT=PATH` (`json`, `markdown`, `sarif`, `text`). Repeatable;
        /// all outputs come from one run. With `--out` and no `--format`/`--json`, nothing goes to stdout.
        #[arg(long = "out", value_name = "FORMAT=PATH", value_parser = parse_out)]
        out: Vec<(Format, PathBuf)>,
        #[command(flatten)]
        fmt: FormatArgs,
    },
    /// Show the blast radius of a node: what it depends on (forward closure) plus its direct callers; use `callers` for what depends on it.
    Blast {
        /// Path to the graph database, or a project directory containing .ferrograph.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to compute blast radius from.
        node_id: String,
        /// Ignore ambiguous (low-confidence) call edges.
        #[arg(long)]
        exclude_low_confidence: bool,
        /// Exit with code 1 if the blast radius has more than N nodes.
        #[arg(long)]
        max: Option<usize>,
        #[command(flatten)]
        fmt: FormatArgs,
    },
    /// Show callers of a node (reverse call graph).
    Callers {
        /// Path to the graph database, or a project directory containing .ferrograph.
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
        #[command(flatten)]
        fmt: FormatArgs,
    },
    /// Show full info for a node (type, payload, edges).
    Info {
        /// Path to the graph database, or a project directory containing .ferrograph.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Node ID to inspect.
        node_id: String,
    },
    /// Show module containment graph.
    Modules {
        /// Path to the graph database, or a project directory containing .ferrograph.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Filter to modules under this path prefix (e.g. "./src/").
        #[arg(short, long)]
        root: Option<String>,
    },
    /// Show trait implementors.
    Traits {
        /// Path to the graph database, or a project directory containing .ferrograph.
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

/// Result of a successful command run; `main` maps it to the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Exit code 0.
    Ok,
    /// A `--max` / `--fail-on-any` threshold was exceeded (exit code 1).
    FindingsOverThreshold,
}

/// Compare a finding count with an optional threshold, noting a breach on stderr.
fn check_threshold(count: usize, max: Option<usize>) -> Outcome {
    match max {
        Some(m) if count > m => {
            eprintln!("threshold exceeded: {count} > {m}");
            Outcome::FindingsOverThreshold
        }
        _ => Outcome::Ok,
    }
}

/// Run the CLI command.
///
/// # Errors
/// Returns an error if the selected command fails (e.g. I/O or graph errors).
// One flat match over every subcommand; splitting it would only scatter the dispatch.
#[allow(clippy::too_many_lines)]
pub fn run(cli: Cli) -> Result<Outcome> {
    let json = cli.json;
    let ok = |r: Result<()>| r.map(|()| Outcome::Ok);
    match cli.command {
        Command::Index { path, output, db } => {
            ok(run_index(&path, merge_output(output, db)?.as_ref()))
        }
        Command::Query { db, query } => ok(run_query(db.as_ref(), &query, json)),
        Command::Search {
            db,
            query,
            case_insensitive,
        } => ok(run_search(db.as_ref(), &query, case_insensitive, json)),
        Command::Status { db, path } => ok(run_status(db, path, json)),
        Command::Watch { path, output, db } => {
            ok(run_watch(&path, merge_output(output, db)?.as_ref()))
        }
        Command::Dead {
            db,
            file,
            strict,
            restricted_vis_not_root,
            no_restricted_vis_not_root,
            bin_pub_not_root,
            no_bin_pub_not_root,
            lib_exported_only,
            no_lib_exported_only,
            entry,
            ignore,
            item,
            fail_on_any,
            max,
            fmt,
        } => run_dead(
            db.as_ref(),
            file.as_deref(),
            crate::ops::DeadOptions {
                strict,
                restricted_vis_not_root: flag(restricted_vis_not_root, no_restricted_vis_not_root),
                bin_pub_not_root: flag(bin_pub_not_root, no_bin_pub_not_root),
                lib_exported_only: flag(lib_exported_only, no_lib_exported_only),
                entry,
                ignore,
                item,
            },
            fail_on_any.then_some(0).or(max),
            fmt.resolve(json)?,
        ),
        Command::Pr {
            db,
            base,
            base_db,
            index_head,
            strict,
            restricted_vis_not_root,
            no_restricted_vis_not_root,
            bin_pub_not_root,
            no_bin_pub_not_root,
            lib_exported_only,
            no_lib_exported_only,
            entry,
            ignore,
            exclude_low_confidence,
            include_consts,
            blast_limit,
            impact_depth,
            fail_on_new_dead,
            max_new_dead,
            fail_on_new_unsafe,
            max_new_unsafe,
            max_blast,
            max_impacted,
            out,
            fmt,
        } => {
            let to_stdout = out.is_empty() || json || fmt.format.is_some();
            let opts = crate::pr::PrOptions {
                root: std::env::current_dir()?,
                base,
                base_db,
                head_db: resolve_db_path(db.as_ref())?,
                index_head,
                strict,
                restricted_vis_not_root: flag(restricted_vis_not_root, no_restricted_vis_not_root),
                bin_pub_not_root: flag(bin_pub_not_root, no_bin_pub_not_root),
                lib_exported_only: flag(lib_exported_only, no_lib_exported_only),
                entry,
                ignore,
                exclude_low_confidence,
                include_consts,
                blast_limit,
                impact_depth,
            };
            run_pr(
                &opts,
                PrThresholds {
                    new_dead: fail_on_new_dead.then_some(0).or(max_new_dead),
                    new_unsafe: fail_on_new_unsafe.then_some(0).or(max_new_unsafe),
                    blast: max_blast,
                    impacted: max_impacted,
                },
                fmt.resolve(json)?,
                &out,
                to_stdout,
            )
        }
        Command::Blast {
            db,
            node_id,
            exclude_low_confidence,
            max,
            fmt,
        } => run_blast(
            db.as_ref(),
            &node_id,
            exclude_low_confidence,
            max,
            fmt.resolve(json)?,
        ),
        Command::Callers {
            db,
            node_id,
            depth,
            exclude_low_confidence,
            fmt,
        } => ok(run_callers(
            db.as_ref(),
            &node_id,
            depth,
            exclude_low_confidence,
            fmt.resolve(json)?,
        )),
        Command::Info { db, node_id } => ok(run_info(db.as_ref(), &node_id, json)),
        Command::Modules { db, root } => ok(run_modules(db.as_ref(), root.as_deref(), json)),
        Command::Traits { db, trait_name } => ok(run_traits(db.as_ref(), &trait_name, json)),
        Command::Claude { ref action } => ok(crate::claude::run(action, json)),
        Command::Mcp => ok(run_mcp()),
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

pub(crate) fn resolve_db_path(db: Option<&PathBuf>) -> Result<PathBuf> {
    let path = db
        .cloned()
        .or_else(default_db_path)
        .context("No graph database path (use --db or run from a directory with .ferrograph)")?;
    Ok(if path.is_dir() {
        path.join(".ferrograph")
    } else {
        path
    })
}

/// Combine `--output` and its `--db` alias; both may be given only if they agree.
fn merge_output(output: Option<PathBuf>, db: Option<PathBuf>) -> Result<Option<PathBuf>> {
    match (output, db) {
        (Some(o), Some(d)) if o != d => {
            anyhow::bail!(
                "--output and --db are aliases but differ ({} vs {})",
                o.display(),
                d.display()
            )
        }
        (o, d) => Ok(o.or(d)),
    }
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
    /// Treat `pub(crate)`/`pub(super)`/`pub(in ..)` functions as non-roots (default true, false with `--strict`).
    #[serde(default)]
    pub restricted_vis_not_root: Option<bool>,
    /// Treat bare `pub` functions in bin/test/example/bench-only files as non-roots (default true, false with `--strict`).
    #[serde(default)]
    pub bin_pub_not_root: Option<bool>,
    /// Treat bare `pub` free functions a library crate does not export as non-roots (default false).
    #[serde(default)]
    pub lib_exported_only: Option<bool>,
}

/// `--x` / `--no-x` pair to a tri-state: unset leaves the config file and the default in charge.
fn flag(on: bool, off: bool) -> Option<bool> {
    on.then_some(true).or(off.then_some(false))
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

/// Blast radius and callers are impact listings, not problems at a file and line, so SARIF
/// (a problem-report format) has no natural mapping for them.
fn ensure_not_sarif(format: Format) -> Result<()> {
    if format == Format::Sarif {
        anyhow::bail!("sarif not supported for blast/callers; use --format json or markdown");
    }
    Ok(())
}

fn run_dead(
    db: Option<&PathBuf>,
    file: Option<&str>,
    mut opts: crate::ops::DeadOptions,
    max: Option<usize>,
    (format, md_limit): (Format, usize),
) -> Result<Outcome> {
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
    opts.restricted_vis_not_root = opts
        .restricted_vis_not_root
        .or(cfg.dead.restricted_vis_not_root);
    opts.bin_pub_not_root = opts.bin_pub_not_root.or(cfg.dead.bin_pub_not_root);
    opts.lib_exported_only = opts.lib_exported_only.or(cfg.dead.lib_exported_only);
    opts.entry.extend(cfg.dead.entry);
    opts.ignore.extend(cfg.dead.ignore);
    let opts_item = opts.item;
    let result = crate::ops::dead_code(&store, file, &opts)?;
    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if format == Format::Sarif {
        println!(
            "{}",
            serde_json::to_string_pretty(&format::sarif_dead(&result))?
        );
    } else if format == Format::Markdown {
        println!("{}", format::markdown_dead(&result, md_limit));
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
        if opts_item != crate::ops::DeadItem::Function {
            println!("\nFor consts and statics: bare `pub` items are roots, except where `--bin-pub-not-root` or `--lib-exported-only` demotes them like functions. `--lib-exported-only` only covers module-level items; `--bin-pub-not-root` also demotes bare `pub` associated consts in bin-only files.");
        }
    }
    Ok(check_threshold(result.count, max))
}

/// Parse `FORMAT=PATH` for `pr --out`.
fn parse_out(s: &str) -> Result<(Format, PathBuf), String> {
    let (name, path) = s
        .split_once('=')
        .filter(|(_, p)| !p.is_empty())
        .ok_or("expected FORMAT=PATH")?;
    let format = <Format as clap::ValueEnum>::from_str(name, true)?;
    Ok((format, PathBuf::from(path)))
}

/// One rendering of a `pr` result, ending in a newline.
fn render_pr(r: &crate::ops::PrResult, format: Format, md_limit: usize) -> Result<String> {
    Ok(match format {
        Format::Json => format!("{}\n", serde_json::to_string_pretty(r)?),
        Format::Sarif => format!("{}\n", serde_json::to_string_pretty(&format::sarif_pr(r))?),
        Format::Markdown => format!("{}\n", format::markdown_pr(r, md_limit)),
        Format::Text => {
            let mut out = String::new();
            write_pr_text(&mut out, r);
            out
        }
    })
}

/// `--max-*` limits of `pr`; exceeding one exits with code 1.
#[derive(Clone, Copy)]
struct PrThresholds {
    new_dead: Option<usize>,
    new_unsafe: Option<usize>,
    blast: Option<usize>,
    impacted: Option<usize>,
}

fn run_pr(
    opts: &crate::pr::PrOptions,
    max: PrThresholds,
    (format, md_limit): (Format, usize),
    out: &[(Format, PathBuf)],
    to_stdout: bool,
) -> Result<Outcome> {
    let r = crate::pr::run(opts)?;
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
    if to_stdout {
        print!("{}", render_pr(&r, format, md_limit)?);
    }
    // A failed report write is reported after stdout and the threshold messages, so neither is lost.
    let mut write_err = None;
    for (f, path) in out {
        let written = render_pr(&r, *f, md_limit).and_then(|text| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            Ok(std::fs::write(path, text)?)
        });
        if let Err(e) = written {
            write_err.get_or_insert(e.context(format!("Failed to write {}", path.display())));
        }
    }
    let mut outcome = Outcome::Ok;
    for (label, count, max) in [
        (
            "new dead",
            r.newly_dead_count + r.newly_dead_items.len(),
            max.new_dead,
        ),
        ("new unsafe", r.unsafe_gained_count, max.new_unsafe),
        ("blast", r.blast.count, max.blast),
        ("impacted", r.impacted.count, max.impacted),
    ] {
        if let Some(m) = max.filter(|&m| count > m) {
            eprintln!("threshold exceeded: {label} {count} > {m}");
            outcome = Outcome::FindingsOverThreshold;
        }
    }
    write_err.map_or(Ok(outcome), Err)
}

fn write_dead_items_text(out: &mut String, r: &crate::ops::PrResult) {
    use std::fmt::Write as _;
    if r.newly_dead_items.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "\nNewly dead consts and statics: {}",
        r.newly_dead_items.len()
    );
    for f in &r.newly_dead_items {
        let _ = writeln!(out, "  {}\t{}\t{}\t{}", f.id, f.name, f.item_type, f.status);
    }
}

fn write_pr_text(out: &mut String, r: &crate::ops::PrResult) {
    use std::fmt::Write as _;
    macro_rules! emit {
        ($($t:tt)*) => {{ let _ = writeln!(out, $($t)*); }};
    }
    emit!(
        "base {} (merge base {:.8}) .. head {:.8}: {} changed .rs file(s)",
        r.base,
        r.merge_base,
        r.head,
        r.changed_files.len()
    );
    emit!("\nNewly dead functions: {}", r.newly_dead_count);
    for f in &r.newly_dead {
        emit!("  {}\t{}\t{}", f.id, f.name, f.status);
    }
    write_dead_items_text(out, r);
    if !r.unsafe_gained.is_empty() || !r.unsafe_lost.is_empty() {
        emit!("\nFunctions that gained unsafe: {}", r.unsafe_gained_count);
        for f in r.unsafe_gained.iter().chain(&r.unsafe_lost) {
            emit!("  {}\t{}\t{}", f.id, f.name, f.status);
        }
    }
    let i = &r.impacted;
    emit!(
        "\nImpacted by this change (callers up to {} hops): {}",
        i.depth,
        format::count_label(i.count, i.truncated)
    );
    for (n, g) in [
        ("tests", &i.tests),
        ("examples/benches", &i.examples),
        ("library", &i.library),
    ] {
        emit!("  {n}: {}", format::impact_group_label(g));
    }
    let (fns, tests): (Vec<_>, Vec<_>) = r.changed_functions.iter().partition(|f| f.kind == "fn");
    emit!("\nChanged functions: {}", fns.len());
    for f in fns {
        emit!(
            "  {}\t{}\timpacted {}\tdepends on {}",
            f.id,
            f.name,
            f.impacted
                .as_ref()
                .map_or_else(String::new, |x| format::count_label(x.count, x.truncated)),
            format::count_label(f.blast_count, f.blast_truncated)
        );
    }
    if !tests.is_empty() {
        emit!(
            "\nTests, test helpers, benches and examples changed: {}",
            tests.len()
        );
        for f in tests {
            emit!(
                "  {}\t{}\timpacted {}",
                f.id,
                f.name,
                f.impacted
                    .as_ref()
                    .map_or_else(String::new, |x| format::count_label(x.count, x.truncated))
            );
        }
    }
    for c in &r.changed_without_function {
        let ranges: Vec<String> = c
            .lines
            .iter()
            .map(|[a, b]| {
                if a == b {
                    a.to_string()
                } else {
                    format!("{a}-{b}")
                }
            })
            .collect();
        emit!("  (no function) {}: lines {}", c.file, ranges.join(", "));
    }
    emit!(
        "\nDepends on (blast radius): {} node(s), showing {}",
        format::count_label(r.blast.count, r.blast.truncated),
        r.blast.shown
    );
    for n in &r.blast.nodes {
        emit!(
            "  {}\t{}\t{}",
            n.id,
            n.node_type,
            n.payload.as_deref().unwrap_or("—")
        );
    }
    for h in &r.coupling {
        emit!(
            "\nOften changes with {}: {}",
            h.file,
            h.coupled_with.join(", ")
        );
    }
}

fn run_blast(
    db: Option<&PathBuf>,
    node_id: &str,
    exclude_low: bool,
    max: Option<usize>,
    (format, md_limit): (Format, usize),
) -> Result<Outcome> {
    ensure_not_sarif(format)?;
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
    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if format == Format::Markdown {
        println!("{}", format::markdown_blast(&result, md_limit));
    } else {
        for node in &result.reachable_nodes {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!(
            "\n{} nodes in blast radius of {}",
            format::count_label(result.count, result.truncated),
            node_id
        );
    }
    Ok(check_threshold(result.count, max))
}

fn run_callers(
    db: Option<&PathBuf>,
    node_id: &str,
    depth: u32,
    exclude_low: bool,
    (format, md_limit): (Format, usize),
) -> Result<()> {
    ensure_not_sarif(format)?;
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
    if format == Format::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if format == Format::Markdown {
        println!("{}", format::markdown_callers(&result, md_limit));
    } else {
        for node in &result.callers {
            let payload = node.payload.as_deref().unwrap_or("—");
            println!("{}\t{}\t{payload}", node.id, node.node_type);
        }
        println!(
            "\n{} callers of {}",
            format::count_label(result.count, result.truncated),
            node_id
        );
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

fn run_status(db: Option<PathBuf>, legacy_path: Option<PathBuf>, json: bool) -> Result<()> {
    if legacy_path.is_some() {
        eprintln!("note: positional PATH for `status` is deprecated; use --db PATH");
    }
    let path = db.or(legacy_path).unwrap_or_else(|| PathBuf::from("."));
    let db_path = resolve_db_path(Some(&path))?;
    if !db_path.exists() {
        anyhow::bail!(
            "Graph database not found at {}. Run 'ferrograph index --output {}' first.",
            db_path.display(),
            db_path.display()
        );
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
