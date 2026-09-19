//! Source-located OpenAPI validation for local and CI use.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

use crate::OutputFormat;
use crate::output::{self, Finding, Severity};

/// Validates one entry document. Input failures become findings so batch
/// validation can still report problems in the other documents.
/// An explicit document allowlist also applies to the entry itself.
#[must_use]
pub fn validate_file(path: &Path, allowed_documents: Option<&[Uri]>) -> Vec<Finding> {
    match validate_loaded(path, allowed_documents) {
        Ok(findings) => findings,
        Err(message) => vec![Finding {
            file: path.display().to_string(),
            severity: Severity::Error,
            code: "validation-input".into(),
            message,
            line: 1,
            col: 1,
            range: None,
        }],
    }
}

fn validate_loaded(path: &Path, allowed_documents: Option<&[Uri]>) -> Result<Vec<Finding>, String> {
    let absolute = path.canonicalize().map_err(|e| e.to_string())?;
    let entry = absolute.to_str().ok_or("path is not UTF-8")?;
    let mut builder = WorkspaceBuilder::new();
    if let Some(documents) = allowed_documents {
        builder = builder.allowed_documents(documents.iter().cloned());
    }
    let workspace = Arc::new(builder.build().map_err(|e| e.to_string())?);
    let handle = workspace.open(entry).map_err(|e| e.to_string())?;
    let mut findings = syntax_findings(handle.doc(), &path.display().to_string());
    if !findings.is_empty() {
        return Ok(findings);
    }

    let session = Session::new(Arc::clone(&workspace));
    let api = session.open(entry).map_err(|e| e.to_string())?;
    // Materialize only the semantic closure before checking its syntax.
    let _ = api.reference_objects();
    for uri in workspace.uris() {
        let owner = workspace.get(&uri).ok_or("document is not loaded")?;
        if owner.id() != handle.id() {
            findings.extend(syntax_findings(owner.doc(), uri.as_str()));
        }
    }
    if !findings.is_empty() {
        return Ok(findings);
    }
    let diagnostics = suspect_validate::validate_openapi(&api);
    for diagnostic in diagnostics {
        let owner = workspace
            .get(&diagnostic.doc)
            .ok_or("diagnostic document is not loaded")?;
        let doc = owner.doc();
        let (line, col) = doc
            .inner()
            .line_index()
            .line_col(doc.inner().bytes(), diagnostic.range.start);
        findings.push(Finding {
            file: if owner.id() == handle.id() {
                path.display().to_string()
            } else {
                diagnostic.doc.to_string()
            },
            severity: match diagnostic.severity {
                suspect_validate::Severity::Error => Severity::Error,
                suspect_validate::Severity::Warning => Severity::Warning,
                suspect_validate::Severity::Info => Severity::Info,
            },
            code: diagnostic.code.into(),
            message: diagnostic.message,
            line: line + 1,
            col: col + 1,
            range: Some(diagnostic.range),
        });
    }
    Ok(findings)
}

fn syntax_findings(doc: &suspect_low::LowDoc, shown: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    if doc
        .uri()
        .as_path()
        .and_then(|path| path.extension().map(|e| e.eq_ignore_ascii_case("json")))
        .unwrap_or(false)
        && let Err(error) = serde_json::from_slice::<serde::de::IgnoredAny>(doc.inner().bytes())
    {
        let line = error.line().saturating_sub(1) as u32;
        let bytes = doc.inner().bytes();
        let index = doc.inner().line_index();
        let offset = index
            .line_range(bytes, line)
            .map_or(0, |range| range.start + error.column().saturating_sub(1));
        let (line, col) = index.line_col(bytes, offset);
        return vec![Finding {
            file: shown.into(),
            severity: Severity::Error,
            code: "syntax-error".into(),
            message: error.to_string(),
            line: line + 1,
            col: col + 1,
            range: None,
        }];
    }
    for error in doc.syntax_errors() {
        let (line, col) = doc
            .inner()
            .line_index()
            .line_col(doc.inner().bytes(), error.range.start);
        findings.push(Finding {
            file: shown.into(),
            severity: Severity::Error,
            code: "syntax-error".into(),
            message: error.message.clone(),
            line: line + 1,
            col: col + 1,
            range: Some(error.range.clone()),
        });
    }
    findings
}

/// Validates all documents and prints deterministic diagnostics. Returns 1
/// when any error exists and 0 otherwise.
///
/// # Errors
/// Propagates invalid allowlist configuration and output serialization errors;
/// source input failures are findings.
pub fn validate(
    paths: &[PathBuf],
    format: OutputFormat,
    reference_allowlist: Option<&Path>,
) -> anyhow::Result<i32> {
    let allowed = reference_allowlist
        .map(load_reference_allowlist)
        .transpose()?;
    let mut findings: Vec<Finding> = paths
        .par_iter()
        .flat_map(|path| validate_file(path, allowed.as_deref()))
        .collect();
    findings.sort_by(|a, b| {
        (&a.file, a.line, a.col, &a.code, &a.message)
            .cmp(&(&b.file, b.line, b.col, &b.code, &b.message))
    });
    match format {
        OutputFormat::Text => output::print_findings(&findings),
        OutputFormat::Json => output::print_json(&findings)?,
    }
    Ok(i32::from(
        findings.iter().any(|f| f.severity == Severity::Error),
    ))
}

fn load_reference_allowlist(path: &Path) -> anyhow::Result<Vec<Uri>> {
    use anyhow::{Context, ensure};

    const MAX_BYTES: u64 = 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("open reference allowlist {}", path.display()))?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "reference allowlist exceeds 1 MiB"
    );
    let paths: Vec<PathBuf> = serde_json::from_slice(&bytes)
        .context("reference allowlist must be a JSON array of absolute document paths")?;
    ensure!(
        paths.len() <= 10_000,
        "reference allowlist exceeds 10000 documents"
    );
    let mut documents = Vec::with_capacity(paths.len());
    for document in paths {
        ensure!(
            document.is_absolute(),
            "reference allowlist paths must be absolute: {}",
            document.display()
        );
        let canonical = document
            .canonicalize()
            .with_context(|| format!("resolve allowed document {}", document.display()))?;
        ensure!(
            canonical.is_file(),
            "allowed document is not a regular file: {}",
            document.display()
        );
        documents.push(Uri::from_path(&canonical)?);
    }
    documents.sort();
    documents.dedup();
    Ok(documents)
}
