//! Code lens and document link handlers.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::{links, run_lenses};

impl Backend {
    /// Serves `textDocument/codeLens`.
    pub(crate) async fn handle_code_lens(
        &self,
        params: CodeLensParams,
    ) -> JsonRpcResult<Option<Vec<CodeLens>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.lenses.clone()).await {
            return Ok(Some((*cached).clone()));
        }
        let ws = self.workspace_for(&uri).await;
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        // Arazzo documents get workflow run lenses instead of the
        // OpenAPI component/operation lenses; those are cheap and uncached.
        if doc.low.sniff_family() == suspect_low::SpecFamily::Arazzo10 {
            return Ok(Some(run_lenses::run_lenses(&doc.low)));
        }
        let Some(ws) = ws else {
            return Ok(None);
        };
        let lenses = std::sync::Arc::new(links::code_lens(&ws, &doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.lenses = v, lenses.clone())
            .await;
        Ok(Some((*lenses).clone()))
    }

    /// Serves `codeLens/resolve`.
    pub(crate) async fn handle_code_lens_resolve(
        &self,
        params: CodeLens,
    ) -> JsonRpcResult<CodeLens> {
        if params.data.as_ref().and_then(|d| d.get("ptr")).is_none() {
            return Ok(params);
        }
        let uri_str = params
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .unwrap_or_default();
        let Ok(uri) = Uri::parse(uri_str) else {
            return Ok(params);
        };
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(params);
        };
        match ws {
            Some(ws) => Ok(links::code_lens_resolve(&ws, &doc.low, params)),
            None => Ok(params),
        }
    }

    /// Serves `textDocument/documentLink`.
    pub(crate) async fn handle_document_link(
        &self,
        params: DocumentLinkParams,
    ) -> JsonRpcResult<Option<Vec<DocumentLink>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // The document is taken and the guard released before the
        // workspace is asked for: holding one across the other is what
        // deadlocked this handler, and with it every request behind it.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.links.clone()).await {
            return Ok(Some((*cached).clone()));
        }
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(Some(Vec::new()));
        };
        let epoch = {
            let st = self.state.read().await;
            st.generation()
        };
        let built = std::sync::Arc::new(links::document_link(ws.as_ref(), &doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.links = v, built.clone())
            .await;
        Ok(Some((*built).clone()))
    }
}
