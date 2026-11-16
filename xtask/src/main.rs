//! Accuracy harness for ferrograph (replaces `scripts/accuracy/*.py`). Run with `cargo xtask <cmd>`.

#![allow(dead_code)] // shared helpers are wired up by later commands (#192-#194)

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod compare;
mod corpus;
mod graph;
mod groundtruth;
mod impact_sample;
mod matrix;
mod miss_cfg;
mod run;
mod rustc;
mod scan;
mod source;
mod stats;
#[cfg(test)]
mod testutil;
mod util;

#[derive(Parser)]
#[command(name = "xtask", about = "Ferrograph accuracy harness")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the corpora in a corpora.toml (checks the file parses).
    Corpora {
        #[arg(long, default_value = "scripts/accuracy/corpora.toml")]
        config: PathBuf,
    },
    /// Measure `ferrograph dead` against rustc's `dead_code` lint.
    Groundtruth {
        #[arg(long, default_value = "scripts/accuracy/corpora.toml")]
        config: PathBuf,
        #[arg(long)]
        ferrograph: String,
        /// Work dir (default: a fresh temp dir; reused if given).
        #[arg(long)]
        work: Option<PathBuf>,
        /// Comma-separated corpus names.
        #[arg(long, default_value = "")]
        only: String,
        /// Per cargo invocation, seconds.
        #[arg(long, default_value_t = 1800)]
        timeout: u64,
        /// Extra args for `ferrograph dead`, e.g. `--restricted-vis-not-root` (the model follows it).
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        extra_args: String,
        #[arg(long, default_value_t = 25)]
        max_check: usize,
        /// Only re-render this groundtruth JSON to --out-md.
        #[arg(long)]
        render_json: Option<PathBuf>,
        #[arg(long, default_value = "groundtruth-report.md")]
        out_md: PathBuf,
        #[arg(long, default_value = "groundtruth-data.json")]
        out_json: PathBuf,
    },
    /// How many misses are kept alive only through cfg-gated callers.
    MissCfg {
        json: PathBuf,
        #[arg(long)]
        work: PathBuf,
        #[arg(long, default_value = "default")]
        mode: String,
    },
    /// Scan candidate repos for rustc dead-code ground truth.
    Scan {
        #[arg(required = true)]
        repos: Vec<String>,
        #[arg(long)]
        work: PathBuf,
        #[arg(long, default_value_t = 1500)]
        timeout: u64,
        #[arg(long, default_value = "scan.tsv")]
        out: PathBuf,
    },
    /// Run `groundtruth` for every url corpus under several arms and merge per arm.
    RunMatrix {
        #[arg(long)]
        ferrograph: String,
        #[arg(long)]
        work: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "scripts/accuracy/corpora.toml")]
        config: PathBuf,
        #[arg(long, default_value = "")]
        only: String,
        /// Comma-separated, from default, restricted, binpub, both.
        #[arg(long, default_value = "default,restricted")]
        arms: String,
        #[arg(long)]
        keep_targets: bool,
    },
    /// Paired comparison of two `groundtruth` JSON runs.
    Compare {
        base: PathBuf,
        variant: PathBuf,
        #[arg(long, default_value = "default")]
        mode: String,
        #[arg(long, default_value = "compare.md")]
        out_md: PathBuf,
    },
    /// Bucket the `pr` impacted set by depth, or score hand labels.
    ImpactSample {
        #[command(subcommand)]
        cmd: ImpactCmd,
    },
}

#[derive(Subcommand)]
enum ImpactCmd {
    /// CSV on stdout from `DIR/pr<N>-d{1,2,3}[-x].json`.
    Bucket { dir: PathBuf, prs: Vec<String> },
    /// Precision by depth, default vs `--exclude-low-confidence`.
    Score { labels: PathBuf },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Corpora { config } => {
            for c in corpus::load(&config)? {
                println!("{}\t{}", c.name, c.cohort());
            }
        }
        Cmd::Groundtruth {
            config,
            ferrograph,
            work,
            only,
            timeout,
            extra_args,
            max_check,
            render_json,
            out_md,
            out_json,
        } => {
            groundtruth::main(&groundtruth::Args {
                config,
                ferrograph,
                work,
                only,
                timeout_secs: timeout,
                extra_args,
                max_check,
                render_json,
                out_md,
                out_json,
                print_report: true,
            })?;
        }
        Cmd::MissCfg { json, work, mode } => miss_cfg::main(&json, &work, &mode)?,
        Cmd::Scan {
            repos,
            work,
            timeout,
            out,
        } => scan::main(&repos, &work, timeout, &out)?,
        Cmd::RunMatrix {
            ferrograph,
            work,
            out,
            config,
            only,
            arms,
            keep_targets,
        } => {
            let missing = matrix::main(&matrix::Args {
                ferrograph,
                work,
                out,
                config,
                only,
                arms,
                keep_targets,
            })?;
            if !missing.is_empty() {
                std::process::exit(1);
            }
        }
        Cmd::Compare {
            base,
            variant,
            mode,
            out_md,
        } => {
            let md = compare::render(&base, &variant, &mode)?;
            std::fs::write(out_md, &md)?;
            println!("{md}");
        }
        Cmd::ImpactSample {
            cmd: ImpactCmd::Bucket { dir, prs },
        } => {
            print!("{}", impact_sample::bucket(&dir, &prs)?);
        }
        Cmd::ImpactSample {
            cmd: ImpactCmd::Score { labels },
        } => print!("{}", impact_sample::score(&labels)?),
    }
    Ok(())
}
