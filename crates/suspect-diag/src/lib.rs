#![deny(missing_docs)]
//! suspect-diag: the one finding model for the suspect toolchain.
//!
//! Holds [`Finding`], [`Severity`], [`Level`] and the severity-name parser
//! ([`parse_level`]), static rule metadata ([`RuleInfo`]), and the text,
//! JSON, SARIF and JUnit renderers ([`render`]). This is a leaf crate: it depends
//! only on `suspect-source`, `serde` and `serde_json`, so configuration,
//! check crates and front ends can all use it without a cycle.
//!
//! Pointers are stored as RFC 6901 text, the form SARIF and baselines use.

pub mod render;
pub mod severity;

use std::borrow::Cow;
use std::ops::Range;

use suspect_source::Uri;

pub use severity::{Level, Severity, parse_level};

/// Which check produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckId {
    /// YAML/JSON syntax.
    Syntax,
    /// `$ref` shape and resolution.
    Ref,
    /// OpenAPI 3.x semantic validation.
    Validate,
    /// Swagger 2.0 semantic validation.
    Swagger,
    /// Arazzo structure and cross-references.
    Arazzo,
    /// Overlay structure.
    Overlay,
    /// Load profiles (`*.load.yaml`).
    LoadProfile,
    /// Spectral-style lint rules.
    Lint,
    /// Markdown in description fields.
    Markdown,
    /// Vendor-extension schemas.
    Extension,
    /// `.suspect.yaml` schema.
    Config,
    /// TypeScript custom rules.
    Rules,
    /// SDK generation admission.
    Admission,
    /// A command-local finding outside the check pass (I/O errors,
    /// `breaking`, `drift` and other report commands).
    Command,
}

impl CheckId {
    /// The kebab-case name used by `--only` and JSON output.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Ref => "ref",
            Self::Validate => "validate",
            Self::Swagger => "swagger",
            Self::Arazzo => "arazzo",
            Self::Overlay => "overlay",
            Self::LoadProfile => "load-profile",
            Self::Lint => "lint",
            Self::Markdown => "markdown",
            Self::Extension => "extension",
            Self::Config => "config",
            Self::Rules => "rules",
            Self::Admission => "admission",
            Self::Command => "command",
        }
    }
}

/// A 0-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Position {
    /// 0-based line.
    pub line: u32,
    /// 0-based column.
    pub col: u32,
}

/// One reported problem in one document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Finding {
    /// Stable code, e.g. `oas-duplicate-operation-id`.
    pub code: Cow<'static, str>,
    /// How severe the problem is.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
    /// The document the finding is in.
    #[serde(serialize_with = "serialize_uri")]
    pub document: Uri,
    /// Byte range in `document`; `None` for document-level findings.
    pub range: Option<Range<usize>>,
    /// Start position when the producer knows a line but no byte offset
    /// (findings computed from text diffs, for instance). Renderers prefer
    /// it over resolving `range.start` through [`render::Sources`].
    pub position: Option<Position>,
    /// RFC 6901 pointer to the offending node, when known.
    pub pointer: Option<Box<str>>,
    /// The check that produced the finding.
    pub check: CheckId,
    /// Related locations (the other side of a duplicate, for instance).
    pub related: Vec<Related>,
}

impl Finding {
    /// A finding with no range, pointer or related locations.
    #[must_use]
    pub fn new(
        check: CheckId,
        code: impl Into<Cow<'static, str>>,
        severity: Severity,
        document: Uri,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            document,
            range: None,
            position: None,
            pointer: None,
            check,
            related: Vec::new(),
        }
    }

    /// Sets the byte range.
    #[must_use]
    pub fn with_range(mut self, range: Range<usize>) -> Self {
        self.range = Some(range);
        self
    }
}

/// A secondary location attached to a finding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Related {
    /// The document the location is in.
    #[serde(serialize_with = "serialize_uri")]
    pub document: Uri,
    /// Byte range in `document`.
    pub range: Range<usize>,
    /// What the location is.
    pub message: String,
}

/// The concern a rule addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleClass {
    /// The document misstates or underspecifies the API.
    Document,
    /// The API's own design; only its owner can change it (`lint.design`).
    Design,
    /// SDK-generation readiness; shown only when codegen is configured.
    Sdk,
}

/// Static metadata for one code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleInfo {
    /// The code findings carry.
    pub code: &'static str,
    /// The check that emits it.
    pub check: CheckId,
    /// Severity before any override.
    pub default: Severity,
    /// The concern it addresses.
    pub class: RuleClass,
    /// One-line summary of what the code means.
    pub summary: &'static str,
    /// Actionable instruction for resolving it; may be empty.
    pub how_to_fix: &'static str,
    /// Other codes that report this same problem.
    pub aliases: &'static [&'static str],
}

impl RuleInfo {
    /// A document-class rule with no fix text and no aliases.
    #[must_use]
    pub const fn new(
        code: &'static str,
        check: CheckId,
        default: Severity,
        summary: &'static str,
    ) -> Self {
        Self {
            code,
            check,
            default,
            class: RuleClass::Document,
            summary,
            how_to_fix: "",
            aliases: &[],
        }
    }
}

fn serialize_uri<S: serde::Serializer>(uri: &Uri, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(uri.as_str())
}
