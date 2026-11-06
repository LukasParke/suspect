//! Workspace file-operation handlers (`will*`/`did*` create, rename, delete).

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::config_files;

impl Backend {
    /// Serves `workspace/didCreateFiles`.
    pub(crate) async fn handle_did_create_files(&self, params: CreateFilesParams) {
        self.drop_workspace_cache().await;
        for f in &params.files {
            let path = match std::path::PathBuf::from(f.uri.as_str()).canonicalize() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let Ok(uri) = Uri::from_path(&path) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            self.state.write().await.open_doc(uri, text);
        }
    }

    /// Serves `workspace/willRenameFiles`.
    pub(crate) async fn handle_will_rename_files(
        &self,
        params: RenameFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        let renames: Vec<FileRename> = params
            .files
            .iter()
            .map(|f| FileRename {
                old_uri: f.old_uri.to_string(),
                new_uri: f.new_uri.to_string(),
            })
            .collect();
        Ok(config_files::will_rename_files(&ws, &renames))
    }

    /// Serves `workspace/didRenameFiles`.
    pub(crate) async fn handle_did_rename_files(&self, params: RenameFilesParams) {
        self.drop_workspace_cache().await;
        let mut st = self.state.write().await;
        for f in &params.files {
            let old_path = std::path::PathBuf::from(f.old_uri.as_str());
            if let Ok(old) = Uri::from_path(&old_path) {
                st.close_doc(&old);
                st.token_cache.remove(&old);
                st.diag_cache.remove(&old);
                st.doc_cache.remove(&old);
            }
            let new_path = std::path::PathBuf::from(f.new_uri.as_str());
            if let Ok(text) = std::fs::read_to_string(&new_path)
                && let Ok(nu) = Uri::from_path(&new_path)
            {
                st.open_doc(nu, text);
            }
        }
    }

    /// Serves `workspace/willDeleteFiles`.
    pub(crate) async fn handle_will_delete_files(
        &self,
        params: DeleteFilesParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let ws = self.any_workspace().await;
        let Some(ws) = ws else { return Ok(None) };
        match config_files::will_delete_files(&ws, &params) {
            Some(diags) => {
                for d in diags.iter().take(1) {
                    self.client
                        .log_message(MessageType::WARNING, d.message.clone())
                        .await;
                }
                Ok(Some(WorkspaceEdit::default()))
            }
            None => Ok(None),
        }
    }
}
