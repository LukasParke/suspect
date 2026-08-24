//! suspect-zed: Zed extension that wires the `suspect` language server into
//! YAML and JSON buffers.
//!
//! The extension never ships its own binary: it locates the `suspect` CLI on
//! the host (honoring a `SUSPECT_PATH` override) and spawns `suspect lsp`
//! over stdio. Workspace configuration is forwarded from the user's Zed
//! `lsp.suspect-lsp.settings` under the `suspect` section shape the server
//! expects (`{ "suspect": { "basePath", "testBaseUrl" } }`).

use std::path::PathBuf;

use zed_extension_api::{
    self as zed, settings::LspSettings, Command, LanguageServerId, Result, Worktree,
};

/// The Zed extension entry point.
struct SuspectExtension;

impl zed::Extension for SuspectExtension {
    fn new() -> Self {
        Self
    }

    /// Configuration sent during `initialize` and refreshed on every
    /// `workspace/didChangeConfiguration`.
    ///
    /// Reads the user's per-worktree `lsp.suspect-lsp.settings` and re-wraps
    /// the two keys the suspect server understands into the `{ "suspect": .. }`
    /// section it expects. Unset keys serialize as `null`.
    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .unwrap_or_default();

        Ok(Some(zed::serde_json::json!({
            "suspect": {
                "basePath": settings.get("basePath"),
                "testBaseUrl": settings.get("testBaseUrl"),
            }
        })))
    }

    /// Locates the `suspect` CLI: `SUSPECT_PATH` wins outright, otherwise the
    /// binary is searched along `PATH`. Spawned as `suspect lsp`.
    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> Result<Command> {
        let path = locate_binary("suspect")?;
        Ok(Command::new(path.to_string_lossy().into_owned()).arg("lsp"))
    }
}

/// Resolves an executable name to a full path.
///
/// Order of precedence:
/// 1. the `SUSPECT_PATH` environment variable, taken verbatim;
/// 2. the first directory on `PATH` containing an executable `<name>`.
fn locate_binary(name: &str) -> Result<PathBuf> {
    if let Some(override_path) = std::env::var_os("SUSPECT_PATH") {
        if !override_path.is_empty() {
            return Ok(PathBuf::from(override_path));
        }
    }

    let search_path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&search_path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "`{name}` was not found on PATH; install the suspect CLI \
         (`cargo install --path crates/suspect-cli`) or point the \
         SUSPECT_PATH environment variable at the binary"
    )
    .into())
}

/// Returns whether `path` exists and is executable by its owner/group/others
/// on unix; other platforms only require regular-file existence.
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => meta.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

zed::register_extension!(SuspectExtension);
