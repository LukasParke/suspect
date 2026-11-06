//! Lifecycle handlers: `initialize`, `initialized`, and configuration and workspace-folder changes.

use std::sync::Arc;

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::{config_files, editor_config};

impl Backend {
    /// Serves `initialize`.
    pub(crate) async fn handle_initialize(
        &self,
        params: InitializeParams,
    ) -> JsonRpcResult<InitializeResult> {
        {
            let mut st = self.state.write().await;
            st.root = params.root_uri.as_ref().and_then(|u| u.to_file_path().ok());
            st.init_options = params.initialization_options.clone();
            st.client_caps = Some(params.capabilities.clone());
            if st.workspace.is_none()
                && let Some(root) = &st.root
                && let Ok(ws) = suspect_ref::WorkspaceBuilder::new().root(root).build()
            {
                // The document cap (`suspect.ref.maxDocs`) applies on
                // rebuilds in `any_workspace`; at initialize time the
                // merged configuration has not arrived yet.
                let _ = ws.load_all("main.yaml");
                st.workspace = Some(Arc::new(ws));
            }
        }
        Ok(InitializeResult {
            capabilities: super::capabilities::server_capabilities(),
            server_info: Some(ServerInfo {
                name: "suspect-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    /// Serves `initialized`.
    pub(crate) async fn handle_initialized(&self) {
        // Fetch merged config when the client supports workspace/configuration.
        let cfg_json = match self
            .client
            .configuration(vec![ConfigurationItem {
                scope_uri: None,
                section: Some("suspect".to_owned()),
            }])
            .await
        {
            Ok(values) => values.into_iter().next(),
            Err(_) => None,
        };
        // The shared schema: the workspace's `.suspect.yaml` is the base,
        // and the client's settings layer on top (an explicit editor
        // setting still wins — the same precedence the CLI enforces).
        let file_config = {
            let st = self.state.read().await;
            editor_config::for_workspace(st.workspace_root().as_deref(), cfg_json.as_ref())
        };
        let client_cfg = cfg_json.as_ref().and_then(config_files::parse_config);
        {
            let mut st = self.state.write().await;
            let init_opts = st.init_options.clone();
            st.client_settings.clone_from(&cfg_json);
            // The base is the committed policy: the manifest's sections
            // under the workspace `.suspect.yaml`, so `lint.rules`,
            // `lint.recommended`, inlay toggles and formatting policy
            // committed in either file reach the battery without any
            // client configuration. Client settings still win per leaf.
            let root = st.workspace_root();
            st.config = config_files::merge(
                init_opts,
                client_cfg,
                config_files::file_layers(root.as_deref()),
            );
            st.editor_config = file_config;
        }
        // Dynamic registration: only clients advertising
        // `didChangeWatchedFiles.dynamicRegistration` need the watcher
        // registered explicitly; others deliver the notification natively.
        let wants_dynamic_watcher = self
            .state
            .read()
            .await
            .client_caps
            .as_ref()
            .and_then(|c| c.workspace.as_ref())
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|d| d.dynamic_registration)
            .unwrap_or(false);
        if wants_dynamic_watcher {
            let options = DidChangeWatchedFilesRegistrationOptions {
                watchers: vec![FileSystemWatcher {
                    glob_pattern: GlobPattern::String("**/*.{yaml,yml,json}".to_owned()),
                    kind: None,
                }],
            };
            let registration = Registration {
                id: "suspect-watched-yaml".to_owned(),
                method: "workspace/didChangeWatchedFiles".to_owned(),
                register_options: serde_json::to_value(options).ok(),
            };
            if let Err(err) = self.client.register_capability(vec![registration]).await {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("watcher registration failed: {err}"),
                    )
                    .await;
            }
        }
    }

    /// Serves `workspace/didChangeConfiguration`.
    pub(crate) async fn handle_did_change_configuration(
        &self,
        params: DidChangeConfigurationParams,
    ) {
        // The new effective client layer over the committed file layers:
        // a manifest or `.suspect.yaml` edited on disk is picked up here
        // too (the files participate in the comparison, not just the
        // client section, or a file-only change would look like no change).
        // Same layering as `initialized`: initialization options, then the
        // client settings, over the committed files.
        let client_layers = {
            let mut st = self.state.write().await;
            st.client_settings = Some(params.settings.clone());
            let root = st.workspace_root();
            config_files::merge(
                st.init_options.clone(),
                config_files::parse_config(&params.settings),
                config_files::file_layers(root.as_deref()),
            )
        };
        let parsed = Some(client_layers);
        // The editor-side settings are re-derived too: `lint.min_severity`
        // lives there, not in `SuspectConfig`, and this handler used to
        // parse it into one object while the severity floor read the other
        // — so changing the floor mid-session did nothing until a restart.
        let file_config = {
            let st = self.state.read().await;
            editor_config::for_workspace(st.workspace_root().as_deref(), Some(&params.settings))
        };
        let changed = {
            let mut st = self.state.write().await;
            let config_changed = match parsed {
                Some(c) if st.config != c => {
                    st.config = c;
                    true
                }
                Some(_) | None => false,
            };
            let editor_changed = st.editor_config != file_config;
            if editor_changed {
                st.editor_config = file_config;
            }
            config_changed || editor_changed
        };
        if !changed {
            return;
        }
        // The lint floor moved, so anything computed under the old one is
        // no longer the answer — including the per-document pull cache,
        // whose entries are pre-filtered by the floor.
        {
            let mut st = self.state.write().await;
            st.ws_diag_cache = None;
            st.diag_cache.clear();
        }
        // The new config re-filters lint findings and inlay tooltips:
        // republish diagnostics for every open document and ask the client
        // to re-pull hints. Semantic tokens and lenses are config-free.
        let uris: Vec<Uri> = {
            let st = self.state.read().await;
            st.docs.keys().cloned().collect()
        };
        // Pull-diagnostics results re-filter under the new config too.
        let _ = self.client.workspace_diagnostic_refresh().await;
        for uri in uris {
            self.schedule_diagnostics(uri);
        }
        if self.client.inlay_hint_refresh().await.is_err() {
            self.client
                .log_message(MessageType::WARNING, "inlayHint refresh rejected")
                .await;
        }
    }

    /// Serves `workspace/didChangeWorkspaceFolders`.
    pub(crate) async fn handle_did_change_workspace_folders(
        &self,
        params: DidChangeWorkspaceFoldersParams,
    ) {
        let mut st = self.state.write().await;
        for added in &params.event.added {
            if let Ok(p) = added.uri.to_file_path() {
                st.root = Some(p);
            }
        }
        if let Some(last) = params.event.removed.last()
            && st
                .root
                .as_ref()
                .zip(last.uri.to_file_path().ok())
                .is_some_and(|(r, p)| r == &p)
        {
            st.root = None;
            st.workspace = None;
        }
    }
}
