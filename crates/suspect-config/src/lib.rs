//! One configuration schema for the whole toolchain.
//!
//! Settings used to be read in three places with three shapes: the CLI's
//! private `.suspect.yaml` loader, the language server's
//! `initializationOptions` struct, and a project manifest. Precedence
//! drifted with them — twice a config value silently overrode an explicit
//! flag, because a clap default and a typed flag were indistinguishable.
//!
//! This crate is the single schema. One table ([`schema::SETTINGS`])
//! defines every key, its environment variable, editor setting and flags;
//! [`resolve()`] / [`resolve_with`] apply one precedence rule, used by the
//! CLI and the language server alike:
//!
//! ```text
//! default < manifest (deprecated) < .suspect.yaml < env < editor = flag
//! ```
//!
//! A project manifest is deliberately *not* configuration: it describes
//! what to build (entry document, overlays, targets, tests), not how the
//! tools behave. Its old `lint`/`validate`/`editor` sections still apply
//! for one release, with a deprecation warning.

#![deny(missing_docs)]

use std::path::PathBuf;

pub mod discover;
pub mod file;
pub mod json_schema;
pub mod layer;
pub mod resolve;
pub mod schema;
pub mod severity;
mod typed;

pub use discover::Found;
pub use file::CONFIG_NAMES;
pub use layer::{Layer, Notice, Origin, Raw};
pub use resolve::{
    ConfigError, Env, ExplainRow, Inputs, Resolved, resolve, resolve_found, resolve_with,
};
pub use severity::{Level, SeverityName, parse_level, parse_severity};

/// The toolchain's settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// `lint:` section.
    pub lint: LintSettings,
    /// `validate:` section.
    pub validate: ValidateSettings,
    /// `format:` section.
    pub format: FormatSettings,
    /// `docs:` section.
    pub docs: DocsSettings,
    /// `codegen:` section.
    pub codegen: CodegenSettings,
    /// `ref:` section.
    pub refs: RefSettings,
    /// `editor:` section.
    pub editor: EditorSettings,
    /// `test:` section.
    pub test: TestSettings,
    /// `gateway:` section.
    pub gateway: GatewaySettings,
    /// `diff:` section.
    pub diff: DiffSettings,
    /// `rules:` section.
    pub rules: RulesSettings,
    /// `extensions:` section, opaque here; the language server's
    /// `extensions_config` owns its shape.
    pub extensions: Option<serde_json::Value>,
}

/// `lint:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct LintSettings {
    /// Custom ruleset document, already resolved against the file that
    /// set it.
    pub ruleset: Option<PathBuf>,
    /// Minimum severity to report, canonical (`error`, `warning`, `info`,
    /// `hint`); `None` is unset (`hint`).
    pub min_severity: Option<String>,
    /// Severity for design-class rules — findings the document cannot fix
    /// without misdocumenting the API (`error`..`hint`, or `off`).
    pub design: Option<String>,
    /// Per-rule severity overrides: rule id → canonical severity
    /// (`error`..`hint`, or `off` to drop the rule). Wins over `design`.
    pub rules: std::collections::BTreeMap<String, String>,
    /// Apply the built-in rule set; `false` with no ruleset runs no rules.
    pub recommended: bool,
}

impl Default for LintSettings {
    fn default() -> Self {
        Self {
            ruleset: None,
            min_severity: None,
            design: None,
            rules: std::collections::BTreeMap::new(),
            recommended: true,
        }
    }
}

impl LintSettings {
    /// The severity floor (`hint` when unset).
    #[must_use]
    pub fn floor(&self) -> SeverityName {
        self.min_severity
            .as_deref()
            .and_then(parse_severity)
            .filter(|s| *s != SeverityName::Off)
            .unwrap_or(SeverityName::Hint)
    }

    /// The design-class severity, when set.
    #[must_use]
    pub fn design_severity(&self) -> Option<SeverityName> {
        self.design.as_deref().and_then(parse_severity)
    }

    /// Per-rule overrides, parsed.
    pub fn rule_severities(&self) -> impl Iterator<Item = (&str, SeverityName)> {
        self.rules
            .iter()
            .filter_map(|(code, name)| parse_severity(name).map(|s| (code.as_str(), s)))
    }
}

/// `validate:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidateSettings {
    /// Assert `format` keywords; the 2020-12 default is annotation-only.
    pub strict_format: bool,
    /// File listing the documents `$ref`s may reach.
    pub reference_allowlist: Option<PathBuf>,
}

/// `format:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatSettings {
    /// Emit JSON regardless of the input extension.
    pub json: bool,
    /// Emit YAML regardless of the input extension.
    pub yaml: bool,
    /// Reorder keys into the canonical OpenAPI order when formatting.
    pub sort_keys: bool,
}

/// `docs:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocsSettings {
    /// Output style (`html`, `markdown`, `sveltekit`).
    pub style: Option<String>,
    /// Output root (`docs.output`; `docs.out` is the deprecated spelling).
    pub output: Option<PathBuf>,
}

/// `codegen:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodegenSettings {
    /// Default output root.
    pub out: Option<PathBuf>,
    /// Default native profile.
    pub profile: Option<String>,
    /// Default package version.
    pub package_version: Option<String>,
}

/// `ref:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RefSettings {
    /// Maximum documents one ref workspace loads.
    pub max_docs: usize,
}

impl Default for RefSettings {
    fn default() -> Self {
        Self { max_docs: 500 }
    }
}

/// `editor:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditorSettings {
    /// `editor.inlay_hints`.
    pub inlay_hints: InlayHintSettings,
}

/// `editor.inlay_hints:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct InlayHintSettings {
    /// Resolved `$ref` target hints.
    pub refs: bool,
    /// Property type-annotation hints.
    pub properties: bool,
}

impl Default for InlayHintSettings {
    fn default() -> Self {
        Self {
            refs: true,
            properties: true,
        }
    }
}

/// `test:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TestSettings {
    /// Base URL requests are sent to.
    pub base_url: String,
    /// Credentials file; `None` discovers `.suspect/credentials.json`.
    pub credentials: Option<PathBuf>,
    /// `suspect test` event stream format (`text`, `ndjson`).
    pub report: String,
}

/// The base URL when nothing sets one.
pub const DEFAULT_BASE_URL: &str = "http://localhost:8080";

impl Default for TestSettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            credentials: None,
            report: "text".to_owned(),
        }
    }
}

/// `gateway:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct GatewaySettings {
    /// Listen port.
    pub port: u16,
    /// Operating mode.
    pub mode: String,
    /// Upstream base URL.
    pub upstream: Option<String>,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            port: 8080,
            mode: "mock".to_owned(),
            upstream: None,
        }
    }
}

/// `diff:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffSettings {
    /// Git ref compared against for breaking changes.
    pub baseline: Option<String>,
}

/// `rules:` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RulesSettings {
    /// Run TypeScript rule plugins.
    pub enabled: bool,
    /// Plugin directories.
    pub roots: Vec<PathBuf>,
}

impl Default for RulesSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            roots: vec![PathBuf::from(".suspect/rules")],
        }
    }
}

/// The project manifest file name.
pub const MANIFEST_NAME: &str = "suspect.project.json";
