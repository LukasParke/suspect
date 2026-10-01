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
