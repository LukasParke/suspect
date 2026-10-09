//! Workspace configuration for the language server.
//!
//! The server used to learn about `lint.min_severity` and friends only
//! from client `initializationOptions`, so a project committed a
//! `.suspect.yaml` that the editor ignored. Settings now come from the one
//! shared schema in [`suspect_config`], read from the workspace root, with
//! the client's initialization options applied *on top* — so an explicit
//! editor setting still wins, which is the same precedence rule the CLI
//! enforces.

use std::path::Path;

use suspect_config::{Loaded, Settings};

/// Editor-side configuration: shared settings plus the LSP-only sections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditorConfig {
    /// The shared toolchain settings, from `.suspect.yaml` and the client.
    pub settings: Settings,
    /// Where the settings were loaded from, for diagnostics.
    pub source: Option<String>,
    /// Why a configuration file was not applied, when one exists but does
    /// not load. Surfaced in the editor so a broken committed config is
    /// visible instead of silently ignored.
    pub error: Option<String>,
}

/// Reads `.suspect.yaml` from the workspace root, then layers the
/// client's `initializationOptions` on top.
///
/// A malformed file is reported through `source` rather than aborting the
/// session: the editor still works, with built-in defaults.
#[must_use]
pub fn for_workspace(
    root: Option<&Path>,
    client_overrides: Option<&serde_json::Value>,
) -> EditorConfig {
    let mut error = None;
    let mut loaded = match root {
        Some(root) => match suspect_config::discover(root) {
            Ok(found) => found,
            // Report the failure and keep working on built-in defaults.
            Err(failure) => {
                error = Some(failure.0);
                None
            }
        },
        None => None,
    };
    if loaded.is_none() {
        loaded = Some(Loaded::default());
    }
    let mut config = EditorConfig {
        settings: loaded
            .as_ref()
            .map_or_else(Settings::default, |l| l.settings.clone()),
        source: loaded.as_ref().and_then(|l| l.path.as_ref()).map(|p| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        }),
        error,
    };

    if let Some(value) = client_overrides {
        apply_client(&mut config, value);
    }
    config
}

impl EditorConfig {
    /// The configured minimum severity, defaulting to `hint` (show
    /// everything). Accepts the LSP/`.suspect.yaml` spellings and the
    /// `"information"` spelling VS Code's settings enum uses.
    #[must_use]
    pub fn min_severity(&self) -> tower_lsp::lsp_types::DiagnosticSeverity {
        match self.settings.lint.min_severity.as_deref() {
            Some("error") => tower_lsp::lsp_types::DiagnosticSeverity::ERROR,
            Some("warning") => tower_lsp::lsp_types::DiagnosticSeverity::WARNING,
            Some("info") | Some("information") => {
                tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION
            }
            _ => tower_lsp::lsp_types::DiagnosticSeverity::HINT,
        }
    }
}

/// Layers the client's `suspect.*` initialization options over the file
/// settings. Only keys the client actually set are applied, so an unset
/// editor option never clears a committed file value.
fn apply_client(config: &mut EditorConfig, value: &serde_json::Value) {
    let section = value.get("suspect").unwrap_or(value);
    if let Some(min_severity) = section
        .get("lint")
        .and_then(|l| l.get("minSeverity"))
        .and_then(|v| v.as_str())
    {
        config.settings.lint.min_severity = Some(min_severity.to_owned());
    }
    if let Some(ruleset) = section
        .get("lint")
        .and_then(|l| l.get("ruleset"))
        .and_then(|v| v.as_str())
    {
        config.settings.lint.ruleset = Some(std::path::PathBuf::from(ruleset));
    }
    if let Some(design) = section
        .get("lint")
        .and_then(|l| l.get("design"))
        .and_then(|v| v.as_str())
    {
        config.settings.lint.design = Some(design.to_owned());
    }
    if let Some(strict) = section
        .get("validate")
        .and_then(|v| v.get("strictFormat"))
        .and_then(|v| v.as_bool())
    {
        config.settings.validate.strict_format = strict;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_settings_are_read_from_the_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\nvalidate:\n  strict_format: true\n",
        )
        .unwrap();
        let config = for_workspace(Some(dir.path()), None);
        assert_eq!(
            config.settings.lint.min_severity.as_deref(),
            Some("warning")
        );
        assert!(config.settings.validate.strict_format);
        assert_eq!(config.source.as_deref(), Some(".suspect.yaml"));
    }

    #[test]
    fn client_options_win_over_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\n",
        )
        .unwrap();
        let client = serde_json::json!({
            "suspect": {"lint": {"minSeverity": "error"}}
        });
        let config = for_workspace(Some(dir.path()), Some(&client));
        assert_eq!(
            config.settings.lint.min_severity.as_deref(),
            Some("error"),
            "an explicit editor setting beats the file"
        );
    }

    #[test]
    fn a_client_option_absent_from_the_payload_does_not_clear_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\n",
        )
        .unwrap();
        // The client sets an unrelated key.
        let client = serde_json::json!({"suspect": {"inlayHints": {"refs": false}}});
        let config = for_workspace(Some(dir.path()), Some(&client));
        assert_eq!(
            config.settings.lint.min_severity.as_deref(),
            Some("warning"),
            "an unset editor option must not clear a committed value"
        );
        // The client's inlayHints section flows through the shared config
        // schema, not the editor overlay.
        let shared = crate::config_files::parse_config(&client).expect("parses");
        assert!(!shared.inlay_refs());
    }

    #[test]
    fn the_vscode_information_spelling_maps_to_information() {
        let mut config = EditorConfig::default();
        config.settings.lint.min_severity = Some("information".to_owned());
        assert_eq!(
            config.min_severity(),
            tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION
        );
        config.settings.lint.min_severity = Some("info".to_owned());
        assert_eq!(
            config.min_severity(),
            tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION
        );
    }

    #[test]
    fn a_malformed_file_leaves_working_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".suspect.yaml"), "lint: [unclosed\n").unwrap();
        let config = for_workspace(Some(dir.path()), None);
        assert!(config.settings.lint.min_severity.is_none());
        assert!(
            config.error.is_some(),
            "a broken config must say so rather than silently vanish"
        );
    }

    #[test]
    fn no_workspace_yields_defaults() {
        let config = for_workspace(None, None);
        assert_eq!(config.settings, Settings::default());
        assert!(config.source.is_none());
    }
}
