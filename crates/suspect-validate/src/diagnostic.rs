//! Diagnostics produced by semantic validation.

use suspect_source::Uri;

/// How severe a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Spec violation; the document is invalid.
    Error,
    /// Likely mistake or style issue; the document still works.
    Warning,
    /// Purely informational note.
    Info,
}

impl Severity {
    /// Lowercase name (`"error"`, `"warning"`, `"info"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

/// One semantic finding: a stable code, severity, human-readable message,
/// byte range into the source document, and the document it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable machine-readable identifier (e.g. `oas-duplicate-operation-id`).
    pub code: &'static str,
    /// Severity of the finding.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
    /// Byte range of the offending node in the document source.
    pub range: std::ops::Range<usize>,
    /// URI of the document the range refers to.
    pub doc: Uri,
}
