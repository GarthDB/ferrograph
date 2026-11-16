//! Accuracy harness for ferrograph (replaces `scripts/accuracy/*.py`). Run with `cargo xtask <cmd>`.

#![allow(dead_code)] // shared helpers are wired up by later commands (#192-#194)

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod compare;
mod corpus;
mod impact_sample;
mod run;
mod stats;

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
