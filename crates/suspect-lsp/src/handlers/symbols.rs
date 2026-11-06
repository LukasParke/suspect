//! Document and workspace symbol and folding-range handlers.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::{symbols, workspace_symbol};

impl Backend {
    /// Serves `textDocument/documentSymbol`.
    pub(crate) async fn handle_document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonRpcResult<Option<DocumentSymbolResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // Cached like its whole-document siblings (folds, links): the
        // outline is re-requested on focus changes, and a tab switch back
        // to an unchanged file should not re-walk the tree.
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.symbols.clone()).await {
            return Ok(
                (!cached.is_empty()).then(|| DocumentSymbolResponse::Nested((*cached).clone()))
            );
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        let syms = std::sync::Arc::new(symbols::document_symbols(&doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.symbols = v, syms.clone())
            .await;
        Ok((!syms.is_empty()).then(|| DocumentSymbolResponse::Nested((*syms).clone())))
    }

    /// Serves `textDocument/foldingRange`.
    pub(crate) async fn handle_folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> JsonRpcResult<Option<Vec<FoldingRange>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.folds.clone()).await {
            return Ok((!cached.is_empty()).then(|| (*cached).clone()));
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        let ranges = std::sync::Arc::new(symbols::folding_ranges(&doc.low));
        self.doc_store(&uri, epoch, |cache, v| cache.folds = v, ranges.clone())
            .await;
        Ok((!ranges.is_empty()).then(|| (*ranges).clone()))
    }

    /// Serves `workspace/symbol`.
    pub(crate) async fn handle_symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonRpcResult<Option<Vec<SymbolInformation>>> {
        // Read-first: the workspace is cached in the common case, and a
        // write lock here would let one symbol-search keystroke stall
        // every other request behind a post-save rebuild.
        let ws = {
            let st = self.state.read().await;
            st.workspace.clone()
        };
        let ws = match ws {
            Some(ws) => ws,
            None => {
                let mut st = self.state.write().await;
                let Some(ws) = st.ensure_workspace() else {
                    return Ok(None);
                };
                ws
            }
        };
        let syms = workspace_symbol::workspace_symbols(&ws, &params.query);
        Ok((!syms.is_empty()).then_some(syms))
    }
}
