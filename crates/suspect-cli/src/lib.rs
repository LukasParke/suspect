#![deny(missing_docs)]
//! suspect-cli: the `suspect` binary. Thin `main.rs` delegates here so every
//! command is a testable library function taking plain arguments and
//! returning an exit code (0 clean, 1 findings at/above Error, 2 usage).

pub mod bundle;
pub mod commands;
pub mod diff;
pub mod output;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
pub use output::{Finding, Severity};
use suspect_source::{Source, Uri};

/// Serialization choice for commands that produce structured output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    /// Human-readable aligned text (the default).
    Text,
    /// One pretty-printed JSON document on stdout, machine-consumable.
    Json,
}

/// Document serialization for emitting materialized trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum DocFormat {
    /// Pretty-printed JSON.
    Json,
    /// YAML (block style, no anchors or aliases).
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

/// Subcommands of the `suspect` binary; each variant is one subcommand and
/// its doc comment becomes the one-line help text shown by `--help`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse documents and report family, syntax errors, `$ref` edges, cycles, and workspace stats.
    Check {
        /// Documents to check.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Output format for the per-file reports.
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
        /// Output format for the finding list.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Apply an Overlay 1.0 document to a target document.
    Overlay {
        /// The overlay subcommand to run.
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
        /// Output format for the counts table.
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
        /// Output format for the difference report.
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
        /// Output format for the stage table.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Compile an Arazzo document into an executable suite and run it.
    Test {
        /// Arazzo document describing the workflows.
        arazzo: PathBuf,
        /// Base URL prepended to operation paths.
        #[arg(long, default_value = "http://localhost:8080")]
        base_url: String,
        /// Run only workflows whose id contains this substring.
        #[arg(long)]
        filter: Option<String>,
        /// Run offline against a recorded Suspect Cassette instead of live HTTP.
        #[arg(long, requires = "offline")]
        cassette: Option<PathBuf>,
        /// Enables offline mode (requires --cassette).
        #[arg(long)]
        offline: bool,
        /// Event stream format: human text or one-JSON-per-line ndjson.
        #[arg(long, value_enum, default_value = "text")]
        report: ReportFormat,
    },
    /// Fuzz operations with schema-mutating requests against a live server.
    Fuzz {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Base URL prepended to operation paths.
        #[arg(long, default_value = "http://localhost:8080")]
        base_url: String,
        /// Mutant requests generated per operation.
        #[arg(long, default_value_t = 25)]
        runs: usize,
        /// Run only operations whose operationId contains this substring.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Re-issue a recorded cassette against an upstream and report drift.
    Replay {
        /// Recorded Suspect Cassette to replay from.
        cassette: PathBuf,
        /// Upstream base URL the recorded traffic is re-issued against.
        #[arg(long)]
        upstream: String,
        /// Print unified diffs of drifted UTF-8 response bodies.
        #[arg(long)]
        diff: bool,
    },
    /// Render documentation or SDK presets from an OpenAPI document.
    Gen {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Shipped preset: docs-md | ts-sdk | rust-sdk.
        #[arg(long, conflicts_with = "manifest")]
        preset: Option<String>,
        /// Custom gen.toml manifest (templates resolved relative to it).
        #[arg(long, conflicts_with = "preset")]
        manifest: Option<PathBuf>,
        /// Output root directory.
        #[arg(short, long, default_value = "gen-out")]
        out: PathBuf,
        /// Print unified diffs without writing files.
        #[arg(long)]
        diff: bool,
    },
    /// Re-run a command whenever documents change under the given roots.
    Watch {
        /// Directories (or files) watched recursively for yaml/yml/json changes.
        #[arg(required = true)]
        roots: Vec<PathBuf>,
        /// Command (with arguments) to run and re-run on each change burst.
        #[arg(last = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Serve a spec as a mock, or proxy/validate/record against an upstream,
    /// or replay a recorded cassette.
    Gateway {
        /// Entry OpenAPI document (mock/validate/record) or ignored (replay).
        spec: PathBuf,
        /// TCP port to bind on 127.0.0.1.
        #[arg(long, short = 'p', default_value_t = 8080)]
        port: u16,
        /// Operating mode: mock | proxy | validate | record | replay.
        #[arg(long, default_value = "mock")]
        mode: String,
        /// Upstream base URL for proxy/validate/record modes.
        #[arg(long)]
        upstream: Option<PathBuf>,
        /// Cassette path for record output.
        #[arg(long)]
        cassette: Option<PathBuf>,
        /// Validate mode only: reject invalid requests with 400 instead of
        /// forwarding them.
        #[arg(long)]
        enforce: bool,
        /// Fault injection: delay in milliseconds.
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
        /// Fault injection: percent of requests delayed.
        #[arg(long, default_value_t = 0)]
        delay_pct: u8,
        /// Fault injection: status returned by faulted requests.
        #[arg(long)]
        error_status: Option<u16>,
        /// Fault injection: percent of requests faulted.
        #[arg(long, default_value_t = 0)]
        error_pct: u8,
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

/// Output style for `suspect test` event streams.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ReportFormat {
    /// Human-readable progress lines.
    Text,
    /// One JSON object per line (machine consumable).
    Ndjson,
}

/// Top-level CLI shape (`suspect --format json <command> ...`).
#[derive(Debug, Parser)]
#[command(name = "suspect", version, about = "OpenAPI/Arazzo/Overlay toolkit")]
pub struct Cli {
    /// The subcommand to run; see [`Command`] for the available operations.
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
        Command::Lint {
            paths,
            ruleset,
            min_severity,
            text,
        } => commands::lint::lint(&paths, ruleset.as_deref(), min_severity, text.format),
        Command::Overlay { cmd } => commands::overlay::run(cmd),
        Command::Fmt {
            input,
            output,
            json,
            yaml,
        } => commands::fmt::fmt(&input, output.as_deref(), json, yaml),
        Command::Stats { path, text } => commands::stats::stats(&path, text.format),
        Command::Bundle {
            input,
            output,
            strategy,
            out_format,
        } => bundle::bundle(&input, output.as_deref(), strategy, out_format),
        Command::Diff { a, b, text } => diff::diff_files(&a, &b, text.format),
        Command::Bench {
            fixture,
            iters,
            text,
        } => commands::bench::bench(&fixture, iters, text.format),
        Command::Fuzz {
            spec,
            base_url,
            runs,
            filter,
        } => commands::fuzz::fuzz(&spec, &base_url, runs, filter.as_deref()),
        Command::Replay {
            cassette,
            upstream,
            diff,
        } => commands::replay::replay(&cassette, &upstream, diff),
        Command::Test {
            arazzo,
            base_url,
            filter,
            cassette,
            offline: _,
            report,
        } => commands::test::test(
            &arazzo,
            &base_url,
            filter.as_deref(),
            cassette.as_deref(),
            matches!(report, ReportFormat::Ndjson),
        ),
        Command::Gen {
            spec,
            preset,
            manifest,
            out,
            diff,
        } => {
            commands::generate::generate(&spec, preset.as_deref(), manifest.as_deref(), &out, diff)
        }
        Command::Gateway {
            spec,
            port,
            mode,
            upstream,
            cassette,
            enforce,
            delay_ms,
            delay_pct,
            error_status,
            error_pct,
        } => commands::gateway::gateway(
            &spec,
            port,
            &mode,
            upstream.as_ref(),
            cassette.as_ref(),
            enforce,
            delay_ms,
            delay_pct,
            error_status,
            error_pct,
        ),
        Command::Watch { roots, command } => commands::watch::watch(&roots, &command),
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
