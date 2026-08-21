//! Shared output types: the CLI's unified severity/finding model, text
//! rendering, and JSON emission. Upstream severities (validate, lint) are
//! mapped into [`Severity`] here; their crates stay untouched.

use serde::Serialize;

/// Unified severity, ordered so `>=` comparisons implement min-severity
/// filtering (`hint < info < warning < error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum, serde::Serialize)]
#[value(rename_all = "lower")]
pub enum Severity {
    Hint,
    Info,
    Warning,
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
    pub file: String,
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub line: u32,
    pub col: u32,
}

impl Finding {
    /// `file:line:col [SEVERITY] code: message`
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{}:{}:{} [{}] {}: {}",
            self.file, self.line, self.col, self.severity.upper(), self.code, self.message
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
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
    };
    if override_path.is_some_and(is_json) || (override_path.is_none() && is_json(reference)) {
        crate::DocFormat::Json
    } else {
        crate::DocFormat::Yaml
    }
}
