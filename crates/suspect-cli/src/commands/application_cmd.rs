//! Application targets - a Go/Cobra API CLI and a TypeScript stdio MCP server
//! - each backed by its own embedded canonical generated SDK.
//!
//! Both commands share one flow: read the closed mapping and target
//! configuration, open the file or verified pinned input, plan the explicitly
//! mapped surface against the canonical contract, emit the complete artifact
//! set, then classify that set against the output root's own ownership
//! registry. Each output root has its own stable owner identity, so neither
//! application can adopt, overwrite or prune the other's artifacts.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};
use suspect_codegen::{
    OutFile, api_cli,
    generation_session::{Input, SessionError},
    mcp,
};
use suspect_ir::contract::Contract;

use crate::OutputFormat;

/// Ownership identity of one Go/Cobra API CLI application root.
pub const CLI_OWNER: &str = "suspect-cli-app:lifecycle-v1";

/// Ownership identity of one TypeScript stdio MCP server application root.
pub const MCP_OWNER: &str = "suspect-mcp-app:lifecycle-v1";

/// The two application targets, each with its own profiles, diagnostic codes
/// and output-root owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Go/Cobra command-line application.
    Cli,
    /// TypeScript stdio MCP server.
    Mcp,
}

impl Target {
    /// Subcommand name, also the diagnostic-code and report discriminator.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::Cli => "codegen-cli",
            Self::Mcp => "codegen-mcp",
        }
    }

    /// The exact mapping profile this target accepts.
    #[must_use]
    pub const fn profile(self) -> &'static str {
        match self {
            Self::Cli => api_cli::PROFILE,
            Self::Mcp => mcp::PROFILE,
        }
    }

    /// The exact application surface manifest format this target emits.
    #[must_use]
    pub const fn surface_format(self) -> &'static str {
        match self {
            Self::Cli => api_cli::SURFACE_FORMAT,
            Self::Mcp => mcp::SURFACE_FORMAT,
        }
    }

    /// Stable ownership identity of this target's output root.
    #[must_use]
    pub const fn owner(self) -> &'static str {
        match self {
            Self::Cli => CLI_OWNER,
            Self::Mcp => MCP_OWNER,
        }
    }

    /// Versioned generation report format.
    #[must_use]
    const fn report_format(self) -> &'static str {
        match self {
            Self::Cli => "suspect.application.cli.generation.v1",
            Self::Mcp => "suspect.application.mcp.generation.v1",
        }
    }

    /// Prefix for the codes this command layer itself produces - input,
    /// artifact and ownership refusals - distinct per target so one report
    /// never mixes two targets' command-layer refusals under one code.
    ///
    /// It does not namespace the codes this layer only forwards: a generator's
    /// own `cli-*` / `mcp-*` refusals already carry their target, and the
    /// shared application family is deliberately target-neutral because both
    /// targets resolve selectors, bind the resolved operation and relativize
    /// manifest documents through the same shared code. That family has
    /// exactly four members - `application-operation-missing`,
    /// `application-operation-ambiguous`,
    /// [`suspect_codegen::application::OPERATION_UNPLANNED`] and
    /// [`suspect_codegen::application::DOCUMENT_OUTSIDE_ENTRY_TREE`].
    const fn slug(self) -> &'static str {
        match self {
            Self::Cli => "application-cli",
            Self::Mcp => "application-mcp",
        }
    }

    fn code(self, reason: &str) -> String {
        format!("{}-{reason}", self.slug())
    }
}

/// Generate one self-contained application root from an explicit mapping.
#[derive(Debug, Args)]
pub struct ApplicationArgs {
    /// Entry OpenAPI document providing every API operation and schema.
    #[arg(required_unless_present = "pins", conflicts_with = "pins")]
    pub spec: Option<PathBuf>,
    /// Immutable pin manifest; generation reads its verified cache only.
    #[arg(long)]
    pub pins: Option<PathBuf>,
    /// Cache populated by `suspect acquire` for --pins input.
    #[arg(long, requires = "pins")]
    pub cache_dir: Option<PathBuf>,
    /// Numeric-loopback origin from an explicitly acquired test manifest.
    #[arg(long, requires = "pins")]
    pub insecure_test_origin: Vec<String>,
    /// Closed, versioned application surface mapping JSON.
    #[arg(long)]
    pub mapping: PathBuf,
    /// Package/toolchain identity, credential policy and runtime bounds JSON.
    #[arg(long)]
    pub target_config: PathBuf,
    /// Output root owning the application and its embedded generated SDK
    /// [default: cli-out for codegen-cli, mcp-out for codegen-mcp].
    #[arg(short, long)]
    pub out: Option<PathBuf>,
    /// Read-only ownership/drift check.
    #[arg(long)]
    pub check: bool,
    /// Text output or one structured generation report.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

impl ApplicationArgs {
    /// The output root, defaulting per target so the two never collide.
    fn root(&self, target: Target) -> PathBuf {
        self.out.clone().unwrap_or_else(|| {
            PathBuf::from(match target {
                Target::Cli => "cli-out",
                Target::Mcp => "mcp-out",
            })
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostic {
    file: String,
    pointer: String,
    mapping_pointer: String,
    line: usize,
    col: usize,
    range: Option<std::ops::Range<usize>>,
    code: String,
    message: String,
}

impl Diagnostic {
    fn file(path: &Path, code: String, error: impl ToString) -> Self {
        Self {
            file: path.display().to_string(),
            pointer: String::new(),
            mapping_pointer: String::new(),
            line: 1,
            col: 1,
            range: None,
            code,
            message: error.to_string(),
        }
    }

    fn json(path: &Path, code: String, error: serde_json::Error) -> Self {
        let mut finding = Self::file(path, code, &error);
        finding.line = error.line();
        finding.col = error.column();
        finding
    }

    fn input(target: Target, path: &Path, error: SessionError) -> Self {
        match error {
            SessionError::Acquisition(error) => Self {
                file: error.manifest_path().display().to_string(),
                pointer: error
                    .resource_index()
                    .map_or_else(String::new, |index| format!("/resources/{index}")),
                mapping_pointer: String::new(),
                line: error.line().unwrap_or(1),
                col: 1,
                range: None,
                code: error.code().into(),
                message: error.to_string(),
            },
            error => Self::file(path, target.code("input"), error),
        }
    }
}

/// The parsed, target-specific mapping and configuration pair. Both are read
/// and validated before any input document is opened, so a malformed mapping
/// never reaches the network, the pin cache or the filesystem output root.
enum Mapping {
    Cli(Box<(api_cli::MappingProfile, api_cli::CliTargetConfig)>),
    Mcp(Box<(mcp::MappingProfile, mcp::McpTargetConfig)>),
}

fn parse_inputs(target: Target, args: &ApplicationArgs) -> Result<Mapping, Vec<Diagnostic>> {
    let mapping = std::fs::read_to_string(&args.mapping).map_err(|error| {
        vec![Diagnostic::file(
            &args.mapping,
            target.code("mapping-input"),
            error,
        )]
    })?;
    let config = std::fs::read(&args.target_config).map_err(|error| {
        vec![Diagnostic::file(
            &args.target_config,
            target.code("target-input"),
            error,
        )]
    })?;
    let mapping_error = |error| {
        vec![Diagnostic::json(
            &args.mapping,
            target.code("mapping-input"),
            error,
        )]
    };
    let config_error = |error| {
        vec![Diagnostic::json(
            &args.target_config,
            target.code("target-input"),
            error,
        )]
    };
    Ok(match target {
        Target::Cli => Mapping::Cli(Box::new((
            api_cli::parse_mapping(&mapping).map_err(mapping_error)?,
            serde_json::from_slice(&config).map_err(config_error)?,
        ))),
        Target::Mcp => Mapping::Mcp(Box::new((
            mcp::parse_mapping(&mapping).map_err(mapping_error)?,
            serde_json::from_slice(&config).map_err(config_error)?,
        ))),
    })
}

struct Prepared {
    files: Vec<OutFile>,
    source: String,
    operations: Vec<Value>,
}

fn prepare(target: Target, args: &ApplicationArgs) -> Result<Prepared, Vec<Diagnostic>> {
    let mapping = parse_inputs(target, args)?;
    let input = match (&args.spec, &args.pins) {
        (Some(path), None) => Input::File { path: path.clone() },
        (None, Some(manifest)) => Input::Pinned {
            manifest: manifest.clone(),
            cache_dir: args
                .cache_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(".suspect-cache")),
            insecure_test_origins: args.insecure_test_origin.clone(),
        },
        _ => {
            return Err(vec![Diagnostic::file(
                &args.mapping,
                target.code("input"),
                "choose an OpenAPI input or --pins manifest",
            )]);
        }
    };
    let input = input.normalized().map_err(|error| {
        vec![Diagnostic::file(
            args.spec
                .as_deref()
                .or(args.pins.as_deref())
                .unwrap_or(&args.mapping),
            target.code("input"),
            error,
        )]
    })?;
    let (workspace, entry) = input
        .open()
        .map_err(|error| vec![Diagnostic::input(target, input.path(), error)])?;
    let contract = Arc::new(
        Contract::from_workspace(&workspace, &entry)
            .map_err(|error| vec![Diagnostic::file(input.path(), target.code("input"), error)])?,
    );
    // Planning refusals address the canonical contract; resolve each one to a
    // line and column in the document that actually declares it.
    let locate = |errors: Vec<suspect_codegen::application::Diagnostic>| {
        errors
            .into_iter()
            .map(|error| {
                let (line, col) =
                    workspace
                        .get(error.source.document())
                        .map_or((0, 0), |document| {
                            let doc = document.doc();
                            doc.inner()
                                .line_index()
                                .line_col(doc.inner().bytes(), error.at.start)
                        });
                Diagnostic {
                    file: error.source.document().to_string(),
                    pointer: error.source.pointer().into(),
                    mapping_pointer: error.mapping_pointer,
                    line: line as usize + 1,
                    col: col as usize + 1,
                    range: Some(error.at),
                    code: error.code.into(),
                    message: error.message,
                }
            })
            .collect::<Vec<_>>()
    };
    let (files, operations) = match mapping {
        Mapping::Cli(inputs) => {
            let (mapping, config) = *inputs;
            let plan = api_cli::plan_cli(contract, mapping, config).map_err(locate)?;
            let operations = plan.sdk_plan().operations().iter().map(|operation| json!({
                "operationId":operation.operation_id, "nativeMethod":operation.method_name,
                "document":operation.source.document().as_str(), "pointer":operation.source.pointer(),
            })).collect();
            (api_cli::emit_cli(&plan), operations)
        }
        Mapping::Mcp(inputs) => {
            let (mapping, config) = *inputs;
            let plan = mcp::plan_server(contract, mapping, config).map_err(locate)?;
            let operations = plan.sdk_plan().operations().iter().map(|operation| json!({
                "operationId":operation.operation_id, "nativeMethod":operation.function_name,
                "document":operation.source.document().as_str(), "pointer":operation.source.pointer(),
            })).collect();
            (mcp::emit_server(&plan), operations)
        }
    };
    Ok(Prepared {
        files,
        source: entry.to_string(),
        operations,
    })
}

/// Emit/check one complete application root under its own stable owner.
///
/// Emission and the ownership classification both complete before any write,
/// so a planning or ownership refusal leaves the output root exactly as it
/// was. `--check` never writes.
///
/// # Errors
/// Propagates report serialization failures. Source, mapping, package,
/// ownership and drift findings return a structured report and exit status 1.
pub fn generate(target: Target, args: ApplicationArgs) -> anyhow::Result<i32> {
    let out = args.root(target);
    let mut status = "failed";
    let mut artifacts = Vec::<String>::new();
    let mut operations = Vec::new();
    let mut source = None;
    let mut diagnostics = Vec::new();
    match prepare(target, &args) {
        Err(findings) => diagnostics = findings,
        Ok(prepared) => {
            source = Some(prepared.source);
            operations = prepared.operations;
            artifacts = prepared
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect();
            match suspect_codegen::check_files_with_owner(&prepared.files, &out, target.owner()) {
                Err(error) => {
                    diagnostics.push(Diagnostic::file(&out, target.code("artifacts"), error))
                }
                Ok(ownership) => {
                    for conflict in ownership.conflicts() {
                        diagnostics.push(Diagnostic::file(
                            &out.join(&conflict.path),
                            target.code("artifact-conflict"),
                            conflict
                                .conflict
                                .as_deref()
                                .unwrap_or("artifact ownership conflict"),
                        ));
                    }
                    if !diagnostics.is_empty() {
                        status = "conflict";
                    } else if ownership.is_current() {
                        status = "current";
                    } else if args.check {
                        status = "drift";
                    } else {
                        match suspect_codegen::write_files_with_owner(
                            &prepared.files,
                            &out,
                            target.owner(),
                            suspect_codegen::Adoption::Refuse,
                        ) {
                            Ok(()) => status = "generated",
                            Err(error) => diagnostics.push(Diagnostic::file(
                                &out,
                                target.code("artifacts"),
                                error,
                            )),
                        }
                    }
                }
            }
        }
    }
    diagnostics.sort_by(|left, right| {
        (&left.file, &left.pointer, &left.mapping_pointer, &left.code).cmp(&(
            &right.file,
            &right.pointer,
            &right.mapping_pointer,
            &right.code,
        ))
    });
    match args.format {
        OutputFormat::Json | OutputFormat::Sarif => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "format":target.report_format(), "profile":target.profile(),
                "surfaceFormat":target.surface_format(), "owner":target.owner(),
                "status":status, "source":source, "mapping":args.mapping,
                "targetConfig":args.target_config, "out":out,
                "operations":operations, "artifacts":artifacts, "diagnostics":diagnostics,
            }))?
        ),
        OutputFormat::Text => {
            for diagnostic in &diagnostics {
                println!(
                    "{}:{}:{} [{}] {} ({}; mapping {})",
                    diagnostic.file,
                    diagnostic.line,
                    diagnostic.col,
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic.pointer,
                    diagnostic.mapping_pointer
                );
            }
            println!(
                "{} {status}: {} mapped SDK operations, {} planned artifacts ({})",
                target.command(),
                operations.len(),
                artifacts.len(),
                target.profile()
            );
        }
    }
    Ok(i32::from(!matches!(status, "generated" | "current")))
}
