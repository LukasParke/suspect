//! Shared output types: the CLI's unified severity/finding model, text
//! rendering, and JSON emission. Upstream severities (validate, lint) are
//! mapped into [`Severity`] here. Text and SARIF output render through
//! `suspect_diag::render`: [`Finding`] converts to and from
//! [`suspect_diag::Finding`] without changing a byte of either format.

use serde::Serialize;

/// Unified severity, ordered so `>=` comparisons implement min-severity
/// filtering (`hint < info < warning < error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum, serde::Serialize)]
#[value(rename_all = "lower")]
pub enum Severity {
    /// Stylistic or preference-level note; never affects the exit code.
    Hint,
    /// Informational message worth surfacing but not actionable.
    #[value(alias = "information")]
    Info,
    /// Suspicious construct that likely deserves attention.
    #[value(alias = "warn")]
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

    /// Maps a configured severity name onto the unified model; `None` for
    /// `off`, which has no rank.
    #[must_use]
    pub fn from_config(name: suspect_config::SeverityName) -> Option<Self> {
        match name {
            suspect_config::SeverityName::Error => Some(Severity::Error),
            suspect_config::SeverityName::Warning => Some(Severity::Warning),
            suspect_config::SeverityName::Info => Some(Severity::Info),
            suspect_config::SeverityName::Hint => Some(Severity::Hint),
            suspect_config::SeverityName::Off => None,
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
    /// Exact half-open range in decoded UTF-8 source bytes, when available.
    /// This retains container identity that a line/column alone cannot express.
    pub range: Option<std::ops::Range<usize>>,
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

impl From<Severity> for suspect_diag::Severity {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Hint => Self::Hint,
            Severity::Info => Self::Info,
            Severity::Warning => Self::Warning,
            Severity::Error => Self::Error,
        }
    }
}

impl From<suspect_diag::Severity> for Severity {
    fn from(severity: suspect_diag::Severity) -> Self {
        match severity {
            suspect_diag::Severity::Hint => Self::Hint,
            suspect_diag::Severity::Info => Self::Info,
            suspect_diag::Severity::Warning => Self::Warning,
            suspect_diag::Severity::Error => Self::Error,
        }
    }
}

impl Finding {
    /// Converts to the shared finding model. The document is the path as
    /// shown (not a canonical URI), the 1-based line/column become the
    /// finding's [`suspect_diag::Position`], and the check is
    /// [`suspect_diag::CheckId::Command`]. Render the result with
    /// [`ShownPaths`] to reproduce [`Finding::to_line`] exactly.
    #[must_use]
    pub fn to_diag(&self) -> suspect_diag::Finding {
        suspect_diag::Finding {
            code: self.code.clone().into(),
            severity: self.severity.into(),
            message: self.message.clone(),
            document: shown_uri(&self.file),
            range: self.range.clone(),
            position: Some(suspect_diag::Position {
                line: self.line.saturating_sub(1),
                col: self.col.saturating_sub(1),
            }),
            pointer: None,
            check: suspect_diag::CheckId::Command,
            related: Vec::new(),
        }
    }

    /// Converts from the shared finding model, resolving the display path
    /// and 1-based position through `src`.
    #[must_use]
    pub fn from_diag(
        finding: &suspect_diag::Finding,
        src: &dyn suspect_diag::render::Sources,
    ) -> Self {
        let (line, col) = suspect_diag::render::start(finding, src);
        Self {
            file: src.display_path(&finding.document),
            severity: finding.severity.into(),
            code: finding.code.to_string(),
            message: finding.message.clone(),
            line,
            col,
            range: finding.range.clone(),
        }
    }
}

/// The document identity [`Finding::to_diag`] uses: the shown path taken
/// verbatim, so distinct spellings never merge.
fn shown_uri(file: &str) -> suspect_source::Uri {
    match suspect_source::Uri::try_from(file.as_bytes()) {
        Ok(uri) => uri,
        // `file` is a `String`, so it is always UTF-8.
        Err(_) => suspect_source::Uri::from(file),
    }
}

/// [`suspect_diag::render::Sources`] for findings made by
/// [`Finding::to_diag`]: the document is the shown path, and positions are
/// carried on each finding, so no offset lookup is needed.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShownPaths;

impl suspect_diag::render::Sources for ShownPaths {
    fn line_col(&self, _doc: &suspect_source::Uri, _offset: usize) -> Option<(u32, u32)> {
        None
    }
    fn display_path(&self, doc: &suspect_source::Uri) -> String {
        doc.as_str().to_owned()
    }
}

/// Converts every finding with [`Finding::to_diag`].
#[must_use]
pub fn to_diag(findings: &[Finding]) -> Vec<suspect_diag::Finding> {
    findings.iter().map(Finding::to_diag).collect()
}

/// The canonical `file:line:col [SEVERITY] code: message` lines, one per
/// finding, rendered by `suspect_diag::render::text`.
#[must_use]
pub fn render_text(findings: &[Finding]) -> String {
    let mut out = Vec::new();
    // Writing into a Vec cannot fail.
    let _ = suspect_diag::render::text(&to_diag(findings), &ShownPaths, &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// Prints findings one per line in the canonical `file:line:col` shape.
pub fn print_findings(findings: &[Finding]) {
    print!("{}", render_text(findings));
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
