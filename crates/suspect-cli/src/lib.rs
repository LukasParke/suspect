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
pub mod legacy;
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

pub use args::{
    Check, DocOut, Findings, FindingsFormat, Report, ReportFiles, ReportFormat, Stream,
    StreamFormat,
};
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
/// Related commands live in groups (`diff`, `gen`, `test`, `traffic`,
/// `release`, `dev`); old flat names are rewritten by [`legacy`].
#[derive(Debug, Subcommand)]
pub enum Command {
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
    /// Re-emit a document in canonical JSON/YAML form.
    Fmt {
        /// Input document.
        input: PathBuf,
        /// Output file and serialization.
        #[command(flatten)]
        out: DocOut,
        /// Old spelling of `--to json`.
        #[arg(long, hide = true, conflicts_with_all = ["yaml", "to"])]
        json: bool,
        /// Old spelling of `--to yaml`.
        #[arg(long, hide = true, conflicts_with = "to")]
        yaml: bool,
    },
    /// Bundle a document and its `$ref` closure into one file.
    Bundle {
        /// Entry document.
        input: PathBuf,
        /// Bundling strategy.
        #[arg(long, value_enum, default_value = "keep")]
        strategy: Strategy,
        /// Output file and serialization (`--to` applies to inline only).
        #[command(flatten)]
        out: DocOut,
    },
    /// Upgrade Swagger 2.0 documents to OpenAPI 3.1.
    Upgrade(commands::upgrade::UpgradeArgs),
    /// Structural counts for a document.
    Stats {
        /// Input document.
        path: PathBuf,
        /// Output format for the counts table.
        #[command(flatten)]
        report: Report,
    },
    /// Apply, explain, or synthesize (`diff`) Overlay 1.0/1.1 documents.
    Overlay {
        /// The overlay subcommand to run.
        #[command(subcommand)]
        cmd: commands::overlay::OverlayCmd,
    },
    /// Compare two documents or revisions: structure, breaks, impact, SDKs, routes.
    #[command(subcommand)]
    Diff(DiffCmd),
    /// Generate docs, SDKs, stubs, contracts, and applications from a spec.
    #[command(subcommand)]
    Gen(GenCmd),
    /// Run workflows, stateful sequences, fuzzing, load, or replay against a server.
    #[command(subcommand)]
    Test(TestCmd),
    /// Serve, record, and analyze HTTP traffic.
    #[command(subcommand)]
    Traffic(TrafficCmd),
    /// Plan, publish, and automate SDK releases.
    #[command(subcommand)]
    Release(ReleaseCmd),
    /// Build and check a suspect project from one manifest.
    #[command(name = "project")]
    Project {
        /// The project subcommand to run.
        #[command(subcommand)]
        cmd: commands::project::ProjectCmd,
    },
    /// Run validate, contract, breaking, codegen, and tests across every
    /// project in a workspace as one gate.
    Ci(commands::ci::CiArgs),
    /// Trace a validation failure to its origin: source location, git
    /// blame, and whether recorded traffic ever passed it.
    Why {
        /// Spec file the failure refers to (with the old `--spec` flag,
        /// this positional is the constraint).
        #[arg(value_name = "SPEC")]
        first: String,
        /// The constraint that fired (e.g. `type` or the diagnostic text).
        #[arg(value_name = "CONSTRAINT", required_unless_present = "spec")]
        constraint: Option<String>,
        /// Old spelling: `why CONSTRAINT --spec SPEC`.
        #[arg(long, hide = true, conflicts_with = "constraint")]
        spec: Option<PathBuf>,
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
    /// Run TS/JS custom rules (Bun sidecar) over documents.
    Rules {
        /// The rules subcommand to run.
        #[command(subcommand)]
        cmd: commands::rules::RulesCmd,
    },
    /// Show the configuration in effect, or with `--explain` every setting
    /// and where its value came from.
    Config(commands::config_cmd::ConfigArgs),
    /// Manage and verify credentials for the testing flows.
    Auth {
        /// The auth subcommand to run.
        #[command(subcommand)]
        cmd: commands::auth::AuthCmd,
    },
    /// Acquire a hash-pinned document closure or verify its offline cache.
    Acquire(commands::acquire_cmd::AcquireArgs),
    /// Re-run a command whenever documents change under the given roots.
    Watch {
        /// Directories (or files) watched recursively for yaml/yml/json changes.
        #[arg(required = true)]
        roots: Vec<PathBuf>,
        /// Command (with arguments) to run and re-run on each change burst.
        #[arg(last = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Run the language server over stdio.
    Lsp,
    /// Developer tools: pipeline benchmarks and the SDK evidence matrix.
    #[command(subcommand)]
    Dev(DevCmd),
}

/// `suspect diff`: compare two documents or revisions.
#[derive(Debug, Subcommand)]
pub enum DiffCmd {
    /// Semantic structural diff between two documents.
    Structural {
        /// Left-hand document.
        a: PathBuf,
        /// Right-hand document.
        b: PathBuf,
        /// Output format for the difference report.
        #[command(flatten)]
        report: Report,
        /// `--check`: exit 1 when anything differs.
        #[command(flatten)]
        check: Check,
    },
    /// Detect consumer-breaking changes between two OpenAPI or Arazzo
    /// revisions (the family of NEW picks the differ).
    Breaking(commands::breaking::BreakingArgs),
    /// Evaluate recorded traffic against a candidate contract revision.
    Impact(commands::impact::ImpactArgs),
    /// Compare wire contracts and native SDK interfaces with migration notes.
    Sdk(commands::codegen_compare::CompareArgs),
    /// Extract server framework routes and cross-reference them with the
    /// spec: undocumented endpoints, spec-only endpoints, method drift.
    Routes {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Server source tree or single file with route registrations.
        source: PathBuf,
        /// Old positional order (`reverse SOURCE SPEC`): swap the two.
        #[arg(long, hide = true)]
        source_first: bool,
        /// `text` for a human report, `json` for a structured report.
        #[command(flatten)]
        report: Report,
        /// `--check`: exit 1 on undocumented or spec-only routes.
        #[command(flatten)]
        check: Check,
    },
}

/// `suspect gen`: generate artifacts from a spec.
#[derive(Debug, Subcommand)]
pub enum GenCmd {
    /// Generate API reference docs (HTML, Markdown, or a SvelteKit site).
    Docs(commands::docs_gen_cmd::DocsGenArgs),
    /// Generate a source-selected native SDK package.
    Sdk(commands::codegen_cmd::CodegenArgs),
    /// Generate typed server stubs from an OpenAPI document.
    Stubs(commands::stubs::StubsArgs),
    /// Emit or verify a machine-readable contract package.
    Contract(commands::contract::ContractArgs),
    /// Generate a Terraform provider through explicit lifecycle mappings and the generated Go SDK.
    Terraform(commands::terraform_cmd::TerraformArgs),
    /// Generate a Go/Cobra API CLI application over its own embedded generated Go SDK.
    Cli(commands::application_cmd::ApplicationArgs),
    /// Generate a TypeScript stdio MCP server over its own embedded generated TypeScript SDK.
    Mcp(commands::application_cmd::ApplicationArgs),
    /// Render custom template manifests (or the `docs-md` starter preset)
    /// from an OpenAPI document.
    Template {
        /// Entry OpenAPI document.
        spec: PathBuf,
        /// Shipped preset: `docs-md`, a starting point for custom
        /// templates (reference docs: `suspect gen docs`).
        #[arg(long, conflicts_with = "manifest", value_parser=["docs-md"])]
        preset: Option<String>,
        /// Custom generation manifest: `gen.toml`, `gen.yaml` or `gen.json`
        /// (templates resolved relative to it).
        #[arg(long, conflicts_with = "preset")]
        manifest: Option<PathBuf>,
        /// Output root directory.
        #[arg(
            short = 'o',
            long = "output",
            visible_alias = "out",
            value_name = "DIR",
            default_value = "gen-out"
        )]
        out: PathBuf,
        /// Write nothing; exit 1 when output or ownership drifts.
        #[command(flatten)]
        check: Check,
        /// Also print unified diffs of drifted files; implies `--check`.
        #[arg(long)]
        diff: bool,
        /// Stable logical owner when sharing an output root across generation streams.
        #[arg(long)]
        owner: Option<String>,
        /// Explicitly adopt only byte-identical unowned output.
        #[arg(long)]
        adopt_identical: bool,
    },
    /// Persistent canonical SDK generation with readonly preview and watch mode.
    Session(commands::codegen_session::SessionArgs),
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
    /// List the native SDK or application profiles available in this CLI build.
    Profiles {
        /// Profile family: native SDK backends, or application targets.
        #[arg(long, value_enum, default_value_t = commands::sdk_profiles::ProfileKind::Sdk)]
        kind: commands::sdk_profiles::ProfileKind,
        /// Human-readable inventory or versioned JSON for editor integration.
        #[command(flatten)]
        report: Report,
    },
    /// Run the generation admission layer over documents without generating.
    Admission(commands::admission::AdmissionArgs),
}

/// `suspect test`: exercise a live (or recorded) server.
#[derive(Debug, Subcommand)]
pub enum TestCmd {
    /// Compile an Arazzo document into an executable suite and run it.
    Run {
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
        #[arg(long)]
        cassette: Option<PathBuf>,
        /// No effect: `--cassette` alone runs offline. Accepted so old
        /// invocations keep working.
        #[arg(long, hide = true)]
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
        #[arg(long = "inputs-file", value_name = "FILE", alias = "inputs")]
        inputs_file: Option<PathBuf>,
        /// JSON and JUnit report files.
        #[command(flatten)]
        report_files: ReportFiles,
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
        /// JSON and JUnit report files.
        #[command(flatten)]
        report_files: ReportFiles,
    },
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
        /// JSON and JUnit report files.
        #[command(flatten)]
        report_files: ReportFiles,
    },
    /// Run Arazzo workflows as weighted virtual-user scenarios under a phased
    /// load profile, with latency/throughput metrics and SLO gates.
    Load(Box<commands::load::LoadArgs>),
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
        /// JSON and JUnit report files.
        #[command(flatten)]
        report_files: ReportFiles,
    },
}

/// `suspect traffic`: serve, record, and analyze HTTP traffic.
#[derive(Debug, Subcommand)]
pub enum TrafficCmd {
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
        /// Operating mode. Default: `gateway.mode`, else `mock`.
        #[arg(long, value_enum)]
        mode: Option<commands::gateway::GatewayMode>,
        /// Scenario mode: JSON file of scripted steps
        /// (`{"steps": [{"method", "path_suffix", "status", "body"}]}`).
        #[arg(long, value_name = "FILE")]
        scenario: Option<PathBuf>,
        /// Upstream base URL for proxy/validate/record modes (default:
        /// `gateway.upstream`).
        #[arg(long, value_name = "URL")]
        upstream: Option<String>,
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
        /// endpoints or parameters (old spelling: `--exit-on-gap`).
        #[arg(long, alias = "exit-on-gap")]
        check: bool,
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
}

/// `suspect release`: SDK release planning and publishing.
#[derive(Debug, Subcommand)]
pub enum ReleaseCmd {
    /// Recommend a release version and changelog from two spec revisions.
    Plan(commands::release::ReleasePlanArgs),
    /// Plan and execute SDK publishing across registries.
    Publish(commands::publish::PublishArgs),
    /// Render the release manifest as a tag-triggered CI workflow.
    Workflow(commands::publish::WorkflowArgs),
}

/// `suspect dev`: tools for developing suspect itself.
#[derive(Debug, Subcommand)]
pub enum DevCmd {
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
    /// Print the SDK verification matrix: backend x feature -> evidence.
    Evidence(commands::evidence::EvidenceArgs),
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
        Command::Fmt {
            input,
            out,
            json,
            yaml,
        } => {
            // A typed --to (or --json/--yaml) wins over both settings; otherwise
            // the file decides.
            let resolved = settings::resolve_one(&input, Layer::flags())?;
            let json = json || out.to == Some(DocFormat::Json);
            let yaml = yaml || out.to == Some(DocFormat::Yaml);
            let (json, yaml) = if json || yaml {
                (json, yaml)
            } else {
                (resolved.settings.format.json, resolved.settings.format.yaml)
            };
            commands::fmt::fmt(&input, out.output.as_deref(), json, yaml)
        }
        Command::Bundle {
            input,
            strategy,
            out,
        } => bundle::bundle(&input, out.output.as_deref(), strategy, out.to),
        Command::Upgrade(args) => commands::upgrade::upgrade(&args),
        Command::Stats { path, report } => commands::stats::stats(&path, report.format),
        Command::Overlay { cmd } => commands::overlay::run(cmd),
        Command::Diff(cmd) => diff_cmd(cmd),
        Command::Gen(cmd) => gen_cmd(cmd),
        Command::Test(cmd) => test_cmd(cmd),
        Command::Traffic(cmd) => traffic_cmd(cmd),
        Command::Release(cmd) => match cmd {
            ReleaseCmd::Plan(args) => commands::release::release_plan(&args),
            ReleaseCmd::Publish(args) => commands::publish::publish(&args),
            ReleaseCmd::Workflow(args) => commands::publish::workflow(&args),
        },
        Command::Project { cmd } => commands::project::run(cmd),
        Command::Ci(args) => {
            // The gate reports its own aggregate, so a failing stage is a
            // reported verdict rather than a raised error.
            commands::ci::ci(&args)
        }
        Command::Why {
            first,
            constraint,
            spec,
            offset,
            why_line,
            why_col,
            cassette_dir,
            report,
        } => {
            // `why SPEC CONSTRAINT`, or the old `why CONSTRAINT --spec SPEC`.
            let (spec, constraint) = match (spec, constraint) {
                (Some(spec), _) => (spec, first),
                (None, Some(constraint)) => (PathBuf::from(first), constraint),
                (None, None) => anyhow::bail!("usage: suspect why <SPEC> <CONSTRAINT>"),
            };
            commands::why::why(
                &constraint,
                &spec,
                offset,
                why_line,
                why_col,
                cassette_dir.as_deref(),
                report.format.is_json(),
            )
        }
        Command::Rules { cmd } => cmd.run(),
        Command::Config(args) => commands::config_cmd::config(&args),
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
        Command::Acquire(args) => commands::acquire_cmd::acquire(args),
        Command::Watch { roots, command } => {
            interrupt::server();
            commands::watch::watch(&roots, &command)
        }
        Command::Lsp => {
            interrupt::server();
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(suspect_lsp::run_server());
            Ok(0)
        }
        Command::Dev(cmd) => match cmd {
            DevCmd::Bench {
                fixture,
                iters,
                report,
            } => commands::bench::bench(&fixture, iters, report.format),
            DevCmd::Evidence(args) => commands::evidence::evidence(&args),
        },
    }
}

fn diff_cmd(cmd: DiffCmd) -> anyhow::Result<i32> {
    match cmd {
        DiffCmd::Structural {
            a,
            b,
            report,
            check,
        } => diff::diff_files(&a, &b, report.format, check.check),
        DiffCmd::Breaking(args) => commands::breaking::breaking(&args),
        DiffCmd::Impact(args) => commands::impact::impact(&args),
        DiffCmd::Sdk(args) => commands::codegen_compare::compare(args),
        DiffCmd::Routes {
            spec,
            source,
            source_first,
            report,
            check,
        } => {
            let (spec, source) = if source_first {
                (source, spec)
            } else {
                (spec, source)
            };
            commands::reverse::reverse(&source, &spec, report.format.is_json(), check.check)
        }
    }
}

fn gen_cmd(cmd: GenCmd) -> anyhow::Result<i32> {
    match cmd {
        GenCmd::Docs(mut args) => {
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
        GenCmd::Sdk(args) => commands::codegen_cmd::codegen(args),
        GenCmd::Stubs(args) => commands::stubs::stubs(&args),
        GenCmd::Contract(args) => commands::contract::contract(&args),
        GenCmd::Terraform(args) => commands::terraform_cmd::generate(args),
        GenCmd::Cli(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Cli, args)
        }
        GenCmd::Mcp(args) => {
            commands::application_cmd::generate(commands::application_cmd::Target::Mcp, args)
        }
        GenCmd::Template {
            spec,
            preset,
            manifest,
            out,
            check,
            diff,
            owner,
            adopt_identical,
        } => commands::generate::generate(
            &spec,
            preset.as_deref(),
            manifest.as_deref(),
            &out,
            commands::generate::Gate {
                check: check.check || diff,
                diff,
            },
            owner.as_deref(),
            adopt_identical,
        ),
        GenCmd::Session(args) => {
            if args.watch {
                interrupt::server();
            }
            commands::codegen_session::generate(args)
        }
        GenCmd::Bridge {
            spec,
            interval_ms,
            max_ticks,
            stream,
        } => {
            interrupt::server();
            commands::bridge::bridge(&spec, interval_ms, max_ticks, stream.format.is_ndjson())
        }
        GenCmd::Profiles { kind, report } => match kind {
            commands::sdk_profiles::ProfileKind::Sdk => commands::sdk_profiles::list(report.format),
            commands::sdk_profiles::ProfileKind::Applications => {
                commands::sdk_profiles::list_applications(report.format)
            }
        },
        GenCmd::Admission(args) => commands::admission::admission(&args),
    }
}

fn test_cmd(cmd: TestCmd) -> anyhow::Result<i32> {
    match cmd {
        TestCmd::Run {
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
            report_files,
        } => {
            let test = test_settings(&arazzo, base_url, credentials, format)?;
            commands::test::test_with_reports(
                &arazzo,
                &test.base_url,
                filter.as_deref(),
                &commands::inputs::parse_inputs(inputs_file.as_deref(), &inputs)?,
                cassette.as_deref(),
                test.report == "ndjson",
                message_broker.as_deref(),
                test.credentials.as_deref(),
                journal.as_deref(),
                &report_files,
            )
        }
        TestCmd::Stateful {
            spec,
            base_url,
            filter,
            emit,
            report_files,
        } => commands::stateful::stateful(
            &spec,
            &test_settings(&spec, base_url, None, None)?.base_url,
            filter.as_deref(),
            emit,
            &report_files,
        ),
        TestCmd::Fuzz {
            spec,
            base_url,
            runs,
            filter,
            journal,
            evolved,
            rounds,
            per_round,
            report_files,
        } => commands::fuzz::fuzz(
            &spec,
            &test_settings(&spec, base_url, None, None)?.base_url,
            runs,
            filter.as_deref(),
            journal.as_deref(),
            evolved,
            rounds,
            per_round,
            &report_files,
        ),
        TestCmd::Load(args) => commands::load::load(*args),
        TestCmd::Replay {
            cassette,
            upstream,
            diff,
            journal,
            report_files,
        } => commands::replay::replay(
            &cassette,
            &upstream,
            diff,
            journal.as_deref(),
            &report_files,
        ),
    }
}

fn traffic_cmd(cmd: TrafficCmd) -> anyhow::Result<i32> {
    match cmd {
        TrafficCmd::Gateway {
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
                .set(
                    "gateway.mode",
                    mode.map(|m| Raw::Str(m.name().to_owned())),
                    "--mode",
                )
                .set("gateway.upstream", upstream.map(Raw::Str), "--upstream");
            let gateway = settings::resolve_one(&spec, flags)?.settings.gateway;
            commands::gateway::gateway(
                &spec,
                gateway.port,
                &gateway.mode,
                gateway.upstream.as_deref(),
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
        TrafficCmd::Drift {
            spec,
            traffic,
            ignore_param,
            check,
            findings,
        } => commands::drift::drift(&spec, &traffic, &ignore_param, check, findings.format),
        TrafficCmd::Har { traffic, output } => commands::har::har(&traffic, output.as_deref()),
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
