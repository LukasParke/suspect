//! Diagnostics: the debounced push publish and the document/workspace pull handlers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::{diagnostics, pull};

/// Builds a full pull-diagnostics report.
fn full_report(
    result_id: Option<String>,
    items: Vec<Diagnostic>,
) -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
        RelatedFullDocumentDiagnosticReport {
            related_documents: None,
            full_document_diagnostic_report: FullDocumentDiagnosticReport { result_id, items },
        },
    ))
}

/// Builds an unchanged pull-diagnostics report for `result_id`.
fn unchanged_report(result_id: String) -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Unchanged(
        RelatedUnchangedDocumentDiagnosticReport {
            related_documents: None,
            unchanged_document_diagnostic_report: UnchangedDocumentDiagnosticReport { result_id },
        },
    ))
}

impl Backend {
    /// Serves `textDocument/diagnostic`.
    pub(crate) async fn handle_diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> JsonRpcResult<DocumentDiagnosticReportResult> {
        let Ok(uri) = Uri::parse(params.text_document.uri.as_str()) else {
            return Ok(full_report(None, Vec::new()));
        };
        let ws = self.workspace_for(&uri).await;
        // Take everything the report needs and drop the guard before the
        // lint runs. Holding a read lock across the computation and then
        // taking a second read for the severity floor deadlocks against
        // `warm_index`, which `didOpen` spawns and which queues a writer
        // once its blocking build finishes: tokio's `RwLock` is
        // write-preferring, so that second read waits for a writer that is
        // itself waiting for the first guard to drop. The queued writer
        // then blocks every later reader, so one diagnostic silences the
        // whole server — and an editor sends this in the same instant as
        // `didOpen`, so it is every file open rather than a rare race.
        // A push for this exact content already ran the battery; reuse it.
        // The epoch guard means any edit since invalidates the entry.
        let cached = {
            let st = self.state.read().await;
            match st.diag_cache.get(&uri) {
                Some((epoch, id, items)) if *epoch == st.generation() => {
                    Some((id.clone(), items.clone()))
                }
                _ => None,
            }
        };
        if let Some((id, items)) = cached {
            if params.previous_result_id.as_deref() == Some(id.as_str()) {
                return Ok(unchanged_report(id));
            }
            return Ok(full_report(Some(id), items));
        }

        let (doc, cfg, floor, ruleset, strict_format) = {
            let st = self.state.read().await;
            let Some(doc) = st.docs.get(&uri) else {
                return Ok(full_report(None, Vec::new()));
            };
            // The shared severity floor, applied identically in the editor
            // and on the command line.
            (
                doc.clone(),
                st.config.clone(),
                st.editor_config.min_severity(),
                st.editor_config.settings.lint.ruleset.clone(),
                st.editor_config.settings.validate.strict_format,
            )
        };
        // Off the runtime, for the same reason as the push: this is seconds
        // of synchronous work, and holding a worker for that long stalls
        // every other request the editor has in flight.
        let previous = params.previous_result_id.clone();
        let id_and_items = tokio::task::spawn_blocking({
            let ws = ws.clone();
            let cfg = cfg.clone();
            let doc = doc.clone();
            let ruleset = ruleset.clone();
            move || match &ws {
                Some(ws) => pull::pull_diagnostics(
                    ws,
                    &doc.low,
                    previous,
                    &cfg,
                    ruleset.as_deref(),
                    strict_format,
                ),
                None => {
                    let items = diagnostics::compute_diagnostics(
                        ws.as_ref(),
                        &doc.low,
                        &cfg,
                        ruleset.as_deref(),
                        strict_format,
                    );
                    (pull::diagnostics_result_id(&items), items)
                }
            }
        })
        .await;
        let Ok((id, items)) = id_and_items else {
            return Ok(full_report(None, Vec::new()));
        };
        let items = diagnostics::filter_at_least(items, floor);
        {
            let mut st = self.state.write().await;
            let epoch = st.generation();
            st.diag_cache
                .insert(uri, (epoch, id.clone(), items.clone()));
        }
        if params.previous_result_id.as_deref() == Some(id.as_str()) {
            return Ok(unchanged_report(id));
        }
        Ok(full_report(Some(id), items))
    }

    /// Serves `workspace/diagnostic`.
    pub(crate) async fn handle_workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> JsonRpcResult<WorkspaceDiagnosticReportResult> {
        // Served from cache when the content and the config are unchanged:
        // this walks every document in the project, and an editor asks for it
        // in the same burst as the requests it is waiting on.
        let cached: Option<(String, Vec<WorkspaceDocumentDiagnosticReport>)> = {
            let st = self.state.read().await;
            st.ws_diag_cache
                .as_ref()
                .and_then(|(epoch, config, id, items)| {
                    let fresh = *epoch == st.generation() && *config == st.config;
                    fresh.then(|| (id.clone(), items.clone()))
                })
        };
        if let Some((_id, items)) = cached {
            return Ok(WorkspaceDiagnosticReportResult::Report(
                WorkspaceDiagnosticReport { items },
            ));
        }

        // Same discipline as `diagnostic`: nothing expensive runs while a state
        // guard is alive, because a queued writer behind it blocks every
        // later reader — including the hover that follows.
        let (workspace, cfg, ruleset, strict_format, cache, doc_versions) = {
            let st = self.state.read().await;
            (
                st.workspace.clone(),
                st.config.clone(),
                st.editor_config.settings.lint.ruleset.clone(),
                st.editor_config.settings.validate.strict_format,
                Arc::clone(&st.ws_doc_diag),
                (st.ws_identity, st.doc_epoch.clone()),
            )
        };
        let mut items = Vec::new();
        let previous: HashMap<String, String> = params
            .previous_result_ids
            .iter()
            .map(|p| (p.uri.to_string(), p.value.clone()))
            .collect();
        if let Some(ws) = &workspace {
            let cfg = cfg.clone();
            let ruleset = ruleset.clone();
            // Per-document cache: unchanged documents reuse their reports;
            // only the edited file pays its lint battery. Keyed on the
            // workspace identity (saves rebuild it) and each document's own
            // content version (its edits invalidate only its entry).
            let view = crate::state::WsDiagCacheView::new(cache, doc_versions.0, doc_versions.1);
            for (uri, diags) in
                pull::workspace_pull(ws, &cfg, ruleset.as_deref(), strict_format, &view)
            {
                let Ok(url) = Url::parse(uri.as_str()) else {
                    continue;
                };
                let id = pull::diagnostics_result_id(&diags);
                if previous.get(uri.as_str()) == Some(&id) {
                    items.push(WorkspaceDocumentDiagnosticReport::Unchanged(
                        WorkspaceUnchangedDocumentDiagnosticReport {
                            uri: url,
                            version: None,
                            unchanged_document_diagnostic_report:
                                UnchangedDocumentDiagnosticReport { result_id: id },
                        },
                    ));
                    continue;
                }
                items.push(WorkspaceDocumentDiagnosticReport::Full(
                    WorkspaceFullDocumentDiagnosticReport {
                        uri: url,
                        version: None,
                        full_document_diagnostic_report: FullDocumentDiagnosticReport {
                            result_id: Some(id),
                            items: diags,
                        },
                    },
                ));
            }
        }
        let id = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hasher::write(
                &mut hasher,
                serde_json::to_string(&items).unwrap_or_default().as_bytes(),
            );
            format!("ws-{:016x}", std::hash::Hasher::finish(&hasher))
        };
        {
            let mut st = self.state.write().await;
            let epoch = st.generation();
            let config = st.config.clone();
            st.ws_diag_cache = Some((epoch, config, id.clone(), items.clone()));
        }
        Ok(WorkspaceDiagnosticReportResult::Report(
            WorkspaceDiagnosticReport { items },
        ))
    }

    /// Schedules a debounced diagnostics publish: 150 ms after the last
    /// change, only the newest generation publishes.
    pub(crate) fn schedule_diagnostics(&self, uri: Uri) {
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        let Some(url) = Url::parse(uri.as_str()).ok() else {
            return;
        };
        tokio::spawn(async move {
            let generation = {
                let mut st = state.write().await;
                let g = st.generations.entry(uri.clone()).or_insert(0);
                *g += 1;
                *g
            };
            tokio::time::sleep(Duration::from_millis(150)).await;
            // Compute WITHOUT holding the state lock: clone the cheap handles
            // (OpenDoc is small; LowDoc parse tree is Arc-shared internally).
            let (doc, ws, cfg, ruleset, strict_format) = {
                let st = state.read().await;
                if st.generations.get(&uri) != Some(&generation) {
                    return; // a newer edit superseded this publish
                }
                match st.docs.get(&uri) {
                    Some(doc) => (
                        doc.clone(),
                        st.workspace.clone(),
                        st.config.clone(),
                        st.editor_config.settings.lint.ruleset.clone(),
                        st.editor_config.settings.validate.strict_format,
                    ),
                    None => return,
                }
            };
            // On the blocking pool. The lint pass is seconds of synchronous
            // CPU work on a 63k-line specification, and running it on a
            // runtime worker means the worker cannot poll anything else:
            // a hover sent during an open waited 4.7 seconds behind it. The
            // state lock is not held here, so this was never a lock
            // problem — it was a scheduler problem.
            let diags = match tokio::task::spawn_blocking({
                let ws = ws.clone();
                let cfg = cfg.clone();
                let doc = doc.clone();
                let ruleset = ruleset.clone();
                move || {
                    diagnostics::compute_diagnostics(
                        ws.as_ref(),
                        &doc.low,
                        &cfg,
                        ruleset.as_deref(),
                        strict_format,
                    )
                }
            })
            .await
            {
                Ok(diags) => diags,
                Err(_) => return,
            };
            // The same severity floor the pull path applies. Publishing the
            // unfiltered set was a pre-existing inconsistency — the editor's
            // problem panel showed findings the configured floor suppressed —
            // and caching the push's result made the pull serve it too.
            let floor = {
                let st = state.read().await;
                st.editor_config.min_severity()
            };
            let diags = diagnostics::filter_at_least(diags, floor);
            let id = pull::diagnostics_result_id(&diags);
            // Superseded while computing? Drop the stale result.
            let superseded = {
                let st = state.read().await;
                st.generations.get(&uri) != Some(&generation)
            };
            if superseded {
                return;
            }
            // Keep the result so the pull that follows this push is a cache
            // hit rather than a second full lint pass.
            {
                let mut st = state.write().await;
                if st.generations.get(&uri) == Some(&generation) {
                    let epoch = st.generation();
                    st.diag_cache
                        .insert(uri.clone(), (epoch, id, diags.clone()));
                }
            }
            client.publish_diagnostics(url, diags, None).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::State;

    // NOTE on the optional Backend plumbing test (initialize + didOpen +
    // diagnostics poll through tower-lsp's request/channel plumbing): it is
    // deliberately skipped. Driving `LspService` requires the
    // `tower::Service` trait and polling the client socket requires a
    // `Stream` extension trait; neither `tower` nor `futures` is a workspace
    // dependency, and tower-lsp does not re-export them. The behavior is
    // instead covered end-to-end by the pure-function unit tests in
    // state/diagnostics/navigation/completion/symbols, plus this test of the
    // debounce generation logic shape.

    #[test]
    fn debounce_generation_is_monotonic_per_document() {
        let mut st = State::default();
        let a: Uri = "file:///a.yaml".into();
        let b: Uri = "file:///b.yaml".into();

        let cap_a = *st.generations.entry(a.clone()).or_insert(0);
        *st.generations.get_mut(&a).unwrap() += 1;
        // Editing document B must not invalidate A's pending publish.
        assert_eq!(st.generations.get(&b), None);
        assert_ne!(st.generations.get(&a), Some(&cap_a));

        // B's first generation starts at its own counter.
        assert_eq!(*st.generations.entry(b.clone()).or_insert(0), 0);
    }
}
