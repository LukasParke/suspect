//! Completion and completion-resolve handlers.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use super::config_kind;
use crate::Backend;
use crate::state::offset_of_utf16;
use crate::{completion, config_schema, rank};

impl Backend {
    /// Serves `textDocument/completion`.
    pub(crate) async fn handle_completion(
        &self,
        params: CompletionParams,
    ) -> JsonRpcResult<Option<CompletionResponse>> {
        let pos = params.text_document_position.position;
        let Ok(uri) = Uri::parse(params.text_document_position.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let ws = self.workspace_for(&uri).await;
        // The document is cloned out and the guard released before the
        // match below: the `Refs` arm needs the semantic index, and taking
        // the write lock that an index build requires while this read guard
        // is alive is the exact inversion that used to wedge the server.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        // suspect's own configuration files complete from their schema:
        // the keys still missing from this section, or the permitted values
        // of the key under the cursor.
        if let Some(kind) = config_kind(&uri) {
            let mut items = config_schema::completions(kind, &doc.low, offset);
            items.sort_by(|a, b| {
                a.sort_text
                    .cmp(&b.sort_text)
                    .then_with(|| a.label.cmp(&b.label))
            });
            return Ok(Some(CompletionResponse::Array(items)));
        }
        let context = completion::context_at(&doc.low, offset);
        // The schema's own permitted values lead whenever the position has
        // a schema: an enum member here is worth more than any keyword.
        let mut schema_first: Vec<CompletionItem> = Vec::new();
        if matches!(
            context,
            completion::CompletionContext::Values(_)
                | completion::CompletionContext::SchemaPropertyNames(_)
        ) {
            for value in rank::schema_values(&doc.low, offset) {
                schema_first.push(CompletionItem {
                    label: value.clone(),
                    detail: Some(rank::Rank::Exact.label().to_owned()),
                    kind: Some(tower_lsp::lsp_types::CompletionItemKind::VALUE),
                    sort_text: Some(format!("0{value}")),
                    ..CompletionItem::default()
                });
            }
        }
        let items = match context {
            completion::CompletionContext::Keys(keys) => completion::key_items(keys),
            completion::CompletionContext::Refs => match ws {
                Some(ws) => {
                    // Rank by what the workspace actually uses, so the
                    // components a maintainer reaches for come first.
                    let all = completion::ref_candidates(&ws, doc.low.uri());
                    let position = rank::position_at(&doc.low, offset);
                    // The cached index, not a rebuild: this is the
                    // per-keystroke completion path, and the build walks
                    // the whole workspace.
                    let index = self.index(&ws).await;
                    let expected: &[&str] = if rank::expects_schema(&doc.low, offset) {
                        &["schemas"]
                    } else {
                        &[]
                    };
                    rank::rank_refs(&index, doc.low.uri().as_str(), &position, expected, all)
                        .into_iter()
                        .map(|proposal| {
                            let sort_text =
                                format!("{}{}", rank::rank_level(proposal.rank), proposal.insert);
                            CompletionItem {
                                label: proposal.insert,
                                detail: Some(proposal.detail),
                                kind: Some(tower_lsp::lsp_types::CompletionItemKind::REFERENCE),
                                sort_text: Some(sort_text),
                                ..CompletionItem::default()
                            }
                        })
                        .collect()
                }
                None => Vec::new(),
            },
            completion::CompletionContext::Values(values) => completion::value_items(values),
            completion::CompletionContext::ComponentNames(section) => {
                completion::component_name_items(
                    completion::component_names(&doc.low, section),
                    section,
                    doc.low.uri(),
                )
            }
            completion::CompletionContext::OperationIds => {
                completion::operation_id_items(completion::operation_id_candidates(&doc.low))
            }
            completion::CompletionContext::TagNames => {
                completion::tag_name_items(completion::tag_name_candidates(&doc.low))
            }
            completion::CompletionContext::SchemaPropertyNames(names) => {
                // Required-and-missing siblings first, then the rest, each
                // labelled with why it is offered.
                let written: Vec<String> = names.clone();
                let ranked = rank::rank_siblings(&doc.low, offset, &written);
                if ranked.is_empty() {
                    completion::property_name_items(names)
                } else {
                    let mut items: Vec<CompletionItem> = ranked
                        .into_iter()
                        .map(|proposal| {
                            let sort_text =
                                format!("{}{}", rank::rank_level(proposal.rank), proposal.insert);
                            CompletionItem {
                                label: proposal.insert,
                                detail: Some(proposal.detail),
                                kind: Some(tower_lsp::lsp_types::CompletionItemKind::PROPERTY),
                                sort_text: Some(sort_text),
                                ..CompletionItem::default()
                            }
                        })
                        .collect();
                    items.extend(completion::property_name_items(written));
                    items
                }
            }
            completion::CompletionContext::MediaTypes => {
                completion::value_items(completion::MEDIA_TYPES)
            }
            completion::CompletionContext::None => return Ok(None),
        };
        let mut items = items;
        items.extend(schema_first);
        Ok((!items.is_empty()).then_some(CompletionResponse::Array(items)))
    }

    /// Serves `completionItem/resolve`.
    pub(crate) async fn handle_completion_resolve(
        &self,
        item: CompletionItem,
    ) -> JsonRpcResult<CompletionItem> {
        let ws = match item
            .data
            .as_ref()
            .and_then(|d| d.get("uri"))
            .and_then(|u| u.as_str())
            .and_then(|u| Uri::parse(u).ok())
        {
            Some(uri) => self.workspace_for(&uri).await,
            None => None,
        };
        Ok(completion::resolve_item(item, ws.as_deref()))
    }
}
