//! CLI entry point for Ferrograph.

use std::process::ExitCode;

use clap::Parser;

use ferrograph::config::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match ferrograph::config::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:?}");
            ExitCode::FAILURE
        }
    }
}
