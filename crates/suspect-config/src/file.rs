//! Settings files and manifests as layers.
//!
//! A malformed file is an error, never a silent absence: a committed
//! config that stops applying must be visible immediately.

use std::path::{Path, PathBuf};

use crate::layer::{Layer, Notice, Origin};

/// File names searched, in order.
pub const CONFIG_NAMES: &[&str] = &[
    ".suspect.yaml",
    ".suspect.yml",
    "suspect.yaml",
    "suspect.yml",
];

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
    // Every section present is told once, even an empty one, so each
    // project `project init` made before this release hears about it.
    for entry in root.entries() {
        if matches!(entry.key, "lint" | "validate" | "editor") {
            layer.warnings.push(Notice {
                origin: origin(entry.key_node.line_col().0 + 1),
                message: manifest_policy_warning(entry.key),
            });
        }
    }
    present.then_some(layer)
}

/// The deprecation text for a manifest policy section.
#[must_use]
pub fn manifest_policy_warning(section: &str) -> String {
    format!(
        "`{section}` in a project manifest is deprecated and is ignored from the next \
         release; move it to .suspect.yaml and add \"config\": \".suspect.yaml\" to the \
         manifest (`suspect config migrate` does both)"
    )
}
