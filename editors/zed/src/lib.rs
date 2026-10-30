//! suspect-zed: Zed extension that wires the `suspect` language server into
//! YAML and JSON buffers.
//!
//! The extension never ships its own binary. It resolves the `suspect` CLI
//! through Zed's host APIs (extensions run in a WASI sandbox, so the host's
//! `PATH` and filesystem are not visible to `std::env` / `std::fs`):
//!
//! 1. `lsp.suspect-lsp.binary.path` (plus optional `arguments` / `env`) from
//!    the user's Zed settings;
//! 2. `suspect` found on the worktree's shell `PATH` (`Worktree::which`).
//!
//! Workspace configuration is forwarded from `lsp.suspect-lsp.settings`
//! under the `suspect` section the server reads
//! (`{ "suspect": { "lint": .., "ref": .., "inlayHints": .. } }`).
//!
//! The initialization options carry `extensionVersion`, which the server
//! compares with its own version (it warns once on a major or minor
//! mismatch), plus whatever the user set in
//! `lsp.suspect-lsp.initialization_options`.

use zed_extension_api::{
    self as zed, settings::LspSettings, Command, LanguageServerId, Result, Worktree,
};

/// The executable name searched on the worktree's `PATH`.
const BINARY: &str = "suspect";

/// The Zed extension entry point.
struct SuspectExtension;

impl zed::Extension for SuspectExtension {
    fn new() -> Self {
        Self
    }

    /// Configuration sent during `initialize` and refreshed on every
    /// `workspace/didChangeConfiguration`.
    ///
    /// Forwards the user's per-worktree `lsp.suspect-lsp.settings` verbatim
    /// under the `{ "suspect": .. }` section. Settings that are already
    /// wrapped in a `suspect` key are passed through unchanged.
    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .unwrap_or_else(|| zed::serde_json::json!({}));
        if settings.get("suspect").is_some() {
            return Ok(Some(settings));
        }
        Ok(Some(zed::serde_json::json!({ "suspect": settings })))
    }

    /// The user's `lsp.suspect-lsp.initialization_options` with this
    /// extension's version added, for the server's version check.
    fn language_server_initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let user = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.initialization_options);
        Ok(Some(with_extension_version(user)))
    }

    /// Spawns `suspect lsp`, honoring a configured binary first.
    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let binary = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);

        if let Some(path) = binary
            .as_ref()
            .and_then(|b| b.path.clone())
            .filter(|p| !p.trim().is_empty())
        {
            let args = binary
                .as_ref()
                .and_then(|b| b.arguments.clone())
                .unwrap_or_else(|| vec!["lsp".to_owned()]);
            let env = binary
                .as_ref()
                .and_then(|b| b.env.clone())
                .unwrap_or_default();
            return Ok(Command::new(path).args(args).envs(env));
        }

        if let Some(path) = worktree.which(BINARY) {
            let args = binary
                .and_then(|b| b.arguments)
                .unwrap_or_else(|| vec!["lsp".to_owned()]);
            return Ok(Command::new(path).args(args).envs(worktree.shell_env()));
        }

        Err(format!(
            "`{BINARY}` was not found on the worktree's PATH. Install the suspect CLI \
             (`cargo install --path crates/suspect-cli`) or set \
             `lsp.{}.binary.path` in your Zed settings to the executable.",
            language_server_id.as_ref()
        ))
    }
}

/// `options` (an object, or nothing) with `extensionVersion` set to this
/// extension's version unless the user already set one.
fn with_extension_version(options: Option<zed::serde_json::Value>) -> zed::serde_json::Value {
    let mut options = match options {
        Some(zed::serde_json::Value::Object(map)) => map,
        _ => zed::serde_json::Map::new(),
    };
    options
        .entry("extensionVersion")
        .or_insert_with(|| env!("CARGO_PKG_VERSION").into());
    zed::serde_json::Value::Object(options)
}

zed::register_extension!(SuspectExtension);

#[cfg(test)]
mod tests {
    use super::*;
    use zed::serde_json::json;

    #[test]
    fn initialization_options_carry_the_extension_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            with_extension_version(None),
            json!({ "extensionVersion": version })
        );
        assert_eq!(
            with_extension_version(Some(json!({ "run": { "baseUrl": "http://x" } }))),
            json!({ "run": { "baseUrl": "http://x" }, "extensionVersion": version })
        );
        assert_eq!(
            with_extension_version(Some(json!({ "extensionVersion": "9.9.9" }))),
            json!({ "extensionVersion": "9.9.9" })
        );
    }
}
