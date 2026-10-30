//! `.suspect.yaml` discovery and parsing.
//!
//! A malformed file is an error, never a silent absence: a committed
//! config that stops applying must be visible immediately.

use std::path::{Path, PathBuf};

use crate::layer::{Layer, Notice, Origin};
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
            explicit: crate::ExplicitBools::default(),
        },
    }
}

/// Parses `path` into a document, or a notice naming the first problem.
fn parse_doc(path: &Path, origin: impl Fn(u32) -> Origin) -> Result<suspect_low::LowDoc, Notice> {
    let text = std::fs::read_to_string(path).map_err(|e| Notice {
        origin: origin(1),
        message: format!("cannot read: {e}"),
    })?;
    let uri = suspect_source::Uri::from_path(path).map_err(|e| Notice {
        origin: origin(1),
        message: e.to_string(),
    })?;
    let doc = suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(text.into_bytes()));
    if let Some(error) = doc.syntax_errors().first() {
        let inner = doc.inner();
        let line = inner
            .line_index()
            .line_col(inner.bytes(), error.range.start)
            .0
            + 1;
        return Err(Notice {
            origin: origin(line),
            message: error.message.clone(),
        });
    }
    Ok(doc)
}

fn file_origin(path: &Path) -> impl Fn(u32) -> Origin + '_ {
    move |line| Origin::File {
        path: path.to_path_buf(),
        line,
    }
}

/// One settings file as a layer, plus the parent its `extends` names.
///
/// # Errors
/// An unreadable or syntactically broken file: the whole layer is dropped.
pub(crate) fn settings_layer(path: &Path) -> Result<(Layer, Option<PathBuf>), Notice> {
    let doc = parse_doc(path, file_origin(path))?;
    let base = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut layer = Layer::default();
    layer.read_node(&doc.root(), "", &base, &file_origin(path));
    let extends = match layer.entries.remove("extends") {
        Some((crate::layer::Raw::Path(parent), _)) => Some(crate::discover::normalize(&parent)),
        _ => None,
    };
    Ok((layer, extends))
}

/// The file an `extends` key names, as the layer under the extending file.
/// One level only: the parent may not extend again.
///
/// # Errors
/// A missing or broken parent, a cycle, or a second level.
pub(crate) fn extended_layer(child: &Path, parent: &Path) -> Result<Layer, Notice> {
    let at = Origin::File {
        path: child.to_path_buf(),
        line: 1,
    };
    if crate::discover::normalize(parent) == crate::discover::normalize(child) {
        return Err(Notice {
            origin: at,
            message: "`extends` names this file itself".to_owned(),
        });
    }
    if !parent.is_file() {
        return Err(Notice {
            origin: at,
            message: format!("`extends`: {} does not exist", parent.display()),
        });
    }
    let (layer, again) = settings_layer(parent)?;
    if again.is_some() {
        return Err(Notice {
            origin: Origin::File {
                path: parent.to_path_buf(),
                line: 1,
            },
            message: "`extends` is one level only; this file is itself extended".to_owned(),
        });
    }
    Ok(layer)
}

/// The deprecated policy sections of a manifest (`lint`, `validate`,
/// `editor`) as the lowest file layer, with the settings file's key
/// spellings. `None` when the manifest carries none of them or does not
/// parse (`suspect project check` reports a broken manifest).
pub(crate) fn manifest_layer(path: &Path) -> Option<Layer> {
    let origin = |line| Origin::Manifest {
        path: path.to_path_buf(),
        line,
    };
    let doc = parse_doc(path, origin).ok()?;
    let root = doc.root();
    let base = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut layer = Layer::default();
    let mut present = false;
    for entry in root.entries() {
        let Some(value) = entry.value else { continue };
        match entry.key {
            "lint" | "validate" => {
                present = true;
                layer.read_node(&value, entry.key, &base, &origin);
            }
            "editor" => {
                present = true;
                for sub in value.entries() {
                    let Some(node) = sub.value else { continue };
                    let scope = match sub.key {
                        "inlay_hints" | "inlayHints" => "editor.inlay_hints",
                        "ref" | "refs" => "ref",
                        "formatting" => {
                            layer.sections.push("format");
                            "format"
                        }
                        _ => continue,
                    };
                    layer.read_node(&node, scope, &base, &origin);
                }
            }
            _ => {}
        }
    }
    // Spelling notices belong to the settings file; `project check` and the
    // editor's schema check cover the manifest.
    layer.warnings.clear();
    present.then_some(layer)
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
    // A YAML/JSON boolean (`true`, `False`, ...); a quoted `"true"` or
    // `"false"` is accepted too, as earlier files spelled it that way.
    // `None` when the key is absent, so a layer can say `false` explicitly.
    let bool_at = |section: &str, key: &str| -> Option<bool> {
        let value = doc.root().get(section)?.get(key)?;
        value.as_bool().or_else(|| match value.as_str()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    };
    let strict_format = bool_at("validate", "strict_format");
    let format_json = bool_at("format", "json");
    let format_yaml = bool_at("format", "yaml");
    // `lint.rules`: rule id → severity, as a plain mapping.
    let lint_rules = doc
        .root()
        .get("lint")
        .and_then(|l| l.get("rules"))
        .map(|rules| {
            rules
                .entries()
                .into_iter()
                .filter_map(|entry| {
                    entry
                        .value
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| (entry.key.to_owned(), s.to_ascii_lowercase()))
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Loaded {
        settings: Settings {
            lint: crate::LintSettings {
                ruleset: text_at("lint", "ruleset").map(resolve),
                min_severity: text_at("lint", "min_severity"),
                design: text_at("lint", "design"),
                rules: lint_rules,
                ..crate::LintSettings::default()
            },
            validate: crate::ValidateSettings {
                strict_format: strict_format.unwrap_or(false),
                ..crate::ValidateSettings::default()
            },
            format: crate::FormatSettings {
                json: format_json.unwrap_or(false),
                yaml: format_yaml.unwrap_or(false),
                ..crate::FormatSettings::default()
            },
            docs: crate::DocsSettings {
                style: text_at("docs", "style"),
                output: text_at("docs", "out").map(resolve),
            },
            codegen: crate::CodegenSettings {
                out: text_at("codegen", "out").map(resolve),
                profile: text_at("codegen", "profile"),
                package_version: text_at("codegen", "package_version"),
            },
            ..Settings::default()
        },
        path: Some(path.to_path_buf()),
        root,
        explicit: crate::ExplicitBools {
            strict_format,
            format_json,
            format_yaml,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_text(text: &str) -> Loaded {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".suspect.yaml");
        std::fs::write(&path, text).expect("write");
        load(&path).expect("load")
    }

    #[test]
    fn booleans_parse_as_yaml_booleans() {
        let loaded = load_text("validate:\n  strict_format: True\nformat:\n  json: false\n");
        assert!(loaded.settings.validate.strict_format);
        assert_eq!(loaded.explicit.strict_format, Some(true));
        assert!(!loaded.settings.format.json);
        assert_eq!(loaded.explicit.format_json, Some(false));
        assert_eq!(loaded.explicit.format_yaml, None, "absent stays unset");
    }

    #[test]
    fn json_style_booleans_parse() {
        let loaded = load_text("{\"validate\": {\"strict_format\": true}}\n");
        assert!(loaded.settings.validate.strict_format);
    }

    #[test]
    fn a_settings_file_false_overrides_a_manifest_true() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(crate::MANIFEST_NAME),
            r#"{"validate": {"strict_format": true}}"#,
        )
        .expect("manifest");
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "validate:\n  strict_format: false\n",
        )
        .expect("settings");
        let found = discover(dir.path().join("openapi.yaml").as_path())
            .expect("discover")
            .expect("found");
        assert!(!found.settings.validate.strict_format);
        assert_eq!(found.explicit.strict_format, Some(false));
    }

    #[test]
    fn an_absent_key_leaves_the_manifest_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(crate::MANIFEST_NAME),
            r#"{"validate": {"strict_format": true}}"#,
        )
        .expect("manifest");
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warn\n",
        )
        .expect("settings");
        let found = discover(dir.path().join("openapi.yaml").as_path())
            .expect("discover")
            .expect("found");
        assert!(found.settings.validate.strict_format);
    }
}
