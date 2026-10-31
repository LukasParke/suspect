//! Configuration for one invocation, read through
//! [`suspect_config::resolve_with`] — the same layers, precedence and
//! discovery the language server uses.
//!
//! Discovery starts at the first input: the nearest settings file above
//! it governs the run. When nothing configures the input, the working
//! directory's configuration applies instead.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use suspect_config::{Inputs, Layer, Resolved};

/// Where discovery starts for `inputs`: the first input, or the working
/// directory when there is none or nothing configures the input.
#[must_use]
pub fn start(inputs: &[PathBuf]) -> PathBuf {
    match inputs.first() {
        Some(first) => {
            let found = suspect_config::discover::discover(first);
            if found.config.is_some() || found.manifest.is_some() {
                first.clone()
            } else {
                PathBuf::from(".")
            }
        }
        None => PathBuf::from("."),
    }
}

/// The configuration for a run over `inputs` with `flags` on top. Prints
/// deprecations and unknown keys on stderr, and a warning for each later
/// input that a different settings file governs.
///
/// # Errors
/// Any rejected value or unreadable settings file, each with `file:line`;
/// the binary exits 2.
pub fn resolve(inputs: &[PathBuf], flags: Layer) -> anyhow::Result<Resolved> {
    let start = start(inputs);
    let resolved = suspect_config::resolve_with(
        &start,
        Inputs {
            flags,
            ..Inputs::default()
        },
    )
    .map_err(|error| {
        emit_warnings(&error.resolved);
        anyhow::anyhow!("{error}")
    })?;
    emit_warnings(&resolved);
    for other in inputs.iter().skip(1) {
        let governed = suspect_config::discover::discover(other).config;
        if governed.is_some() && governed != resolved.config {
            warn_once(format!(
                "{} is governed by {}; using {} for this run",
                other.display(),
                shown(governed.as_deref()),
                shown(resolved.config.as_deref()),
            ));
        }
    }
    Ok(resolved)
}

/// [`resolve`] for one input.
///
/// # Errors
/// As [`resolve`].
pub fn resolve_one(input: &Path, flags: Layer) -> anyhow::Result<Resolved> {
    resolve(&[input.to_path_buf()], flags)
}

/// The nearest settings file at or above `dir`, by file name only.
#[must_use]
pub fn settings_file_above(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(suspect_config::discover::start_dir(dir));
    while let Some(dir) = current {
        if let Some(found) = suspect_config::CONFIG_NAMES
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
        {
            return Some(found);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    None
}

fn shown(path: Option<&Path>) -> String {
    path.map_or_else(
        || "the built-in defaults".to_owned(),
        |p| suspect_config::layer::shown(p).display().to_string(),
    )
}

/// Prints each warning once per process, on stderr: never on stdout, so
/// JSON and SARIF reports keep their shape.
pub fn emit_warnings(resolved: &Resolved) {
    for notice in &resolved.warnings {
        warn_once(notice.to_string());
    }
}

fn warn_once(message: String) {
    static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let fresh = SEEN
        .lock()
        .map(|mut seen| seen.insert(message.clone()))
        .unwrap_or(true);
    if fresh {
        eprintln!("warning: {message}");
    }
}
