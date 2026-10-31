#![deny(missing_docs)]
//! The `suspect` binary: rewrite old command names, parse args, install
//! the one Ctrl-C listener, dispatch into the library, map the result onto
//! process exit codes (0 clean, 1 findings, 2 usage/error, 130 interrupted).

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let (argv, note) = suspect_cli::legacy::rewrite(std::env::args_os().collect());
    if let Some(note) = note {
        eprintln!("{note}");
    }
    // clap exits 2 on usage errors and 0 on --help/--version.
    let cli = suspect_cli::Cli::parse_from(argv);
    suspect_cli::interrupt::install();
    match suspect_cli::execute(cli) {
        Ok(exit) => exit.into(),
        Err(e) => {
            eprintln!("error: {e:#}");
            suspect_cli::Exit::Usage.into()
        }
    }
}
