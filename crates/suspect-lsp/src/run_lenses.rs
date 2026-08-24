//! Run lenses, the `suspect/runWorkflow` custom request, and preview
//! rendering.
//!
//! On Arazzo documents every workflow gets a fully-resolved code lens
//! (`▶ Run <workflowId>`) anchored at its `workflowId` key; invoking it (or
//! sending the [`RunWorkflowRequest`] custom method directly) compiles the
//! document into a [`suspect_test::Plan`], executes the selected workflow
//! against a live base URL, streams step events as window logs plus
//! `window/workDoneProgress`, and publishes one error diagnostic per failed
//! success criterion anchored at the criterion's recorded source range.
//! `suspect.renderPreview` renders a [`suspect_gen`] preset for the spec
//! under `<workspace-root>/.suspect/preview/<preset>/` and opens the first
//! written artifact.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_low::{LowDoc, SpecFamily};
use suspect_test::{HttpClient, HttpRequest, HttpResponse, Plan, RunSummary, TestEvent, WfPlan};
use tower_lsp::lsp_types::{
    CodeLens, Command, Diagnostic, DiagnosticSeverity, NumberOrString, request,
};

/// Command invoked by a workflow run lens.
pub const RUN_WORKFLOW_COMMAND: &str = "suspect.runWorkflow";

/// Command that renders a generation preset and opens its output.
pub const RENDER_PREVIEW_COMMAND: &str = "suspect.renderPreview";

/// Params of the [`RunWorkflowRequest`] custom request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunWorkflowParams {
    /// URI of the Arazzo document to run.
    pub uri: String,
    /// `workflowId` of the workflow to execute.
    pub workflow: String,
}

/// Result of a workflow run: the executor summary.
pub type RunResult = RunSummary;

/// Custom LSP request that runs one Arazzo workflow and reports criterion
/// failures as diagnostics.
pub enum RunWorkflowRequest {}

impl request::Request for RunWorkflowRequest {
    type Params = RunWorkflowParams;
    type Result = RunResult;
    const METHOD: &'static str = "suspect/runWorkflow";
}

// ---- run lenses ---------------------------------------------------------

/// One `▶ Run <workflowId>` lens per workflow of an Arazzo document,
/// anchored at the workflow's `workflowId` key line. Lenses arrive fully
/// resolved (command attached); no resolve round-trip is needed.
#[must_use]
pub fn run_lenses(doc: &LowDoc) -> Vec<CodeLens> {
    if doc.sniff_family() != SpecFamily::Arazzo10 {
        return Vec::new();
    }
    let bytes = doc.inner().bytes();
    let li = doc.inner().line_index();
    let Some(workflows) = doc.root().get("workflows") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for wf in workflows.items() {
        let Some(id_node) = wf.get("workflowId") else {
            continue;
        };
        let Some(id) = id_node.as_str() else {
            continue;
        };
        let Some(key) = super::links::key_node_of(&id_node) else {
            continue;
        };
        out.push(CodeLens {
            range: super::state::lsp_range(bytes, li, key.byte_range()),
            command: Some(Command {
                title: format!("▶ Run {id}"),
                command: RUN_WORKFLOW_COMMAND.to_owned(),
                arguments: Some(vec![
                    serde_json::json!(doc.uri().as_str()),
                    serde_json::json!(id),
                ]),
            }),
            data: None,
        });
    }
    out
}

// ---- core runner --------------------------------------------------------

/// One failed success criterion resolved against the compiled plan.
///
/// `crit` is the criterion description from the event stream; the source
/// range is recovered by matching it against the plan's compiled criteria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CriterionFailure {
    /// Step whose criterion failed.
    pub step_id: String,
    /// Criterion description (`crit.describe()`).
    pub crit: String,
    /// Expected rendering from the executor.
    pub expected: String,
    /// Actual rendering from the executor.
    pub actual: String,
}

/// Byte range in the Arazzo document that a failure should be anchored at:
/// the recorded range of the failing criterion's condition string, falling
/// back to the document start for transport-level failures with no
/// criterion counterpart.
pub(crate) fn failure_range(wf: &WfPlan, failure: &CriterionFailure) -> Range<usize> {
    if failure.crit == "transport" {
        return 0..0;
    }
    let Some(step) = wf.steps.iter().find(|s| s.step_id == failure.step_id) else {
        return 0..0;
    };
    step.success
        .iter()
        .find(|c| c.describe() == failure.crit)
        .map(|c| c.range.clone())
        .unwrap_or(0..0)
}

/// Runs a single compiled workflow against `transport`, mirroring every
/// [`TestEvent`] into `mirror` (when given) so callers can stream progress,
/// and returning the run summary plus the criteria that failed.
pub(crate) async fn run_workflow_core(
    wf: &WfPlan,
    base_url: &str,
    transport: &dyn HttpClient,
    mirror: Option<&tokio::sync::mpsc::Sender<TestEvent>>,
) -> (RunSummary, Vec<CriterionFailure>) {
    let plan = Plan {
        workflows: vec![wf.clone()],
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TestEvent>(256);
    let collector = async {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let Some(m) = mirror
                && m.send(ev.clone()).await.is_err()
            {
                // Mirror consumer gone; keep collecting locally.
            }
            events.push(ev);
        }
        events
    };
    let (summary, events) = tokio::join!(
        suspect_test::run_plan(&plan, base_url, transport, tx),
        collector
    );

    let failures = events
        .iter()
        .filter_map(|ev| match ev {
            TestEvent::CriterionFail {
                step,
                crit,
                expected,
                actual,
                ..
            } => Some(CriterionFailure {
                step_id: step.clone(),
                crit: crit.clone(),
                expected: expected.clone(),
                actual: actual.clone(),
            }),
            _ => None,
        })
        .collect();
    (summary, failures)
}

/// Converts criterion failures into editor diagnostics anchored at each
/// failing criterion's recorded condition range.
pub(crate) fn failures_to_diagnostics(
    wf: &WfPlan,
    failures: &[CriterionFailure],
    bytes: &[u8],
    li: &suspect_source::LineIndex,
) -> Vec<Diagnostic> {
    failures
        .iter()
        .map(|f| {
            let range = super::state::lsp_range(bytes, li, failure_range(wf, f));
            Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(f.crit.clone())),
                code_description: None,
                source: Some("suspect-test".to_owned()),
                message: format!("expected {}, got {}", f.expected, f.actual),
                related_information: None,
                tags: None,
                data: None,
            }
        })
        .collect()
}

// ---- transports ---------------------------------------------------------

/// Live [`HttpClient`] over `reqwest` (rustls TLS). Only available with the
/// default `live-run` feature.
#[cfg(feature = "live-run")]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

#[cfg(feature = "live-run")]
impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "live-run")]
impl ReqwestTransport {
    /// Builds a transport with a shared connection pool.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[cfg(feature = "live-run")]
#[async_trait::async_trait]
impl HttpClient for ReqwestTransport {
    async fn execute(
        &self,
        req: HttpRequest,
    ) -> Result<HttpResponse, suspect_test::TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| suspect_test::TransportError(e.to_string()))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.to_vec());
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| suspect_test::TransportError(e.to_string()))?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    v.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| suspect_test::TransportError(e.to_string()))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

// ---- workspace / preview helpers ----------------------------------------

/// Loads every YAML/JSON document next to `spec` into one workspace
/// (mirrors the CLI's directory-scan approach: Arazzo `sourceDescriptions`
/// reference sibling files without `$ref`).
pub(crate) fn workspace_dir_all(spec_path: &Path) -> Option<Arc<suspect_ref::Workspace>> {
    use suspect_ref::WorkspaceBuilder;
    let dir = dir_of(spec_path);
    let ws = WorkspaceBuilder::new().root(&dir).build().ok()?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml") | Some("yml") | Some("json")
            )
        })
        .collect();
    entries.sort();
    for path in entries {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let _ = ws.load_all(name);
        }
    }
    Some(Arc::new(ws))
}

/// Parent directory of `path`, defaulting to `.` for bare file names.
pub(crate) fn dir_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Renders the named [`suspect_gen::presets`] preset for `ir` under
/// `out_root` and returns the outcomes.
pub(crate) fn render_preset(
    preset_name: &str,
    ir: &suspect_ir::IrSpec,
    out_root: &Path,
) -> Result<Vec<suspect_gen::RenderOutcome>, String> {
    use suspect_gen::{FilterRegistry, MinijinjaEngine, TemplateEngine};

    let Some(preset) = suspect_gen::presets::get(preset_name) else {
        return Err(format!("unknown preset {preset_name:?}"));
    };
    let mut engine = MinijinjaEngine::new();
    FilterRegistry::register(&mut engine);
    for (name, src) in preset.templates {
        engine
            .add_template(name, src)
            .map_err(|e| format!("template {name:?}: {e}"))?;
    }
    let manifest = suspect_gen::parse_manifest_str(preset.manifest_toml)
        .map_err(|e| format!("manifest: {e}"))?;
    let ctx = (preset.ctx_builder)(ir);
    std::fs::create_dir_all(out_root).map_err(|e| format!("create_dir_all: {e}"))?;
    suspect_gen::render_manifest(&engine, &manifest, &ctx, out_root, false)
        .map_err(|e| e.to_string())
}

/// Picks which rendered artifact to open: the first actually-written file,
/// else the first outcome.
pub(crate) fn pick_outcome(outcomes: &[suspect_gen::RenderOutcome]) -> Option<&Path> {
    outcomes
        .iter()
        .find(|o| o.wrote)
        .or_else(|| outcomes.first())
        .map(|o| o.path.as_path())
}

/// One-line human rendering of a [`TestEvent`] for `window/logMessage`.
#[must_use]
pub(crate) fn describe_event(ev: &TestEvent) -> String {
    match ev {
        TestEvent::WfStarted { id } => format!("workflow '{id}' started"),
        TestEvent::StepStarted { wf, step } => format!("[{wf}] step '{step}' started"),
        TestEvent::RequestSent {
            wf,
            step,
            method,
            url,
        } => format!("[{wf}/{step}] {method} {url}"),
        TestEvent::ResponseGot {
            wf,
            step,
            status,
            duration_ms,
        } => format!("[{wf}/{step}] response {status} ({duration_ms} ms)"),
        TestEvent::CriterionOk { wf, step, crit } => format!("[{wf}/{step}] ok: {crit}"),
        TestEvent::CriterionFail {
            wf,
            step,
            crit,
            expected,
            actual,
        } => format!("[{wf}/{step}] FAIL {crit}: expected {expected}, got {actual}"),
        TestEvent::OutputSet { wf, key, value } => format!("[{wf}] output {key} = {value}"),
        TestEvent::WfDone { wf, passed } => {
            format!(
                "workflow '{wf}' {}",
                if *passed { "passed" } else { "failed" }
            )
        }
        TestEvent::RunDone { passed, failed } => {
            format!("run done: {passed} passed, {failed} failed")
        }
    }
}

/// Resolves the base URL used by live runs: initialization options
/// (`suspect.run.baseUrl` or `run.baseUrl`, then top-level `baseUrl`), then
/// `SUSPECT_BASE_URL`, then `http://localhost:8080`.
#[must_use]
pub fn base_url_from_options(init_options: Option<&serde_json::Value>) -> String {
    if let Some(opts) = init_options {
        if let Some(v) = opts.get("baseUrl").and_then(|v| v.as_str()) {
            return v.to_owned();
        }
        for path in [
            ["suspect", "run", "baseUrl"],
            ["run", "baseUrl", ""],
            ["baseUrl", "", ""],
        ] {
            let mut cur = opts;
            let mut found = None;
            for key in path {
                if key.is_empty() {
                    break;
                }
                match cur.get(key) {
                    Some(v) if key == "baseUrl" => {
                        found = v.as_str();
                        break;
                    }
                    Some(next) => cur = next,
                    None => break,
                }
            }
            if let Some(v) = found {
                return v.to_owned();
            }
        }
    }
    std::env::var("SUSPECT_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_owned())
}

#[cfg(test)]
mod tests;
