//! Which files configure a start directory.
//!
//! Walk up from the start directory. In each directory: a manifest is
//! remembered when it is the first one seen; a settings file
//! ([`crate::CONFIG_NAMES`]) is used and ends the walk; else a manifest
//! naming its settings file (`"config": ".suspect.yaml"`) is used and ends
//! the walk. Without either, the walk runs to the filesystem root.

use std::path::{Path, PathBuf};

/// What [`discover`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The settings file in effect.
    pub config: Option<PathBuf>,
    /// The nearest project manifest at or below the settings file.
    pub manifest: Option<PathBuf>,
}

/// The directory a walk starts in: `start` itself when it is a directory,
/// else its parent; made absolute so the walk can climb above it.
#[must_use]
pub fn start_dir(start: &Path) -> PathBuf {
    let start = if start.as_os_str().is_empty() {
        Path::new(".")
    } else {
        start
    };
    let absolute = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    let dir = if absolute.is_dir() {
        absolute
    } else {
        absolute
            .parent()
            .map_or_else(|| absolute.clone(), Path::to_path_buf)
    };
    normalize(&dir)
}

/// Drops `.` components and folds `..` lexically, so displayed paths read
/// like the ones the user typed.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Walks up from `start` (a file or a directory).
#[must_use]
pub fn discover(start: &Path) -> Found {
    let mut found = Found::default();
    let mut dir = Some(start_dir(start));
    while let Some(current) = dir {
        let manifest = current.join(crate::MANIFEST_NAME);
        let manifest = manifest.is_file().then_some(manifest);
        if found.manifest.is_none() {
            found.manifest.clone_from(&manifest);
        }
        if let Some(config) = crate::CONFIG_NAMES
            .iter()
            .map(|name| current.join(name))
            .find(|candidate| candidate.is_file())
        {
            found.config = Some(config);
            return found;
        }
        if let Some(manifest) = &manifest
            && let Some(config) = manifest_config(manifest)
        {
            found.config = Some(normalize(&current.join(config)));
            return found;
        }
        dir = current.parent().map(Path::to_path_buf);
    }
    found
}

/// The manifest's `"config"` value, when it names a settings file.
#[must_use]
pub fn manifest_config(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("config")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_settings_file_ends_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("api/v1")).unwrap();
        std::fs::write(root.join(".suspect.yaml"), "").unwrap();
        std::fs::write(root.join("api/.suspect.yaml"), "").unwrap();
        std::fs::write(root.join("api/suspect.project.json"), "{}").unwrap();
        let found = discover(&root.join("api/v1/openapi.yaml"));
        assert_eq!(found.config, Some(root.join("api/.suspect.yaml")));
        assert_eq!(found.manifest, Some(root.join("api/suspect.project.json")));
    }

    #[test]
    fn a_manifest_naming_its_config_ends_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("svc/config")).unwrap();
        std::fs::write(root.join(".suspect.yaml"), "").unwrap();
        std::fs::write(
            root.join("svc/suspect.project.json"),
            r#"{"config": "config/suspect.yaml"}"#,
        )
        .unwrap();
        let found = discover(&root.join("svc"));
        assert_eq!(found.config, Some(root.join("svc/config/suspect.yaml")));
    }

    #[test]
    fn a_manifest_without_config_keeps_walking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("svc")).unwrap();
        std::fs::write(root.join(".suspect.yaml"), "").unwrap();
        std::fs::write(root.join("svc/suspect.project.json"), "{}").unwrap();
        let found = discover(&root.join("svc/openapi.yaml"));
        assert_eq!(found.config, Some(root.join(".suspect.yaml")));
        assert_eq!(found.manifest, Some(root.join("svc/suspect.project.json")));
    }
}
