//! Custom `suspect/*` requests: workflow runs, generation contract, SDK render and previews.

use serde_json::Value;
use suspect_source::Uri;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::generation_contract::GenerationContractResult;
use crate::{generation_contract, render_sdk, run_lenses};

/// Base URL for live runs: initialization options, then `SUSPECT_BASE_URL`,
/// then the default local address.
#[cfg(feature = "live-run")]
fn base_url(init_options: Option<&serde_json::Value>) -> String {
    run_lenses::base_url_from_options(init_options)
}

impl Backend {
    /// Handler for the `suspect/runWorkflow` custom request.
    pub(crate) async fn run_workflow_request(
        &self,
        params: run_lenses::RunWorkflowParams,
    ) -> JsonRpcResult<run_lenses::RunResult> {
        self.run_workflow_uri(&params.uri, &params.workflow).await
    }

    /// Handler for the `suspect/generationContract` custom request: the
    /// generation admission verdict for the live document.
    pub(crate) async fn generation_contract_request(
        &self,
        params: generation_contract::GenerationContractParams,
    ) -> JsonRpcResult<generation_contract::GenerationContractResult> {
        let Ok(uri) = Uri::parse(&params.uri) else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        let doc = {
            let st = self.state.read().await;
            st.docs.get(&uri).cloned()
        };
        let Some(doc) = doc else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        // The live overlay: the review sees the current buffer, not the
        // last saved state, so unsaved edits get their admission verdict
        // before anyone generates from them.
        let Some(ws) = self.live_workspace(&uri).await else {
            return Ok(GenerationContractResult {
                operations: Vec::new(),
                findings: Vec::new(),
                admissible: false,
                refusals: 0,
            });
        };
        Ok(
            generation_contract::generation_contract(&ws, &doc).unwrap_or(
                GenerationContractResult {
                    operations: Vec::new(),
                    findings: Vec::new(),
                    admissible: false,
                    refusals: 0,
                },
            ),
        )
    }

    /// Handler for `suspect/renderSdk`: render one native SDK from the
    /// live document. Open documents compile from the current buffer (the
    /// incremental reparse keeps that state current on every keystroke);
    /// closed documents fall back to the disk workspace. Scoped
    /// compilation applies when operation selectors are present.
    pub(crate) async fn render_sdk_request(
        &self,
        params: render_sdk::RenderSdkParams,
    ) -> JsonRpcResult<render_sdk::RenderSdkResult> {
        use render_sdk::{RenderDiagnostic, RenderSdkResult};
        let empty = |code: &str, message: String| RenderSdkResult {
            rendered: false,
            artifacts: Vec::new(),
            diagnostics: vec![RenderDiagnostic {
                code: code.to_owned(),
                message,
            }],
            operations: 0,
        };
        let Ok(uri) = Uri::parse(&params.uri) else {
            return Ok(empty(
                "sdk-uri-invalid",
                format!("unparseable uri {:?}", params.uri),
            ));
        };
        let open = self.state.read().await.docs.contains_key(&uri);
        let ws = if open {
            self.live_workspace(&uri).await
        } else {
            self.workspace_for(&uri).await
        };
        let Some(ws) = ws else {
            return Ok(empty(
                "sdk-workspace-unavailable",
                "no workspace for the document".to_owned(),
            ));
        };
        let contract = if params.operation_ids.is_empty() {
            suspect_ir::contract::Contract::from_workspace(&ws, &uri).ok()
        } else {
            let selection = suspect_ir::contract::OperationSelection::new(
                params.operation_ids.iter().map(String::as_str),
            );
            suspect_ir::contract::Contract::from_workspace_scoped(&ws, &uri, &selection).ok()
        };
        let Some(contract) = contract else {
            return Ok(empty(
                "sdk-compile-failed",
                "the document did not compile".to_owned(),
            ));
        };
        Ok(render_sdk::render_sdk(
            std::sync::Arc::new(contract),
            &params,
        ))
    }

    /// Compiles the Arazzo document at `uri_s`, executes the workflow named
    /// `workflow` against the configured base URL, streams progress/logs to
    /// the client, and publishes failure diagnostics on the document
    /// (empty diagnostics when everything passes).
    pub(crate) async fn run_workflow_uri(
        &self,
        uri_s: &str,
        workflow: &str,
    ) -> JsonRpcResult<run_lenses::RunResult> {
        use tower_lsp::jsonrpc::Error as RpcError;

        let invalid = |msg: String| RpcError {
            code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
            message: msg.into(),
            data: None,
        };
        let Ok(uri) = Uri::parse(uri_s) else {
            return Err(invalid(format!("unparseable uri {uri_s:?}")));
        };

        // Prefer the live buffer; fall back to the on-disk copy so runs work
        // for closed documents too. The Arc keeps the parsed doc alive across
        // awaits even if the editor closes it mid-run.
        let open = self.state.read().await.docs.get(&uri).cloned();
        let low_owned;
        let low = match &open {
            Some(doc) => &doc.low,
            None => {
                let path = uri.as_path().ok_or_else(|| {
                    invalid(format!(
                        "document {uri_s:?} is neither open nor a local file"
                    ))
                })?;
                let bytes = std::fs::read(&path)
                    .map_err(|e| invalid(format!("cannot read {}: {e}", path.display())))?;
                low_owned = suspect_low::LowDoc::parse(
                    uri.clone(),
                    suspect_source::Source::from_vec(bytes),
                );
                &low_owned
            }
        };

        // Arazzo source descriptions reference sibling files without `$ref`,
        // so a directory scan is the reliable way to load the workspace.
        let spec_path = uri
            .as_path()
            .ok_or_else(|| invalid("non-file uri".into()))?;
        let ws = run_lenses::workspace_dir_all(&spec_path)
            .ok_or_else(|| invalid("failed to load spec directory".into()))?;
        let plan = suspect_test::compile_plan(low, &ws).map_err(|e| invalid(e.to_string()))?;
        let wf = plan
            .workflows
            .iter()
            .find(|w| w.workflow_id == workflow)
            .ok_or_else(|| invalid(format!("workflow {workflow:?} not found in {uri_s:?}")))?;

        #[cfg(not(feature = "live-run"))]
        {
            let _ = wf;
            Err(RpcError {
                code: tower_lsp::jsonrpc::ErrorCode::InternalError,
                message: "suspect-lsp was built without the live-run feature".into(),
                data: None,
            })
        }
        #[cfg(feature = "live-run")]
        {
            let init_options = self.state.read().await.init_options.clone();
            let base = base_url(init_options.as_ref());
            let transport = run_lenses::ReqwestTransport::new();

            self.progress_begin(
                "suspect.runWorkflow",
                &format!("Running workflow '{workflow}'"),
            )
            .await;

            // Forward step events as window logs while the run progresses.
            let (mirror_tx, mut mirror_rx) =
                tokio::sync::mpsc::channel::<suspect_test::TestEvent>(256);
            let logger_client = self.client.clone();
            let logger = tokio::spawn(async move {
                while let Some(ev) = mirror_rx.recv().await {
                    logger_client
                        .log_message(MessageType::INFO, run_lenses::describe_event(&ev))
                        .await;
                }
            });

            let (summary, failures) =
                run_lenses::run_workflow_core(wf, &base, &transport, Some(&mirror_tx)).await;
            drop(mirror_tx);
            let _ = logger.await;

            // Publish criterion failures anchored at their source ranges;
            // an all-pass run clears previous test diagnostics instead.
            if let Ok(url) = Url::parse(uri.as_str()) {
                let inner = low.inner();
                let diags = run_lenses::failures_to_diagnostics(
                    wf,
                    &failures,
                    inner.bytes(),
                    inner.line_index(),
                );
                self.client.publish_diagnostics(url, diags, None).await;
            }

            self.progress_end(
                "suspect.runWorkflow",
                Some(format!(
                    "{passed} passed, {failed} failed",
                    passed = summary.passed,
                    failed = summary.failed
                )),
            )
            .await;
            Ok(summary)
        }
    }

    /// Renders `preset` for the OpenAPI spec at `uri_s` under
    /// `<workspace-root>/.suspect/preview/<preset>/` and opens the first
    /// written artifact.
    pub(crate) async fn render_preview(&self, uri_s: &str, preset: &str) -> Option<Value> {
        let uri = Uri::parse(uri_s).ok()?;
        let spec_path = uri.as_path()?;
        // The live overlay when the document is open: previews render the
        // current buffer, so what you preview is what you are looking at.
        // Closed documents fall back to the disk workspace.
        let ws = if self.state.read().await.docs.contains_key(&uri) {
            self.live_workspace(&uri).await?
        } else {
            run_lenses::workspace_dir_all(&spec_path)?
        };
        let ir = suspect_ir::IrSpec::from_workspace(&ws, &uri).ok()?;

        let root = self
            .state
            .read()
            .await
            .root
            .clone()
            .unwrap_or_else(|| run_lenses::dir_of(&spec_path));
        let out_root = root.join(".suspect").join("preview").join(preset);
        let outcomes = match run_lenses::render_preset(preset, &ir, &out_root) {
            Ok(outcomes) => outcomes,
            Err(e) => {
                self.client.log_message(MessageType::ERROR, e).await;
                return None;
            }
        };
        let opened = run_lenses::pick_outcome(&outcomes)?;
        let url = Url::from_file_path(opened).ok()?;
        let shown = self
            .client
            .show_document(ShowDocumentParams {
                uri: url,
                external: None,
                take_focus: Some(true),
                selection: None,
            })
            .await
            .ok()?;
        Some(serde_json::json!({
            "rendered": outcomes.len(),
            "opened": opened.display().to_string(),
            "shown": shown,
        }))
    }
}
