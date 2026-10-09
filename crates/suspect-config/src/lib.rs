//! One configuration schema for the whole toolchain.
//!
//! Settings used to be read in three places with three shapes: the CLI's
//! private `.suspect.yaml` loader, the language server's
//! `initializationOptions` struct, and a project manifest. Precedence
//! drifted with them — twice a config value silently overrode an explicit
//! flag, because a clap default and a typed flag were indistinguishable.
//!
//! This crate is the single schema. It defines the settings, the file
//! format, and the precedence rule, and it is consumed by the CLI and the
//! language server alike:
//!
//! ```text
//! flag  >  environment  >  config file  >  built-in default
//! ```
//!
//! A project manifest is deliberately *not* configuration: it describes
//! what to build (entry document, overlays, targets, tests), not how the
//! tools behave. Mixing the two would make a build file silently change
//! editor diagnostics.

#![deny(missing_docs)]

use std::path::{Path, PathBuf};

pub mod file;
pub mod resolve;

pub use file::{CONFIG_NAMES, discover, load};
pub use resolve::{Layer, Resolution, resolve_bool, resolve_str};

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
}

/// `lint:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LintSettings {
    /// Custom ruleset document, relative to the config root.
    pub ruleset: Option<PathBuf>,
    /// Minimum severity to report (`error`, `warning`, `info`, `hint`).
    pub min_severity: Option<String>,
}

/// `validate:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidateSettings {
    /// Assert `format` keywords; the 2020-12 default is annotation-only.
    pub strict_format: bool,
}

/// `format:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatSettings {
    /// Emit JSON regardless of the input extension.
    pub json: bool,
    /// Emit YAML regardless of the input extension.
    pub yaml: bool,
}

/// `docs:` settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocsSettings {
    /// Output style (`html`, `markdown`, `sveltekit`).
    pub style: Option<String>,
    /// Output root.
    pub out: Option<PathBuf>,
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

/// A loaded configuration: the settings plus where they came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    /// The effective settings.
    pub settings: Settings,
    /// The file they came from, when one was found.
    pub path: Option<PathBuf>,
    /// The directory relative paths resolve against.
    pub root: PathBuf,
}

impl Loaded {
    /// Settings with no configuration file in effect.
    #[must_use]
    pub fn none() -> Self {
        Self {
            settings: Settings::default(),
            path: None,
            root: PathBuf::from("."),
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
        if let Some(out) = &s.docs.out {
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
                },
                validate: crate::ValidateSettings {
                    strict_format: validate
                        .and_then(|v| v.get("strict_format"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                },
                ..Settings::default()
            },
            path: None,
            root,
        }
    }

    /// Layers `upper` over `self`: every value `upper` carries wins, and
    /// `self` fills the rest. The manifest is the base; the discovered
    /// settings file overrides it.
    #[must_use]
    pub fn layered(base: Loaded, upper: Loaded) -> Loaded {
        let mut s = base.settings;
        let u = upper.settings;
        s.lint.ruleset = u.lint.ruleset.or(s.lint.ruleset);
        s.lint.min_severity = u.lint.min_severity.or(s.lint.min_severity);
        s.validate.strict_format |= u.validate.strict_format;
        s.format.json |= u.format.json;
        s.format.yaml |= u.format.yaml;
        s.docs.style = u.docs.style.or(s.docs.style);
        s.docs.out = u.docs.out.or(s.docs.out);
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
                },
                validate: ValidateSettings {
                    strict_format: true,
                },
                format: FormatSettings {
                    json: true,
                    yaml: false,
                },
                docs: DocsSettings {
                    style: Some("markdown".to_owned()),
                    out: Some(PathBuf::from("site")),
                },
                codegen: CodegenSettings {
                    out: Some(PathBuf::from("sdk")),
                    profile: Some("typescript-http".to_owned()),
                    package_version: Some("1.0.0".to_owned()),
                },
            },
            path: Some(PathBuf::from("/p/.suspect.yaml")),
            root: PathBuf::from("/p"),
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
                },
                ..Settings::default()
            },
            path: None,
            root: dir.path().to_path_buf(),
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
