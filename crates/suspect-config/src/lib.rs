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
//! [`resolve`] / [`resolve_with`] apply one precedence rule, used by the
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

use std::path::{Path, PathBuf};

pub mod discover;
pub mod file;
pub mod layer;
pub mod resolve;
pub mod schema;
pub mod severity;
mod typed;

pub use discover::Found;
pub use file::{CONFIG_NAMES, discover, load};
pub use layer::{Layer, Notice, Origin, Raw};
pub use resolve::{
    ConfigError, Env, ExplainRow, Inputs, LayerKind, Resolution, Resolved, resolve, resolve_bool,
    resolve_found, resolve_str, resolve_with,
};
pub use severity::{SeverityName, parse_severity};

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

/// A loaded configuration: the settings plus where they came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    /// The effective settings.
    pub settings: Settings,
    /// The file they came from, when one was found.
    pub path: Option<PathBuf>,
    /// The directory relative paths resolve against.
    pub root: PathBuf,
    /// Which boolean settings this layer set explicitly, so layering can
    /// let a higher layer turn a lower layer's `true` back off.
    pub explicit: ExplicitBools,
}

/// The boolean settings a layer carried explicitly: `None` when the key
/// was absent, `Some(value)` when it was written (including `false`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExplicitBools {
    /// `validate.strict_format`.
    pub strict_format: Option<bool>,
    /// `format.json`.
    pub format_json: Option<bool>,
    /// `format.yaml`.
    pub format_yaml: Option<bool>,
}

impl Loaded {
    /// Settings with no configuration file in effect.
    #[must_use]
    pub fn none() -> Self {
        Self {
            settings: Settings::default(),
            path: None,
            root: PathBuf::from("."),
            explicit: ExplicitBools::default(),
        }
    }

    /// A one-line, human-readable summary for `suspect config`.
    #[must_use]
    pub fn describe(&self) -> String {
        let Some(path) = &self.path else {
            return "no configuration file found (using built-in defaults)".to_owned();
        };
        let mut lines = vec![format!("config: {}", path.display())];
        let s = &self.settings;
        if let Some(ruleset) = &s.lint.ruleset {
            lines.push(format!("  lint.ruleset: {}", ruleset.display()));
        }
        if let Some(severity) = &s.lint.min_severity {
            lines.push(format!("  lint.min_severity: {severity}"));
        }
        if let Some(design) = &s.lint.design {
            lines.push(format!("  lint.design: {design}"));
        }
        for (code, severity) in &s.lint.rules {
            lines.push(format!("  lint.rules.{code}: {severity}"));
        }
        if s.validate.strict_format {
            lines.push("  validate.strict_format: true".to_owned());
        }
        if s.format.json {
            lines.push("  format.json: true".to_owned());
        }
        if s.format.yaml {
            lines.push("  format.yaml: true".to_owned());
        }
        if let Some(style) = &s.docs.style {
            lines.push(format!("  docs.style: {style}"));
        }
        if let Some(out) = &s.docs.output {
            lines.push(format!("  docs.out: {}", out.display()));
        }
        if let Some(out) = &s.codegen.out {
            lines.push(format!("  codegen.out: {}", out.display()));
        }
        if let Some(profile) = &s.codegen.profile {
            lines.push(format!("  codegen.profile: {profile}"));
        }
        if let Some(version) = &s.codegen.package_version {
            lines.push(format!("  codegen.package_version: {version}"));
        }
        lines.join("\n")
    }
}

/// Loads configuration for an invocation: the nearest file at or above the
/// working directory, falling back to walking up from `input` when the
/// caller named a file elsewhere.
///
/// # Errors
/// Propagates a malformed configuration file: a committed config that does
/// not parse must be loud, not silently absent.
impl Loaded {
    /// The manifest's policy sections, parsed with the settings file's key
    /// spellings: the project manifest (`suspect` + `.suspect.project.json`)
    /// carries `lint` and `validate` as committed project policy, layered
    /// under the workspace config file so `.suspect.yaml` remains a local
    /// override and client settings still win over both.
    ///
    /// # Errors
    /// Never: unrecognized or absent sections parse as absent settings.
    #[must_use]
    pub fn from_manifest_policy(dir: &Path, policy: &serde_json::Value) -> Loaded {
        let root = dir.to_path_buf();
        let resolve = |relative: &str| root.join(relative);
        let lint = policy.get("lint");
        let validate = policy.get("validate");
        let strict_format = validate
            .and_then(|v| v.get("strict_format"))
            .and_then(serde_json::Value::as_bool);
        Loaded {
            settings: Settings {
                lint: crate::LintSettings {
                    ruleset: lint
                        .and_then(|l| l.get("ruleset"))
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(resolve),
                    min_severity: lint
                        .and_then(|l| l.get("min_severity"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    design: lint
                        .and_then(|l| l.get("design"))
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    rules: lint
                        .and_then(|l| l.get("rules"))
                        .and_then(serde_json::Value::as_object)
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| {
                                    v.as_str()
                                        .filter(|s| !s.is_empty())
                                        .map(|s| (k.clone(), s.to_ascii_lowercase()))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    ..crate::LintSettings::default()
                },
                validate: crate::ValidateSettings {
                    strict_format: strict_format.unwrap_or(false),
                    ..crate::ValidateSettings::default()
                },
                ..Settings::default()
            },
            path: None,
            root,
            explicit: ExplicitBools {
                strict_format,
                ..ExplicitBools::default()
            },
        }
    }

    /// Layers `upper` over `self`: every value `upper` carries wins, and
    /// `self` fills the rest. The manifest is the base; the discovered
    /// settings file overrides it.
    #[must_use]
    pub fn layered(base: Loaded, upper: Loaded) -> Loaded {
        let mut s = base.settings;
        let u = upper.settings;
        // Booleans: the upper layer wins whenever it wrote the key, `false`
        // included. A hand-built layer without `explicit` still counts a
        // `true` as set.
        let pick = |upper_explicit: Option<bool>,
                    upper_value: bool,
                    base_explicit: Option<bool>,
                    base_value: bool| {
            upper_explicit
                .or(upper_value.then_some(true))
                .or(base_explicit)
                .or(base_value.then_some(true))
        };
        let explicit = ExplicitBools {
            strict_format: pick(
                upper.explicit.strict_format,
                u.validate.strict_format,
                base.explicit.strict_format,
                s.validate.strict_format,
            ),
            format_json: pick(
                upper.explicit.format_json,
                u.format.json,
                base.explicit.format_json,
                s.format.json,
            ),
            format_yaml: pick(
                upper.explicit.format_yaml,
                u.format.yaml,
                base.explicit.format_yaml,
                s.format.yaml,
            ),
        };
        s.lint.ruleset = u.lint.ruleset.or(s.lint.ruleset);
        s.lint.min_severity = u.lint.min_severity.or(s.lint.min_severity);
        s.lint.design = u.lint.design.or(s.lint.design);
        for (code, severity) in u.lint.rules {
            s.lint.rules.insert(code, severity);
        }
        s.validate.strict_format = explicit.strict_format.unwrap_or(false);
        s.format.json = explicit.format_json.unwrap_or(false);
        s.format.yaml = explicit.format_yaml.unwrap_or(false);
        s.docs.style = u.docs.style.or(s.docs.style);
        s.docs.output = u.docs.output.or(s.docs.output);
        s.codegen.out = u.codegen.out.or(s.codegen.out);
        s.codegen.profile = u.codegen.profile.or(s.codegen.profile);
        s.codegen.package_version = u.codegen.package_version.or(s.codegen.package_version);
        Loaded {
            settings: s,
            path: upper.path.or(base.path),
            root: if upper.root.as_os_str().is_empty() {
                base.root
            } else {
                upper.root
            },
            explicit,
        }
    }
}

/// The manifest file name the loader composes policy from.
pub const MANIFEST_NAME: &str = "suspect.project.json";

/// The configuration for one invocation: the settings file discovered
/// from the input's directory, with the project manifest's policy
/// sections layered underneath when one sits on the walk up.
///
/// # Errors
/// Config file discovery and parsing failures.
pub fn for_invocation(input: Option<&Path>) -> Result<Loaded, file::ConfigError> {
    if let Some(found) = discover(Path::new("."))? {
        return Ok(found);
    }
    match input {
        Some(path) => Ok(discover(path)?.unwrap_or_default()),
        None => Ok(Loaded::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_settings_describe_as_absent() {
        assert!(Loaded::none().describe().starts_with("no configuration"));
    }

    #[test]
    fn describe_lists_every_configured_value() {
        let loaded = Loaded {
            settings: Settings {
                lint: LintSettings {
                    ruleset: Some(PathBuf::from("rules.yaml")),
                    min_severity: Some("warning".to_owned()),
                    ..LintSettings::default()
                },
                validate: ValidateSettings {
                    strict_format: true,
                    ..ValidateSettings::default()
                },
                format: FormatSettings {
                    json: true,
                    yaml: false,
                    ..FormatSettings::default()
                },
                docs: DocsSettings {
                    style: Some("markdown".to_owned()),
                    output: Some(PathBuf::from("site")),
                },
                codegen: CodegenSettings {
                    out: Some(PathBuf::from("sdk")),
                    profile: Some("typescript-http".to_owned()),
                    package_version: Some("1.0.0".to_owned()),
                },
                ..Settings::default()
            },
            path: Some(PathBuf::from("/p/.suspect.yaml")),
            root: PathBuf::from("/p"),
            explicit: ExplicitBools::default(),
        };
        let text = loaded.describe();
        for expected in [
            "lint.min_severity: warning",
            "validate.strict_format: true",
            "format.json: true",
            "docs.style: markdown",
            "codegen.profile: typescript-http",
            "codegen.package_version: 1.0.0",
        ] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
        assert!(
            !text.contains("format.yaml"),
            "unset values stay out:\n{text}"
        );
    }
}

#[cfg(test)]
mod manifest_layer_tests {
    use super::*;

    #[test]
    fn manifest_policy_parses_with_the_settings_spellings() {
        let policy = serde_json::json!({
            "lint": {"min_severity": "warning", "ruleset": "rules/my-rules.yaml"},
            "validate": {"strict_format": true}
        });
        let loaded = Loaded::from_manifest_policy(Path::new("/proj"), &policy);
        assert_eq!(
            loaded.settings.lint.min_severity.as_deref(),
            Some("warning")
        );
        assert_eq!(
            loaded.settings.lint.ruleset,
            Some(PathBuf::from("/proj/rules/my-rules.yaml"))
        );
        assert!(loaded.settings.validate.strict_format);
    }

    #[test]
    fn manifest_policy_carries_design_and_per_rule_overrides() {
        let policy = serde_json::json!({
            "lint": {
                "design": "off",
                "rules": {
                    "security-rate-limit-documented": "info",
                    "operation-operationId": "OFF"
                }
            }
        });
        let loaded = Loaded::from_manifest_policy(Path::new("/proj"), &policy);
        assert_eq!(loaded.settings.lint.design.as_deref(), Some("off"));
        // Severity names are normalized to lowercase; the map preserves
        // every entry.
        assert_eq!(
            loaded
                .settings
                .lint
                .rules
                .get("security-rate-limit-documented"),
            Some(&"info".to_owned())
        );
        assert_eq!(
            loaded.settings.lint.rules.get("operation-operationId"),
            Some(&"off".to_owned())
        );
        assert_eq!(loaded.settings.lint.rules.len(), 2);
    }

    #[test]
    fn settings_file_overrides_the_manifest_design_layer_per_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = Loaded::from_manifest_policy(
            dir.path(),
            &serde_json::json!({"lint": {
                "design": "off",
                "rules": {"operation-operationId": "off", "info-contact": "hint"}
            }}),
        );
        let upper = Loaded {
            settings: Settings {
                lint: crate::LintSettings {
                    design: Some("info".to_owned()),
                    rules: [(
                        std::string::String::from("operation-operationId"),
                        "error".to_owned(),
                    )]
                    .into_iter()
                    .collect(),
                    ..crate::LintSettings::default()
                },
                ..Settings::default()
            },
            path: None,
            root: dir.path().to_path_buf(),
            explicit: ExplicitBools::default(),
        };
        let layered = Loaded::layered(base, upper);
        // The settings file wins for the keys it carries; the manifest's
        // other per-rule entries survive.
        assert_eq!(layered.settings.lint.design.as_deref(), Some("info"));
        assert_eq!(
            layered.settings.lint.rules.get("operation-operationId"),
            Some(&"error".to_owned())
        );
        assert_eq!(
            layered.settings.lint.rules.get("info-contact"),
            Some(&"hint".to_owned())
        );
    }

    #[test]
    fn settings_file_overrides_the_manifest_layer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = Loaded::from_manifest_policy(
            dir.path(),
            &serde_json::json!({"lint": {"min_severity": "warning"}}),
        );
        let upper = Loaded {
            settings: Settings {
                lint: crate::LintSettings {
                    min_severity: Some("info".to_owned()),
                    ruleset: None,
                    ..crate::LintSettings::default()
                },
                ..Settings::default()
            },
            path: None,
            root: dir.path().to_path_buf(),
            explicit: ExplicitBools::default(),
        };
        let layered = Loaded::layered(base, upper);
        // The settings file wins where it carries a value; the manifest
        // fills the rest.
        assert_eq!(layered.settings.lint.min_severity.as_deref(), Some("info"));
        assert!(layered.settings.lint.ruleset.is_none());
    }

    #[test]
    fn discover_composes_a_manifest_beneath_the_settings_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("suspect.project.json"),
            r#"{"lint": {"min_severity": "error"}, "validate": {"strict_format": true}}"#,
        )
        .expect("manifest");
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\n",
        )
        .expect("settings");
        let found = discover(dir.path().join("openapi.yaml").as_path())
            .expect("discover")
            .expect("found");
        // The settings file's floor wins over the manifest's; the
        // manifest's strict_format survives underneath.
        assert_eq!(found.settings.lint.min_severity.as_deref(), Some("warning"));
        assert!(found.settings.validate.strict_format);
    }

    #[test]
    fn a_manifest_alone_supplies_its_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("suspect.project.json"),
            r#"{"lint": {"min_severity": "error"}}"#,
        )
        .expect("manifest");
        let found = discover(dir.path().join("openapi.yaml").as_path())
            .expect("discover")
            .expect("found");
        assert_eq!(found.settings.lint.min_severity.as_deref(), Some("error"));
    }
}
