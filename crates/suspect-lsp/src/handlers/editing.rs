//! Editing handlers: rename, linked editing, code actions and formatting.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use super::to_url;
use crate::Backend;
use crate::state::{lsp_range, offset_of_utf16};
use crate::{actions, links, pull, rename, type_hierarchy_fmt};

impl Backend {
    /// Serves `textDocument/prepareRename`.
    pub(crate) async fn handle_prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonRpcResult<Option<PrepareRenameResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            params.position.line,
            params.position.character,
        ) else {
            return Ok(None);
        };
        match rename::prepare_rename(doc, offset) {
            Some(ph) => Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: lsp_range(inner.bytes(), inner.line_index(), ph.range),
                placeholder: ph.placeholder,
            })),
            // Honest failure: the position is not on a renameable key.
            None => Err(tower_lsp::jsonrpc::Error::invalid_params(
                "no renameable component key at this position".to_owned(),
            )),
        }
    }

    /// Serves `textDocument/rename`.
    pub(crate) async fn handle_rename(
        &self,
        params: RenameParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let pos = params.text_document_position.position;
        let Ok(uri) = Uri::parse(params.text_document_position.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        match rename::rename(&ws, doc, &params.new_name, offset) {
            Ok(edit) => Ok(Some(edit)),
            Err(msg) => Err(tower_lsp::jsonrpc::Error::invalid_params(msg)),
        }
    }

    /// Serves `textDocument/linkedEditingRange`.
    pub(crate) async fn handle_linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> JsonRpcResult<Option<LinkedEditingRanges>> {
        self.with_doc_offset(
            params.text_document_position_params.clone(),
            |_ws, low, off| {
                Ok(
                    pull::linked_editing_range(low, off).map(|ranges| LinkedEditingRanges {
                        ranges,
                        word_pattern: None,
                    }),
                )
            },
        )
        .await
    }

    /// Serves `textDocument/codeAction`.
    pub(crate) async fn handle_code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // Resolve the workspace first: `workspace_for` may take the state
        // write lock, so it must not run under this handler's read guard.
        let ws = self.workspace_for(&uri).await;
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let Some(url) = to_url(&uri) else {
            return Ok(None);
        };
        // Defer the whole-document fixAll sweep to `codeAction/resolve`
        // only when the client declared `codeAction.resolveSupport`.
        let defer_fix_all = st
            .client_caps
            .as_ref()
            .and_then(|c| c.text_document.as_ref())
            .and_then(|td| td.code_action.as_ref())
            .and_then(|ca| ca.resolve_support.as_ref())
            .is_some();
        let mut actions = actions::code_actions(
            doc,
            &url,
            params.range,
            &params.context.diagnostics,
            ws.as_ref(),
            defer_fix_all,
        );
        if let Some(open) = ws
            .as_deref()
            .and_then(|ws| links::open_ref_action(ws, doc, params.range))
        {
            actions.push(open);
        }
        Ok((!actions.is_empty()).then(|| {
            actions
                .into_iter()
                .map(CodeActionOrCommand::CodeAction)
                .collect()
        }))
    }

    /// Serves `codeAction/resolve`.
    pub(crate) async fn handle_code_action_resolve(
        &self,
        action: CodeAction,
    ) -> JsonRpcResult<CodeAction> {
        let is_fix_all = action
            .data
            .as_ref()
            .and_then(|d| d.get("suspect"))
            .and_then(|s| s.as_str())
            == Some("fixAll");
        if !is_fix_all {
            return Ok(action);
        }
        let Some(uri_str) = action
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
        else {
            return Ok(action);
        };
        let Ok(uri) = Uri::parse(uri_str) else {
            return Ok(action);
        };
        let ws = self.workspace_for(&uri).await;
        let cfg = self.state.read().await.config.clone();
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(action);
        };
        let Some(url) = to_url(&uri) else {
            return Ok(action);
        };
        match actions::resolve_code_action(doc, &url, ws.as_ref(), &cfg) {
            Some(resolved) => Ok(resolved),
            None => Ok(action),
        }
    }

    /// Serves `textDocument/formatting`.
    pub(crate) async fn handle_formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        // Formatting options are ignored: the canonical form uses a
        // two-space indent regardless of editor configuration. Key
        // reordering follows `suspect.formatting.sortKeys`.
        let _ = &params.options;
        let Some(url) = to_url(&uri) else {
            return Ok(None);
        };
        let sort_keys = st.config.format_sort_keys();
        let extensions = st.config.extensions.clone().unwrap_or_default();
        Ok(actions::format_document_full(doc, &url, sort_keys, &extensions).map(|e| vec![e]))
    }

    /// Serves `textDocument/rangeFormatting`.
    pub(crate) async fn handle_range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let bytes = inner.bytes();
        let li = inner.line_index();
        let Some(start) = offset_of_utf16(
            bytes,
            li,
            params.range.start.line,
            params.range.start.character,
        ) else {
            return Ok(None);
        };
        let Some(end) =
            offset_of_utf16(bytes, li, params.range.end.line, params.range.end.character)
        else {
            return Ok(None);
        };
        let edits = type_hierarchy_fmt::range_formatting(doc, start..end);
        Ok((!edits.is_empty()).then_some(edits))
    }
}
