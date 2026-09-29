//! Project configuration discovery: `.suspect.yaml`.
//!
//! Every suspect command that takes a flag also accepts a project default
//! from a `.suspect.yaml` found by walking up from the working directory (or
//! from an input path). Explicit flags always win: configuration supplies
//! defaults, it never overrides an argument the user typed. That property is
//! what makes a config file safe to commit and a flag safe to type.
//!
//! The same file is read by the language server, so a project's lint
//! ruleset, severity floor, and strictness are identical in the editor and
//! in CI.

use std::path::{Path, PathBuf};

/// Configuration file names, in discovery order.
pub const CONFIG_NAMES: &[&str] = &[
    ".suspect.yaml",
    ".suspect.yml",
    "suspect.yaml",
    "suspect.yml",
];

/// The parsed `.suspect.yaml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectConfig {
    /// Directory containing the file that produced this configuration.
    pub root: PathBuf,
    /// The file that produced this configuration.
    pub path: PathBuf,
    /// `lint:` section.
    pub lint: LintConfig,
    /// `validate:` section.
    pub validate: ValidateConfig,
    /// `codegen:` section.
    pub codegen: CodegenConfig,
    /// `docs:` section.
    pub docs: DocsConfig,
    /// `format:` section (formatter defaults).
    pub format: FormatConfig,
}

/// `lint:` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LintConfig {
    /// Path to a custom ruleset, relative to the config root.
    pub ruleset: Option<PathBuf>,
    /// Minimum severity reported.
    pub min_severity: Option<String>,
}

/// `validate:` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidateConfig {
    /// Assert `format` keywords (the 2020-12 annotation default stays).
    pub strict_format: bool,
}

/// `codegen:` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodegenConfig {
    /// Default output root.
    pub out: Option<PathBuf>,
    /// Default native profile.
    pub profile: Option<String>,
    /// Default package version.
    pub package_version: Option<String>,
}

/// `docs:` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocsConfig {
    /// Default output style.
    pub style: Option<String>,
    /// Default output root.
    pub out: Option<PathBuf>,
}

/// `format:` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatConfig {
    /// Emit JSON regardless of the input extension.
    pub json: bool,
    /// Emit YAML regardless of the input extension.
    pub yaml: bool,
}

/// A configuration failure (malformed file, unreadable path).
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Walks up from `start` looking for a configuration file.
///
/// Returns `Ok(None)` when no file is found, which is the normal case for
/// a one-off invocation outside a project.
///
/// # Errors
/// Propagates a parse failure rather than ignoring it: a malformed
/// committed config must be loud, not silently absent.
pub fn discover(start: &Path) -> Result<Option<ProjectConfig>, ConfigError> {
    let mut dir = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(current) = dir {
        for name in CONFIG_NAMES {
            let candidate = current.join(name);
            if candidate.is_file() {
                return load(&candidate).map(Some);
            }
        }
        dir = current.parent();
    }
    Ok(None)
}

/// Parses one configuration file.
///
/// # Errors
/// IO or parse failures, naming the file and the offending section.
pub fn load(path: &Path) -> Result<ProjectConfig, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    let doc = suspect_low::LowDoc::parse(
        suspect_source::Uri::from_path(path)
            .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?,
        suspect_source::Source::from_vec(text.into_bytes()),
    );
    if let Some(error) = doc.syntax_errors().first() {
        return Err(ConfigError(format!(
            "{}: {}",
            path.display(),
            error.message
        )));
    }
    let root = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let str_at = |section: &str, key: &str| -> Option<String> {
        doc.root()
            .get(section)?
            .get(key)?
            .as_str()
            .map(str::to_owned)
    };
    let bool_at = |section: &str, key: &str| -> bool {
        doc.root()
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "true")
    };
    let resolve = |relative: String| -> PathBuf { root.join(relative) };

    let mut lint = LintConfig {
        ruleset: str_at("lint", "ruleset").map(resolve),
        min_severity: str_at("lint", "min_severity"),
    };
    if lint.min_severity.as_deref() == Some("") {
        lint.min_severity = None;
    }
    Ok(ProjectConfig {
        root: root.clone(),
        path: path.to_path_buf(),
        lint,
        validate: ValidateConfig {
            strict_format: bool_at("validate", "strict_format"),
        },
        codegen: CodegenConfig {
            out: str_at("codegen", "out").map(resolve),
            profile: str_at("codegen", "profile"),
            package_version: str_at("codegen", "package_version"),
        },
        docs: DocsConfig {
            style: str_at("docs", "style"),
            out: str_at("docs", "out").map(resolve),
        },
        format: FormatConfig {
            json: bool_at("format", "json"),
            yaml: bool_at("format", "yaml"),
        },
    })
}

/// Loads configuration for an invocation: the nearest file at or above the
/// working directory, falling back to walking up from `input` when the
/// caller named a file elsewhere.
///
/// # Errors
/// Propagates parse failures.
pub fn for_invocation(input: Option<&Path>) -> Result<Option<ProjectConfig>, ConfigError> {
    if let Some(config) = discover(Path::new("."))? {
        return Ok(Some(config));
    }
    match input {
        Some(path) => discover(path),
        None => Ok(None),
    }
}

/// Reports which configuration file is in effect, for `--help`-style
/// transparency (`suspect config`).
#[must_use]
pub fn describe(config: Option<&ProjectConfig>) -> String {
    match config {
        None => "no configuration file found (using command defaults)".to_owned(),
        Some(config) => {
            let mut lines = vec![format!("config: {}", config.path.display())];
            if let Some(ruleset) = &config.lint.ruleset {
                lines.push(format!("  lint.ruleset: {}", ruleset.display()));
            }
            if let Some(severity) = &config.lint.min_severity {
                lines.push(format!("  lint.min_severity: {severity}"));
            }
            if config.validate.strict_format {
                lines.push("  validate.strict_format: true".to_owned());
            }
            if let Some(out) = &config.codegen.out {
                lines.push(format!("  codegen.out: {}", out.display()));
            }
            if let Some(profile) = &config.codegen.profile {
                lines.push(format!("  codegen.profile: {profile}"));
            }
            if let Some(version) = &config.codegen.package_version {
                lines.push(format!("  codegen.package_version: {version}"));
            }
            if let Some(style) = &config.docs.style {
                lines.push(format!("  docs.style: {style}"));
            }
            if let Some(out) = &config.docs.out {
                lines.push(format!("  docs.out: {}", out.display()));
            }
            if config.format.json {
                lines.push("  format.json: true".to_owned());
            }
            if config.format.yaml {
                lines.push("  format.yaml: true".to_owned());
            }
            lines.join("\n")
        }
    }
}
