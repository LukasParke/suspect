//! Semantic tokens (full/range/delta), inlay hints, highlights, selection ranges and colors.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::state::offset_of_utf16;
use crate::{colors, pull, semantic};

/// Encodes classified tokens into the LSP relative-encoding envelope.
fn encode_tokens(tokens: Vec<SemanticToken>) -> Vec<SemanticToken> {
    tokens
}

impl Backend {
    /// Serves `textDocument/semanticTokens/full`.
    pub(crate) async fn handle_semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonRpcResult<Option<SemanticTokensResult>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // The delta handler keeps the token cache warm on every request, so
        // this is usually a hit; a miss computes once and stores for both.
        // One guard for the epoch and the entry: reading them under two
        // separate guards leaves a window where an edit lands between them
        // and stale tokens are served as current.
        if let Some(data) = {
            let st = self.state.read().await;
            let epoch = st.generation();
            st.token_cache
                .get(&uri)
                .and_then(|(cached, _id, data)| (*cached == epoch).then(|| data.clone()))
        } {
            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data,
            })));
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (doc.clone(), st.generation())
        };
        // Off the runtime worker, for the same reason as the delta path:
        // ~270ms of synchronous CPU must not pin a worker that the
        // requests behind this one need.
        let tokens = {
            let doc = doc.clone();
            tokio::task::spawn_blocking(move || semantic::semantic_tokens_full(doc.as_ref()))
                .await
                .unwrap_or_default()
        };
        let id = pull::tokens_result_id(&tokens.data);
        self.state
            .write()
            .await
            .token_cache
            .insert(uri, (epoch, id, tokens.data.clone()));
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens.data,
        })))
    }

    /// Serves `textDocument/semanticTokens/range`.
    pub(crate) async fn handle_semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> JsonRpcResult<Option<SemanticTokensRangeResult>> {
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
        let start = offset_of_utf16(
            bytes,
            li,
            params.range.start.line,
            params.range.start.character,
        )
        .unwrap_or(0);
        let end = offset_of_utf16(bytes, li, params.range.end.line, params.range.end.character)
            .unwrap_or(bytes.len());
        let tokens = pull::semantic_tokens_range(doc, li, start..end.max(start));
        Ok(Some(SemanticTokensRangeResult::Partial(
            SemanticTokensPartialResult {
                data: encode_tokens(tokens),
            },
        )))
    }

    /// Serves `textDocument/semanticTokens/full/delta`.
    pub(crate) async fn handle_semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> JsonRpcResult<Option<SemanticTokensFullDeltaResult>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(None);
        };
        // Read the cached entry before any compute: it is both the
        // unchanged-content fast path and the "previous" set a delta is
        // computed from. This path used to recompute every token on each
        // request — 272ms on a 63k-line specification, per request — with no
        // way to tell whether the cache was current.
        let (doc, epoch, cached) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            (
                doc.clone(),
                st.generation(),
                st.token_cache.get(&uri).cloned(),
            )
        };
        let (new_id, full_data) = match &cached {
            // Unchanged content: the cached set is the answer.
            Some((cached_epoch, id, data)) if *cached_epoch == epoch => (id.clone(), data.clone()),
            // Changed content (or nothing cached): compute, and keep the
            // previous set around long enough to delta from it below.
            // Off the runtime worker: this is ~270ms of synchronous CPU on
            // a 63k-line specification, and a worker pinned here cannot
            // poll the requests queued behind it (the same scheduler stall
            // the diagnostics comment documents).
            _ => {
                let doc = doc.clone();
                let full = tokio::task::spawn_blocking(move || {
                    semantic::semantic_tokens_full(doc.as_ref())
                })
                .await
                .unwrap_or_default();
                let id = pull::tokens_result_id(&full.data);
                let data = full.data;
                self.state
                    .write()
                    .await
                    .token_cache
                    .insert(uri.clone(), (epoch, id.clone(), data.clone()));
                (id, data)
            }
        };
        if params.previous_result_id == new_id {
            return Ok(Some(SemanticTokensFullDeltaResult::Tokens(
                SemanticTokens {
                    result_id: Some(new_id),
                    data: Vec::new(),
                },
            )));
        }
        // The client's previous result is one we still hold: delta from it.
        if let Some((_, prev_id, prev)) = cached
            && prev_id == params.previous_result_id
        {
            let delta = pull::semantic_tokens_delta(&prev, &full_data);
            return Ok(Some(delta));
        }
        // Otherwise the client's previous result is one this server never
        // held (a restart, or eviction): hand back the full set.
        Ok(Some(SemanticTokensFullDeltaResult::Tokens(
            SemanticTokens {
                result_id: None,
                data: full_data,
            },
        )))
    }

    /// Serves `textDocument/inlayHint`.
    pub(crate) async fn handle_inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> JsonRpcResult<Option<Vec<InlayHint>>> {
        let uri = Uri::parse(params.text_document.uri.as_str()).ok();
        let Some(uri) = uri else { return Ok(None) };
        let Some(ws) = self.workspace_for(&uri).await else {
            return Ok(None);
        };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let mut hints = semantic::inlay_hints(doc, &ws, params.range);
        // Ref-target hints carry resolve `data`; type-annotation hints
        // have a `: ` label. Both are user-togglable; the `· required`
        // markers are not.
        if !st.config.inlay_refs() {
            hints.retain(|h| h.data.is_none());
        }
        if !st.config.inlay_properties() {
            hints.retain(|h| {
                !matches!(&h.label, tower_lsp::lsp_types::InlayHintLabel::String(l) if l.starts_with(": "))
            });
        }
        Ok(Some(hints))
    }

    /// Serves `inlayHint/resolve`.
    pub(crate) async fn handle_inlay_hint_resolve(
        &self,
        hint: InlayHint,
    ) -> JsonRpcResult<InlayHint> {
        let Some(uri) = hint
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .and_then(|s| Uri::parse(s).ok())
        else {
            return Ok(hint);
        };
        match self.workspace_for(&uri).await {
            Some(ws) => Ok(semantic::resolve_inlay_hint(hint, &ws)),
            None => Ok(hint),
        }
    }

    /// Serves `textDocument/documentHighlight`.
    pub(crate) async fn handle_document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> JsonRpcResult<Option<Vec<DocumentHighlight>>> {
        let pos = params.text_document_position_params.position;
        let Ok(uri) = Uri::parse(
            params
                .text_document_position_params
                .text_document
                .uri
                .as_str(),
        ) else {
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
        Ok(Some(semantic::document_highlights(doc, offset)))
    }

    /// Serves `textDocument/documentColor`.
    pub(crate) async fn handle_document_color(
        &self,
        params: DocumentColorParams,
    ) -> JsonRpcResult<Vec<ColorInformation>> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(Vec::new());
        };
        if let Some(cached) = self.doc_cached(&uri, |cache| cache.colors.clone()).await {
            return Ok((*cached).clone());
        }
        let (doc, epoch) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(Vec::new());
            };
            (doc.clone(), st.generation())
        };
        let colors_out = std::sync::Arc::new(colors::document_colors(doc.as_ref()));
        self.doc_store(&uri, epoch, |cache, v| cache.colors = v, colors_out.clone())
            .await;
        Ok((*colors_out).clone())
    }
}
