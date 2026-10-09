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
    /// The response body matched the schema declared for its status.
    ResponseValidated {
        /// Workflow id.
        wf: String,
        /// Step id.
        step: String,
        /// Response status code.
        status: u16,
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
    run_plan_with_messages(plan, base_url, http, None, events).await
}

/// [`run_plan`] with a message transport for Arazzo 1.1 AsyncAPI steps.
pub async fn run_plan_with_messages(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    messages: Option<&dyn crate::messaging::MessageTransport>,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    run_plan_full(
        plan,
        base_url,
        http,
        &crate::auth::AuthState::default(),
        &crate::auth::AuthConfig::default(),
        &serde_json::Map::new(),
        messages,
        events,
    )
    .await
}

/// Same as [`run_plan`] with explicit credential resolution: configured
/// schemes inject tokens/API keys into each request before it is sent.
pub async fn run_plan_with_auth(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    auth_state: &crate::auth::AuthState,
    auth_config: &crate::auth::AuthConfig,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    run_plan_full(
        plan,
        base_url,
        http,
        auth_state,
        auth_config,
        &serde_json::Map::new(),
        None,
        events,
    )
    .await
}

/// The full entry point: auth state + config + a message transport, for
/// callers that have credentials to inject (the CLI's `suspect test`
/// after loading its credentials file).
#[allow(clippy::too_many_arguments)]
pub async fn run_plan_with_auth_and_messages(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    auth_state: &crate::auth::AuthState,
    auth_config: &crate::auth::AuthConfig,
    inputs: &serde_json::Map<String, serde_json::Value>,
    messages: Option<&dyn crate::messaging::MessageTransport>,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    run_plan_full(
        plan,
        base_url,
        http,
        auth_state,
        auth_config,
        inputs,
        messages,
        events,
    )
    .await
}

/// [`run_plan_with_auth`] plus a message transport for Arazzo 1.1 AsyncAPI
/// send/receive steps.
#[allow(clippy::too_many_arguments)]
pub async fn run_plan_full(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    auth_state: &crate::auth::AuthState,
    auth_config: &crate::auth::AuthConfig,
    inputs: &serde_json::Map<String, serde_json::Value>,
    messages: Option<&dyn crate::messaging::MessageTransport>,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    let start = Instant::now();
    let base = base_url.trim_end_matches('/').to_owned();

    // One future per workflow; all borrow `http` and are driven
    // cooperatively inside this task until every workflow completes.
    let mut running: Vec<std::pin::Pin<Box<dyn Future<Output = WfCounts> + Send + '_>>> =
        Vec::with_capacity(plan.workflows.len());
    for wf in &plan.workflows {
        running.push(Box::pin(run_workflow(
            wf,
            &base,
            http,
            auth_state,
            auth_config,
            inputs,
            &plan.components,
            messages,
            events.clone(),
        )));
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
#[allow(clippy::too_many_arguments)]
async fn run_workflow(
    wf: &crate::plan::WfPlan,
    base: &str,
    http: &dyn HttpClient,
    auth_state: &crate::auth::AuthState,
    auth_config: &crate::auth::AuthConfig,
    input_overrides: &serde_json::Map<String, serde_json::Value>,
    components: &std::collections::BTreeMap<String, serde_json::Value>,
    messages: Option<&dyn crate::messaging::MessageTransport>,
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

    // Inputs, most specific first: caller-provided overrides, then the
    // workflow's static `parameters`, then schema-declared defaults.
    let mut effective_inputs = input_overrides.clone();
    for (key, value) in &wf.inputs {
        effective_inputs
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    for (key, default_val) in &wf.input_defaults {
        effective_inputs
            .entry(key.clone())
            .or_insert_with(|| default_val.clone());
    }
    let mut counts = WfCounts {
        passed: 0,
        failed: 0,
        skipped: 0,
        ok: true,
    };

    // Arazzo 1.1 §5.8.5.2.4: explicit `dependsOn` and implicit output
    // references both order execution. The scheduler takes the first
    // remaining step whose dependencies are all satisfied, in document
    // order, so a sequential run still satisfies the graph and a step
    // whose dependency has not run yet waits rather than reading a
    // missing output.
    let mut completed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut remaining: Vec<usize> = (0..wf.steps.len()).collect();

    while !remaining.is_empty() {
        let ready = remaining.iter().copied().find(|index| {
            wf.steps[*index]
                .depends_on
                .iter()
                .all(|dependency| completed.contains(dependency))
        });
        let Some(index) = ready else {
            // No remaining step can run: the dependency graph cannot be
            // satisfied (a cycle, or a dependency on an unrunnable step).
            let blocked: Vec<&str> = remaining
                .iter()
                .map(|index| wf.steps[*index].step_id.as_str())
                .collect();
            send(
                &events,
                TestEvent::CriterionFail {
                    wf: wf.workflow_id.clone(),
                    step: blocked.first().copied().unwrap_or("").to_owned(),
                    crit: "depends-on".to_owned(),
                    expected: "a satisfiable step order".to_owned(),
                    actual: format!(
                        "blocked by unsatisfied dependencies: {}",
                        blocked.join(", ")
                    ),
                },
            )
            .await;
            counts.skipped += remaining.len();
            counts.ok = false;
            remaining.clear();
            break;
        };
        remaining.retain(|candidate| *candidate != index);
        let step = &wf.steps[index];
        send(
            &events,
            TestEvent::StepStarted {
                wf: wf.workflow_id.clone(),
                step: step.step_id.clone(),
            },
        )
        .await;
        match run_step(
            wf,
            step,
            base,
            http,
            &effective_inputs,
            &steps_outputs,
            auth_state,
            auth_config,
            components,
            messages,
            &events,
        )
        .await
        {
            StepOutcome::Passed(outputs) => {
                steps_outputs.insert(step.step_id.clone(), serde_json::Value::Object(outputs));
                completed.insert(step.step_id.clone());
                counts.passed += 1;
            }
            StepOutcome::Failed => {
                counts.failed += 1;
                counts.ok = false;
                // Follow an `onFailure` goto when the target step exists:
                // the scheduler re-enters at that step next.
                if let Some(target) = &step.failure_goto
                    && let Some(next) = wf.steps.iter().position(|s| &s.step_id == target)
                {
                    if completed.remove(&wf.steps[next].step_id) {
                        // Re-running a completed step invalidates what
                        // depended on it.
                        for dependent in wf.steps.iter() {
                            if dependent
                                .depends_on
                                .iter()
                                .any(|dependency| dependency == &wf.steps[next].step_id)
                                && !completed.contains(&dependent.step_id)
                            {
                                completed.remove(&dependent.step_id);
                            }
                        }
                    }
                    // Put the target back at the front of the queue.
                    remaining.retain(|candidate| *candidate != next);
                    remaining.insert(0, next);
                    continue;
                }
                // No recovery path: the rest of the workflow cannot run.
                counts.skipped += remaining.len();
                remaining.clear();
                counts.ok = false;
            }
        }
    }
    if counts.skipped == 0 {
        counts.skipped = wf.steps.len().saturating_sub(counts.passed + counts.failed);
    }

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

/// Runs one Arazzo 1.1 AsyncAPI step: `send` publishes the evaluated
/// payload, `receive` awaits a matching message. The outcome is projected
/// onto the response shape the criteria evaluator already understands, so
/// `$statusCode`, `$message.payload`, and step outputs behave the same for
/// HTTP and message steps.
#[allow(clippy::too_many_arguments)]
async fn run_message_step(
    wf_id: &str,
    step: &StepPlan,
    message_step: &crate::plan::MessageStep,
    inputs: &serde_json::Map<String, serde_json::Value>,
    steps_outputs: &serde_json::Map<String, serde_json::Value>,
    messages: Option<&dyn crate::messaging::MessageTransport>,
    events: &mpsc::Sender<TestEvent>,
) -> StepOutcome {
    let state_ctx = || {
        RexCtx::default()
            .inputs(inputs)
            .steps_outputs(steps_outputs)
    };
    let Some(transport) = messages else {
        send(
            events,
            TestEvent::CriterionFail {
                wf: wf_id.to_owned(),
                step: step.step_id.clone(),
                crit: "transport".to_owned(),
                expected: "a message transport".to_owned(),
                actual: "the run configured no message broker".to_owned(),
            },
        )
        .await;
        return StepOutcome::Failed;
    };

    let correlation_id = message_step
        .correlation_id
        .as_ref()
        .and_then(|rex| eval_rex(rex, &state_ctx()))
        .and_then(|value| match value {
            serde_json::Value::String(text) => Some(text),
            serde_json::Value::Null | serde_json::Value::Bool(_) => None,
            other => Some(other.to_string()),
        });

    let timeout = std::time::Duration::from_millis(step.timeout_ms.unwrap_or(5000));
    let outcome = match message_step.direction {
        crate::plan::MessageDirection::Send => {
            let payload = match &message_step.payload_template {
                Some(template) if !template.is_null() => {
                    resolve_payload_template(template, &state_ctx())
                }
                _ => message_step
                    .payload
                    .as_ref()
                    .and_then(|rex| eval_rex(rex, &state_ctx()))
                    .unwrap_or(serde_json::Value::Null),
            };
            transport
                .send(crate::messaging::Message {
                    channel: message_step.channel.clone(),
                    message_type: message_step.message_type.clone(),
                    correlation_id: correlation_id.clone(),
                    payload,
                })
                .map(|()| crate::messaging::Message {
                    channel: message_step.channel.clone(),
                    message_type: message_step.message_type.clone(),
                    correlation_id: correlation_id.clone(),
                    payload: serde_json::Value::Null,
                })
        }
        crate::plan::MessageDirection::Receive => {
            transport.receive(&message_step.channel, correlation_id.as_deref(), timeout)
        }
    };

    let message = match outcome {
        Ok(message) => message,
        Err(e) => {
            send(
                events,
                TestEvent::CriterionFail {
                    wf: wf_id.to_owned(),
                    step: step.step_id.clone(),
                    crit: "transport".to_owned(),
                    expected: format!("a message on `{}`", message_step.channel),
                    actual: e.to_string(),
                },
            )
            .await;
            return StepOutcome::Failed;
        }
    };

    // A received payload must satisfy the AsyncAPI message's declared
    // schema, exactly like a response body against its contract. A send
    // step publishes; its payload is the producer's business.
    if let (crate::plan::MessageDirection::Receive, Some(schema)) =
        (message_step.direction, &message_step.payload_schema)
    {
        let wrapper = format!(
            "{{\"schema\": {}, \"instance\": {}}}",
            serde_json::to_string(schema).unwrap_or_else(|_| "null".to_owned()),
            serde_json::to_string(&message.payload).unwrap_or_else(|_| "null".to_owned())
        );
        let mut violations = Vec::new();
        if let Ok(uri) = suspect_source::Uri::parse("mem://message-check.json") {
            let doc = suspect_low::LowDoc::parse(
                uri,
                suspect_source::Source::from_vec(wrapper.into_bytes()),
            );
            if doc.syntax_errors().is_empty()
                && let (Some(schema_node), Some(instance_node)) =
                    (doc.root().get("schema"), doc.root().get("instance"))
                && let Ok(compiled) =
                    suspect_schema::Compiler::new(suspect_schema::Config::default())
                        .compile(schema_node)
            {
                violations = compiled
                    .validate(instance_node)
                    .iter()
                    .map(|e| e.message.clone())
                    .collect();
            }
        }
        if !violations.is_empty() {
            send(
                events,
                TestEvent::CriterionFail {
                    wf: wf_id.to_owned(),
                    step: step.step_id.clone(),
                    crit: "payload".to_owned(),
                    expected: "a payload matching the declared message schema".to_owned(),
                    actual: violations.join("; "),
                },
            )
            .await;
            return StepOutcome::Failed;
        }
    }

    // Project the message onto a response the criteria evaluator reads:
    // 200 with the payload as the body and the correlation id as a header.
    let response = HttpResponse {
        status: 200,
        headers: message
            .correlation_id
            .as_ref()
            .map(|id| vec![("x-correlation-id".to_owned(), id.clone())])
            .unwrap_or_default(),
        body: Bytes::from(serde_json::to_vec(&message.payload).unwrap_or_default()),
    };
    evaluate_response(step, &response, inputs, steps_outputs, events, wf_id).await
}

/// Shared post-response path: success criteria, then output capture. HTTP
/// steps and Arazzo 1.1 message steps both end here, so `$message.payload`
/// and `$response.body#...` resolve through one evaluator.
async fn evaluate_response(
    step: &StepPlan,
    response: &HttpResponse,
    inputs: &serde_json::Map<String, serde_json::Value>,
    steps_outputs: &serde_json::Map<String, serde_json::Value>,
    events: &mpsc::Sender<TestEvent>,
    wf_id: &str,
) -> StepOutcome {
    let body_text = String::from_utf8_lossy(&response.body).into_owned();
    let body_json: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();

    let mut all_ok = true;
    for crit in &step.success {
        // Runtime-expectation criteria resolve their right-hand side
        // against the run state first (`$steps.<id>.outputs.<name>`,
        // `$inputs.<name>`), then evaluate as ordinary equality.
        let resolved = resolve_criterion(&crit.kind, inputs, steps_outputs);
        match eval_criterion(&resolved, response.status, body_json.as_ref(), &body_text) {
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

    let capture_ctx = RexCtx::default()
        .method("")
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

/// Resolves an equality criterion's runtime expectations against the run
/// state: `$steps.<id>.outputs.<name>` and `$inputs.<name>` become the
/// referenced values (or `null` when absent). Literals pass through
/// untouched, as do non-equality criteria.
fn resolve_criterion(
    crit: &CriterionKind,
    inputs: &serde_json::Map<String, serde_json::Value>,
    steps_outputs: &serde_json::Map<String, serde_json::Value>,
) -> CriterionKind {
    let CriterionKind::Equals { pointer, expected } = crit else {
        return crit.clone();
    };
    let expected = match expected {
        crate::plan::Expected::Literal(value) => {
            return CriterionKind::Equals {
                pointer: pointer.clone(),
                expected: crate::plan::Expected::Literal(value.clone()),
            };
        }
        crate::plan::Expected::StepOutput { step, name } => steps_outputs
            .get(step)
            .and_then(|outputs| outputs.get(name))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        crate::plan::Expected::Input(name) => {
            inputs.get(name).cloned().unwrap_or(serde_json::Value::Null)
        }
    };
    CriterionKind::Equals {
        pointer: pointer.clone(),
        expected: crate::plan::Expected::Literal(expected),
    }
}

/// Materializes a send payload: string leaves starting with `$` are
/// runtime expressions evaluated against the step context; everything else
/// is literal, and embedded expressions interpolate into their text.
fn resolve_payload_template(template: &serde_json::Value, ctx: &RexCtx<'_>) -> serde_json::Value {
    match template {
        serde_json::Value::String(text) => {
            if text.starts_with('$')
                && let Ok(rex) = suspect_rex::parse_rex(text)
                && let Some(value) = eval_rex(&rex, ctx)
            {
                return value;
            }
            let embedded: Option<Vec<suspect_arazzo::ExprPart>> = (!text.contains('{'))
                .then_some(Vec::new())
                .or_else(|| Some(suspect_arazzo::parse_embedded(text)))
                .filter(|parts| !parts.is_empty());
            if let Some(parts) = embedded {
                return serde_json::Value::String(
                    parts
                        .iter()
                        .map(|part| match part {
                            suspect_arazzo::ExprPart::Expr(rex) => {
                                // Re-parse the source expression text: the
                                // part carries the parsed form, but the rex
                                // evaluator works on the rex grammar.
                                let text = format!("${}", render_expr(rex));
                                let evaluated = suspect_rex::parse_rex(&text)
                                    .ok()
                                    .and_then(|parsed| eval_rex(&parsed, ctx));
                                evaluated.map_or_else(
                                    || text.clone(),
                                    |value| match value {
                                        serde_json::Value::String(s) => s,
                                        other => other.to_string(),
                                    },
                                )
                            }
                            suspect_arazzo::ExprPart::Text(text) => text.clone(),
                        })
                        .collect(),
                );
            }
            template.clone()
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| resolve_payload_template(item, ctx))
                .collect(),
        ),
        serde_json::Value::Object(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), resolve_payload_template(value, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Renders a parsed Arazzo expression back to its `$…` source form.
fn render_expr(expr: &suspect_arazzo::Expr) -> String {
    use suspect_arazzo::{ComponentKind, Expr, HttpPart};
    match expr {
        Expr::Method => "method".to_owned(),
        Expr::Url => "url".to_owned(),
        Expr::StatusCode => "statusCode".to_owned(),
        Expr::Request { part } | Expr::Response { part } | Expr::Message { part } => {
            let name = match part {
                HttpPart::Header(name) => format!("header.{name}"),
                HttpPart::Query(name) => format!("query.{name}"),
                HttpPart::Path(name) => format!("path.{name}"),
                HttpPart::Body(None) => "body".to_owned(),
                HttpPart::Body(Some(pointer)) => format!("body#{}", pointer.to_path()),
            };
            if matches!(expr, Expr::Request { .. }) {
                format!("request.{name}")
            } else if matches!(expr, Expr::Response { .. }) {
                format!("response.{name}")
            } else {
                format!("message.{name}")
            }
        }
        Expr::MessageHeader { name } => format!("message.headers.{name}"),
        Expr::CorrelationId => "message.correlationId".to_owned(),
        Expr::Outputs { name } => format!("outputs.{name}"),
        Expr::Inputs { name } => format!("inputs.{name}"),
        Expr::WorkflowOutput {
            workflow,
            step,
            name,
        } => format!("workflows.{workflow}.steps.{step}.outputs.{name}"),
        Expr::Component { kind, name } => {
            let kind = match kind {
                ComponentKind::Parameters => "parameters",
                ComponentKind::SucceedOn => "succeedOn",
                ComponentKind::FailureOn => "failureOn",
                ComponentKind::RetryOn => "retryOn",
            };
            format!("components.{kind}.{name}")
        }
        Expr::SourceDescription { name, path } => {
            if path.is_empty() {
                format!("sourceDescriptions.{name}")
            } else {
                format!("sourceDescriptions.{name}{path}")
            }
        }
        Expr::Text(text) => text.clone(),
    }
}

async fn send(events: &mpsc::Sender<TestEvent>, ev: TestEvent) {
    let _ = events.send(ev).await;
}

/// Builds and executes one step, then evaluates its success criteria.
#[allow(clippy::too_many_arguments)]
async fn run_step(
    wf: &crate::plan::WfPlan,
    step: &StepPlan,
    base: &str,
    http: &dyn HttpClient,
    inputs: &serde_json::Map<String, serde_json::Value>,
    steps_outputs: &serde_json::Map<String, serde_json::Value>,
    auth_state: &crate::auth::AuthState,
    auth_config: &crate::auth::AuthConfig,
    components: &std::collections::BTreeMap<String, serde_json::Value>,
    messages: Option<&dyn crate::messaging::MessageTransport>,
    events: &mpsc::Sender<TestEvent>,
) -> StepOutcome {
    let wf_id = wf.workflow_id.as_str();

    // Arazzo 1.1 AsyncAPI step: publish or await a message instead of
    // issuing an HTTP exchange. The result is shaped like a response so
    // criteria, outputs and `$message.payload` all evaluate uniformly.
    if let Some(message_step) = &step.message {
        return run_message_step(
            wf_id,
            step,
            message_step,
            inputs,
            steps_outputs,
            messages,
            events,
        )
        .await;
    }

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

    // Template-literal substitution: resolve `${$inputs.x}`, `${$steps.y.outputs.z}`,
    // etc. inside parameter values that contain embedded expressions.
    let resolve_templates = |text: &str| -> String {
        if !text.contains("${") {
            return text.to_owned();
        }
        let mut result = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            result.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find('}') {
                Some(end) => {
                    let expr_text = &after[..end];
                    match suspect_rex::parse_rex(expr_text) {
                        Ok(rex) => {
                            let val = eval_rex(&rex, &state_ctx());
                            match val {
                                Some(serde_json::Value::String(v)) => result.push_str(&v),
                                Some(other) => result.push_str(&other.to_string()),
                                None => {} // unresolved → empty
                            }
                        }
                        Err(_) => result.push_str(&rest[start..start + end + 1]), // keep literal
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    result.push_str(rest);
                    break;
                }
            }
        }
        result.push_str(rest);
        result
    };
    query = query
        .into_iter()
        .map(|(k, v)| (k, resolve_templates(&v)))
        .collect();
    headers = headers
        .into_iter()
        .map(|(k, v)| (k, resolve_templates(&v)))
        .collect();
    path = resolve_templates(&path);

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
    // Ask for JSON responses so JSONPath criteria work against APIs that
    // support content negotiation (Plex returns XML by default).
    if !request_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept"))
    {
        request_headers.push(("Accept".to_owned(), "application/json".to_owned()));
    }

    // Security injection: for every configured scheme named by the
    // operation's security requirements, resolve credentials and inject
    // them (explicit step parameters win).
    let mut injected: Vec<crate::auth::Injected> = Vec::new();
    for alternative in &step.security {
        for scheme in alternative {
            if let Some(credential) = auth_config.schemes.get(scheme) {
                match auth_state.resolve(http, scheme, credential).await {
                    Ok(Some(placement)) => injected.push(placement),
                    Ok(None) => {}
                    Err(message) => {
                        send(
                            events,
                            TestEvent::CriterionFail {
                                wf: wf_id.to_owned(),
                                step: step.step_id.clone(),
                                crit: format!("auth:{scheme}"),
                                expected: "credential acquisition".to_owned(),
                                actual: message,
                            },
                        )
                        .await;
                        return StepOutcome::Failed;
                    }
                }
            }
        }
    }

    let url = join_url(base, &path, &query);
    let mut request = HttpRequest {
        method: method.to_owned(),
        url: url.clone(),
        headers: request_headers,
        body: Bytes::from(body.unwrap_or_default()),
    };
    crate::auth::inject(&mut request, &injected);

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
    // Arazzo 1.1 step `timeout`: exceeding it fails the step as a
    // transport error.
    let dispatched = if let Some(timeout_ms) = step.timeout_ms {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            http.execute(request),
        )
        .await
        .unwrap_or_else(|_| {
            Err(TransportError(format!(
                "step exceeded its {timeout_ms}ms timeout"
            )))
        })
    } else {
        http.execute(request).await
    };
    let response = match dispatched {
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

    // Contract validation: the shared contract runtime owns the decision,
    // so the runner and the gateway cannot disagree about whether a
    // response conforms. It runs whether or not the step wrote criteria.
    let response_failures_local: Option<Vec<String>> = {
        let schemas = suspect_runtime::Schemas::from_any_map(components.iter());
        let violations = suspect_runtime::validate_response(
            &step.declared_responses(),
            &schemas,
            response.status,
            &response.body,
        );
        if !declares_response(step, response.status) {
            // Nothing declared for this status: nothing to check.
            None
        } else if violations.is_empty() {
            send(
                events,
                TestEvent::ResponseValidated {
                    wf: wf_id.to_owned(),
                    step: step.step_id.clone(),
                    status: response.status,
                },
            )
            .await;
            None
        } else {
            Some(
                violations
                    .iter()
                    .map(|violation| violation.message.clone())
                    .collect(),
            )
        }
    };
    let response_failures: Vec<String> = response_failures_local.unwrap_or_default();

    let mut all_ok = true;
    if !response_failures.is_empty() {
        all_ok = false;
        send(
            events,
            TestEvent::CriterionFail {
                wf: wf_id.to_owned(),
                step: step.step_id.clone(),
                crit: "response-schema".to_owned(),
                expected: "response matching the declared schema".to_owned(),
                actual: response_failures.join("; "),
            },
        )
        .await;
    }
    for crit in &step.success {
        // Runtime-expectation criteria resolve their right-hand side
        // against the run state first (`$steps.<id>.outputs.<name>`,
        // `$inputs.<name>`), then evaluate as ordinary equality.
        let resolved = resolve_criterion(&crit.kind, inputs, steps_outputs);
        match eval_criterion(&resolved, response.status, body_json.as_ref(), &body_text) {
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

/// Whether the step declares any response for this status (exact or
/// `default`), which is the condition for a conformance check at all.
fn declares_response(step: &StepPlan, status: u16) -> bool {
    step.response_schemas
        .iter()
        .any(|(declared, _)| *declared == Some(status) || declared.is_none())
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
        CriterionKind::Equals { pointer, expected } => {
            // Runtime expectations are resolved against the run state
            // before evaluation; only literals reach here.
            let expected = match expected {
                crate::plan::Expected::Literal(value) => value,
                _ => &serde_json::Value::Null,
            };
            match pointer {
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
            }
        }
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
        CriterionKind::Compare {
            pointer,
            comparison,
            bound,
        } => {
            let render = |actual: &str| {
                (
                    format!("body{pointer} {} {bound}", comparison.sign()),
                    format!("body{pointer} = {actual}"),
                )
            };
            let Some(actual) = resolve_pointer(body_json, pointer) else {
                return Err(render("<missing>"));
            };
            // Numbers compare numerically; strings lexicographically;
            // anything else cannot be ordered.
            let ordered = match (actual, bound) {
                (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a
                    .as_f64()
                    .zip(b.as_f64())
                    .map(|(a, b)| comparison.holds(a, b)),
                (serde_json::Value::String(a), serde_json::Value::String(b)) => {
                    Some(comparison.holds(a, b))
                }
                _ => None,
            };
            match ordered {
                Some(true) => Ok(()),
                Some(false) => {
                    let (expected_render, actual_render) = render(&actual.to_string());
                    Err((expected_render, actual_render))
                }
                None => Err((
                    format!("body{pointer} {} {bound}", comparison.sign()),
                    format!("body{pointer} = {actual} (not orderable with {bound})"),
                )),
            }
        }
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
        CriterionKind::AlwaysTrue => Ok(()),
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

#[cfg(test)]
mod debug_wrapper_tests {

    #[tokio::test]
    async fn wrapper_resolves_component_refs() {
        let schema: serde_json::Value =
            serde_json::from_str(r##"{"$ref": "#/components/schemas/Pet"}"##).unwrap();
        let mut components = std::collections::BTreeMap::new();
        components.insert(
            "Pet".to_owned(),
            serde_json::from_str::<serde_json::Value>(
                r##"{"type": "object", "required": ["id", "name"]}"##,
            )
            .unwrap(),
        );
        let mut schemas_section = serde_json::Map::new();
        for (name, schema_json) in &components {
            schemas_section.insert(name.clone(), schema_json.clone());
        }
        let mut components_json = serde_json::Map::new();
        components_json.insert(
            "components".to_owned(),
            serde_json::Value::Object({
                let mut top = serde_json::Map::new();
                top.insert(
                    "schemas".to_owned(),
                    serde_json::Value::Object(schemas_section),
                );
                top
            }),
        );
        let schema_serialized = serde_json::to_string(&schema).unwrap();
        let components_serialized =
            serde_json::to_string(&serde_json::Value::Object(components_json)).unwrap();
        let (components_body, _) = components_serialized.rsplit_once('}').unwrap_or(("", ""));
        let wrapper = components_body.to_owned()
            + ", \"schema\": "
            + &schema_serialized
            + ", \"instance\": {}";
        println!("wrapper: {wrapper}");
        let uri = suspect_source::Uri::parse("mem://w.json").unwrap();
        let doc = suspect_low::LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(wrapper.as_bytes().to_vec()),
        );
        let schema_node = doc.root().get("schema").unwrap();
        let instance_node = doc.root().get("instance").unwrap();
        let compiled = suspect_schema::Compiler::new(suspect_schema::Config::default())
            .compile(schema_node)
            .unwrap();
        let errors = compiled.validate(instance_node);
        assert_eq!(
            errors.len(),
            1,
            "instance {{}} must fail required [id, name]"
        );
    }
}
