#![deny(missing_docs)]
//! The `suspect` binary: parse args, dispatch into the library, map the
//! result onto process exit codes (0 clean, 1 findings, 2 usage/error).

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = suspect_cli::Cli::parse();
    match suspect_cli::execute(cli) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
