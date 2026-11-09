//! CLI entry point for Ferrograph.
//!
//! Exit codes: 0 = ok, 1 = findings over threshold, 2 = error.

use std::process::ExitCode;

use clap::Parser;

use ferrograph::config::{Cli, Outcome};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match ferrograph::config::run(cli) {
        Ok(Outcome::Ok) => ExitCode::SUCCESS,
        Ok(Outcome::FindingsOverThreshold) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
