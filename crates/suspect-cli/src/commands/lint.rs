//! `suspect lint`: run spectral-style rules over documents. The linter is
//! built once (built-in or from a ruleset document); findings are mapped
//! into the CLI's unified severity model, sorted by (file, line), and
//! filtered by `--min-severity`.

use std::path::Path;

use rayon::prelude::*;
use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

use crate::output::{self, Finding, Severity};
use crate::OutputFormat;

/// Maps the lint crate's severity into the CLI's unified model.
#[must_use]
pub fn map_severity(s: suspect_lint::Severity) -> Severity {
    match s {
        suspect_lint::Severity::Error => Severity::Error,
        suspect_lint::Severity::Warn => Severity::Warning,
        suspect_lint::Severity::Info => Severity::Info,
        suspect_lint::Severity::Hint | suspect_lint::Severity::Off => Severity::Hint,
    }
}

/// Builds a linter from an optional ruleset file.
///
/// # Errors
/// IO on the ruleset path or a malformed ruleset document.
pub fn build_linter(ruleset: Option<&Path>) -> anyhow::Result<Linter> {
    match ruleset {
        None => Ok(Linter::spectral_default()),
        Some(path) => {
            let source = Source::from_path(path)?;
            let uri = Uri::from_path(path)?;
            let doc = LowDoc::parse(uri, source);
            Linter::from_ruleset(&doc).map_err(|e| anyhow::anyhow!("ruleset error: {e}"))
        }
    }
}

/// Lints one already-parsed document into located findings.
#[must_use]
pub fn lint_doc(linter: &Linter, doc: &LowDoc, shown: &str) -> Vec<Finding> {
    let bytes = doc.inner().bytes();
    let index = doc.inner().line_index();
    linter
        .run(doc)
        .into_iter()
        .map(|f| {
            let (line, col) = index.line_col(bytes, f.range.start);
            Finding {
                file: shown.to_owned(),
                severity: map_severity(f.severity),
                code: f.code.to_string(),
                message: f.message,
                line,
                col: col + 1,
            }
        })
        .collect()
}

/// Computes the filtered, deterministically ordered finding set for
/// `suspect lint` (testable core; no printing).
///
/// # Errors
/// Ruleset loading failures.
pub fn lint_findings(
    paths: &[std::path::PathBuf],
    ruleset: Option<&Path>,
    min_severity: Severity,
) -> anyhow::Result<Vec<Finding>> {
    let linter = build_linter(ruleset)?;
    let mut findings: Vec<Finding> = paths
        .par_iter()
        .map(|p| match crate::load_doc(p) {
            Ok(doc) => lint_doc(&linter, &doc, &p.display().to_string()),
            Err(e) => vec![Finding {
                file: p.display().to_string(),
                severity: Severity::Error,
                code: "io-error".into(),
                message: format!("{e:#}"),
                line: 1,
                col: 1,
            }],
        })
        .collect::<Vec<_>>()
        .concat();
    findings.retain(|f| f.severity >= min_severity);
    findings.sort_by(|a, b| {
        (&*a.file, a.line, a.col, &a.code).cmp(&(&*b.file, b.line, b.col, &b.code))
    });
    Ok(findings)
}

/// `suspect lint <PATH>... [--ruleset FILE] [--min-severity S]`: parallel
/// linting, deterministic (file, line) order; exit 1 when any Error finding
/// survives the min-severity filter.
///
/// # Errors
/// Ruleset loading failures.
pub fn lint(
    paths: &[std::path::PathBuf],
    ruleset: Option<&Path>,
    min_severity: Severity,
    format: OutputFormat,
) -> anyhow::Result<i32> {
    let findings = lint_findings(paths, ruleset, min_severity)?;

    match format {
        OutputFormat::Text => output::print_findings(&findings),
        OutputFormat::Json => output::print_json(&findings)?,
    }

    let has_error = findings.iter().any(|f| f.severity == Severity::Error);
    Ok(i32::from(has_error))
}
