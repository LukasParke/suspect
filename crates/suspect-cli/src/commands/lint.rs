//! `suspect lint`: run spectral-style rules over documents. The linter is
//! built once (built-in or from a ruleset document); findings are mapped
//! into the CLI's unified severity model, sorted by (file, line), and
//! filtered by `--min-severity`.

use std::path::Path;

use rayon::prelude::*;
use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

use crate::FindingsFormat;
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
            let source = Source::from_path(path)
                .map_err(|e| anyhow::anyhow!("ruleset {}: {e}", path.display()))?;
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
    /// `lint.recommended: false`: without a ruleset, run no rules.
    pub skip_builtin: bool,
}

/// The severity floor `lint` settings name.
#[must_use]
pub fn floor(lint: &suspect_config::LintSettings) -> Severity {
    Severity::from_config(lint.floor()).unwrap_or(Severity::Hint)
}

impl LintOverrides {
    /// The policy resolved settings carry. `resolve` already rejected
    /// unknown names, so nothing is re-parsed here.
    #[must_use]
    pub fn from_settings(lint: &suspect_config::LintSettings) -> Self {
        let to_lint = |name: suspect_config::SeverityName| match name {
            suspect_config::SeverityName::Error => suspect_lint::Severity::Error,
            suspect_config::SeverityName::Warning => suspect_lint::Severity::Warn,
            suspect_config::SeverityName::Info => suspect_lint::Severity::Info,
            suspect_config::SeverityName::Hint => suspect_lint::Severity::Hint,
            suspect_config::SeverityName::Off => suspect_lint::Severity::Off,
        };
        Self {
            design: lint.design_severity().map(to_lint),
            rules: lint
                .rule_severities()
                .map(|(code, s)| (code.to_owned(), to_lint(s)))
                .collect(),
            skip_builtin: !lint.recommended,
        }
    }

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
            let severity = suspect_diag::parse_level(name).map(suspect_lint::Severity::from);
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
        Ok(Self {
            design,
            rules: map,
            skip_builtin: false,
        })
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
    // `lint.recommended: false` and no ruleset: no rules run, but the
    // documents are still read so a missing input is reported.
    let linter = if ruleset.is_none() && overrides.skip_builtin {
        None
    } else {
        Some(build_linter(ruleset)?)
    };
    let mut findings: Vec<Finding> = paths
        .par_iter()
        .map(|p| match crate::load_doc(p) {
            Ok(doc) => linter
                .as_ref()
                .map(|linter| lint_doc(linter, &doc, &p.display().to_string()))
                .unwrap_or_default(),
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
            linter
                .as_ref()
                .is_some_and(|l| l.category_of(&f.code) == suspect_lint::Category::Design)
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
    format: FindingsFormat,
) -> anyhow::Result<i32> {
    let findings = lint_findings(paths, ruleset, min_severity, overrides)?;

    output::print_findings_as(&findings, format)?;

    let has_error = findings.iter().any(|f| f.severity == Severity::Error);
    Ok(i32::from(has_error))
}
