//! Admission pre-flight: the shared generation admission verdict for a
//! document, without generating anything.
//!
//! Runs the same source-level admission the backends run before emitting
//! artifacts — shared HTTP admission, incoming webhook/callback admission,
//! contract limitations, and cross-convention naming analysis — and prints
//! the report. Text output lists located findings plus an operations
//! summary; JSON output is machine consumable for CI gates.

use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;

use crate::commands::validate::validate_file;
use crate::output::{self, Finding, Severity};
use crate::{OutputFormat, TextFormat};

/// One admission review input.
#[derive(Debug, clap::Args)]
pub struct AdmissionArgs {
    /// OpenAPI documents to review.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    /// Output format for the report.
    #[command(flatten)]
    pub text: TextFormat,
}

/// Reviews one document through the admission layer. Input and syntax
/// failures surface as error findings; documents that do not compile into
/// a contract surface one finding describing why.
#[must_use]
pub fn admission_file(path: &std::path::Path) -> Vec<Finding> {
    let mut findings = validate_file(path, None);
    if findings.iter().any(|f| f.severity == Severity::Error) {
        // Syntax and semantic validation errors would corrupt the contract;
        // admission runs only on documents validation accepts.
        findings.push(Finding {
            file: path.display().to_string(),
            severity: Severity::Error,
            code: "admission-input".into(),
            message: "admission review skipped: fix the validation errors above".into(),
            line: 1,
            col: 1,
            range: None,
        });
        return findings;
    }
    match review_loaded(path) {
        Ok(review_findings) => findings.extend(review_findings),
        Err(message) => findings.push(Finding {
            file: path.display().to_string(),
            severity: Severity::Error,
            code: "admission-input".into(),
            message,
            line: 1,
            col: 1,
            range: None,
        }),
    }
    findings
}

/// Builds the contract and runs the admission review, mapping findings to
/// the shared output shape with located line/column information.
fn review_loaded(path: &std::path::Path) -> Result<Vec<Finding>, String> {
    let absolute = path.canonicalize().map_err(|e| e.to_string())?;
    let entry = absolute.to_str().ok_or("path is not UTF-8")?;
    let workspace = Arc::new(WorkspaceBuilder::new().build().map_err(|e| e.to_string())?);
    workspace.open(entry).map_err(|e| e.to_string())?;
    let uri = suspect_source::Uri::parse(entry).map_err(|e| e.to_string())?;
    let contract = Contract::from_workspace(&workspace, &uri).map_err(|e| e.to_string())?;
    let entry_uri = uri.clone();
    let report = suspect_codegen::admission::review(&contract);

    let mut findings = Vec::new();
    for f in &report.findings {
        let uri = f
            .source
            .as_ref()
            .map_or_else(|| contract.entry().as_str(), |s| s.document().as_str());
        let owner = workspace
            .get(&suspect_source::Uri::parse(uri).map_err(|e| e.to_string())?)
            .ok_or("finding document is not loaded")?;
        let doc = owner.doc();
        let (line, col) = doc
            .inner()
            .line_index()
            .line_col(doc.inner().bytes(), f.at.start);
        let severity = match f.kind {
            suspect_codegen::admission::FindingKind::Refusal => Severity::Error,
            suspect_codegen::admission::FindingKind::Advice => Severity::Info,
            suspect_codegen::admission::FindingKind::Contract => Severity::Warning,
        };
        let mut message = if f.message.is_empty() {
            f.summary.to_owned()
        } else {
            f.message.clone()
        };
        if !f.how_to_fix.is_empty() {
            message.push_str("\nhow to fix: ");
            message.push_str(f.how_to_fix);
        }
        findings.push(Finding {
            file: if owner.id()
                == workspace
                    .get(&entry_uri)
                    .ok_or("entry document is not loaded")?
                    .id()
            {
                path.display().to_string()
            } else {
                uri.to_owned()
            },
            severity,
            code: f.code.into(),
            message,
            line: line + 1,
            col: col + 1,
            range: Some(f.at.clone()),
        });
    }
    Ok(findings)
}

/// Runs admission review over every document and prints the report; exits
/// non-zero when any refusal-class finding exists.
///
/// # Errors
/// Propagates output serialization failures.
pub fn admission(args: &AdmissionArgs) -> anyhow::Result<i32> {
    let mut findings: Vec<Finding> = args
        .paths
        .par_iter()
        .flat_map(|p| admission_file(p))
        .collect();
    findings.sort_by(|a, b| {
        (&a.file, a.line, a.col, &a.code, &a.message)
            .cmp(&(&b.file, b.line, b.col, &b.code, &b.message))
    });
    match args.text.format {
        OutputFormat::Text => output::print_findings(&findings),
        OutputFormat::Json => output::print_json(&findings)?,
    }
    Ok(i32::from(
        findings.iter().any(|f| f.severity == Severity::Error),
    ))
}
