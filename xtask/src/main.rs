//! Accuracy harness for ferrograph (replaces `scripts/accuracy/*.py`). Run with `cargo xtask <cmd>`.

#![allow(dead_code)] // shared helpers are wired up by later commands (#191-#194)

use clap::{Parser, Subcommand};

mod corpus;
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
        config: std::path::PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Corpora { config } => {
            for c in corpus::load(&config)? {
                println!("{}\t{}", c.name, c.cohort());
            }
        }
    }
    Ok(())
}
