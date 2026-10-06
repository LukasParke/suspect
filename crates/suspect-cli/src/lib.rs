#![deny(missing_docs)]
//! suspect-cli: the `suspect` binary. Thin `main.rs` delegates here so every
//! command is a testable library function taking plain arguments and
//! returning an exit code (0 clean, 1 findings at/above Error, 2 usage).

/// Git-ref baselines for comparisons.
pub mod baseline;
pub mod bundle;
pub mod commands;
pub mod diff;
pub mod output;
/// SARIF 2.1.0 serialization for CI code-scanning integration.
pub mod sarif;
/// FD-level output silencing for machine-readable runs.
pub mod silence;

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
    /// SARIF 2.1.0 log for code-scanning integrations.
    Sarif,
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
    /// Acquire a hash-pinned document closure or verify its offline cache.
    Acquire(commands::acquire_cmd::AcquireArgs),
    /// Manage and verify credentials for the testing flows.
    Auth {
        /// The auth subcommand to run.
        #[command(subcommand)]
        cmd: commands::auth::AuthCmd,
    },
    /// Parse documents and report family, syntax errors, `$ref` edges, cycles, and workspace stats.
    Check {
        /// Documents to check.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Output format for the per-file reports.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Validate OpenAPI syntax and semantic requirements, reporting located findings.
    Validate {
        /// OpenAPI documents to validate.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// JSON array of absolute document paths; deny every unlisted reference load.
        #[arg(long)]
        reference_allowlist: Option<PathBuf>,
        /// Assert JSON-Schema `format` keywords (RFC 2020-12 makes them
        /// annotations by default; this validates declared formats).
        #[arg(long)]
        strict_format: bool,
        /// Output format for the finding list.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Run the generation admission layer over documents without generating.
    Admission(commands::admission::AdmissionArgs),
    /// Detect consumer-breaking changes between two spec revisions.
    Breaking(commands::breaking::BreakingArgs),
    /// Generate API reference docs (HTML, Markdown, or a SvelteKit site).
    #[command(name = "docs", alias = "docs-gen")]
    DocsGen(commands::docs_gen_cmd::DocsGenArgs),
    /// Generate typed server stubs from an OpenAPI document.
    Stubs(commands::stubs::StubsArgs),
    /// Detect breaking changes between two Arazzo documents.
    ArazzoDiff(commands::arazzo_diff::ArazzoDiffArgs),
    /// Report overlay action target matches without applying.
    OverlayDryRun(commands::overlay_dry_run::OverlayDryRunArgs),
    /// Upgrade Swagger 2.0 documents to OpenAPI 3.1.
    Upgrade(commands::upgrade::UpgradeArgs),
    /// Run TS/JS custom rules (Bun sidecar) over documents.
    Rules {
        /// The rules subcommand to run.
        #[command(subcommand)]
        cmd: commands::rules::RulesCmd,
    },
    /// Run spectral-style lint rules over documents.
    Lint {
        /// Documents to lint.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Ruleset document (default: built-in spectral ruleset).
        #[arg(long)]
        ruleset: Option<PathBuf>,
        /// Report only findings at or above this severity (default: hint,
        /// or `lint.min_severity` from `.suspect.yaml`).
        #[arg(long)]
        min_severity: Option<output::Severity>,
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
    /// Run the live contract bridge: watch the spec and emit regeneration
    /// plans, evolution proposals, and reconciliation conflicts.
    Bridge {
        /// Entry OpenAPI document to watch.
        spec: PathBuf,
        /// Polling interval in milliseconds.
        #[arg(long, default_value_t = 250)]
        interval_ms: u64,
        /// Stop after this many ticks (for scripts; omit to watch).
        #[arg(long, hide = true)]
        max_ticks: Option<u64>,
        /// Newline-delimited JSON tick records instead of human text.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Extract server framework routes and cross-reference them with the
    /// spec: undocumented endpoints, spec-only endpoints, method drift.
    Reverse {
        /// Server source tree or single file with route registrations.
        source: PathBuf,
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Structured JSON report instead of human text.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Generate and run stateful dependency-graph test sequences against a
    /// live server.
    Stateful {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Base URL prepended to operation paths.
        #[arg(long, default_value = "http://localhost:8080")]
        base_url: String,
        /// Run only sequences whose target operationId contains this.
        #[arg(long)]
        filter: Option<String>,
        /// Print the generated sequences as JSON instead of running them.
        #[arg(long)]
        emit: bool,
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
        /// Message broker directory for Arazzo 1.1 AsyncAPI steps:
        /// `inbox.jsonl` supplies pre-recorded messages and `outbox.jsonl`
        /// collects what the workflow published.
        #[arg(long, value_name = "DIR")]
        message_broker: Option<PathBuf>,
        /// Credentials file for the security schemes the workflows
        /// exercise. Default: `.suspect/credentials.json` discovered by
        /// walking up from the Arazzo document; `SUSPECT_CREDENTIALS`
        /// overrides.
        #[arg(long, value_name = "FILE")]
        credentials: Option<PathBuf>,
        /// Write the journal to this file (append) instead of stdout.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
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
        /// Write the journal to this file (append) instead of stdout.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
        /// Grammar-evolved, coverage-guided fuzzing: novel response shapes
        /// become seeds the campaign exploits.
        #[arg(long)]
        evolved: bool,
        /// Evolved fuzzing: rounds per operation.
        #[arg(long, default_value_t = 8)]
        rounds: u32,
        /// Evolved fuzzing: requests per round.
        #[arg(long, default_value_t = 12)]
        per_round: u32,
    },
    /// Trace a validation failure to its origin: source location, git
    /// blame, and whether recorded traffic ever passed it.
    Why {
        /// The constraint that fired (e.g. `type` or the diagnostic text).
        constraint: String,
        /// Spec file the failure refers to.
        #[arg(long)]
        spec: PathBuf,
        /// Byte offset of the failing constraint in the spec, when known.
        #[arg(long)]
        offset: Option<usize>,
        /// Line of the failing constraint (1-based), with `--why-col`.
        #[arg(long)]
        why_line: Option<usize>,
        /// Column of the failing constraint (1-based), with `--why-line`.
        #[arg(long)]
        why_col: Option<usize>,
        /// Directory of recorded cassettes searched for historical
        /// traffic (`.jsonl`/`.ndjson`/`.json` files).
        #[arg(long, value_name = "DIR")]
        cassette_dir: Option<PathBuf>,
        /// Human timeline or one JSON document.
        #[command(flatten)]
        text: TextFormat,
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
        /// Write the journal to this file (append) instead of stdout.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
    },
    /// Render documentation or custom template manifests from an OpenAPI document.
    Gen {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Shipped documentation preset.
        #[arg(long, conflicts_with = "manifest", value_parser=["docs-md"])]
        preset: Option<String>,
        /// Custom gen.toml manifest (templates resolved relative to it).
        #[arg(long, conflicts_with = "preset")]
        manifest: Option<PathBuf>,
        /// Output root directory.
        #[arg(short, long, default_value = "gen-out")]
        out: PathBuf,
        /// Print unified diffs without writing files; exit 1 when output or ownership drifts.
        #[arg(long)]
        diff: bool,
        /// Stable logical owner when sharing an output root across generation streams.
        #[arg(long)]
        owner: Option<String>,
        /// Explicitly adopt only byte-identical unowned output.
        #[arg(long)]
        adopt_identical: bool,
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
        /// Operating mode: mock | proxy | validate | record | replay |
        /// scenario.
        #[arg(long, default_value = "mock")]
        mode: String,
        /// Scenario mode: JSON file of scripted steps
        /// (`{"steps": [{"method", "path_suffix", "status", "body"}]}`).
        #[arg(long, value_name = "FILE")]
        scenario: Option<PathBuf>,
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
        /// Write the journal to this file (append) instead of stdout.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
        /// Header name redacted from journals and cassettes beyond the
        /// default denylist (repeatable).
        #[arg(long = "redact-header", value_name = "NAME")]
        redact_headers: Vec<String>,
        /// JSON body key redacted from journals and cassettes beyond the
        /// default denylist (repeatable).
        #[arg(long = "redact-json-key", value_name = "KEY")]
        redact_json_keys: Vec<String>,
    },
    /// Generate a source-selected native SDK package.
    Codegen(commands::codegen_cmd::CodegenArgs),
    /// Generate a Terraform provider through explicit lifecycle mappings and the generated Go SDK.
    CodegenTerraform(commands::terraform_cmd::TerraformArgs),
    /// Generate a Go/Cobra API CLI application over its own embedded generated Go SDK.
    CodegenCli(commands::application_cmd::ApplicationArgs),
    /// Generate a TypeScript stdio MCP server over its own embedded generated TypeScript SDK.
    CodegenMcp(commands::application_cmd::ApplicationArgs),
    /// List the native SDK or application profiles available in this CLI build.
    CodegenProfiles {
        /// Profile family: native SDK backends, or application targets.
        #[arg(long, value_enum, default_value_t = commands::sdk_profiles::ProfileKind::Sdk)]
        kind: commands::sdk_profiles::ProfileKind,
        /// Human-readable inventory or versioned JSON for editor integration.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Persistent canonical SDK generation with readonly preview and watch mode.
    CodegenSession(commands::codegen_session::SessionArgs),
    /// Compare wire contracts and native SDK interfaces with migration notes.
    CodegenCompare(commands::codegen_compare::CompareArgs),
    /// Recommend a release version and changelog from two spec revisions.
    #[command(name = "release-plan")]
    ReleasePlan(commands::release::ReleasePlanArgs),
    /// Evaluate recorded traffic against a candidate contract revision.
    #[command(name = "impact")]
    Impact(commands::impact::ImpactArgs),
    /// Plan and execute SDK publishing across registries.
    #[command(name = "release-publish")]
    ReleasePublish(commands::publish::PublishArgs),
    /// Render the release manifest as a tag-triggered CI workflow.
    #[command(name = "release-workflow")]
    ReleaseWorkflow(commands::publish::WorkflowArgs),
    /// Print the SDK verification matrix: backend x feature -> evidence.
    Evidence(commands::evidence::EvidenceArgs),
    /// Run validate, contract, breaking, codegen, and tests across every
    /// project in a workspace as one gate.
    Ci(commands::ci::CiArgs),
    /// Emit or verify a machine-readable contract package.
    Contract(commands::contract::ContractArgs),
    /// Show the project configuration in effect for this invocation.
    Config {
        /// Report the configuration that would apply to this input, instead
        /// of the one above the working directory.
        input: Option<PathBuf>,
    },
    /// Build and check a suspect project from one manifest.
    #[command(name = "project")]
    Project {
        /// The project subcommand to run.
        #[command(subcommand)]
        cmd: commands::project::ProjectCmd,
    },
    /// Run the language server over stdio.
    Lsp,
}

/// Parses a severity name from a configuration file.
fn parse_severity(name: &str) -> Option<output::Severity> {
    match name.trim().to_ascii_lowercase().as_str() {
        "error" => Some(output::Severity::Error),
        "warning" => Some(output::Severity::Warning),
        "info" => Some(output::Severity::Info),
        "hint" => Some(output::Severity::Hint),
        _ => None,
    }
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
        Command::Validate {
            paths,
            reference_allowlist,
            strict_format,
            text,
        } => {
            let loaded = suspect_config::for_invocation(paths.first().map(PathBuf::as_path))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let strict_format = suspect_config::resolve_bool(
                strict_format,
                "SUSPECT_STRICT_FORMAT",
                loaded.settings.validate.strict_format,
            );
            commands::validate::validate(
                &paths,
                text.format,
                reference_allowlist.as_deref(),
                strict_format,
            )
        }
        Command::Admission(args) => commands::admission::admission(&args),
        Command::Breaking(args) => commands::breaking::breaking(&args),
        Command::DocsGen(mut args) => {
            let loaded = suspect_config::for_invocation(Some(&args.input))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let settings = &loaded.settings;
            if args.style.is_none()
                && let Some(style) = settings.docs.style.as_deref()
            {
                args.style = Some(match style {
                    "markdown" => commands::docs_gen_cmd::DocsStyle::Markdown,
                    "sveltekit" => commands::docs_gen_cmd::DocsStyle::Sveltekit,
                    "html" => commands::docs_gen_cmd::DocsStyle::Html,
                    other => {
                        return Err(anyhow::anyhow!(
                            "docs style `{other}` is not one of: markdown, sveltekit, html"
                        ));
                    }
                });
            }
            if args.output.is_none()
                && let Some(out) = &settings.docs.out
            {
                args.output = Some(out.clone());
            }
            commands::docs_gen_cmd::docs_gen(&args)
        }
        Command::Stubs(args) => commands::stubs::stubs(&args),
        Command::ArazzoDiff(args) => commands::arazzo_diff::arazzo_diff(&args),
        Command::OverlayDryRun(args) => commands::overlay_dry_run::overlay_dry_run(&args),
        Command::Upgrade(args) => commands::upgrade::upgrade(&args),
        Command::Rules { cmd } => cmd.run().map(|_| 0),
        Command::Lint {
            paths,
            ruleset,
            min_severity,
            text,
        } => {
            // Configuration supplies defaults; an explicit flag wins.
            let loaded = suspect_config::for_invocation(paths.first().map(PathBuf::as_path))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let settings = &loaded.settings;
            let ruleset = ruleset.or_else(|| settings.lint.ruleset.clone());
            // flag > env > config > default, decided in one place.
            let min_severity = suspect_config::resolve::Resolution::new(
                min_severity,
                std::env::var("SUSPECT_LINT_MIN_SEVERITY")
                    .ok()
                    .and_then(|v| parse_severity(&v)),
                settings
                    .lint
                    .min_severity
                    .as_deref()
                    .and_then(parse_severity),
                output::Severity::Hint,
            )
            .value;
            commands::lint::lint(&paths, ruleset.as_deref(), min_severity, text.format)
        }
        Command::Overlay { cmd } => commands::overlay::run(cmd),
        Command::Fmt {
            input,
            output,
            json,
            yaml,
        } => {
            let loaded =
                suspect_config::for_invocation(Some(&input)).map_err(|e| anyhow::anyhow!("{e}"))?;
            // A typed --json/--yaml flag wins; otherwise the file decides.
            let (json, yaml) = if json || yaml {
                (json, yaml)
            } else {
                (loaded.settings.format.json, loaded.settings.format.yaml)
            };
            commands::fmt::fmt(&input, output.as_deref(), json, yaml)
        }
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
            journal,
            evolved,
            rounds,
            per_round,
        } => commands::fuzz::fuzz(
            &spec,
            &base_url,
            runs,
            filter.as_deref(),
            journal.as_deref(),
            evolved,
            rounds,
            per_round,
        ),
        Command::Replay {
            cassette,
            upstream,
            diff,
            journal,
        } => commands::replay::replay(&cassette, &upstream, diff, journal.as_deref()),
        Command::Bridge {
            spec,
            interval_ms,
            max_ticks,
            format,
        } => commands::bridge::bridge(
            &spec,
            interval_ms,
            max_ticks,
            matches!(format, OutputFormat::Json),
        ),
        Command::Reverse {
            source,
            spec,
            format,
        } => commands::reverse::reverse(&source, &spec, matches!(format, OutputFormat::Json)),
        Command::Stateful {
            spec,
            base_url,
            filter,
            emit,
        } => commands::stateful::stateful(&spec, &base_url, filter.as_deref(), emit),
        Command::Auth {
            cmd: commands::auth::AuthCmd::Check { credentials },
        } => commands::auth::check(credentials.as_deref()),
        Command::Test {
            arazzo,
            base_url,
            filter,
            cassette,
            offline: _,
            report,
            message_broker,
            credentials,
            journal,
        } => commands::test::test_with_messages(
            &arazzo,
            &base_url,
            filter.as_deref(),
            cassette.as_deref(),
            matches!(report, ReportFormat::Ndjson),
            message_broker.as_deref(),
            credentials.as_deref(),
            journal.as_deref(),
        ),
        Command::Why {
            constraint,
            spec,
            offset,
            why_line,
            why_col,
            cassette_dir,
            text,
        } => commands::why::why(
            &constraint,
            &spec,
            offset,
            why_line,
            why_col,
            cassette_dir.as_deref(),
            matches!(text.format, OutputFormat::Json),
        ),
        Command::Gen {
            spec,
            preset,
            manifest,
            out,
            diff,
            owner,
            adopt_identical,
        } => commands::generate::generate(
            &spec,
            preset.as_deref(),
            manifest.as_deref(),
            &out,
            diff,
            owner.as_deref(),
            adopt_identical,
        ),
        Command::Gateway {
            spec,
            port,
            mode,
            scenario,
            upstream,
            cassette,
            enforce,
            delay_ms,
            delay_pct,
            error_status,
            error_pct,
            journal,
            redact_headers,
            redact_json_keys,
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
            scenario.as_ref(),
            journal.as_deref(),
            &redact_headers,
            &redact_json_keys,
        ),
        Command::Watch { roots, command } => commands::watch::watch(&roots, &command),
        Command::Acquire(args) => commands::acquire_cmd::acquire(args),
        Command::Codegen(args) => commands::codegen_cmd::codegen(args),
        Command::CodegenTerraform(args) => commands::terraform_cmd::generate(args),
        Command::CodegenCli(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Cli, args)
        }
        Command::CodegenMcp(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Mcp, args)
        }
        Command::CodegenProfiles { kind, text } => match kind {
            commands::sdk_profiles::ProfileKind::Sdk => commands::sdk_profiles::list(text.format),
            commands::sdk_profiles::ProfileKind::Applications => {
                commands::sdk_profiles::list_applications(text.format)
            }
        },
        Command::CodegenSession(args) => commands::codegen_session::generate(args),
        Command::CodegenCompare(args) => commands::codegen_compare::compare(args),
        Command::Config { input } => {
            let loaded = suspect_config::for_invocation(input.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{}", loaded.describe());
            Ok(0)
        }
        Command::Evidence(args) => commands::evidence::evidence(&args),
        Command::Ci(args) => {
            // The gate reports its own aggregate, so a failing stage is a
            // reported verdict rather than a raised error.
            commands::ci::ci(&args)
        }
        Command::Contract(args) => commands::contract::contract(&args),
        Command::ReleasePlan(args) => commands::release::release_plan(&args),
        Command::ReleasePublish(args) => commands::publish::publish(&args),
        Command::ReleaseWorkflow(args) => commands::publish::workflow(&args),
        Command::Impact(args) => commands::impact::impact(&args),
        Command::Project { cmd } => commands::project::run(cmd),
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
