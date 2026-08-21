//! suspect-cli: the `suspect` binary. Thin `main.rs` delegates here so every
//! command is a testable library function taking plain arguments and
//! returning an exit code (0 clean, 1 findings at/above Error, 2 usage).

pub mod bundle;
pub mod commands;
pub mod diff;
pub mod output;

use std::path::PathBuf;

pub use output::{Finding, Severity};
use clap::{Parser, Subcommand, ValueEnum};
use suspect_source::{Source, Uri};

/// Serialization choice for commands that produce structured output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    Text,
    Json,
}

/// Document serialization for emitting materialized trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum DocFormat {
    Json,
    Yaml,
}

/// Bundling strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Strategy {
    /// Load every reachable document, validate all `$ref`s, emit input unchanged.
    Keep,
    /// Materialize the document with every `$ref` replaced by its resolved target.
    Inline,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Check {
        /// Documents to check.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[command(flatten)]
        text: TextFormat,
    },
    /// Run spectral-style lint rules over documents.
    Lint {
        /// Documents to lint.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Ruleset document (default: built-in spectral ruleset).
        #[arg(long)]
        ruleset: Option<PathBuf>,
        /// Report only findings at or above this severity.
        #[arg(long, default_value = "hint")]
        min_severity: output::Severity,
        #[command(flatten)]
        text: TextFormat,
    },
    /// Apply an Overlay 1.0 document to a target document.
    Overlay {
        #[command(subcommand)]
        cmd: commands::overlay::OverlayCmd,
    },
    /// Re-emit a document in canonical JSON/YAML form.
    Fmt {
        /// Input document.
        input: PathBuf,
        /// Write to this file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Force JSON output.
        #[arg(long, conflicts_with = "yaml")]
        json: bool,
        /// Force YAML output.
        #[arg(long)]
        yaml: bool,
    },
    /// Structural counts for a document.
    Stats {
        /// Input document.
        path: PathBuf,
        #[command(flatten)]
        text: TextFormat,
    },
    /// Bundle a document and its `$ref` closure into one file.
    Bundle {
        /// Entry document.
        input: PathBuf,
        /// Write to this file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Bundling strategy.
        #[arg(long, value_enum, default_value = "keep")]
        strategy: Strategy,
        /// Output serialization (inline only; default: input extension).
        #[arg(long = "format", value_enum, id = "bundle_format")]
        out_format: Option<DocFormat>,
    },
    /// Semantic structural diff between two documents.
    Diff {
        /// Left-hand document.
        a: PathBuf,
        /// Right-hand document.
        b: PathBuf,
        #[command(flatten)]
        text: TextFormat,
    },
    /// Wall-clock micro-benchmark of the pipeline stages on one fixture.
    Bench {
        /// Fixture document.
        fixture: PathBuf,
        /// Iterations per stage (mean reported).
        #[arg(long, default_value_t = 3)]
        iters: usize,
        #[command(flatten)]
        text: TextFormat,
    },
    /// Run the language server over stdio.
    Lsp,
}

/// `--format json|text` for commands with structured output. Declared per
/// subcommand (not global) so `bundle` can own `--format json|yaml`.
#[derive(Debug, clap::Args)]
pub struct TextFormat {
    /// Output format for structured results.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

/// Top-level CLI shape (`suspect --format json <command> ...`).
#[derive(Debug, Parser)]
#[command(name = "suspect", version, about = "OpenAPI/Arazzo/Overlay toolkit")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Dispatches a parsed CLI invocation, returning the process exit code.
///
/// # Errors
/// Propagates unexpected IO/model failures; the binary prints them and exits 2.
pub fn execute(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Command::Check { paths, text } => commands::check::check(&paths, text.format),
        Command::Lint { paths, ruleset, min_severity, text } => {
            commands::lint::lint(&paths, ruleset.as_deref(), min_severity, text.format)
        }
        Command::Overlay { cmd } => commands::overlay::run(cmd),
        Command::Fmt { input, output, json, yaml } => {
            commands::fmt::fmt(&input, output.as_deref(), json, yaml)
        }
        Command::Stats { path, text } => commands::stats::stats(&path, text.format),
        Command::Bundle { input, output, strategy, out_format } => {
            bundle::bundle(&input, output.as_deref(), strategy, out_format)
        }
        Command::Diff { a, b, text } => diff::diff_files(&a, &b, text.format),
        Command::Bench { fixture, iters, text } => {
            commands::bench::bench(&fixture, iters, text.format)
        }
        Command::Lsp => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(suspect_lsp::run_server());
            Ok(0)
        }
    }
}

/// Loads and parses one document, pairing its canonical URI with the low model.
///
/// # Errors
/// Filesystem IO or an unrepresentable path.
pub fn load_doc(path: &std::path::Path) -> anyhow::Result<suspect_low::LowDoc> {
    let source = Source::from_path(path)?;
    let uri = Uri::from_path(path)?;
    Ok(suspect_low::LowDoc::parse(uri, source))
}
