//! Navigation handlers: definitions, references, implementations, call and type hierarchies.

use std::sync::Arc;

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use super::to_url;
use crate::Backend;
use crate::state::{OpenDoc, lsp_range, offset_of_utf16};
use crate::{navigation, type_hierarchy_fmt};

/// Converts a navigation target into an LSP `Location`, preferring the open
/// (possibly dirty) buffer when it is the same document, else the
/// workspace-loaded copy.
fn to_location(
    ws: Option<&Arc<suspect_ref::Workspace>>,
    open: Option<&OpenDoc>,
    def: &navigation::Definition,
) -> Option<Location> {
    let (bytes, li) = if open.is_some_and(|d| d.low.uri() == &def.uri) {
        let inner = open.unwrap().low.inner();
        (inner.bytes(), inner.line_index())
    } else {
        let handle = ws?.get(&def.uri)?;
        let inner = handle.doc().inner();
        (inner.bytes(), inner.line_index())
    };
    Some(Location {
        uri: to_url(&def.uri)?,
        range: lsp_range(bytes, li, def.range.clone()),
    })
}

impl Backend {
    /// Serves `textDocument/definition`.
    pub(crate) async fn handle_goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
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
        let def = navigation::goto_definition(&ws, &doc.low, offset)
            .or_else(|| navigation::self_definition(&doc.low, offset));
        let open = st.docs.get(&uri).map(|d| d.as_ref());
        Ok(def
            .as_ref()
            .and_then(|d| to_location(Some(&ws), open, d))
            .map(GotoDefinitionResponse::Scalar))
    }

    /// Serves `textDocument/implementation`.
    // implementation = schemas that compose this one (allOf subtypes)
    pub(crate) async fn handle_goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let tdp = params.text_document_position_params.clone();
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
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
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        let Some(item) = type_hierarchy_fmt::prepare_type_hierarchy(ws.as_ref(), doc, off)
            .and_then(|items| items.into_iter().next())
        else {
            return Ok(None);
        };
        let locs: Vec<Location> = type_hierarchy_fmt::subtypes(ws.as_ref(), &item)
            .into_iter()
            .map(|t| Location {
                uri: t.uri,
                range: t.range,
            })
            .collect();
        if locs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(GotoDefinitionResponse::Array(locs)))
        }
    }

    /// Serves `textDocument/references`.
    pub(crate) async fn handle_references(
        &self,
        params: ReferenceParams,
    ) -> JsonRpcResult<Option<Vec<Location>>> {
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
        let defs =
            navigation::references(&ws, &doc.low, offset, params.context.include_declaration);
        let open = st.docs.get(&uri).map(|d| d.as_ref());
        let locs = defs
            .iter()
            .filter_map(|d| to_location(Some(&ws), open, d))
            .collect::<Vec<_>>();
        Ok(Some(locs).filter(|l| !l.is_empty()))
    }

    /// Serves `textDocument/prepareTypeHierarchy`.
    pub(crate) async fn handle_prepare_type_hierarchy(
        &self,
        params: TypeHierarchyPrepareParams,
    ) -> JsonRpcResult<Option<Vec<TypeHierarchyItem>>> {
        let tdp = params.text_document_position_params.clone();
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
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
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        Ok(type_hierarchy_fmt::prepare_type_hierarchy(
            ws.as_ref(),
            doc,
            off,
        ))
    }

    /// Shared plumbing for goto-style requests: resolve doc + offset, run `f`.
    pub(crate) async fn with_doc_offset<T>(
        &self,
        tdp: TextDocumentPositionParams,
        f: impl FnOnce(&suspect_ref::Workspace, &suspect_low::LowDoc, usize) -> JsonRpcResult<Option<T>>,
    ) -> JsonRpcResult<Option<T>> {
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let off = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        );
        let Some(off) = off else { return Ok(None) };
        f(&ws, &doc.low, off)
    }

    /// Goto-style helper returning a definition response.
    pub(crate) async fn goto_like(
        &self,
        tdp: TextDocumentPositionParams,
        f: impl FnOnce(
            &suspect_ref::Workspace,
            &suspect_low::LowDoc,
            usize,
        ) -> Option<GotoDefinitionResponse>,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let Ok(uri) = Uri::parse(tdp.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        let Some(ws) = ws else { return Ok(None) };
        let st = self.state.read().await;
        let Some(doc) = st.docs.get(&uri) else {
            return Ok(None);
        };
        let inner = doc.low.inner();
        let Some(off) = offset_of_utf16(
            inner.bytes(),
            inner.line_index(),
            tdp.position.line,
            tdp.position.character,
        ) else {
            return Ok(None);
        };
        Ok(f(&ws, &doc.low, off))
    }
}
