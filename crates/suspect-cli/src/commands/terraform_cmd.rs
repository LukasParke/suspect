//! Terraform lifecycle artifacts backed by the canonical generated Go SDK.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    generation_session::{Input, SessionError},
    terraform,
};
use suspect_ir::contract::Contract;

use crate::OutputFormat;

const OWNER: &str = "suspect-terraform:lifecycle-v1";

/// Generate a separate provider artifact from an explicit lifecycle mapping.
#[derive(Debug, Args)]
pub struct TerraformArgs {
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
    /// Closed, versioned Terraform lifecycle/state mapping JSON.
    #[arg(long)]
    pub mapping: PathBuf,
    /// Provider/toolchain identity and pinned generated-Go-SDK dependency JSON.
    #[arg(long)]
    pub target_config: PathBuf,
    /// Output root containing go/ and the dependent terraform/ artifacts.
    #[arg(short, long, default_value = "terraform-out")]
    pub out: PathBuf,
    /// Read-only ownership/drift check.
    #[arg(long)]
    pub check: bool,
    /// Text output or one structured generation report.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
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
    fn file(path: &Path, code: &str, error: impl ToString) -> Self {
        Self {
            file: path.display().to_string(),
            pointer: String::new(),
            mapping_pointer: String::new(),
            line: 1,
            col: 1,
            range: None,
            code: code.into(),
            message: error.to_string(),
        }
    }

    fn json(path: &Path, code: &str, error: serde_json::Error) -> Self {
        let mut finding = Self::file(path, code, &error);
        finding.line = error.line();
        finding.col = error.column();
        finding
    }

    fn input(path: &Path, error: SessionError) -> Self {
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
            error => Self::file(path, "terraform-input", error),
        }
    }
}

struct Prepared {
    files: Vec<OutFile>,
    source: String,
    operations: Vec<Value>,
    dependency: Value,
}

fn prepare(args: &TerraformArgs) -> Result<Prepared, Vec<Diagnostic>> {
    let mapping = std::fs::read_to_string(&args.mapping).map_err(|error| {
        vec![Diagnostic::file(
            &args.mapping,
            "terraform-mapping-input",
            error,
        )]
    })?;
    let mapping = terraform::parse_mapping(&mapping).map_err(|error| {
        vec![Diagnostic::json(
            &args.mapping,
            "terraform-mapping-input",
            error,
        )]
    })?;
    let config = std::fs::read(&args.target_config).map_err(|error| {
        vec![Diagnostic::file(
            &args.target_config,
            "terraform-target-input",
            error,
        )]
    })?;
    let config: terraform::TargetConfig = serde_json::from_slice(&config).map_err(|error| {
        vec![Diagnostic::json(
            &args.target_config,
            "terraform-target-input",
            error,
        )]
    })?;
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
                "terraform-input",
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
            "terraform-input",
            error,
        )]
    })?;
    let (workspace, entry) = input
        .open()
        .map_err(|error| vec![Diagnostic::input(input.path(), error)])?;
    let contract = Arc::new(
        Contract::from_workspace(&workspace, &entry)
            .map_err(|error| vec![Diagnostic::file(input.path(), "terraform-input", error)])?,
    );
    let plan = terraform::plan_provider(contract, mapping, config).map_err(|errors| {
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
    })?;
    let operations = plan.sdk_plan().operations().iter().map(|operation| json!({
        "operationId":operation.operation_id, "nativeMethod":operation.method_name,
        "document":operation.source.document().as_str(), "pointer":operation.source.pointer(),
    })).collect();
    Ok(Prepared {
        files: terraform::emit_provider(&plan),
        source: entry.to_string(),
        operations,
        dependency: json!(plan.config().sdk),
    })
}

/// Emit/check a complete provider plus its exact generated SDK dependency.
///
/// # Errors
/// Propagates report serialization failures. Source, mapping, package, ownership
/// and drift findings return a structured report and exit status 1.
pub fn generate(args: TerraformArgs) -> anyhow::Result<i32> {
    let mut status = "failed";
    let mut artifacts = Vec::<String>::new();
    let mut operations = Vec::new();
    let mut source = None;
    let mut dependency = Value::Null;
    let mut diagnostics = Vec::new();
    match prepare(&args) {
        Err(findings) => diagnostics = findings,
        Ok(prepared) => {
            source = Some(prepared.source);
            dependency = prepared.dependency;
            operations = prepared.operations;
            artifacts = prepared
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect();
            match suspect_codegen::check_files_with_owner(&prepared.files, &args.out, OWNER) {
                Err(error) => {
                    diagnostics.push(Diagnostic::file(&args.out, "terraform-artifacts", error))
                }
                Ok(ownership) => {
                    for conflict in ownership.conflicts() {
                        diagnostics.push(Diagnostic::file(
                            &args.out.join(&conflict.path),
                            "terraform-artifact-conflict",
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
                            &args.out,
                            OWNER,
                            suspect_codegen::Adoption::Refuse,
                        ) {
                            Ok(()) => status = "generated",
                            Err(error) => diagnostics.push(Diagnostic::file(
                                &args.out,
                                "terraform-artifacts",
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
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "format":"suspect.terraform.generation.v1", "profile":terraform::PROFILE,
                "status":status, "source":source, "mapping":args.mapping,
                "targetConfig":args.target_config, "sdkDependency":dependency,
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
                "Terraform {status}: {} mapped SDK operations, {} planned artifacts ({})",
                operations.len(),
                artifacts.len(),
                terraform::PROFILE
            );
        }
    }
    Ok(i32::from(!matches!(status, "generated" | "current")))
}
