//! `.suspect.yaml` discovery and parsing.
//!
//! A malformed file is an error, never a silent absence: a committed
//! config that stops applying must be visible immediately.

use std::path::{Path, PathBuf};

use crate::{Loaded, Settings};

/// File names searched, in order.
pub const CONFIG_NAMES: &[&str] = &[
    ".suspect.yaml",
    ".suspect.yml",
    "suspect.yaml",
    "suspect.yml",
];

/// A configuration failure.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Walks up from `start` looking for a configuration file, composing the
/// project manifest's policy sections underneath when one sits on the
/// walk. Both searches are independent: the settings file and the
/// manifest need not sit in the same directory, and each contributes its
/// own layer — the settings file wins wherever both carry a value.
///
/// # Errors
/// Propagates a parse failure rather than ignoring it.
pub fn discover(start: &Path) -> Result<Option<Loaded>, ConfigError> {
    let mut dir = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    let mut manifest: Option<Loaded> = None;
    while let Some(current) = dir {
        // The manifest is recorded BEFORE the settings check: the two
        // usually sit in the same directory, and the walk would otherwise
        // return at the settings file without ever seeing it.
        if manifest.is_none() {
            let candidate = current.join(crate::MANIFEST_NAME);
            if candidate.is_file() {
                manifest = Some(load_manifest_policy(&candidate));
            }
        }
        for name in CONFIG_NAMES {
            let candidate = current.join(name);
            if candidate.is_file() {
                let settings = load(&candidate)?;
                let composed = match manifest {
                    Some(base) => Loaded::layered(base, settings),
                    None => settings,
                };
                return Ok(Some(composed));
            }
        }
        dir = current.parent();
    }
    Ok(manifest)
}

/// The manifest's `lint`/`validate` policy sections as a [`Loaded`] base
/// layer. Absent or malformed sections parse as absent settings — a
/// broken manifest is not silently swallowed (`suspect project check`
/// owns that surface), but policy it does not carry is not invented.
fn load_manifest_policy(path: &Path) -> Loaded {
    let policy: Option<serde_json::Value> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let dir = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    match policy {
        Some(policy) => Loaded::from_manifest_policy(&dir, &policy),
        None => Loaded {
            settings: crate::Settings::default(),
            path: None,
            root: dir,
        },
    }
}

/// Parses one configuration file.
///
/// # Errors
/// IO or parse failures, naming the file and the offending content.
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    let uri = suspect_source::Uri::from_path(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    let doc = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(text.into_bytes()));
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
    let resolve = |relative: String| -> PathBuf { root.join(relative) };
    let text_at = |section: &str, key: &str| -> Option<String> {
        doc.root()
            .get(section)?
            .get(key)?
            .as_str()
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
    };
    let bool_at = |section: &str, key: &str| -> bool {
        doc.root()
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "true")
    };

    Ok(Loaded {
        settings: Settings {
            lint: crate::LintSettings {
                ruleset: text_at("lint", "ruleset").map(resolve),
                min_severity: text_at("lint", "min_severity"),
            },
            validate: crate::ValidateSettings {
                strict_format: bool_at("validate", "strict_format"),
            },
            format: crate::FormatSettings {
                json: bool_at("format", "json"),
                yaml: bool_at("format", "yaml"),
            },
            docs: crate::DocsSettings {
                style: text_at("docs", "style"),
                out: text_at("docs", "out").map(resolve),
            },
            codegen: crate::CodegenSettings {
                out: text_at("codegen", "out").map(resolve),
                profile: text_at("codegen", "profile"),
                package_version: text_at("codegen", "package_version"),
            },
        },
        path: Some(path.to_path_buf()),
        root,
    })
}
