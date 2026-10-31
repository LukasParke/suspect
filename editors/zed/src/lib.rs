//! suspect-zed: Zed extension that wires the `suspect` language server into
//! YAML and JSON buffers.
//!
//! The server binary is resolved in this order:
//!
//! 1. `lsp.suspect-lsp.binary.path` (plus optional `arguments` / `env`) from
//!    the user's Zed settings;
//! 2. the release build matching this extension's version, downloaded once
//!    from the GitHub release `v{version}` into the extension's work
//!    directory and checked against the release's `.sha256` before it is
//!    unpacked (`download.rs`);
//! 3. `suspect` found on the worktree's shell `PATH` (`Worktree::which`):
//!    platforms without a release build, or no network on first start.
//!
//! Extensions run in a WASI sandbox: the host's `PATH` and files are not
//! visible to `std::env` / `std::fs`, but the extension's own work directory
//! is, which is where the download is cached (`suspect-{version}/`).
//!
//! Workspace configuration is forwarded from `lsp.suspect-lsp.settings`
//! under the `suspect` section the server reads
//! (`{ "suspect": { "lint": .., "ref": .., "editor": .. } }`).
//!
//! The initialization options carry `extensionVersion`, which the server
//! compares with its own version (it warns once on a major or minor
//! mismatch), plus whatever the user set in
//! `lsp.suspect-lsp.initialization_options`.

mod download;

use std::fs;

use zed_extension_api::{
    self as zed, http_client::HttpMethod, http_client::HttpRequest, http_client::RedirectPolicy,
    settings::LspSettings, Command, LanguageServerId, LanguageServerInstallationStatus, Result,
    Worktree,
};

/// The executable name searched on the worktree's `PATH`.
const BINARY: &str = "suspect";
/// The repository whose releases carry the binaries.
const REPOSITORY: &str = "LukasParke/suspect";
/// Where the binary for this version is unpacked, under the work directory.
const VERSION_DIR: &str = concat!("suspect-", env!("CARGO_PKG_VERSION"));

/// The Zed extension entry point.
struct SuspectExtension {
    /// The managed binary once it is in place (relative to the work dir).
    cached: Option<String>,
}

impl zed::Extension for SuspectExtension {
    fn new() -> Self {
        Self { cached: None }
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

    /// Spawns `suspect lsp`: a configured binary, else the managed
    /// download, else `suspect` on `PATH`.
    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let binary = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);
        let args = binary
            .as_ref()
            .and_then(|b| b.arguments.clone())
            .unwrap_or_else(|| vec!["lsp".to_owned()]);

        if let Some(path) = binary
            .as_ref()
            .and_then(|b| b.path.clone())
            .filter(|p| !p.trim().is_empty())
        {
            let env = binary
                .as_ref()
                .and_then(|b| b.env.clone())
                .unwrap_or_default();
            return Ok(Command::new(path).args(args).envs(env));
        }

        let managed = self.managed_binary(language_server_id);
        if let Ok(path) = &managed {
            return Ok(Command::new(path).args(args).envs(worktree.shell_env()));
        }

        if let Some(path) = worktree.which(BINARY) {
            zed::set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::None,
            );
            return Ok(Command::new(path).args(args).envs(worktree.shell_env()));
        }

        let reason = managed.err().unwrap_or_default();
        let message = format!(
            "The suspect {} release binary is unavailable ({reason}), and `{BINARY}` is not on \
             the worktree's PATH. Install the suspect CLI (see \
             https://github.com/{REPOSITORY}#install) or set `lsp.{}.binary.path` in your Zed \
             settings to the executable.",
            env!("CARGO_PKG_VERSION"),
            language_server_id.as_ref()
        );
        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Failed(message.clone()),
        );
        Err(message)
    }
}

impl SuspectExtension {
    /// The release binary for this extension version: reused when already
    /// unpacked, else downloaded, checked against its `.sha256`, unpacked
    /// and made executable. Older versions' directories are removed.
    fn managed_binary(&mut self, id: &LanguageServerId) -> Result<String> {
        let (os, arch) = zed::current_platform();
        let exe = format!("{VERSION_DIR}/{}", download::exe_name(os));
        if let Some(path) = &self.cached {
            if is_file(path) {
                return Ok(path.clone());
            }
        }
        if is_file(&exe) {
            self.cached = Some(exe.clone());
            return Ok(exe);
        }
        let triple = download::triple(os, arch)
            .ok_or_else(|| format!("no release build for {os:?} {arch:?}"))?;
        zed::set_language_server_installation_status(
            id,
            &LanguageServerInstallationStatus::Downloading,
        );
        let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
        let release = zed::github_release_by_tag_name(REPOSITORY, &tag)
            .map_err(|e| format!("release {tag}: {e}"))?;
        let (archive, kind) = download::asset_name(triple);
        let url_of = |name: &str| {
            release
                .assets
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.download_url.clone())
                .ok_or_else(|| format!("release {tag} has no {name}"))
        };
        let checksum = fetch(&url_of(&format!("{archive}.sha256"))?)?;
        let want = download::parse_sha256(&String::from_utf8_lossy(&checksum))?;
        let bytes = fetch(&url_of(&archive)?)?;
        download::verify_sha256(&bytes, &want).map_err(|e| format!("{archive}: {e}"))?;
        let binary = download::extract_binary(&bytes, kind, download::exe_name(os))?;

        // Unpack beside the final directory, then move it in, so an
        // interrupted start never leaves a half-written binary to reuse.
        let partial = format!("{VERSION_DIR}.partial");
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(&partial).map_err(|e| format!("cannot create {partial}: {e}"))?;
        let staged = format!("{partial}/{}", download::exe_name(os));
        fs::write(&staged, &binary).map_err(|e| format!("cannot write {staged}: {e}"))?;
        let _ = fs::remove_dir_all(VERSION_DIR);
        fs::rename(&partial, VERSION_DIR)
            .map_err(|e| format!("cannot move {partial} to {VERSION_DIR}: {e}"))?;
        zed::make_file_executable(&exe)?;
        remove_other_versions();
        zed::set_language_server_installation_status(id, &LanguageServerInstallationStatus::None);
        self.cached = Some(exe.clone());
        Ok(exe)
    }
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// GET `url`, following GitHub's redirect to its asset storage.
fn fetch(url: &str) -> Result<Vec<u8>> {
    let request = HttpRequest::builder()
        .method(HttpMethod::Get)
        .url(url)
        .header("Accept", "application/octet-stream")
        .redirect_policy(RedirectPolicy::FollowAll)
        .build()?;
    request
        .fetch()
        .map(|response| response.body)
        .map_err(|e| format!("GET {url}: {e}"))
}

/// Removes `suspect-*` directories of other versions from the work dir.
fn remove_other_versions() {
    let Ok(entries) = fs::read_dir(".") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("suspect-") && name != VERSION_DIR {
            let _ = fs::remove_dir_all(entry.path());
        }
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

    #[test]
    fn the_cache_directory_names_this_version() {
        assert_eq!(
            VERSION_DIR,
            format!("suspect-{}", env!("CARGO_PKG_VERSION"))
        );
    }
}
