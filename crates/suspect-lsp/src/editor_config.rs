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
    // Relative client paths resolve where the file's own relative paths
    // do: the directory of the configuration that was found, else the
    // workspace root — never the server process's working directory.
    let base = loaded
        .as_ref()
        .map(|l| l.root.clone())
        .or_else(|| root.map(Path::to_path_buf));
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
        apply_client(&mut config, value, base.as_deref());
    }
    config
}

impl EditorConfig {
    /// The configured minimum severity, defaulting to `hint` (show
    /// everything). Accepts the LSP/`.suspect.yaml` spellings and the
    /// `"information"` spelling VS Code's settings enum uses, plus the
    /// `warn`/`info` short forms.
    #[must_use]
    pub fn min_severity(&self) -> tower_lsp::lsp_types::DiagnosticSeverity {
        match self.settings.lint.min_severity.as_deref() {
            Some("error") => tower_lsp::lsp_types::DiagnosticSeverity::ERROR,
            Some("warning") | Some("warn") => tower_lsp::lsp_types::DiagnosticSeverity::WARNING,
            Some("info") | Some("information") => {
                tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION
            }
            _ => tower_lsp::lsp_types::DiagnosticSeverity::HINT,
        }
    }
}

/// Layers the client's `suspect.*` initialization options over the file
/// settings. Only keys the client actually set are applied, so an unset
/// editor option never clears a committed file value. "Set" means a
/// non-blank string: VS Code reports an untouched string setting as its
/// default `""` (and a cleared one as `null`), and treating that as an
/// explicit choice silently replaced the committed `.suspect.yaml` value.
///
/// A relative `lint.ruleset` resolves against `base` (the configuration
/// root), matching how the file's own `lint.ruleset` resolves.
fn apply_client(config: &mut EditorConfig, value: &serde_json::Value, base: Option<&Path>) {
    let section = value.get("suspect").unwrap_or(value);
    let lint = |key: &str| section.get("lint").and_then(|l| l.get(key));
    if let Some(min_severity) = lint("minSeverity").and_then(set_string) {
        config.settings.lint.min_severity = Some(min_severity.to_owned());
    }
    if let Some(ruleset) = lint("ruleset").and_then(set_string) {
        let path = std::path::PathBuf::from(ruleset);
        config.settings.lint.ruleset = Some(match base {
            Some(base) if path.is_relative() => base.join(path),
            _ => path,
        });
    }
    if let Some(design) = lint("design").and_then(set_string) {
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

/// A client string setting that was actually set: present, a string, and
/// not blank. Returned trimmed.
fn set_string(value: &serde_json::Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
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
    fn blank_or_null_client_strings_do_not_override_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "lint:\n  min_severity: warning\n  ruleset: rules.yaml\n  design: warn\n",
        )
        .unwrap();
        // VS Code's defaults for untouched string settings, and a cleared one.
        for blank in [
            serde_json::json!(""),
            serde_json::json!("   "),
            serde_json::Value::Null,
        ] {
            let client = serde_json::json!({
                "suspect": {"lint": {"minSeverity": blank, "ruleset": blank, "design": blank}}
            });
            let config = for_workspace(Some(dir.path()), Some(&client));
            assert_eq!(
                config.settings.lint.min_severity.as_deref(),
                Some("warning"),
                "{blank}: a blank editor setting must not replace the file's floor"
            );
            assert_eq!(
                config.min_severity(),
                tower_lsp::lsp_types::DiagnosticSeverity::WARNING
            );
            assert_eq!(
                config.settings.lint.ruleset.as_deref(),
                Some(dir.path().join("rules.yaml").as_path()),
                "{blank}: a blank editor ruleset must not replace the file's"
            );
            assert_eq!(config.settings.lint.design.as_deref(), Some("warn"));
        }
    }

    #[test]
    fn a_relative_client_ruleset_resolves_against_the_config_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".suspect.yaml"), "lint: {}\n").unwrap();
        let client = serde_json::json!({"suspect": {"lint": {"ruleset": "policy/rules.yaml"}}});
        let config = for_workspace(Some(dir.path()), Some(&client));
        assert_eq!(
            config.settings.lint.ruleset.as_deref(),
            Some(dir.path().join("policy/rules.yaml").as_path()),
            "a relative editor ruleset resolves against the workspace, not the cwd"
        );

        // No configuration file: the workspace root is the base.
        let bare = tempfile::tempdir().unwrap();
        let config = for_workspace(Some(bare.path()), Some(&client));
        assert_eq!(
            config.settings.lint.ruleset.as_deref(),
            Some(bare.path().join("policy/rules.yaml").as_path())
        );

        // An absolute path is taken as given.
        let absolute = dir.path().join("elsewhere.yaml");
        let client = serde_json::json!({"suspect": {"lint": {"ruleset": absolute}}});
        let config = for_workspace(Some(dir.path()), Some(&client));
        assert_eq!(
            config.settings.lint.ruleset.as_deref(),
            Some(absolute.as_path())
        );
    }

    #[test]
    fn warn_is_an_alias_for_warning() {
        let mut config = EditorConfig::default();
        config.settings.lint.min_severity = Some("warn".to_owned());
        assert_eq!(
            config.min_severity(),
            tower_lsp::lsp_types::DiagnosticSeverity::WARNING
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
