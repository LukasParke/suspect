//! `suspect lint`: run spectral-style rules over documents. The linter is
//! built once (built-in or from a ruleset document); findings are mapped
//! into the CLI's unified severity model, sorted by (file, line), and
//! filtered by `--min-severity`.

use std::path::Path;

use rayon::prelude::*;
use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

use crate::OutputFormat;
use crate::output::{self, Finding, Severity};

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

/// Committed policy applied to raw lint findings before the severity
/// floor, in this order: the design-class severity, then per-rule
/// overrides (a per-rule entry wins over the class), then the floor.
///
/// Design-class findings flag the API's own design rather than the
/// document's accuracy — a project documenting an API it does not own
/// cannot fix them without misdocumenting the API, so the committed
/// policy may silence or downgrade the whole class at once.
#[derive(Debug, Default, Clone)]
pub struct LintOverrides {
    /// Severity for design-class findings, or `None` to keep them as-is.
    pub design: Option<suspect_lint::Severity>,
    /// Per-rule severity overrides; [`suspect_lint::Severity::Off`] drops
    /// the rule. Wins over `design`.
    pub rules: std::collections::BTreeMap<String, suspect_lint::Severity>,
}

impl LintOverrides {
    /// Parses policy names as they appear in committed configuration:
    /// `error`, `warn`/`warning`, `info`/`information`, `hint`, `off`.
    ///
    /// # Errors
    /// Names every unrecognized severity instead of silently ignoring it.
    pub fn from_config(
        design: Option<&str>,
        rules: &std::collections::BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let mut bad: Vec<String> = Vec::new();
        let parse = |name: &str, bad: &mut Vec<String>| {
            let severity = suspect_lint::Severity::from_policy(name);
            if severity.is_none() {
                bad.push(name.to_owned());
            }
            severity
        };
        let design = design
            .filter(|d| !d.is_empty())
            .and_then(|d| parse(d, &mut bad));
        let mut map = std::collections::BTreeMap::new();
        for (code, name) in rules {
            if let Some(severity) = parse(name, &mut bad) {
                map.insert(code.clone(), severity);
            }
        }
        if !bad.is_empty() {
            return Err(anyhow::anyhow!(
                "unrecognized lint severity: {}",
                bad.join(", ")
            ));
        }
        Ok(Self { design, rules: map })
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
                line: line + 1,
                col: col + 1,
                range: Some(f.range),
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
    overrides: &LintOverrides,
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
                range: None,
            }],
        })
        .collect::<Vec<_>>()
        .concat();
    // Committed policy: per-rule overrides first (a specific rule wins over
    // its class), then the design-class severity; `off` drops the finding.
    // The floor filters after, so a downgraded finding obeys the floor too.
    findings.retain_mut(|f| {
        let target = overrides.rules.get(&f.code).copied().or_else(|| {
            (linter.category_of(&f.code) == suspect_lint::Category::Design)
                .then_some(overrides.design)
                .flatten()
        });
        match target {
            Some(suspect_lint::Severity::Off) => false,
            Some(severity) => {
                f.severity = map_severity(severity);
                true
            }
            None => true,
        }
    });
    findings.retain(|f| f.severity >= min_severity);
    findings.sort_by(|a, b| {
        (&*a.file, a.line, a.col, &a.code).cmp(&(&*b.file, b.line, b.col, &b.code))
    });
    Ok(findings)
}

/// `suspect lint <PATH>... [--ruleset FILE] [--min-severity S]`: parallel
/// linting, deterministic (file, line) order; exit 1 when any Error finding
/// survives the committed policy (design-class severity, per-rule
/// overrides) and the min-severity filter.
///
/// # Errors
/// Ruleset loading failures.
pub fn lint(
    paths: &[std::path::PathBuf],
    ruleset: Option<&Path>,
    min_severity: Severity,
    overrides: &LintOverrides,
    format: OutputFormat,
) -> anyhow::Result<i32> {
    let findings = lint_findings(paths, ruleset, min_severity, overrides)?;

    match format {
        OutputFormat::Text => output::print_findings(&findings),
        OutputFormat::Json => output::print_json(&findings)?,
        OutputFormat::Sarif => crate::sarif::print_sarif(&findings)?,
    }

    let has_error = findings.iter().any(|f| f.severity == Severity::Error);
    Ok(i32::from(has_error))
}
