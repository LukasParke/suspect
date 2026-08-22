//! Shared output types: the CLI's unified severity/finding model, text
//! rendering, and JSON emission. Upstream severities (validate, lint) are
//! mapped into [`Severity`] here; their crates stay untouched.

use serde::Serialize;

/// Unified severity, ordered so `>=` comparisons implement min-severity
/// filtering (`hint < info < warning < error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum, serde::Serialize)]
#[value(rename_all = "lower")]
pub enum Severity {
    /// Stylistic or preference-level note; never affects the exit code.
    Hint,
    /// Informational message worth surfacing but not actionable.
    Info,
    /// Suspicious construct that likely deserves attention.
    Warning,
    /// Definite problem; makes the command exit 1.
    Error,
}

impl Severity {
    /// Lowercase label used in text output and JSON.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Hint => "hint",
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// Uppercase label for `[SEVERITY]` spans in lint lines.
    #[must_use]
    pub fn upper(self) -> &'static str {
        match self {
            Severity::Hint => "HINT",
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }
}

/// One reportable problem, located at a 1-based line and column.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Path as given on the command line.
    pub file: String,
    /// How severe the problem is.
    pub severity: Severity,
    /// Stable machine identifier, e.g. `oas3-schema` or `E001`.
    pub code: String,
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line of the offending span.
    pub line: u32,
    /// 1-based column of the offending span.
    pub col: u32,
}

impl Finding {
    /// `file:line:col [SEVERITY] code: message`
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{}:{}:{} [{}] {}: {}",
            self.file,
            self.line,
            self.col,
            self.severity.upper(),
            self.code,
            self.message
        )
    }
}

/// Prints findings one per line in the canonical `file:line:col` shape.
pub fn print_findings(findings: &[Finding]) {
    for f in findings {
        println!("{}", f.to_line());
    }
}

/// Pretty-prints any serializable report as JSON on stdout.
///
/// # Errors
/// Serialization failures (should be unreachable for plain data).
pub fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Writes `text` to `out`, or stdout when `out` is `None`.
///
/// # Errors
/// Filesystem IO.
pub fn write_or_stdout(text: &str, out: Option<&std::path::Path>) -> anyhow::Result<()> {
    match out {
        Some(path) => {
            std::fs::write(path, text)?;
            Ok(())
        }
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

/// Picks document emission format: `override_path`'s extension wins, then
/// the reference document's; anything non-`.json` is YAML.
#[must_use]
pub fn pick_doc_format(
    override_path: Option<&std::path::Path>,
    reference: &std::path::Path,
) -> crate::DocFormat {
    let is_json = |p: &std::path::Path| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    };
    if override_path.is_some_and(is_json) || (override_path.is_none() && is_json(reference)) {
        crate::DocFormat::Json
    } else {
        crate::DocFormat::Yaml
    }
}
