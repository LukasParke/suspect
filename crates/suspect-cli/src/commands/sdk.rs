//! Canonical SDK planning and ownership-aware output through the public CLI.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use clap::ValueEnum;
use serde::Serialize;
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig},
    generation_session::{Input, SessionError},
    http_protocol::CompatibilityProfile,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::Workspace;

use crate::{OutputFormat, TextFormat};

/// An explicit experimental canonical generation capability.
#[derive(Debug, Clone, Copy)]
pub struct SdkProfile(Backend);

impl ValueEnum for SdkProfile {
    fn value_variants<'a>() -> &'a [Self] {
        const PROFILES: [SdkProfile; Backend::ALL.len()] = {
            let mut profiles = [SdkProfile(Backend::TypescriptHttp); Backend::ALL.len()];
            let mut index = 0;
            while index < profiles.len() {
                profiles[index] = SdkProfile(Backend::ALL[index]);
                index += 1;
            }
            profiles
        };
        &PROFILES
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.0.name()).help(self.0.description()))
    }
}

impl SdkProfile {
    fn name(self) -> &'static str {
        self.0.name()
    }
    fn owner(self) -> &'static str {
        self.0.owner()
    }
}

/// Closed, explicitly versioned source-interpretation choice.
#[derive(Debug, Clone, Copy)]
pub struct SdkCompatibilityProfile(pub(super) CompatibilityProfile);

impl ValueEnum for SdkCompatibilityProfile {
    fn value_variants<'a>() -> &'a [Self] {
        const PROFILES: [SdkCompatibilityProfile; CompatibilityProfile::ALL.len()] = {
            let mut values = [SdkCompatibilityProfile(CompatibilityProfile::LegacyBinaryStringV1);
                CompatibilityProfile::ALL.len()];
            let mut index = 0;
            while index < values.len() {
                values[index] = SdkCompatibilityProfile(CompatibilityProfile::ALL[index]);
                index += 1;
            }
            values
        };
        &PROFILES
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.0.name()))
    }
}

/// Options for source-selected canonical SDK generation.
#[derive(Debug)]
pub(super) struct SdkArgs {
    /// Entry OpenAPI document. Only its semantic reference closure is loaded.
    pub input: Input,
    /// Implemented generation profile; unsupported contracts fail before output.
    pub profile: SdkProfile,
    /// Exact operationId to select (repeatable). Omit to select all outgoing operations.
    pub operation_id: Vec<String>,
    /// Portable native package name, independent of the OpenAPI API title.
    pub package_name: String,
    /// Exact package SemVer, independent of the OpenAPI API version.
    pub package_version: String,
    /// Explicit native import, module or namespace identity where applicable.
    pub import_name: Option<String>,
    pub generation: GenerationOptions,
    /// Root for the selected native package and ownership manifest.
    pub out: PathBuf,
    /// Plan and check ownership/drift without writing; exit 1 for findings or drift.
    pub check: bool,
    /// Human-readable output or one structured experimental report.
    pub text: TextFormat,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    format: &'static str,
    profile: &'static str,
    compatibility_profiles: BTreeSet<CompatibilityProfile>,
    release_ready: bool,
    status: &'static str,
    operations: Vec<SelectedOperation>,
    artifacts: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedOperation {
    operation_id: Option<String>,
    method: String,
    path: Option<String>,
    document: String,
    pointer: String,
}

#[derive(Serialize)]
struct Diagnostic {
    file: String,
    pointer: String,
    line: u32,
    col: u32,
    range: Option<std::ops::Range<usize>>,
    code: String,
    message: String,
}

impl Diagnostic {
    fn located(
        ws: &Workspace,
        contract: &Contract,
        source: &SourceId,
        code: &str,
        message: String,
    ) -> Self {
        let range = contract.source_span(source);
        let (line, col) = ws.get(source.document()).map_or((0, 0), |owner| {
            let doc = owner.doc();
            let offset = range.as_ref().map_or(0, |range| range.start);
            doc.inner()
                .line_index()
                .line_col(doc.inner().bytes(), offset)
        });
        Self {
            file: source.document().to_string(),
            pointer: source.pointer().into(),
            line: line + 1,
            col: col + 1,
            range,
            code: code.into(),
            message,
        }
    }

    fn input(args: &SdkArgs, code: &str, message: String) -> Self {
        Self {
            file: args.input.path().display().to_string(),
            pointer: String::new(),
            line: 1,
            col: 1,
            range: None,
            code: code.into(),
            message,
        }
    }
}

struct Prepared {
    files: Vec<OutFile>,
    operations: Vec<SelectedOperation>,
}

fn prepare(args: &SdkArgs) -> Result<Prepared, Vec<Diagnostic>> {
    let input_error = |error: String| vec![Diagnostic::input(args, "sdk-input", error)];
    let (ws, uri) = args.input.open().map_err(|error| match error {
        SessionError::Acquisition(error) => vec![Diagnostic {
            file: error.manifest_path().display().to_string(),
            pointer: error
                .resource_index()
                .map_or_else(String::new, |index| format!("/resources/{index}")),
            line: error.line().unwrap_or(1).try_into().unwrap_or(u32::MAX),
            col: 1,
            range: None,
            code: error.code().into(),
            message: error.to_string(),
        }],
        error => input_error(error.to_string()),
    })?;
    let contract =
        Arc::new(Contract::from_workspace(&ws, &uri).map_err(|e| input_error(e.to_string()))?);
    let entry = SourceId::new(uri, Default::default());
    let available: Vec<_> = contract.operations().collect();
    let mut by_name: BTreeMap<&str, Vec<SourceId>> = BTreeMap::new();
    for operation in &available {
        if let Some(name) = operation.operation_id() {
            by_name
                .entry(name)
                .or_default()
                .push(operation.source().clone());
        }
    }
    let mut errors = Vec::new();
    let mut selected = BTreeSet::new();
    if args.operation_id.is_empty() {
        selected.extend(available.iter().map(|o| o.source().clone()));
    } else {
        for name in &args.operation_id {
            match by_name.get(name.as_str()) {
                Some(sources) if sources.len() == 1 => {
                    selected.insert(sources[0].clone());
                }
                Some(sources) => {
                    for source in sources {
                        errors.push(Diagnostic::located(
                            &ws,
                            &contract,
                            &source.child("operationId"),
                            "sdk-operation-ambiguous",
                            format!(
                                "operationId {name:?} identifies more than one outgoing operation"
                            ),
                        ));
                    }
                }
                None => errors.push(Diagnostic::located(
                    &ws,
                    &contract,
                    &entry.child("paths"),
                    "sdk-operation-not-found",
                    format!("no outgoing operation has operationId {name:?}"),
                )),
            }
        }
    }
    if selected.is_empty() && errors.is_empty() {
        errors.push(Diagnostic::located(
            &ws,
            &contract,
            &entry.child("paths"),
            "sdk-no-operations",
            "this HTTP profile requires at least one outgoing operation".into(),
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let selected: Vec<_> = selected.into_iter().collect();
    let backend = args.profile.0;
    let files = suspect_codegen::backend::generate_with_options(
        contract.clone(),
        &selected,
        &TargetConfig {
            backend,
            package_name: args.package_name.clone(),
            package_version: args.package_version.clone(),
            import_name: args.import_name.clone(),
        },
        &args.generation,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| {
                if let Some(source) = error.source {
                    Diagnostic::located(&ws, &contract, &source, error.code, error.message)
                } else {
                    Diagnostic::input(args, error.code, error.message)
                }
            })
            .collect::<Vec<_>>()
    })?;
    let operations = available
        .iter()
        .filter(|operation| selected.contains(operation.source()))
        .map(|operation| SelectedOperation {
            operation_id: operation.operation_id().map(str::to_owned),
            method: operation.method().as_str().to_owned(),
            path: operation.path_template().map(str::to_owned),
            document: operation.source().document().to_string(),
            pointer: operation.source().pointer().into(),
        })
        .collect();
    Ok(Prepared { files, operations })
}

/// Plans every selected operation before checking or writing the complete owned artifact set.
///
/// This command never invokes a package manager or publishes artifacts. Success
/// describes the selected experimental profile, not full-document validation or
/// a release-ready SDK. Planning/input/ownership failures and drift exit 1.
///
/// # Errors
/// Propagates report serialization failures.
pub(super) fn generate(args: &SdkArgs) -> anyhow::Result<i32> {
    let mut report = Report {
        format: "suspect.sdk.experimental.v1",
        profile: args.profile.name(),
        compatibility_profiles: args.generation.compatibility_profiles.clone(),
        release_ready: false,
        status: "failed",
        operations: Vec::new(),
        artifacts: Vec::new(),
        diagnostics: Vec::new(),
    };
    match prepare(args) {
        Err(diagnostics) => report.diagnostics = diagnostics,
        Ok(prepared) => {
            report.operations = prepared.operations;
            report.artifacts = prepared
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect();
            match suspect_codegen::check_files_with_owner(
                &prepared.files,
                &args.out,
                args.profile.owner(),
            ) {
                Err(error) => {
                    report
                        .diagnostics
                        .push(Diagnostic::input(args, "sdk-artifacts", error))
                }
                Ok(ownership) => {
                    for conflict in ownership.conflicts() {
                        report.diagnostics.push(Diagnostic {
                            file: args.out.join(&conflict.path).display().to_string(),
                            pointer: String::new(),
                            line: 1,
                            col: 1,
                            range: None,
                            code: "sdk-artifact-conflict".into(),
                            message: conflict
                                .conflict
                                .clone()
                                .unwrap_or_else(|| "artifact ownership conflict".into()),
                        });
                    }
                    if !report.diagnostics.is_empty() {
                        report.status = "conflict";
                    } else if ownership.is_current() {
                        report.status = "current";
                    } else if args.check {
                        report.status = "drift";
                    } else {
                        match suspect_codegen::write_files_with_owner(
                            &prepared.files,
                            &args.out,
                            args.profile.owner(),
                            suspect_codegen::Adoption::Refuse,
                        ) {
                            Ok(()) => report.status = "generated",
                            Err(error) => report.diagnostics.push(Diagnostic::input(
                                args,
                                "sdk-artifacts",
                                error,
                            )),
                        }
                    }
                }
            }
        }
    }
    report.diagnostics.sort_by(|a, b| {
        (&a.file, a.line, a.col, &a.pointer, &a.code, &a.message)
            .cmp(&(&b.file, b.line, b.col, &b.pointer, &b.code, &b.message))
    });
    report.diagnostics.dedup_by(|a, b| {
        a.file == b.file && a.pointer == b.pointer && a.code == b.code && a.message == b.message
    });
    match args.text.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Text => {
            for diagnostic in &report.diagnostics {
                println!(
                    "{}:{}:{} [{}] {} ({})",
                    diagnostic.file,
                    diagnostic.line,
                    diagnostic.col,
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic.pointer
                );
            }
            println!(
                "SDK {}: {} selected operations, {} planned artifacts (experimental {}; release-ready: false)",
                report.status,
                report.operations.len(),
                report.artifacts.len(),
                report.profile
            );
        }
    }
    Ok(i32::from(!matches!(report.status, "generated" | "current")))
}
