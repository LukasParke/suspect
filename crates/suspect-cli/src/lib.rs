#![deny(missing_docs)]
//! suspect-cli: the `suspect` binary. Thin `main.rs` delegates here so every
//! command is a testable library function taking plain arguments and
//! returning an [`Exit`] (0 clean, 1 findings at/above Error, 2 usage, 130
//! interrupted).

pub mod args;
/// Git-ref baselines for comparisons.
pub mod baseline;
pub mod bundle;
pub mod commands;
pub mod diff;
pub mod exit;
pub mod interrupt;
pub mod output;
pub mod reverse;
/// SARIF 2.1.0 serialization for CI code-scanning integration.
pub mod sarif;
pub mod settings;
/// FD-level output silencing for machine-readable runs.
pub mod silence;
pub mod upgrade;
pub mod watch;

use std::path::PathBuf;

pub use args::{Findings, FindingsFormat, Report, ReportFormat, Stream, StreamFormat};
use clap::{Parser, Subcommand, ValueEnum};
pub use exit::Exit;
pub use output::{Finding, Severity};
use suspect_config::{Layer, Raw};
use suspect_source::{Source, Uri};

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
        report: Report,
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
        findings: Findings,
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
        findings: Findings,
    },
    /// Apply, explain, or synthesize (`diff`) Overlay 1.0/1.1 documents.
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
        report: Report,
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
        report: Report,
    },
    /// Cross-reference recorded traffic against the spec: endpoints the
    /// traffic exercised that the spec does not declare, undeclared query
    /// parameters, and declared operations the traffic never touched.
    Drift {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Traffic file: a Suspect Cassette (gateway `--mode record`) or a
        /// Suspect Journal (gateway `--journal`); records are recognized
        /// line by line.
        traffic: PathBuf,
        /// Query parameter names to ignore in gap analysis (repeatable);
        /// the CI-passed auth token lives here, for instance.
        #[arg(long = "ignore-param", value_name = "NAME")]
        ignore_param: Vec<String>,
        /// Exit with status 1 when the traffic addresses undeclared
        /// endpoints or parameters.
        #[arg(long)]
        exit_on_gap: bool,
        /// Output format for the drift report.
        #[command(flatten)]
        findings: Findings,
    },
    /// Export recorded traffic as an HTTP Archive (HAR 1.2) — the
    /// interchange format profiling and debugging tools read. Accepts a
    /// Suspect Cassette or a Suspect Journal.
    Har {
        /// Traffic file: a Suspect Cassette or Suspect Journal.
        traffic: PathBuf,
        /// Write the archive here instead of stdout.
        #[arg(long, short = 'o', value_name = "FILE")]
        output: Option<PathBuf>,
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
        report: Report,
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
        /// `text` for human lines, `ndjson` (or `json`) for one JSON tick
        /// record per line.
        #[command(flatten)]
        stream: Stream,
    },
    /// Extract server framework routes and cross-reference them with the
    /// spec: undocumented endpoints, spec-only endpoints, method drift.
    Reverse {
        /// Server source tree or single file with route registrations.
        source: PathBuf,
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// `text` for a human report, `json` for a structured report.
        #[command(flatten)]
        report: Report,
    },
    /// Generate and run stateful dependency-graph test sequences against a
    /// live server.
    Stateful {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Base URL prepended to operation paths (default: `test.base_url`
        /// from `.suspect.yaml`, `SUSPECT_BASE_URL`, else
        /// `http://localhost:8080`).
        #[arg(long)]
        base_url: Option<String>,
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
        /// Base URL prepended to operation paths (default: `test.base_url`
        /// from `.suspect.yaml`, `SUSPECT_BASE_URL`, else
        /// `http://localhost:8080`).
        #[arg(long)]
        base_url: Option<String>,
        /// Run only workflows whose id contains this substring.
        #[arg(long)]
        filter: Option<String>,
        /// Provide a workflow input: `--input name=value` (repeatable).
        /// The value is parsed as JSON when it parses, else taken as a
        /// string; applies to every workflow that declares the input.
        #[arg(long = "input", value_name = "NAME=VALUE")]
        inputs: Vec<String>,
        /// Run offline against a recorded Suspect Cassette instead of live HTTP.
        #[arg(long, requires = "offline")]
        cassette: Option<PathBuf>,
        /// Enables offline mode (requires --cassette).
        #[arg(long)]
        offline: bool,
        /// Event stream format: human text or one-JSON-per-line ndjson
        /// (default: `test.report`, else `text`).
        #[arg(long, value_enum, alias = "report")]
        format: Option<StreamFormat>,
        /// Message broker directory for Arazzo 1.1 AsyncAPI steps:
        /// `inbox.jsonl` supplies pre-recorded messages and `outbox.jsonl`
        /// collects what the workflow published.
        #[arg(long, value_name = "DIR")]
        message_broker: Option<PathBuf>,
        /// Credentials file for the security schemes the workflows
        /// exercise. Default: `.suspect/credentials.json` discovered by
        /// walking up from the Arazzo document; `test.credentials` and
        /// `SUSPECT_CREDENTIALS` override it.
        #[arg(long, value_name = "FILE")]
        credentials: Option<PathBuf>,
        /// Write the journal to this file (append) instead of stdout.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
        /// JSON or YAML object file of workflow inputs, applied before `--input`
        /// pairs (which override it).
        #[arg(long = "inputs", value_name = "FILE")]
        inputs_file: Option<PathBuf>,
    },
    /// Run Arazzo workflows as weighted virtual-user scenarios under a phased
    /// load profile, with latency/throughput metrics and SLO gates.
    Load(Box<commands::load::LoadArgs>),
    /// Fuzz operations with schema-mutating requests against a live server.
    Fuzz {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Base URL prepended to operation paths (default: `test.base_url`
        /// from `.suspect.yaml`, `SUSPECT_BASE_URL`, else
        /// `http://localhost:8080`).
        #[arg(long)]
        base_url: Option<String>,
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
        report: Report,
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
        /// Custom generation manifest: `gen.toml`, `gen.yaml` or `gen.json`
        /// (templates resolved relative to it).
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
        /// TCP port to bind on 127.0.0.1 (`0` picks a free port). The bound
        /// address is printed on stderr once the gateway can serve.
        /// Default: `gateway.port`, else 8080.
        #[arg(long, short = 'p')]
        port: Option<u16>,
        /// Operating mode: mock | proxy | validate | record | replay |
        /// scenario. Default: `gateway.mode`, else `mock`.
        #[arg(long)]
        mode: Option<String>,
        /// Scenario mode: JSON file of scripted steps
        /// (`{"steps": [{"method", "path_suffix", "status", "body"}]}`).
        #[arg(long, value_name = "FILE")]
        scenario: Option<PathBuf>,
        /// Upstream base URL for proxy/validate/record modes (default:
        /// `gateway.upstream`).
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
        report: Report,
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
    /// Show the configuration in effect, or with `--explain` every setting
    /// and where its value came from.
    Config(commands::config_cmd::ConfigArgs),
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

/// Parses a severity floor named in configuration or the environment
/// (`error`, `warning`/`warn`, `info`/`information`, `hint`); `origin`
/// names where it came from in the error.
///
/// # Errors
/// An unrecognized name, or `off` (a floor cannot be switched off).
pub fn parse_severity_floor(name: &str, origin: &str) -> anyhow::Result<output::Severity> {
    match suspect_config::parse_severity(name) {
        Some(parsed) => output::Severity::from_config(parsed).ok_or_else(|| {
            anyhow::anyhow!("{origin}: severity `off` is not a valid minimum severity")
        }),
        None => Err(anyhow::anyhow!(
            "{origin}: unrecognized severity `{name}` (expected error, warning, info, or hint)"
        )),
    }
}

/// Top-level CLI shape (`suspect <command> ...`); output-format flags such
/// as `--format` belong to each subcommand (`suspect lint --format json`).
#[derive(Debug, Parser)]
#[command(name = "suspect", version, about = "OpenAPI/Arazzo/Overlay toolkit")]
pub struct Cli {
    /// The subcommand to run; see [`Command`] for the available operations.
    #[command(subcommand)]
    pub command: Command,
}

/// Dispatches a parsed CLI invocation, returning the process status.
///
/// # Errors
/// Propagates unexpected IO/model failures; the binary prints them and exits 2.
pub fn execute(cli: Cli) -> anyhow::Result<Exit> {
    dispatch(cli).map(Exit::from_legacy)
}

fn dispatch(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Command::Check { paths, report } => commands::check::check(&paths, report.format),
        Command::Validate {
            paths,
            reference_allowlist,
            strict_format,
            findings,
        } => {
            // A clap bool cannot say "explicitly false": only a passed flag
            // enters the flag layer.
            let flags = Layer::flags()
                .set(
                    "validate.strict_format",
                    strict_format.then_some(Raw::Bool(true)),
                    "--strict-format",
                )
                .set(
                    "validate.reference_allowlist",
                    reference_allowlist.map(Raw::Path),
                    "--reference-allowlist",
                );
            let resolved = settings::resolve(&paths, flags)?;
            let validate = &resolved.settings.validate;
            commands::validate::validate(
                &paths,
                findings.format,
                validate.reference_allowlist.as_deref(),
                validate.strict_format,
            )
        }
        Command::Admission(args) => commands::admission::admission(&args),
        Command::Breaking(args) => commands::breaking::breaking(&args),
        Command::DocsGen(mut args) => {
            let style = args
                .style
                .and_then(|s| ValueEnum::to_possible_value(&s))
                .map(|v| Raw::Str(v.get_name().to_owned()));
            let flags = Layer::flags().set("docs.style", style, "--style").set(
                "docs.output",
                args.output.take().map(Raw::Path),
                "--output",
            );
            let resolved = settings::resolve_one(&args.input, flags)?;
            let docs = &resolved.settings.docs;
            // The table restricts `docs.style` to the enum's names.
            args.style = docs
                .style
                .as_deref()
                .and_then(|style| commands::docs_gen_cmd::DocsStyle::from_str(style, false).ok());
            args.output.clone_from(&docs.output);
            commands::docs_gen_cmd::docs_gen(&args)
        }
        Command::Stubs(args) => commands::stubs::stubs(&args),
        Command::ArazzoDiff(args) => commands::arazzo_diff::arazzo_diff(&args),
        Command::OverlayDryRun(args) => commands::overlay_dry_run::overlay_dry_run(&args),
        Command::Upgrade(args) => commands::upgrade::upgrade(&args),
        Command::Rules { cmd } => cmd.run(),
        Command::Lint {
            paths,
            ruleset,
            min_severity,
            findings,
        } => {
            // flag > env > config > default, decided by `resolve`.
            let flags = Layer::flags()
                .set("lint.ruleset", ruleset.map(Raw::Path), "--ruleset")
                .set(
                    "lint.min_severity",
                    min_severity.map(|s| Raw::Str(s.label().to_owned())),
                    "--min-severity",
                );
            let resolved = settings::resolve(&paths, flags)?;
            let lint = &resolved.settings.lint;
            // Committed policy: design-class severity and per-rule
            // overrides reach the CLI the same way they reach the editor.
            let overrides = commands::lint::LintOverrides::from_settings(lint);
            commands::lint::lint(
                &paths,
                lint.ruleset.as_deref(),
                commands::lint::floor(lint),
                &overrides,
                findings.format,
            )
        }
        Command::Overlay { cmd } => commands::overlay::run(cmd),
        Command::Fmt {
            input,
            output,
            json,
            yaml,
        } => {
            // A typed --json/--yaml flag wins over both settings; otherwise
            // the file decides.
            let resolved = settings::resolve_one(&input, Layer::flags())?;
            let (json, yaml) = if json || yaml {
                (json, yaml)
            } else {
                (resolved.settings.format.json, resolved.settings.format.yaml)
            };
            commands::fmt::fmt(&input, output.as_deref(), json, yaml)
        }
        Command::Stats { path, report } => commands::stats::stats(&path, report.format),
        Command::Bundle {
            input,
            output,
            strategy,
            out_format,
        } => bundle::bundle(&input, output.as_deref(), strategy, out_format),
        Command::Diff { a, b, report } => diff::diff_files(&a, &b, report.format),
        Command::Drift {
            spec,
            traffic,
            ignore_param,
            exit_on_gap,
            findings,
        } => commands::drift::drift(&spec, &traffic, &ignore_param, exit_on_gap, findings.format),
        Command::Har { traffic, output } => commands::har::har(&traffic, output.as_deref()),
        Command::Bench {
            fixture,
            iters,
            report,
        } => commands::bench::bench(&fixture, iters, report.format),
        Command::Load(args) => commands::load::load(*args),
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
            &test_settings(&spec, base_url, None, None)?.base_url,
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
            stream,
        } => {
            interrupt::server();
            commands::bridge::bridge(&spec, interval_ms, max_ticks, stream.format.is_ndjson())
        }
        Command::Reverse {
            source,
            spec,
            report,
        } => commands::reverse::reverse(&source, &spec, report.format.is_json()),
        Command::Stateful {
            spec,
            base_url,
            filter,
            emit,
        } => commands::stateful::stateful(
            &spec,
            &test_settings(&spec, base_url, None, None)?.base_url,
            filter.as_deref(),
            emit,
        ),
        Command::Auth {
            cmd: commands::auth::AuthCmd::Check { credentials },
        } => {
            let flags = Layer::flags().set(
                "test.credentials",
                credentials.map(Raw::Path),
                "--credentials",
            );
            let resolved = settings::resolve(&[], flags)?;
            commands::auth::check(resolved.settings.test.credentials.as_deref())
        }
        Command::Test {
            arazzo,
            base_url,
            filter,
            inputs,
            cassette,
            offline: _,
            format,
            message_broker,
            credentials,
            journal,
            inputs_file,
        } => {
            let test = test_settings(&arazzo, base_url, credentials, format)?;
            commands::test::test_with_messages(
                &arazzo,
                &test.base_url,
                filter.as_deref(),
                &commands::inputs::parse_inputs(inputs_file.as_deref(), &inputs)?,
                cassette.as_deref(),
                test.report == "ndjson",
                message_broker.as_deref(),
                test.credentials.as_deref(),
                journal.as_deref(),
            )
        }
        Command::Why {
            constraint,
            spec,
            offset,
            why_line,
            why_col,
            cassette_dir,
            report,
        } => commands::why::why(
            &constraint,
            &spec,
            offset,
            why_line,
            why_col,
            cassette_dir.as_deref(),
            report.format.is_json(),
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
        } => {
            let flags = Layer::flags()
                .set(
                    "gateway.port",
                    port.map(|p| Raw::Int(u64::from(p))),
                    "--port",
                )
                .set("gateway.mode", mode.map(Raw::Str), "--mode")
                .set(
                    "gateway.upstream",
                    upstream.map(|u| Raw::Str(u.display().to_string())),
                    "--upstream",
                );
            let gateway = settings::resolve_one(&spec, flags)?.settings.gateway;
            let upstream = gateway.upstream.map(PathBuf::from);
            commands::gateway::gateway(
                &spec,
                gateway.port,
                &gateway.mode,
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
            )
        }
        Command::Watch { roots, command } => {
            interrupt::server();
            commands::watch::watch(&roots, &command)
        }
        Command::Acquire(args) => commands::acquire_cmd::acquire(args),
        Command::Codegen(args) => commands::codegen_cmd::codegen(args),
        Command::CodegenTerraform(args) => commands::terraform_cmd::generate(args),
        Command::CodegenCli(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Cli, args)
        }
        Command::CodegenMcp(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Mcp, args)
        }
        Command::CodegenProfiles { kind, report } => match kind {
            commands::sdk_profiles::ProfileKind::Sdk => commands::sdk_profiles::list(report.format),
            commands::sdk_profiles::ProfileKind::Applications => {
                commands::sdk_profiles::list_applications(report.format)
            }
        },
        Command::CodegenSession(args) => {
            if args.watch {
                interrupt::server();
            }
            commands::codegen_session::generate(args)
        }
        Command::CodegenCompare(args) => commands::codegen_compare::compare(args),
        Command::Config(args) => commands::config_cmd::config(&args),
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
            interrupt::server();
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(suspect_lsp::run_server());
            Ok(0)
        }
    }
}

/// The `test` settings for a run on `input`, with the flags that set
/// them on top.
fn test_settings(
    input: &std::path::Path,
    base_url: Option<String>,
    credentials: Option<PathBuf>,
    format: Option<StreamFormat>,
) -> anyhow::Result<suspect_config::TestSettings> {
    let format = format.map(|f| Raw::Str(f.name().to_owned()));
    let flags = Layer::flags()
        .set("test.base_url", base_url.map(Raw::Str), "--base-url")
        .set(
            "test.credentials",
            credentials.map(Raw::Path),
            "--credentials",
        )
        .set("test.report", format, "--format");
    Ok(settings::resolve_one(input, flags)?.settings.test)
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
