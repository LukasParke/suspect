//! Plan execution: HTTP transport abstraction, event stream, and
//! [`run_plan`].
//!
//! Workflows execute concurrently (each as its own future driven
//! cooperatively inside the caller's task); steps within a workflow run
//! sequentially. Every step builds its request by evaluating parameter/body
//! runtime expressions against the workflow state (`$inputs...`,
//! `$steps.<id>.outputs.<key>`), sends it over the injected [`HttpClient`],
//! evaluates success criteria against the response, and captures declared
//! outputs into the workflow-scoped step map that later steps read.

use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;
use suspect_ir::ParamIn;
use suspect_rex::{RexCtx, eval_rex};
use tokio::sync::mpsc;

use crate::plan::{CriterionKind, Plan, StepPlan};

/// An outbound HTTP request built by the executor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpRequest {
    /// HTTP method (uppercase).
    pub method: String,
    /// Absolute URL (base URL joined with the operation path template).
    pub url: String,
    /// Request headers in insertion order.
    pub headers: Vec<(String, String)>,
    /// Request body bytes.
    pub body: Bytes,
}

/// An inbound HTTP response returned by a transport.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpResponse {
    /// Response status code.
    pub status: u16,
    /// Response headers in wire order.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Bytes,
}

/// Transport-level failure (connection error, no canned rule, cassette
/// exhausted, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(
    /// Human-readable description of the failure.
    pub String,
);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportError {}

/// Minimal async HTTP client abstraction used by [`run_plan`].
///
/// Real network transports ship with the CLI; this crate provides only
/// deterministic in-process implementations.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Executes one request and returns its response.
    ///
    /// # Errors
    /// Returns a [`TransportError`] when no response can be produced.
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Progress event emitted while a plan runs.
///
/// Serialized form is one NDJSON object per event with an `"event"` tag.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TestEvent {
    /// A workflow started.
    WfStarted {
        /// Workflow id.
        id: String,
    },
    /// A step started.
    StepStarted {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
    },
    /// A request left the executor.
    RequestSent {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
        /// Request method.
        method: String,
        /// Resolved request URL.
        url: String,
    },
    /// A response arrived.
    ResponseGot {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
        /// Response status code.
        status: u16,
        /// Exchange duration in milliseconds.
        duration_ms: u64,
    },
    /// One success criterion passed.
    CriterionOk {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
        /// Criterion description.
        crit: String,
    },
    /// One success criterion failed.
    CriterionFail {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
        /// Criterion description.
        crit: String,
        /// Expected value rendering.
        expected: String,
        /// Actual value rendering.
        actual: String,
    },
    /// A step output was captured.
    OutputSet {
        /// Workflow id.
        wf: String,
        /// Output name.
        key: String,
        /// Captured value.
        value: serde_json::Value,
    },
    /// A workflow finished.
    WfDone {
        /// Workflow id.
        wf: String,
        /// Whether every executed step passed.
        passed: bool,
    },
    /// The whole plan finished.
    RunDone {
        /// Steps passed across all workflows.
        passed: usize,
        /// Steps failed across all workflows.
        failed: usize,
    },
}

/// Aggregate outcome of one [`run_plan`] invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
pub struct RunSummary {
    /// Steps that passed all success criteria.
    pub passed: usize,
    /// Steps whose transport or criteria failed.
    pub failed: usize,
    /// Steps never attempted because an earlier step in their workflow
    /// failed (workflows stop at the first failing step).
    pub skipped: usize,
    /// Wall-clock duration of the whole run in milliseconds.
    pub duration_ms: u64,
}

/// Per-workflow step counts produced by [`run_workflow`].
struct WfCounts {
    passed: usize,
    failed: usize,
    skipped: usize,
    ok: bool,
}

/// Runs a compiled [`Plan`] against `http`, emitting progress on `events`.
///
/// Workflows run concurrently; steps run sequentially inside each workflow.
/// The first failing step ends its workflow; remaining steps are counted as
/// skipped.
#[must_use]
pub async fn run_plan(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    let start = Instant::now();
    let base = base_url.trim_end_matches('/').to_owned();

    // One future per workflow; all borrow `http` and are driven
    // cooperatively inside this task until every workflow completes.
    let mut running: Vec<std::pin::Pin<Box<dyn Future<Output = WfCounts> + Send + '_>>> =
        Vec::with_capacity(plan.workflows.len());
    for wf in &plan.workflows {
        running.push(Box::pin(run_workflow(wf, &base, http, events.clone())));
    }

    let mut finished: Vec<WfCounts> = Vec::new();
    if !running.is_empty() {
        std::future::poll_fn(|cx| {
            for idx in (0..running.len()).rev() {
                if let std::task::Poll::Ready(counts) = running[idx].as_mut().poll(cx) {
                    finished.push(counts);
                    drop(running.remove(idx));
                }
            }
            if running.is_empty() {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
    }

    let mut summary = RunSummary::default();
    for counts in finished {
        summary.passed += counts.passed;
        summary.failed += counts.failed;
        summary.skipped += counts.skipped;
    }
    summary.duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _ = events
        .send(TestEvent::RunDone {
            passed: summary.passed,
            failed: summary.failed,
        })
        .await;
    summary
}

/// Runs a single workflow sequentially, returning its step counts.
async fn run_workflow(
    wf: &crate::plan::WfPlan,
    base: &str,
    http: &dyn HttpClient,
    events: mpsc::Sender<TestEvent>,
) -> WfCounts {
    send(
        &events,
        TestEvent::WfStarted {
            id: wf.workflow_id.clone(),
        },
    )
    .await;

    // Step outputs keyed by stepId; each value is that step's outputs object.
    let mut steps_outputs = serde_json::Map::<String, serde_json::Value>::new();
    let mut counts = WfCounts {
        passed: 0,
        failed: 0,
        skipped: 0,
        ok: true,
    };

    'steps: for step in &wf.steps {
        send(
            &events,
            TestEvent::StepStarted {
                wf: wf.workflow_id.clone(),
                step: step.step_id.clone(),
            },
        )
        .await;
        match run_step(wf, step, base, http, &wf.inputs, &steps_outputs, &events).await {
            StepOutcome::Passed(outputs) => {
                steps_outputs.insert(step.step_id.clone(), serde_json::Value::Object(outputs));
                counts.passed += 1;
            }
            StepOutcome::Failed => {
                counts.failed += 1;
                counts.ok = false;
                break 'steps;
            }
        }
    }
    counts.skipped = wf.steps.len().saturating_sub(counts.passed + counts.failed);

    send(
        &events,
        TestEvent::WfDone {
            wf: wf.workflow_id.clone(),
            passed: counts.ok,
        },
    )
    .await;
    counts
}

/// Terminal outcome of one executed step.
enum StepOutcome {
    /// All criteria passed; carries the captured output object.
    Passed(serde_json::Map<String, serde_json::Value>),
    /// Transport failure or at least one failing criterion.
    Failed,
}

async fn send(events: &mpsc::Sender<TestEvent>, ev: TestEvent) {
    let _ = events.send(ev).await;
}

/// Builds and executes one step, then evaluates its success criteria.
async fn run_step(
    wf: &crate::plan::WfPlan,
    step: &StepPlan,
    base: &str,
    http: &dyn HttpClient,
    inputs: &serde_json::Map<String, serde_json::Value>,
    steps_outputs: &serde_json::Map<String, serde_json::Value>,
    events: &mpsc::Sender<TestEvent>,
) -> StepOutcome {
    let wf_id = wf.workflow_id.as_str();

    // State-only context: parameters may reference workflow inputs and
    // earlier step outputs but not the exchange currently being built.
    let state_ctx = || {
        RexCtx::default()
            .inputs(inputs)
            .steps_outputs(steps_outputs)
    };

    let mut path = step.operation.path.clone();
    let mut query: Vec<(String, String)> = Vec::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut cookies: Vec<String> = Vec::new();

    for p in &step.parameters {
        let text = match eval_rex(&p.value, &state_ctx()) {
            Some(serde_json::Value::String(s)) => s,
            Some(other) => other.to_string(),
            None => String::new(),
        };
        match p.location {
            ParamIn::Path => {
                path = path.replace(&format!("{{{}}}", p.name), &text);
            }
            ParamIn::Query => query.push((p.name.clone(), text)),
            ParamIn::Header => headers.push((p.name.clone(), text)),
            ParamIn::Cookie => cookies.push(format!("{}={}", p.name, text)),
        }
    }
    if !cookies.is_empty() {
        headers.push(("Cookie".to_owned(), cookies.join("; ")));
    }

    let method = step.operation.method.as_str();

    let mut body: Option<Vec<u8>> = None;
    if let Some(rex) = &step.request_body {
        body = Some(match eval_rex(rex, &state_ctx()) {
            // Pre-serialized JSON (from object bodies) or plain text goes out
            // verbatim; any other scalar is rendered as JSON.
            Some(serde_json::Value::String(s)) => s.into_bytes(),
            Some(other) => other.to_string().into_bytes(),
            None => Vec::new(),
        });
    }

    let mut request_headers = headers;
    if body.is_some()
        && !request_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        request_headers.push(("content-type".to_owned(), "application/json".to_owned()));
    }

    let url = join_url(base, &path, &query);
    let request = HttpRequest {
        method: method.to_owned(),
        url: url.clone(),
        headers: request_headers,
        body: Bytes::from(body.unwrap_or_default()),
    };

    send(
        events,
        TestEvent::RequestSent {
            wf: wf_id.to_owned(),
            step: step.step_id.clone(),
            method: request.method.clone(),
            url: request.url.clone(),
        },
    )
    .await;

    let started = Instant::now();
    let response = match http.execute(request).await {
        Ok(resp) => resp,
        Err(e) => {
            send(
                events,
                TestEvent::CriterionFail {
                    wf: wf_id.to_owned(),
                    step: step.step_id.clone(),
                    crit: "transport".to_owned(),
                    expected: "an HTTP response".to_owned(),
                    actual: e.to_string(),
                },
            )
            .await;
            return StepOutcome::Failed;
        }
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    send(
        events,
        TestEvent::ResponseGot {
            wf: wf_id.to_owned(),
            step: step.step_id.clone(),
            status: response.status,
            duration_ms,
        },
    )
    .await;

    let body_text = String::from_utf8_lossy(&response.body).into_owned();
    let body_json: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();

    let mut all_ok = true;
    for crit in &step.success {
        match eval_criterion(&crit.kind, response.status, body_json.as_ref(), &body_text) {
            Ok(()) => {
                send(
                    events,
                    TestEvent::CriterionOk {
                        wf: wf_id.to_owned(),
                        step: step.step_id.clone(),
                        crit: crit.describe(),
                    },
                )
                .await;
            }
            Err((expected, actual)) => {
                all_ok = false;
                send(
                    events,
                    TestEvent::CriterionFail {
                        wf: wf_id.to_owned(),
                        step: step.step_id.clone(),
                        crit: crit.describe(),
                        expected,
                        actual,
                    },
                )
                .await;
            }
        }
    }
    if !all_ok {
        return StepOutcome::Failed;
    }

    // Capture outputs with the full exchange context available to rex.
    let capture_ctx = RexCtx::default()
        .method(method)
        .status(response.status)
        .request_headers(&[])
        .response_headers(&response.headers)
        .request_body("")
        .response_body(&body_text)
        .inputs(inputs)
        .steps_outputs(steps_outputs);

    let mut captured = serde_json::Map::new();
    for (name, rex) in &step.outputs {
        if let Some(value) = eval_rex(rex, &capture_ctx) {
            send(
                events,
                TestEvent::OutputSet {
                    wf: wf_id.to_owned(),
                    key: name.clone(),
                    value: value.clone(),
                },
            )
            .await;
            captured.insert(name.clone(), value);
        }
    }
    StepOutcome::Passed(captured)
}

/// Evaluates one criterion against a response.
///
/// Returns `Err((expected, actual))` renderings when it fails.
fn eval_criterion(
    crit: &CriterionKind,
    status: u16,
    body_json: Option<&serde_json::Value>,
    body_text: &str,
) -> Result<(), (String, String)> {
    match crit {
        CriterionKind::StatusInRange(lo, hi) => {
            let class = status / 100;
            if (u16::from(*lo)..=u16::from(*hi)).contains(&class) {
                Ok(())
            } else {
                Err((crit.describe(), status.to_string()))
            }
        }
        CriterionKind::Equals { pointer, expected } => match pointer {
            None if expected.as_u64() == Some(u64::from(status)) => Ok(()),
            None => Err((expected.to_string(), status.to_string())),
            Some(pointer) => match resolve_pointer(body_json, pointer) {
                Some(actual) if actual == expected => Ok(()),
                Some(actual) => Err((
                    format!("{pointer} == {expected}"),
                    format!("{pointer} == {actual}"),
                )),
                None => Err((
                    format!("{pointer} == {expected}"),
                    format!("{pointer} <missing>"),
                )),
            },
        },
        CriterionKind::NotNull { pointer } => match resolve_pointer(body_json, pointer) {
            Some(serde_json::Value::Null) | None => Err((
                format!("{pointer} != null"),
                format!("{pointer} resolves to null/missing"),
            )),
            Some(_) => Ok(()),
        },
        CriterionKind::Regex { pattern } => match regex::Regex::new(pattern) {
            Ok(re) if re.is_match(body_text) => Ok(()),
            Ok(_) => Err((
                format!("body =~ /{pattern}/"),
                "body does not match".to_owned(),
            )),
            Err(e) => Err((format!("body =~ /{pattern}/"), e.to_string())),
        },
        CriterionKind::JsonPathTrue { expr } => {
            let pointer = crate::plan::fragment_to_pointer(expr);
            match resolve_pointer(body_json, &pointer) {
                Some(serde_json::Value::Null) | None => Err((
                    format!("$response.body#/{expr} exists"),
                    "path resolves to null/missing".to_owned(),
                )),
                Some(_) => Ok(()),
            }
        }
    }
}

/// Resolves an RFC 6901 pointer against an optional parsed JSON document.
/// An empty pointer addresses the document root.
fn resolve_pointer<'v>(
    doc: Option<&'v serde_json::Value>,
    pointer: &str,
) -> Option<&'v serde_json::Value> {
    let mut current = doc?;
    for token in pointer.trim_start_matches('/').split('/') {
        current = match current {
            serde_json::Value::Object(map) => map.get(token)?,
            serde_json::Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Joins the base URL, substituted path template, and query parameters.
fn join_url(base: &str, path: &str, query: &[(String, String)]) -> String {
    let mut url = format!(
        "{base}{}",
        if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        }
    );
    if !query.is_empty() {
        let pairs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
            .collect();
        url.push('?');
        url.push_str(&pairs.join("&"));
    }
    url
}

/// Percent-encodes one query component (RFC 3986 unreserved set kept raw).
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
