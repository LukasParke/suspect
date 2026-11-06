//! The hover handler and the model-derived meaning/impact half of a hover.

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use super::config_kind;
use crate::Backend;
use crate::state::offset_of_utf16;
use crate::{config_schema, impact, meaning, navigation};

/// The model-derived half of a hover: where the cursor is, and what a
/// change here would reach.
///
/// A competitor's hover stops at the schema. Ours answers the question an
/// author actually has before editing a definition: who depends on this?
pub(crate) fn hover_meaning(
    low: &suspect_low::LowDoc,
    offset: usize,
    index: &meaning::Index,
) -> Option<String> {
    let model = meaning::Model::new(low);
    let m = model.at(offset)?;
    // The italic kind/dialect line matches the subtitle register of the
    // card it is appended to, so the context reads as the card's footer
    // rather than a second, clashing design.
    let mut out = format!("*{} — {}*", m.kind.label(), model.dialect().label());
    // The pointer of a property buried in an operation is mostly path
    // noise: what an author wants is the way down from the schema they
    // are editing. Everything after the last `schema` keyword is it.
    let tokens: Vec<&str> = m
        .pointer
        .tokens()
        .iter()
        .map(|token| token.as_ref())
        .collect();
    if m.within(meaning::ObjectKind::Schema)
        && let Some(at) = tokens.iter().rposition(|token| *token == "schema")
    {
        let below: Vec<&str> = tokens[at + 1..]
            .iter()
            .copied()
            .filter(|token| !token.is_empty())
            .collect();
        if !below.is_empty() {
            out.push_str(&format!("\n\ninside `{}`", below.join(" \u{203a} ")));
        }
    }
    if m.pointer.to_path() != "/" {
        out.push_str(&format!("\n\n`{}`", m.pointer.to_path()));
    }
    if let Some(target) = &m.ref_target {
        out.push_str(&format!("\n\nresolves to `{}`", target.pointer.to_path()));
    }

    // Change impact, from the reference index only: no contract compile.
    let uri = low.uri().as_str();
    let empty: [(String, suspect_arazzo::ArazzoDoc<'_>); 0] = [];
    let no_artifacts = std::collections::BTreeMap::new();
    let no_traffic = std::collections::BTreeMap::new();
    let context = impact::ImpactContext {
        index,
        workflows: &empty,
        artifacts: &no_artifacts,
        traffic: &no_traffic,
    };
    let report = context.impact_of(uri, &m);
    if !report.is_local() {
        out.push_str(&format!("\n\n**Impact** — {}\n\n", report.summary()));
        for entry in report.impacts.iter().take(6) {
            out.push_str(&format!(
                "- {} **{}** — {}\n",
                entry.kind.label(),
                entry.subject,
                entry.via
            ));
        }
    }
    Some(out)
}

impl Backend {
    /// Serves `textDocument/hover`.
    pub(crate) async fn handle_hover(&self, params: HoverParams) -> JsonRpcResult<Option<Hover>> {
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
        // Take only what the answer needs, then release the lock. Holding
        // the read guard across the computation blocked the diagnostics
        // writer and every other hover, which is what a client sees as a
        // hover that never resolves.
        let doc = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(None);
            };
            doc.clone()
        };
        let index = self.index(&ws).await;
        let inner = doc.low.inner();
        let Some(offset) =
            offset_of_utf16(inner.bytes(), inner.line_index(), pos.line, pos.character)
        else {
            return Ok(None);
        };
        // A suspect configuration file is not an API document, and its keys
        // have documentation no OpenAPI vocabulary carries.
        if let Some(kind) = config_kind(&uri) {
            return Ok(
                config_schema::hover(kind, &doc.low, offset).map(|value| Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range: None,
                }),
            );
        }
        let Some(base) = navigation::hover_markdown(&ws, &doc.low, offset) else {
            return Ok(None);
        };
        // The semantic model narrates where the cursor is and what a change
        // here would cost, on top of whatever the hover already resolved.
        let value = match hover_meaning(&doc.low, offset, &index) {
            Some(extra) if !base.is_empty() => format!("{base}\n\n---\n{extra}"),
            Some(extra) => extra,
            None => base,
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }
}

#[cfg(test)]
mod hover_latency_tests {
    use super::*;
    use crate::state::State;
    use std::sync::Arc;

    /// A spec with a realistic reference count: enough that walking every
    /// `$ref` is measurable.
    fn spec_with_refs(operations: usize) -> String {
        let mut out = String::from("openapi: 3.1.1\ninfo: {title: Hover, version: '1'}\npaths:\n");
        for index in 0..operations {
            out.push_str(&format!(
                "  /resource{index}:\n    get:\n      operationId: op{index}\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {{$ref: '#/components/schemas/Model'}}\n"
            ));
        }
        out.push_str(
            "components:\n  schemas:\n    Model:\n      type: object\n      required: [id]\n      properties:\n        id: {type: string}\n",
        );
        out
    }

    /// Hover must not cost a workspace walk and a contract compile per
    /// request.
    ///
    /// Three bugs met here, all found by driving the real server against a
    /// real specification: the read guard was held across the computation
    /// (so concurrent hovers queued behind the diagnostics writer), the
    /// reference index was rebuilt per hover, and the contract was
    /// recompiled per hover. Together they made a 63k-line document take
    /// the better part of a second per cursor move, which a client reports
    /// as a hover that never resolves.
    #[test]
    fn hover_answers_within_a_budget_and_reuses_its_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        let operations = 400;
        std::fs::write(dir.path().join("openapi.yaml"), spec_with_refs(operations)).expect("write");
        let ws = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(dir.path())
                .build()
                .expect("workspace"),
        );
        ws.load_all("openapi.yaml").expect("load");
        let low = ws.get(&ws.uris()[0]).expect("handle").doc();

        // Stand in for the server's cached index: built once per generation.
        let index = std::sync::Arc::new(meaning::Index::build(&ws));
        let text = spec_with_refs(operations);
        let line = text
            .lines()
            .position(|l| l.contains("$ref"))
            .expect("a ref line");
        let byte = text.lines().take(line).map(str::len).sum::<usize>()
            + text
                .lines()
                .nth(line)
                .expect("line")
                .find("$ref")
                .expect("ref")
            + 2;

        let started = std::time::Instant::now();
        let first = hover_meaning(low, byte, &index);
        let first_cost = started.elapsed();

        let started = std::time::Instant::now();
        let second = hover_meaning(low, byte, &index);
        let second_cost = started.elapsed();

        assert!(first.is_some(), "hover must say something: {first:?}");
        assert!(second.is_some());

        // Generous, but it must fail if a per-request workspace walk or a
        // contract compile returns: both are seconds on a real document.
        let budget = std::time::Duration::from_millis(250);
        assert!(
            first_cost < budget,
            "first hover took {first_cost:?} over a {operations}-operation spec"
        );
        assert!(
            second_cost < budget / 4,
            "repeated hover took {second_cost:?}; the index is not being reused"
        );
    }

    /// The cache must survive an unrelated read and die with the tree it
    /// described.
    ///
    /// Warming on open only pays off if the answer survives until the user
    /// asks, and only stays correct if it goes when the workspace does.
    #[test]
    fn the_index_cache_is_reused_until_the_documents_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("openapi.yaml"), spec_with_refs(4)).expect("write");
        let ws = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(dir.path())
                .build()
                .expect("workspace"),
        );
        ws.load_all("openapi.yaml").expect("load");

        let mut state = State::default();
        let generation = state.generation();
        let first = Arc::new(crate::meaning::Index::build(&ws));
        state.store_index(generation, first.clone());
        assert!(
            state
                .cached_index(generation)
                .is_some_and(|again| Arc::ptr_eq(&first, &again)),
            "cache not reused"
        );

        state.open_doc(
            Uri::parse("file:///tmp/warm.yaml").expect("uri"),
            "openapi: 3.1.1\n".to_owned(),
        );
        let after_edit = state.generation();
        assert!(
            state.cached_index(after_edit).is_none(),
            "an edit must invalidate the cached index"
        );

        state.drop_workspace();
        assert!(
            state.index_cache.is_none(),
            "a dropped workspace must take its index with it"
        );
    }
}
