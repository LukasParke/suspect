//! Plan execution: HTTP transport abstraction, observer seam, and the
//! workflow runners [`run_plan`] / [`run_workflow`] / [`run_workflow_owned`].
//!
//! Every step builds its request by evaluating parameter/body runtime
//! expressions against the workflow state (`$inputs...`,
//! `$steps.<id>.outputs.<key>`, `$workflows.<id>.outputs.<key>`), injects
//! configured credentials, sends it over the injected [`HttpClient`] (or a
//! [`MessageTransport`] for Arazzo 1.1
//! message steps), validates the response against the declared contract,
//! evaluates success criteria, and captures declared outputs into the
//! workflow-scoped step map that later steps read.
//!
//! Progress is reported through an [`ExecObserver`]. [`EventObserver`]
//! renders the classic [`TestEvent`] stream used by `suspect test` and the
//! editors; load generation installs a lightweight recorder that only
//! listens to [`ExecObserver::on_step_done`].
//!
//! [`run_plan`] runs a whole document once: workflows execute concurrently
//! within [`dependency_waves`](crate::plan::dependency_waves) (so workflow
//! `dependsOn` prerequisites finish first); inside each workflow a
//! scheduler runs steps one at a time in document order, honouring
//! step-level `dependsOn` and implicit output references.
//! [`run_workflow_owned`] runs one workflow as a `'static` future suitable for
//! `tokio::spawn`, which is how load generation runs thousands of virtual
//! users over one shared [`Plan`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use suspect_arazzo::rex::{RexCtx, eval_rex};
use suspect_ir::ParamIn;
use tokio::sync::mpsc;

use crate::auth::{AuthConfig, AuthState};
use crate::messaging::MessageTransport;
use crate::plan::{CriterionKind, CriterionPlan, OpKey, Plan, StepPlan, ThinkSpec, WfPlan};

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

/// Coarse classification of a transport failure, used for error
/// breakdowns in load reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// The request exceeded the client timeout.
    Timeout,
    /// The connection could not be established (refused, DNS, unreachable).
    Connect,
    /// TLS handshake or certificate failure.
    Tls,
    /// The peer closed or reset an established connection mid-exchange.
    Reset,
    /// Anything else (including canned/replay transport misses).
    Other,
}

impl ErrorClass {
    /// Stable lowercase name used as a message prefix and metric key.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Reset => "reset",
            Self::Other => "other",
        }
    }
}

/// Transport-level failure (connection error, no canned rule, cassette
/// exhausted, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(
    /// Human-readable description of the failure.
    pub String,
);

impl TransportError {
    /// Builds an error whose message is prefixed with the class name
    /// (`timeout: ...`), so [`TransportError::class`] recovers it exactly.
    #[must_use]
    pub fn classified(class: ErrorClass, detail: impl fmt::Display) -> Self {
        Self(format!("{}: {detail}", class.as_str()))
    }

    /// Classifies the failure. Messages built with
    /// [`TransportError::classified`] are matched by prefix; other messages
    /// fall back to keyword heuristics and finally [`ErrorClass::Other`].
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        for class in [
            ErrorClass::Timeout,
            ErrorClass::Connect,
            ErrorClass::Tls,
            ErrorClass::Reset,
            ErrorClass::Other,
        ] {
            if self
                .0
                .strip_prefix(class.as_str())
                .is_some_and(|rest| rest.starts_with(':'))
            {
                return class;
            }
        }
        // Order matters: "connection reset" must classify as Reset, not
        // Connect, so the reset keywords are checked first.
        let lower = self.0.to_ascii_lowercase();
        if lower.contains("timed out") || lower.contains("timeout") {
            ErrorClass::Timeout
        } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
            ErrorClass::Tls
        } else if lower.contains("reset") || lower.contains("broken pipe") || lower.contains("eof")
        {
            ErrorClass::Reset
        } else if lower.contains("refused")
            || lower.contains("connect")
            || lower.contains("dns")
            || lower.contains("unreachable")
        {
            ErrorClass::Connect
        } else {
            ErrorClass::Other
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportError {}

/// Minimal async HTTP client abstraction used by the runners.
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

    /// Like [`HttpClient::execute`], additionally reporting time to first
    /// byte (headers received) when the transport can measure it. The
    /// default implementation reports `None`.
    ///
    /// # Errors
    /// Returns a [`TransportError`] when no response can be produced.
    async fn execute_timed(
        &self,
        req: HttpRequest,
    ) -> Result<(HttpResponse, Option<Duration>), TransportError> {
        self.execute(req).await.map(|resp| (resp, None))
    }
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

/// Identifies one execution of a workflow: a virtual user in load mode,
/// `VuId(0)` in contract-test mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct VuId(pub u64);

/// Final verdict of one executed step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepVerdict {
    /// Response received and every check passed.
    Passed,
    /// Response received but this many checks failed (success criteria
    /// plus response-schema conformance).
    CriteriaFailed(usize),
    /// No response: the transport failed, or the request could not be
    /// sent at all (credential acquisition, missing message broker).
    Transport(ErrorClass),
}

/// Compact measurement of one executed step, delivered to
/// [`ExecObserver::on_step_done`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSample {
    /// Response status, `None` on transport failure.
    pub status: Option<u16>,
    /// Time from sending the request to the last body byte (or to the
    /// transport failure).
    pub latency: Duration,
    /// Time to first byte when the transport reports it.
    pub ttfb: Option<Duration>,
    /// Request body bytes sent.
    pub bytes_out: usize,
    /// Response body bytes received.
    pub bytes_in: usize,
    /// Final verdict.
    pub verdict: StepVerdict,
}

impl StepSample {
    /// Whether the step passed.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.verdict == StepVerdict::Passed
    }

    /// A sample for a step that never produced a response.
    fn unsent(class: ErrorClass, latency: Duration, bytes_out: usize) -> Self {
        Self {
            status: None,
            latency,
            ttfb: None,
            bytes_out,
            bytes_in: 0,
            verdict: StepVerdict::Transport(class),
        }
    }
}

/// Where an observer callback originates: workflow, step and target
/// operation, plus the executing virtual user.
#[derive(Debug, Clone, Copy)]
pub struct StepCx<'a> {
    /// The Arazzo `workflowId`.
    pub workflow: &'a str,
    /// Index of the workflow in [`Plan::workflows`].
    pub wf_index: usize,
    /// The Arazzo `stepId`.
    pub step: &'a str,
    /// Index of the step in [`WfPlan::steps`].
    pub step_index: usize,
    /// Resolved target operation.
    pub op: &'a OpKey,
    /// Executing virtual user.
    pub vu: VuId,
}

/// Outcome of one workflow execution.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WfOutcome {
    /// Whether every executed step passed.
    pub ok: bool,
    /// Steps that passed.
    pub passed: usize,
    /// Steps that failed (transport or criteria).
    pub failed: usize,
    /// Steps never attempted.
    pub skipped: usize,
    /// Wall-clock duration, think time included.
    pub duration: Duration,
    /// Workflow-level outputs (`WfPlan::outputs`) evaluated after the run.
    pub outputs: serde_json::Map<String, serde_json::Value>,
    /// Every step's captured outputs keyed by step id.
    pub steps_outputs: serde_json::Map<String, serde_json::Value>,
}

/// Receives execution progress. Every method is a no-op by default so
/// implementations subscribe only to what they need.
#[async_trait]
pub trait ExecObserver: Send + Sync {
    /// A workflow execution began.
    async fn on_workflow_started(&self, _wf: &WfPlan, _vu: VuId) {}
    /// A step is about to build its request.
    async fn on_step_started(&self, _cx: &StepCx<'_>) {}
    /// A request is leaving the executor.
    async fn on_request(&self, _cx: &StepCx<'_>, _req: &HttpRequest) {}
    /// A response arrived (before criteria evaluation).
    async fn on_response(&self, _cx: &StepCx<'_>, _status: u16, _latency: Duration) {}
    /// The response body conformed to the schema declared for its status.
    async fn on_response_validated(&self, _cx: &StepCx<'_>, _status: u16) {}
    /// The transport failed to produce a response.
    async fn on_transport_error(
        &self,
        _cx: &StepCx<'_>,
        _error: &TransportError,
        _latency: Duration,
    ) {
    }
    /// A built-in check that is not a declared success criterion failed:
    /// credential acquisition (`auth:<scheme>`), response-schema
    /// conformance (`response-schema`), message transport/payload checks,
    /// or an unsatisfiable step order (`depends-on`).
    async fn on_check_failed(
        &self,
        _cx: &StepCx<'_>,
        _check: &str,
        _expected: &str,
        _actual: &str,
    ) {
    }
    /// One criterion was evaluated; `failure` carries `(expected, actual)`
    /// renderings when it did not hold.
    async fn on_criterion(
        &self,
        _cx: &StepCx<'_>,
        _crit: &CriterionPlan,
        _failure: Option<(&str, &str)>,
    ) {
    }
    /// A step output was captured.
    async fn on_output(&self, _cx: &StepCx<'_>, _key: &str, _value: &serde_json::Value) {}
    /// A step finished with its final verdict and measurements.
    async fn on_step_done(&self, _cx: &StepCx<'_>, _sample: &StepSample) {}
    /// A workflow execution finished.
    async fn on_workflow_done(&self, _wf: &WfPlan, _vu: VuId, _outcome: &WfOutcome) {}
}

/// Observer that discards everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullObserver;

#[async_trait]
impl ExecObserver for NullObserver {}

/// Observer rendering the [`TestEvent`] stream consumed by `suspect test`
/// and the editor integrations. Sends are awaited, so a bounded channel
/// applies backpressure to the executor.
#[derive(Debug, Clone)]
pub struct EventObserver(pub mpsc::Sender<TestEvent>);

impl EventObserver {
    async fn send(&self, ev: TestEvent) {
        let _ = self.0.send(ev).await;
    }
}

#[async_trait]
impl ExecObserver for EventObserver {
    async fn on_workflow_started(&self, wf: &WfPlan, _vu: VuId) {
        self.send(TestEvent::WfStarted {
            id: wf.workflow_id.clone(),
        })
        .await;
    }

    async fn on_step_started(&self, cx: &StepCx<'_>) {
        self.send(TestEvent::StepStarted {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
        })
        .await;
    }

    async fn on_request(&self, cx: &StepCx<'_>, req: &HttpRequest) {
        self.send(TestEvent::RequestSent {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
            method: req.method.clone(),
            url: req.url.clone(),
        })
        .await;
    }

    async fn on_response(&self, cx: &StepCx<'_>, status: u16, latency: Duration) {
        self.send(TestEvent::ResponseGot {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
            status,
            duration_ms: u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
        })
        .await;
    }

    async fn on_response_validated(&self, cx: &StepCx<'_>, status: u16) {
        self.send(TestEvent::ResponseValidated {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
            status,
        })
        .await;
    }

    async fn on_transport_error(
        &self,
        cx: &StepCx<'_>,
        error: &TransportError,
        _latency: Duration,
    ) {
        self.send(TestEvent::CriterionFail {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
            crit: "transport".to_owned(),
            expected: "an HTTP response".to_owned(),
            actual: error.to_string(),
        })
        .await;
    }

    async fn on_check_failed(&self, cx: &StepCx<'_>, check: &str, expected: &str, actual: &str) {
        self.send(TestEvent::CriterionFail {
            wf: cx.workflow.to_owned(),
            step: cx.step.to_owned(),
            crit: check.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
        .await;
    }

    async fn on_criterion(
        &self,
        cx: &StepCx<'_>,
        crit: &CriterionPlan,
        failure: Option<(&str, &str)>,
    ) {
        let ev = match failure {
            None => TestEvent::CriterionOk {
                wf: cx.workflow.to_owned(),
                step: cx.step.to_owned(),
                crit: crit.describe(),
            },
            Some((expected, actual)) => TestEvent::CriterionFail {
                wf: cx.workflow.to_owned(),
                step: cx.step.to_owned(),
                crit: crit.describe(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            },
        };
        self.send(ev).await;
    }

    async fn on_output(&self, cx: &StepCx<'_>, key: &str, value: &serde_json::Value) {
        self.send(TestEvent::OutputSet {
            wf: cx.workflow.to_owned(),
            key: key.to_owned(),
            value: value.clone(),
        })
        .await;
    }

    async fn on_workflow_done(&self, wf: &WfPlan, _vu: VuId, outcome: &WfOutcome) {
        self.send(TestEvent::WfDone {
            wf: wf.workflow_id.clone(),
            passed: outcome.ok,
        })
        .await;
    }
}

/// Whether and how step think time (`x-suspect-think`) is honoured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkPolicy {
    /// Never pause (contract-test default: fast and deterministic).
    #[default]
    Ignore,
    /// Pause after each non-final step: the step's own `x-suspect-think`,
    /// else `default` when set, else nothing.
    Honour {
        /// Fallback pause for steps without an explicit extension.
        default: Option<ThinkSpec>,
    },
}

/// Workflow inputs: a JSON object keyed by input name.
pub type InputMap = serde_json::Map<String, serde_json::Value>;

/// Credentials a run injects into requests: the configured schemes plus
/// the token cache shared by every execution that holds this value (so
/// virtual users reuse one OAuth token instead of each acquiring one).
#[derive(Default)]
pub struct RunAuth {
    /// Token cache for client-credentials / refresh-token grants.
    pub state: AuthState,
    /// Per-scheme credential configuration.
    pub config: AuthConfig,
}

impl RunAuth {
    /// Wraps a credential configuration with a fresh token cache.
    #[must_use]
    pub fn new(config: AuthConfig) -> Self {
        Self {
            state: AuthState::default(),
            config,
        }
    }
}

impl fmt::Debug for RunAuth {
    // Never render credential values: only the configured scheme names.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunAuth")
            .field("schemes", &self.config.schemes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Per-execution options for [`run_workflow`] / [`run_workflow_owned`].
#[derive(Debug, Clone)]
pub struct RunOpts {
    /// Base URL prepended to operation paths.
    pub base_url: Arc<str>,
    /// Caller-supplied inputs; override schema defaults and the Arazzo
    /// `parameters` block.
    pub inputs: InputMap,
    /// Outputs of already-finished workflows, keyed by workflow id, each as
    /// `{"outputs": {...}, "steps": {stepId: {...}}}`; read by
    /// `$workflows.<id>...` expressions.
    pub workflows_outputs: Arc<InputMap>,
    /// Think-time policy.
    pub think: ThinkPolicy,
    /// Identity of this execution.
    pub vu: VuId,
    /// Seed for think-time jitter (combined with `vu`).
    pub seed: u64,
    /// Credentials injected per the operations' security requirements.
    pub auth: Arc<RunAuth>,
    /// Validate each response body against the schema the operation
    /// declares for its status (contract conformance). On by default; load
    /// generation turns it off unless asked, since it costs a schema
    /// compile per response on the generator.
    pub validate_responses: bool,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            base_url: Arc::from(""),
            inputs: InputMap::new(),
            workflows_outputs: Arc::new(InputMap::new()),
            think: ThinkPolicy::Ignore,
            vu: VuId(0),
            seed: 0,
            auth: Arc::new(RunAuth::default()),
            validate_responses: true,
        }
    }
}

impl RunOpts {
    /// Options with only a base URL set.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: Arc::from(base_url.trim_end_matches('/')),
            ..Self::default()
        }
    }
}

/// Borrowed resources every step of one run shares.
#[derive(Clone, Copy)]
struct Env<'a> {
    http: &'a dyn HttpClient,
    observer: &'a dyn ExecObserver,
    auth_state: &'a AuthState,
    auth_config: &'a AuthConfig,
    messages: Option<&'a dyn MessageTransport>,
    components: &'a BTreeMap<String, serde_json::Value>,
}

impl<'a> Env<'a> {
    /// Environment drawing credentials from `opts.auth`, without a message
    /// transport.
    fn from_opts(
        plan: &'a Plan,
        opts: &'a RunOpts,
        http: &'a dyn HttpClient,
        observer: &'a dyn ExecObserver,
    ) -> Self {
        Self {
            http,
            observer,
            auth_state: &opts.auth.state,
            auth_config: &opts.auth.config,
            messages: None,
            components: &plan.components,
        }
    }
}

/// Upper bound on `onFailure` goto jumps within one workflow run. A goto
/// re-enters the workflow at its target, so a target that keeps failing
/// back into the same goto would otherwise re-send requests forever.
pub const MAX_GOTO_JUMPS: usize = 16;

/// Runs a compiled [`Plan`] once against `http`, emitting the classic
/// [`TestEvent`] stream on `events`.
///
/// Workflows run concurrently inside each dependency wave; steps run one at
/// a time inside each workflow. The first failing step without an
/// `onFailure` goto ends its workflow; remaining steps are counted as
/// skipped. A goto re-runs the workflow from its target step onward, at
/// most [`MAX_GOTO_JUMPS`] times per workflow run.
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
    messages: Option<&dyn MessageTransport>,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    run_plan_full(
        plan,
        base_url,
        http,
        &AuthState::default(),
        &AuthConfig::default(),
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
    auth_state: &AuthState,
    auth_config: &AuthConfig,
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
    auth_state: &AuthState,
    auth_config: &AuthConfig,
    inputs: &serde_json::Map<String, serde_json::Value>,
    messages: Option<&dyn MessageTransport>,
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

/// [`run_plan_with_auth`] plus caller inputs and a message transport for
/// Arazzo 1.1 AsyncAPI send/receive steps.
#[allow(clippy::too_many_arguments)]
pub async fn run_plan_full(
    plan: &Plan,
    base_url: &str,
    http: &dyn HttpClient,
    auth_state: &AuthState,
    auth_config: &AuthConfig,
    inputs: &serde_json::Map<String, serde_json::Value>,
    messages: Option<&dyn MessageTransport>,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    let opts = RunOpts {
        inputs: inputs.clone(),
        ..RunOpts::new(base_url)
    };
    let observer = EventObserver(events.clone());
    let env = Env {
        http,
        observer: &observer,
        auth_state,
        auth_config,
        messages,
        components: &plan.components,
    };
    let summary = run_plan_in(plan, &opts, env).await;
    let _ = events
        .send(TestEvent::RunDone {
            passed: summary.passed,
            failed: summary.failed,
        })
        .await;
    summary
}

/// [`run_plan`] with explicit options (caller inputs, think policy,
/// credentials). `opts.workflows_outputs` and `opts.vu` are ignored: the
/// runner owns them.
#[must_use]
pub async fn run_plan_with(
    plan: &Plan,
    opts: &RunOpts,
    http: &dyn HttpClient,
    events: mpsc::Sender<TestEvent>,
) -> RunSummary {
    let observer = EventObserver(events.clone());
    let summary = run_plan_observed(plan, opts, http, &observer).await;
    let _ = events
        .send(TestEvent::RunDone {
            passed: summary.passed,
            failed: summary.failed,
        })
        .await;
    summary
}

/// Runs a compiled [`Plan`] once, reporting through any observer. Unlike
/// [`run_plan`], no terminal event is emitted; the summary is the result.
#[must_use]
pub async fn run_plan_observed(
    plan: &Plan,
    opts: &RunOpts,
    http: &dyn HttpClient,
    observer: &dyn ExecObserver,
) -> RunSummary {
    run_plan_in(plan, opts, Env::from_opts(plan, opts, http, observer)).await
}

/// Runs every workflow of `plan` once, wave by wave.
async fn run_plan_in(plan: &Plan, opts: &RunOpts, env: Env<'_>) -> RunSummary {
    let start = Instant::now();
    let waves = crate::plan::dependency_waves(&plan.workflows)
        .unwrap_or_else(|_| vec![(0..plan.workflows.len()).collect()]);

    let mut summary = RunSummary::default();
    let mut workflows_outputs = InputMap::new();
    for wave in waves {
        let shared = Arc::new(workflows_outputs.clone());
        let mut running: Vec<BorrowedFuture<'_, (usize, WfOutcome)>> =
            Vec::with_capacity(wave.len());
        for idx in wave {
            let opts = RunOpts {
                workflows_outputs: Arc::clone(&shared),
                vu: VuId(0),
                ..opts.clone()
            };
            running.push(Box::pin(async move {
                let outcome = run_workflow_in(plan, idx, &opts, env).await;
                (idx, outcome)
            }));
        }
        for (idx, outcome) in join_all_borrowed(running).await {
            summary.passed += outcome.passed;
            summary.failed += outcome.failed;
            summary.skipped += outcome.skipped;
            workflows_outputs.insert(
                plan.workflows[idx].workflow_id.clone(),
                workflow_outputs_value(&outcome),
            );
        }
    }
    summary.duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    summary
}

/// Renders a finished workflow's outputs in the shape `$workflows.<id>...`
/// expressions read: `{"outputs": {...}, "steps": {stepId: {...}}}`.
#[must_use]
pub fn workflow_outputs_value(outcome: &WfOutcome) -> serde_json::Value {
    serde_json::json!({
        "outputs": outcome.outputs,
        "steps": outcome.steps_outputs,
    })
}

/// A pinned, boxed future borrowing the caller's stack.
type BorrowedFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Drives borrowed futures to completion cooperatively inside the current
/// task (they cannot be spawned because they borrow the caller's stack).
async fn join_all_borrowed<T>(mut running: Vec<BorrowedFuture<'_, T>>) -> Vec<T> {
    let mut finished = Vec::with_capacity(running.len());
    if running.is_empty() {
        return finished;
    }
    std::future::poll_fn(|cx| {
        for idx in (0..running.len()).rev() {
            if let std::task::Poll::Ready(out) = running[idx].as_mut().poll(cx) {
                finished.push(out);
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
    finished
}

/// Runs one workflow of a shared plan as a `'static` future, suitable for
/// `tokio::spawn`. `wf_index` addresses [`Plan::workflows`].
///
/// # Panics
/// Panics when `wf_index` is out of range.
pub async fn run_workflow_owned(
    plan: Arc<Plan>,
    wf_index: usize,
    opts: RunOpts,
    http: Arc<dyn HttpClient>,
    observer: Arc<dyn ExecObserver>,
) -> WfOutcome {
    run_workflow(&plan, wf_index, &opts, http.as_ref(), observer.as_ref()).await
}

/// Runs workflow `wf_index` of `plan` once and returns its outcome.
/// Credentials come from `opts.auth`; message steps fail (no broker).
///
/// # Panics
/// Panics when `wf_index` is out of range.
pub async fn run_workflow(
    plan: &Plan,
    wf_index: usize,
    opts: &RunOpts,
    http: &dyn HttpClient,
    observer: &dyn ExecObserver,
) -> WfOutcome {
    run_workflow_in(
        plan,
        wf_index,
        opts,
        Env::from_opts(plan, opts, http, observer),
    )
    .await
}

/// Runs a single workflow, one step at a time, and returns its outcome.
async fn run_workflow_in(plan: &Plan, wf_index: usize, opts: &RunOpts, env: Env<'_>) -> WfOutcome {
    let wf = &plan.workflows[wf_index];
    let observer = env.observer;
    let started = Instant::now();
    observer.on_workflow_started(wf, opts.vu).await;

    // Inputs, most specific first: caller-provided overrides, then the
    // workflow's static `parameters`, then schema-declared defaults.
    let mut effective_inputs = opts.inputs.clone();
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

    // Step outputs keyed by stepId; each value is that step's outputs object.
    let mut steps_outputs = InputMap::new();
    let mut outcome = WfOutcome {
        ok: true,
        ..WfOutcome::default()
    };
    let mut rng: Option<rand::rngs::StdRng> = None;
    let step_cx = |index: usize| {
        let step = &wf.steps[index];
        StepCx {
            workflow: &wf.workflow_id,
            wf_index,
            step: &step.step_id,
            step_index: index,
            op: &step.operation,
            vu: opts.vu,
        }
    };

    // Arazzo 1.1 §5.8.5.2.4: explicit `dependsOn` and implicit output
    // references both order execution. The scheduler takes the first
    // remaining step whose dependencies are all satisfied, in document
    // order, so a sequential run still satisfies the graph and a step
    // whose dependency has not run yet waits rather than reading a
    // missing output. A dependency on another workflow's step is met when
    // that workflow (a prerequisite, so already finished) captured the
    // step's outputs, i.e. the step passed.
    let mut completed: BTreeSet<String> = BTreeSet::new();
    let mut remaining: Vec<usize> = (0..wf.steps.len()).collect();
    let mut goto_jumps = 0usize;
    let foreign_done = |dependency: &crate::plan::WorkflowStepRef| {
        opts.workflows_outputs
            .get(&dependency.workflow)
            .and_then(|outputs| outputs.get("steps"))
            .and_then(|steps| steps.get(&dependency.step))
            .is_some()
    };

    while !remaining.is_empty() {
        let ready = remaining.iter().copied().find(|index| {
            let step = &wf.steps[*index];
            step.depends_on
                .iter()
                .all(|dependency| completed.contains(dependency))
                && step.workflow_depends_on.iter().all(foreign_done)
        });
        let Some(index) = ready else {
            // No remaining step can run: the dependency graph cannot be
            // satisfied (a cycle, or a dependency on a step that failed or
            // never ran). The first blocked step fails with the diagnosis
            // so the run cannot pass; the rest are skipped.
            let blocked: Vec<String> = remaining
                .iter()
                .map(|index| {
                    let step = &wf.steps[*index];
                    let unmet: Vec<String> = step
                        .depends_on
                        .iter()
                        .filter(|dependency| !completed.contains(*dependency))
                        .cloned()
                        .chain(
                            step.workflow_depends_on
                                .iter()
                                .filter(|dependency| !foreign_done(dependency))
                                .map(|dependency| {
                                    format!(
                                        "$workflows.{}.steps.{}",
                                        dependency.workflow, dependency.step
                                    )
                                }),
                        )
                        .collect();
                    format!("{} (waits on {})", step.step_id, unmet.join(", "))
                })
                .collect();
            observer
                .on_check_failed(
                    &step_cx(remaining[0]),
                    "depends-on",
                    "a satisfiable step order",
                    &format!(
                        "blocked by unsatisfied dependencies: {}",
                        blocked.join("; ")
                    ),
                )
                .await;
            outcome.failed += 1;
            outcome.skipped += remaining.len() - 1;
            outcome.ok = false;
            remaining.clear();
            break;
        };
        remaining.retain(|candidate| *candidate != index);
        let step = &wf.steps[index];
        let cx = step_cx(index);
        observer.on_step_started(&cx).await;
        let result = run_step(&cx, step, opts, env, &effective_inputs, &steps_outputs).await;
        observer.on_step_done(&cx, &result.sample).await;
        if result.sample.ok() {
            steps_outputs.insert(
                step.step_id.clone(),
                serde_json::Value::Object(result.outputs),
            );
            completed.insert(step.step_id.clone());
            outcome.passed += 1;
            if !remaining.is_empty()
                && let Some(pause) = think_after(step, opts.think, &mut rng, opts)
            {
                tokio::time::sleep(pause).await;
            }
        } else {
            outcome.failed += 1;
            outcome.ok = false;
            // Follow an `onFailure` goto when the target step exists:
            // execution jumps to the target and continues from there in
            // document order, so the target and every later step (the
            // failed one included) run again against the fresh state.
            if let Some(target) = &step.failure_goto
                && let Some(next) = wf.steps.iter().position(|s| &s.step_id == target)
            {
                goto_jumps += 1;
                if goto_jumps > MAX_GOTO_JUMPS {
                    observer
                        .on_check_failed(
                            &cx,
                            "on-failure-goto",
                            &format!("at most {MAX_GOTO_JUMPS} onFailure goto jumps"),
                            &format!("goto '{target}' did not recover the workflow"),
                        )
                        .await;
                    outcome.skipped += remaining.len();
                    remaining.clear();
                    continue;
                }
                for rerun in next..wf.steps.len() {
                    let step_id = &wf.steps[rerun].step_id;
                    steps_outputs.remove(step_id);
                    if completed.remove(step_id) {
                        // Its re-run is counted afresh.
                        outcome.passed -= 1;
                    }
                    if !remaining.contains(&rerun) {
                        remaining.push(rerun);
                    }
                }
                remaining.sort_unstable();
                continue;
            }
            // No recovery path: the rest of the workflow cannot run.
            outcome.skipped += remaining.len();
            remaining.clear();
        }
    }
    if outcome.skipped == 0 {
        outcome.skipped = wf
            .steps
            .len()
            .saturating_sub(outcome.passed + outcome.failed);
    }

    if outcome.ok {
        let ctx = RexCtx::default()
            .inputs(&effective_inputs)
            .steps_outputs(&steps_outputs)
            .workflows_outputs(&opts.workflows_outputs);
        for (name, rex) in &wf.outputs {
            if let Some(value) = eval_rex(rex, &ctx) {
                outcome.outputs.insert(name.clone(), value);
            }
        }
    }
    outcome.steps_outputs = steps_outputs;
    outcome.duration = started.elapsed();

    observer.on_workflow_done(wf, opts.vu, &outcome).await;
    outcome
}

/// Resolves the pause to apply after `step`, lazily seeding the jitter RNG
/// from the run seed and VU id so runs are reproducible.
fn think_after(
    step: &StepPlan,
    policy: ThinkPolicy,
    rng: &mut Option<rand::rngs::StdRng>,
    opts: &RunOpts,
) -> Option<Duration> {
    let ThinkPolicy::Honour { default } = policy else {
        return None;
    };
    let spec = step.think.or(default)?;
    if spec.max <= spec.min {
        return (spec.min > Duration::ZERO).then_some(spec.min);
    }
    let rng = rng.get_or_insert_with(|| {
        rand::rngs::StdRng::seed_from_u64(opts.seed ^ opts.vu.0.rotate_left(17))
    });
    let span = spec.max - spec.min;
    let frac: f64 = rng.r#gen();
    Some(spec.min + span.mul_f64(frac))
}

/// Result of one executed step.
struct StepResult {
    sample: StepSample,
    /// Captured outputs; empty unless the step passed.
    outputs: InputMap,
}

impl StepResult {
    fn failed(sample: StepSample) -> Self {
        Self {
            sample,
            outputs: InputMap::new(),
        }
    }
}

/// Runs one Arazzo 1.1 AsyncAPI step: `send` publishes the evaluated
/// payload, `receive` awaits a matching message. The outcome is projected
/// onto the response shape the criteria evaluator already understands, so
/// `$statusCode`, `$message.payload`, and step outputs behave the same for
/// HTTP and message steps.
async fn run_message_step(
    cx: &StepCx<'_>,
    step: &StepPlan,
    message_step: &crate::plan::MessageStep,
    opts: &RunOpts,
    env: Env<'_>,
    inputs: &InputMap,
    steps_outputs: &InputMap,
) -> StepResult {
    let observer = env.observer;
    let state_ctx = || {
        RexCtx::default()
            .inputs(inputs)
            .steps_outputs(steps_outputs)
            .workflows_outputs(&opts.workflows_outputs)
    };
    let started = Instant::now();
    let Some(transport) = env.messages else {
        observer
            .on_check_failed(
                cx,
                "transport",
                "a message transport",
                "the run configured no message broker",
            )
            .await;
        return StepResult::failed(StepSample::unsent(ErrorClass::Other, started.elapsed(), 0));
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

    let timeout = Duration::from_millis(step.timeout_ms.unwrap_or(5000));
    let mut bytes_out = 0usize;
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
            bytes_out = serde_json::to_vec(&payload).map_or(0, |bytes| bytes.len());
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
            observer
                .on_check_failed(
                    cx,
                    "transport",
                    &format!("a message on `{}`", message_step.channel),
                    &e.to_string(),
                )
                .await;
            let class = TransportError(e.to_string()).class();
            return StepResult::failed(StepSample::unsent(class, started.elapsed(), bytes_out));
        }
    };
    let latency = started.elapsed();

    // A received payload must satisfy the AsyncAPI message's declared
    // schema, exactly like a response body against its contract. A send
    // step publishes; its payload is the producer's business.
    let mut pre_failures = 0usize;
    if let (crate::plan::MessageDirection::Receive, Some(schema)) =
        (message_step.direction, &message_step.payload_schema)
    {
        let violations = message_payload_violations(schema, &message.payload);
        if !violations.is_empty() {
            observer
                .on_check_failed(
                    cx,
                    "payload",
                    "a payload matching the declared message schema",
                    &violations.join("; "),
                )
                .await;
            pre_failures += 1;
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
    let sample = StepSample {
        status: Some(response.status),
        latency,
        ttfb: None,
        bytes_out,
        bytes_in: response.body.len(),
        verdict: StepVerdict::Passed,
    };
    if pre_failures > 0 {
        // A non-conforming payload fails the step before criteria run.
        return StepResult::failed(StepSample {
            verdict: StepVerdict::CriteriaFailed(pre_failures),
            ..sample
        });
    }
    evaluate_response(
        cx,
        step,
        &response,
        "",
        opts,
        env,
        inputs,
        steps_outputs,
        sample,
        0,
    )
    .await
}

/// Validation messages for a received message payload against its declared
/// AsyncAPI schema; empty when it conforms (or cannot be compiled).
fn message_payload_violations(
    schema: &serde_json::Value,
    payload: &serde_json::Value,
) -> Vec<String> {
    let wrapper = format!(
        "{{\"schema\": {}, \"instance\": {}}}",
        serde_json::to_string(schema).unwrap_or_else(|_| "null".to_owned()),
        serde_json::to_string(payload).unwrap_or_else(|_| "null".to_owned())
    );
    let mut violations = Vec::new();
    if let Ok(uri) = suspect_source::Uri::parse("mem://message-check.json") {
        let doc =
            suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(wrapper.into_bytes()));
        if doc.syntax_errors().is_empty()
            && let (Some(schema_node), Some(instance_node)) =
                (doc.root().get("schema"), doc.root().get("instance"))
            && let Ok(compiled) = suspect_schema::Compiler::new(suspect_schema::Config::default())
                .compile(schema_node)
        {
            violations = compiled
                .validate(instance_node)
                .iter()
                .map(|e| e.message.clone())
                .collect();
        }
    }
    violations
}

/// Shared post-response path: success criteria, then output capture. HTTP
/// steps and Arazzo 1.1 message steps both end here, so `$message.payload`
/// and `$response.body#...` resolve through one evaluator.
/// `pre_failures` counts checks that already failed (response-schema
/// conformance); the step fails if any did, but criteria still report.
#[allow(clippy::too_many_arguments)]
async fn evaluate_response(
    cx: &StepCx<'_>,
    step: &StepPlan,
    response: &HttpResponse,
    method: &str,
    opts: &RunOpts,
    env: Env<'_>,
    inputs: &InputMap,
    steps_outputs: &InputMap,
    mut sample: StepSample,
    pre_failures: usize,
) -> StepResult {
    let observer = env.observer;
    let body_text = String::from_utf8_lossy(&response.body).into_owned();
    let body_json: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();

    let mut failures = pre_failures;
    for crit in &step.success {
        // Runtime-expectation criteria resolve their right-hand side
        // against the run state first (`$steps.<id>.outputs.<name>`,
        // `$inputs.<name>`), then evaluate as ordinary equality.
        let resolved = resolve_criterion(&crit.kind, inputs, steps_outputs);
        match eval_criterion(&resolved, response.status, body_json.as_ref(), &body_text) {
            Ok(()) => observer.on_criterion(cx, crit, None).await,
            Err((expected, actual)) => {
                failures += 1;
                observer
                    .on_criterion(cx, crit, Some((&expected, &actual)))
                    .await;
            }
        }
    }
    if failures > 0 {
        sample.verdict = StepVerdict::CriteriaFailed(failures);
        return StepResult::failed(sample);
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
        .steps_outputs(steps_outputs)
        .workflows_outputs(&opts.workflows_outputs);
    let mut captured = InputMap::new();
    for (name, rex) in &step.outputs {
        if let Some(value) = eval_rex(rex, &capture_ctx) {
            observer.on_output(cx, name, &value).await;
            captured.insert(name.clone(), value);
        }
    }
    StepResult {
        sample,
        outputs: captured,
    }
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
                && let Ok(rex) = suspect_arazzo::rex::parse_rex(text)
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
                                let evaluated = suspect_arazzo::rex::parse_rex(&text)
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

/// Builds and executes one step, then validates and evaluates the response.
async fn run_step(
    cx: &StepCx<'_>,
    step: &StepPlan,
    opts: &RunOpts,
    env: Env<'_>,
    inputs: &InputMap,
    steps_outputs: &InputMap,
) -> StepResult {
    // Arazzo 1.1 AsyncAPI step: publish or await a message instead of
    // issuing an HTTP exchange. The result is shaped like a response so
    // criteria, outputs and `$message.payload` all evaluate uniformly.
    if let Some(message_step) = &step.message {
        return run_message_step(cx, step, message_step, opts, env, inputs, steps_outputs).await;
    }
    let observer = env.observer;
    let http = env.http;

    // State-only context: parameters may reference workflow inputs and
    // earlier step outputs but not the exchange currently being built.
    let state_ctx = || {
        RexCtx::default()
            .inputs(inputs)
            .steps_outputs(steps_outputs)
            .workflows_outputs(&opts.workflows_outputs)
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

    let resolve_templates = |text: &str| -> String { render_embedded(text, &state_ctx()) };
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
            // Pre-serialized JSON (from object bodies) or plain text; runtime
            // expressions inside it are resolved per request.
            Some(serde_json::Value::String(s)) => render_body(&s, &state_ctx()).into_bytes(),
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
        request_headers.push((
            "content-type".to_owned(),
            step.request_content_type
                .clone()
                .unwrap_or_else(|| "application/json".to_owned()),
        ));
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
            if let Some(credential) = env.auth_config.schemes.get(scheme) {
                let acquiring = Instant::now();
                match env.auth_state.resolve(http, scheme, credential).await {
                    Ok(Some(placement)) => injected.push(placement),
                    Ok(None) => {}
                    Err(message) => {
                        observer
                            .on_check_failed(
                                cx,
                                &format!("auth:{scheme}"),
                                "credential acquisition",
                                &message,
                            )
                            .await;
                        return StepResult::failed(StepSample::unsent(
                            ErrorClass::Other,
                            acquiring.elapsed(),
                            0,
                        ));
                    }
                }
            }
        }
    }

    let url = join_url(&opts.base_url, &path, &query);
    let mut request = HttpRequest {
        method: method.to_owned(),
        url,
        headers: request_headers,
        body: Bytes::from(body.unwrap_or_default()),
    };
    crate::auth::inject(&mut request, &injected);
    let bytes_out = request.body.len();

    observer.on_request(cx, &request).await;

    let started = Instant::now();
    // Arazzo 1.1 step `timeout`: exceeding it fails the step as a
    // transport error.
    let dispatched = if let Some(timeout_ms) = step.timeout_ms {
        tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            http.execute_timed(request),
        )
        .await
        .unwrap_or_else(|_| {
            Err(TransportError(format!(
                "step exceeded its {timeout_ms}ms timeout"
            )))
        })
    } else {
        http.execute_timed(request).await
    };
    let (response, ttfb) = match dispatched {
        Ok(pair) => pair,
        Err(e) => {
            let latency = started.elapsed();
            observer.on_transport_error(cx, &e, latency).await;
            return StepResult::failed(StepSample::unsent(e.class(), latency, bytes_out));
        }
    };
    let latency = started.elapsed();
    observer.on_response(cx, response.status, latency).await;

    // Contract validation: the shared contract runtime owns the decision,
    // so the runner and the gateway cannot disagree about whether a
    // response conforms. It runs whether or not the step wrote criteria.
    let mut pre_failures = 0usize;
    if opts.validate_responses && declares_response(step, response.status) {
        let schemas = suspect_runtime::Schemas::from_any_map(env.components.iter());
        let violations = suspect_runtime::validate_response(
            &step.declared_responses(),
            &schemas,
            response.status,
            &response.body,
        );
        if violations.is_empty() {
            observer.on_response_validated(cx, response.status).await;
        } else {
            let actual: Vec<String> = violations
                .iter()
                .map(|violation| violation.message.clone())
                .collect();
            observer
                .on_check_failed(
                    cx,
                    "response-schema",
                    "response matching the declared schema",
                    &actual.join("; "),
                )
                .await;
            pre_failures += 1;
        }
    }

    let sample = StepSample {
        status: Some(response.status),
        latency,
        ttfb,
        bytes_out,
        bytes_in: response.body.len(),
        verdict: StepVerdict::Passed,
    };
    evaluate_response(
        cx,
        step,
        &response,
        method,
        opts,
        env,
        inputs,
        steps_outputs,
        sample,
        pre_failures,
    )
    .await
}

/// Whether the step declares any response for this status (exact or
/// `default`), which is the condition for a conformance check at all.
fn declares_response(step: &StepPlan, status: u16) -> bool {
    step.response_schemas
        .iter()
        .any(|(declared, _)| *declared == Some(status) || declared.is_none())
}

/// Renders embedded runtime expressions inside text. Both the Arazzo
/// embedding form `{$inputs.x}` and the `${$inputs.x}` spelling are
/// recognized; unresolved expressions render empty, malformed ones are kept
/// literally. Text without either marker is returned unchanged.
#[must_use]
pub fn render_embedded(text: &str, ctx: &RexCtx<'_>) -> String {
    if !text.contains("{$") && !text.contains("${") {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let dollar_brace = rest.find("${");
        let brace_dollar = rest.find("{$");
        let (start, open_len) = match (dollar_brace, brace_dollar) {
            (Some(a), Some(b)) if a <= b => (a, 2),
            (Some(a), None) => (a, 2),
            (_, Some(b)) => (b, 1),
            (None, None) => break,
        };
        result.push_str(&rest[..start]);
        let after = &rest[start + open_len..];
        let Some(end) = after.find('}') else {
            result.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let expr_text = &after[..end];
        match suspect_arazzo::rex::parse_rex(expr_text) {
            Ok(rex) if !matches!(rex, suspect_arazzo::rex::Rex::Text(_)) => {
                match eval_rex(&rex, ctx) {
                    Some(serde_json::Value::String(v)) => result.push_str(&v),
                    Some(other) => result.push_str(&other.to_string()),
                    None => {}
                }
            }
            _ => result.push_str(&rest[start..start + open_len + end + 1]),
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    result
}

/// Resolves runtime expressions inside a request body. JSON bodies are
/// walked: a string leaf that is exactly one expression (`$inputs.qty`) is
/// replaced by the evaluated value with its JSON type preserved; leaves with
/// embedded expressions are rendered as text. Non-JSON bodies get embedded
/// rendering only. Bodies without a `$` are returned unchanged.
#[must_use]
pub fn render_body(text: &str, ctx: &RexCtx<'_>) -> String {
    if !text.contains('$') {
        return text.to_owned();
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut value) => {
            resolve_body_value(&mut value, ctx);
            value.to_string()
        }
        Err(_) => render_embedded(text, ctx),
    }
}

fn resolve_body_value(value: &mut serde_json::Value, ctx: &RexCtx<'_>) {
    match value {
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                resolve_body_value(v, ctx);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                resolve_body_value(v, ctx);
            }
        }
        serde_json::Value::String(s) => {
            // Whole-leaf expression keeps the evaluated JSON type.
            if s.starts_with('$')
                && let Ok(rex) = suspect_arazzo::rex::parse_rex(s)
                && !matches!(rex, suspect_arazzo::rex::Rex::Text(_))
            {
                *value = eval_rex(&rex, ctx).unwrap_or(serde_json::Value::Null);
            } else if s.contains("{$") || s.contains("${") {
                *value = serde_json::Value::String(render_embedded(s, ctx));
            }
        }
        _ => {}
    }
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
