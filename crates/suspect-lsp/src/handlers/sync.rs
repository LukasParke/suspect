//! Document sync handlers: open, change, save and close.

use std::sync::Arc;

use suspect_source::Uri;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::state::{self, OpenDoc};

impl Backend {
    /// Serves `textDocument/didOpen`.
    pub(crate) async fn handle_did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        let Ok(uri) = Uri::parse(item.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            st.open_doc(uri.clone(), item.text);
        }
        // Force the workspace load here rather than letting the first
        // hover discover it, then warm the index against it.
        let _ = self.workspace_for(&uri).await;
        self.warm_index();
        self.schedule_diagnostics(uri);
    }

    /// Serves `textDocument/didChange`.
    pub(crate) async fn handle_did_change(&self, params: DidChangeTextDocumentParams) {
        // Incremental sync: apply every change in order against the live
        // buffer (full-text changes are the range-less special case), then
        // reparse incrementally — tree-sitter reuses every unchanged
        // subtree, so a keystroke costs the edit, not the whole document.
        // The reparse equals a from-scratch parse of the new text (pinned
        // by suspect-low's equivalence tests); it is an optimization, not
        // a different tree.
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            let Some(doc) = st.docs.get(&uri).cloned() else {
                return;
            };
            let Some((text, edits)) = state::apply_with_edits(&doc, &params.content_changes) else {
                return; // malformed ranges: keep the last good buffer
            };
            // Skip the reparse + republish cycle when the text is unchanged.
            if text == doc.text {
                return;
            }
            let next = OpenDoc::reparse(&doc, text, &edits);
            st.replace_doc(uri.clone(), Arc::new(next));
        }
        self.warm_index();
        self.schedule_diagnostics(uri);
    }

    /// Serves `textDocument/didSave`.
    pub(crate) async fn handle_did_save(&self, params: DidSaveTextDocumentParams) {
        // The open buffer stays authoritative (include_text is false, the
        // save carries no text); what changes is the on-disk copy that
        // *other* documents' `$ref`s resolve against, so drop the cached
        // workspace for a fresh disk-backed rebuild.
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        if self.state.read().await.docs.contains_key(&uri) {
            let mut st = self.state.write().await;
            st.drop_workspace();
        }
    }

    /// Serves `textDocument/didClose`.
    pub(crate) async fn handle_did_close(&self, params: DidCloseTextDocumentParams) {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return;
        };
        {
            let mut st = self.state.write().await;
            st.close_doc(&uri);
        }
        // Clear diagnostics for the closed document.
        if let Ok(url) = Url::parse(uri.as_str()) {
            self.client.publish_diagnostics(url, Vec::new(), None).await;
        }
    }
}
