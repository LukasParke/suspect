//! `suspect check`: parse documents, report family, syntax errors, `$ref`
//! edges, cycle census, and workspace stats. Files are processed in parallel
//! and merged in deterministic path order.

use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;
use suspect_low::SpecFamily;
use suspect_ref::{CycleKind, WorkspaceBuilder};

use crate::OutputFormat;
use crate::output::{self, Finding, Severity};

/// Per-file check result.
#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    /// Path as given on the command line.
    pub path: String,
    /// Display label of the detected spec family (OpenAPI/Arazzo/Overlay/unknown).
    pub family: String,
    /// Number of syntax errors found while parsing the document.
    pub syntax_errors: usize,
    /// Number of `$ref` edges leaving this document.
    pub ref_edges: usize,
    /// Cycles in the reference graph that are legal (e.g. schema recursion).
    pub legal_cycles: usize,
    /// Cycles in the reference graph that are illegal (unresolvable expansion).
    pub illegal_cycles: usize,
    /// Total documents in the loaded workspace closure.
    pub workspace_docs: usize,
    /// Total `$ref` edges across the loaded workspace closure.
    pub workspace_edges: usize,
    /// Problems found in this file, including IO/parse failures as Error findings.
    pub findings: Vec<Finding>,
}

/// Maps a spec family to its display label.
#[must_use]
pub fn family_label(family: SpecFamily) -> &'static str {
    match family {
        SpecFamily::Oas2 => "Oas2",
        SpecFamily::Oas30 => "Oas30",
        SpecFamily::Oas31 => "Oas31",
        SpecFamily::Oas32 => "Oas32",
        SpecFamily::Arazzo10 => "Arazzo10",
        SpecFamily::Overlay10 => "Overlay10",
        SpecFamily::Unknown => "Unknown",
    }
}

/// Checks one file: parse, census, workspace stats. Never fails — problems
/// become Error findings so a broken file cannot abort the batch.
pub fn check_file(path: &Path) -> FileReport {
    let shown = path.display().to_string();
    let mut report = FileReport {
        path: shown.clone(),
        family: "Unknown".into(),
        syntax_errors: 0,
        ref_edges: 0,
        legal_cycles: 0,
        illegal_cycles: 0,
        workspace_docs: 0,
        workspace_edges: 0,
        findings: Vec::new(),
    };
    let io = |report: &mut FileReport, msg: String| {
        report.findings.push(Finding {
            file: shown.clone(),
            severity: Severity::Error,
            code: "io-error".into(),
            message: msg,
            line: 1,
            col: 1,
        });
    };

    let Ok(source) = suspect_source::Source::from_path(path) else {
        io(&mut report, format!("cannot read {}", path.display()));
        return report;
    };
    let Ok(uri) = suspect_source::Uri::from_path(path) else {
        io(
            &mut report,
            format!("cannot canonicalize {}", path.display()),
        );
        return report;
    };
    let doc = suspect_low::LowDoc::parse(uri, source);
    report.family = family_label(doc.sniff_family()).into();

    let bytes = doc.inner().bytes();
    let index = doc.inner().line_index();
    report.syntax_errors = doc.syntax_errors().len();
    for err in doc.syntax_errors() {
        let (line, col) = index.line_col(bytes, err.range.start);
        report.findings.push(Finding {
            file: shown.clone(),
            severity: Severity::Error,
            code: "syntax-error".into(),
            message: err.message.clone(),
            line,
            col: col + 1,
        });
    }

    // Workspace load: edges, cycle census, aggregate stats. Failures become
    // findings rather than errors so the remaining files still report.
    match WorkspaceBuilder::new().build() {
        Ok(ws) => match ws.open(&shown) {
            Ok(handle) => {
                report.ref_edges = handle.edges().len();
                let census = handle.cycles();
                report.legal_cycles = census
                    .cycles
                    .iter()
                    .filter(|c| c.kind == CycleKind::LegalRecursion)
                    .count();
                report.illegal_cycles = census
                    .cycles
                    .iter()
                    .filter(|c| c.kind == CycleKind::Illegal)
                    .count();
                let stats = ws.stats();
                report.workspace_docs = stats.docs;
                report.workspace_edges = stats.edges;
            }
            Err(e) => io(&mut report, format!("workspace load failed: {e}")),
        },
        Err(e) => io(&mut report, format!("workspace build failed: {e}")),
    }
    report
}

/// `suspect check <PATH>...`: parallel check, deterministic order, exit 1 on
/// any Error finding.
///
/// # Errors
/// Only propagates JSON serialization failures; per-file problems become findings.
pub fn check(paths: &[std::path::PathBuf], format: OutputFormat) -> anyhow::Result<i32> {
    let mut reports: Vec<FileReport> = paths.par_iter().map(|p| check_file(p)).collect::<Vec<_>>();
    reports.sort_by(|a, b| a.path.cmp(&b.path));

    match format {
        OutputFormat::Text => {
            for r in &reports {
                println!("{}", r.path);
                println!("  family:           {}", r.family);
                println!("  syntax errors:    {}", r.syntax_errors);
                println!("  $ref edges:       {}", r.ref_edges);
                println!(
                    "  cycles:           {} legal, {} illegal",
                    r.legal_cycles, r.illegal_cycles
                );
                println!(
                    "  workspace:        {} docs, {} edges",
                    r.workspace_docs, r.workspace_edges
                );
            }
        }
        OutputFormat::Json => output::print_json(&reports)?,
    }

    let has_error = reports
        .iter()
        .any(|r| r.findings.iter().any(|f| f.severity == Severity::Error));
    Ok(i32::from(has_error))
}
