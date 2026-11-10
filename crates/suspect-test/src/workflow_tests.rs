//! Arazzo workflow semantics through the runner: the criterion language
//! (`simple` conditions, `regex` and `jsonpath` with a context) and the
//! `onSuccess`/`onFailure` actions (`end`, `goto`, `retry`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Source;

use crate::exec::{
    HttpClient, HttpRequest, HttpResponse, RunOpts, RunSummary, TestEvent, TransportError,
    run_plan_with,
};
use crate::plan::{CompileError, Plan, compile_plan};
use crate::reporters;

/// Every operation declares `default`, so any status is a declared one and
/// these tests are about the workflow, not the contract.
const API: &str = r##"
openapi: 3.1.0
info: { title: Jobs, version: "1" }
paths:
  /jobs:
    get:
      operationId: listJobs
      responses:
        default: { description: any }
    post:
      operationId: startJob
      responses:
        default: { description: any }
  /jobs/{jobId}:
    get:
      operationId: showJob
      parameters:
        - { name: jobId, in: path, required: true, schema: { type: string } }
      responses:
        default: { description: any }
  /cleanup:
    post:
      operationId: cleanup
      responses:
        default: { description: any }
"##;

fn compile(workflows: &str) -> Result<Plan, CompileError> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("api.yaml"), API).expect("api");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("api.yaml").expect("load");
    std::mem::forget(dir);
    let text = format!(
        "arazzo: 1.0.0\ninfo: {{ title: t, version: '1' }}\nsourceDescriptions:\n  - {{ name: api, url: api.yaml, type: openapi }}\n{workflows}"
    );
    let doc = LowDoc::parse(
        "mem://flows.arazzo.yaml".into(),
        Source::from_vec(text.into_bytes()),
    );
    compile_plan(&doc, &Arc::new(ws))
}

/// One step calling `operation`, with `extra` YAML appended to the step.
fn one_step(operation: &str, extra: &str) -> String {
    format!(
        "workflows:\n  - workflowId: w\n    steps:\n      - stepId: s\n        operationId: {operation}\n{extra}"
    )
}

/// A reply: status, headers and body.
#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
}

fn reply(status: u16, body: &'static str) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body,
    }
}

/// Answers each `METHOD path` with its scripted replies in order; the last
/// one repeats. Counts the requests it saw.
#[derive(Default)]
struct Script {
    replies: HashMap<String, Vec<Reply>>,
    seen: Mutex<Vec<String>>,
    bodies: Mutex<Vec<String>>,
}

impl Script {
    fn on(mut self, route: &str, replies: Vec<Reply>) -> Self {
        self.replies.insert(route.to_owned(), replies);
        self
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().expect("lock").clone()
    }
}

#[async_trait]
impl HttpClient for Script {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let path = req
            .url
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|slash| &rest[slash..]))
            .unwrap_or("/");
        let route = format!("{} {}", req.method, path.split('?').next().unwrap_or(path));
        self.bodies
            .lock()
            .expect("lock")
            .push(String::from_utf8_lossy(&req.body).into_owned());
        let nth = {
            let mut seen = self.seen.lock().expect("lock");
            let nth = seen.iter().filter(|r| **r == route).count();
            seen.push(route.clone());
            nth
        };
        let Some(replies) = self.replies.get(&route) else {
            return Err(TransportError(format!(
                "connect: nothing listens for {route}"
            )));
        };
        let reply = replies.get(nth).or(replies.last()).expect("a reply");
        Ok(HttpResponse {
            status: reply.status,
            headers: reply
                .headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: Bytes::from(reply.body),
        })
    }
}

fn run_with(
    plan: &Plan,
    server: &Script,
    inputs: serde_json::Value,
) -> (RunSummary, Vec<TestEvent>) {
    let opts = RunOpts {
        inputs: inputs.as_object().cloned().unwrap_or_default(),
        ..RunOpts::new("http://api.test")
    };
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
            let summary = run_plan_with(plan, &opts, server, tx).await;
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            (summary, events)
        })
}

fn run(workflows: &str, server: &Script) -> (RunSummary, Vec<TestEvent>) {
    let plan = compile(workflows).expect("compiles");
    run_with(&plan, server, serde_json::json!({}))
}

/// `(passed, failed, skipped)`.
fn counts(summary: &RunSummary) -> (usize, usize, usize) {
    (summary.passed, summary.failed, summary.skipped)
}

fn failures(events: &[TestEvent]) -> Vec<(String, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            TestEvent::CriterionFail {
                crit,
                expected,
                actual,
                ..
            } => Some((crit.clone(), expected.clone(), actual.clone())),
            _ => None,
        })
        .collect()
}

fn criteria(conditions: &[&str]) -> String {
    let mut out = String::from("        successCriteria:\n");
    for condition in conditions {
        out.push_str(&format!("          - condition: \"{condition}\"\n"));
    }
    out
}

// ------------------------------------------------------------- conditions

#[test]
fn a_logical_condition_decides_the_step() {
    let flow = one_step(
        "listJobs",
        &criteria(&["$statusCode == 200 || $statusCode == 201"]),
    );
    let created = Script::default().on("GET /jobs", vec![reply(201, "{}")]);
    assert_eq!(counts(&run(&flow, &created).0), (1, 0, 0));

    let no_content = Script::default().on("GET /jobs", vec![reply(204, "")]);
    let (summary, events) = run(&flow, &no_content);
    assert_eq!(counts(&summary), (0, 1, 0), "{events:#?}");
    assert_eq!(
        failures(&events),
        [(
            "$statusCode == 200 || $statusCode == 201".to_owned(),
            "$statusCode == 200 || $statusCode == 201".to_owned(),
            "$statusCode = 204".to_owned()
        )]
    );
}

#[test]
fn status_inequality_and_ranges_are_conditions() {
    let flow = one_step(
        "listJobs",
        &criteria(&[
            "$statusCode != 500",
            "$statusCode >= 200 && $statusCode < 300",
        ]),
    );
    let ok = Script::default().on("GET /jobs", vec![reply(200, "{}")]);
    assert_eq!(counts(&run(&flow, &ok).0), (1, 0, 0));
    let redirect = Script::default().on("GET /jobs", vec![reply(302, "")]);
    let (summary, events) = run(&flow, &redirect);
    assert_eq!(counts(&summary), (0, 1, 0));
    assert_eq!(
        failures(&events).len(),
        1,
        "only the range fails: {events:#?}"
    );
}

#[test]
fn a_header_condition_reads_the_response_header() {
    let flow = one_step(
        "listJobs",
        &criteria(&[
            "$response.header.Content-Type == 'application/json'",
            "$response.header.X-Total > 5",
        ]),
    );
    let with_headers = |total: &'static str| {
        Script::default().on(
            "GET /jobs",
            vec![Reply {
                status: 200,
                headers: vec![("content-type", "application/json"), ("x-total", total)],
                body: "{}",
            }],
        )
    };
    assert_eq!(counts(&run(&flow, &with_headers("12")).0), (1, 0, 0));
    let (summary, events) = run(&flow, &with_headers("3"));
    assert_eq!(counts(&summary), (0, 1, 0));
    assert_eq!(failures(&events)[0].2, "$response.header.X-Total = \"3\"");
}

#[test]
fn a_body_inequality_is_a_condition() {
    let flow = one_step(
        "listJobs",
        &criteria(&["$response.body#/state != 'failed'"]),
    );
    let running = Script::default().on("GET /jobs", vec![reply(200, r#"{"state":"running"}"#)]);
    assert_eq!(counts(&run(&flow, &running).0), (1, 0, 0));
    let failed = Script::default().on("GET /jobs", vec![reply(200, r#"{"state":"failed"}"#)]);
    assert_eq!(counts(&run(&flow, &failed).0), (0, 1, 0));
}

#[test]
fn an_input_condition_is_evaluated_not_assumed() {
    let flow = one_step("listJobs", &criteria(&["$inputs.expected == 3"]));
    let plan = compile(&flow).expect("compiles");
    let server = Script::default().on("GET /jobs", vec![reply(200, "{}")]);
    let (summary, _) = run_with(&plan, &server, serde_json::json!({"expected": 3}));
    assert_eq!(counts(&summary), (1, 0, 0));
    let (summary, events) = run_with(&plan, &server, serde_json::json!({"expected": 4}));
    assert_eq!(counts(&summary), (0, 1, 0), "{events:#?}");
}

#[test]
fn string_equality_is_case_insensitive() {
    let flow = one_step("listJobs", &criteria(&["$response.body#/name == 'rex'"]));
    let server = Script::default().on("GET /jobs", vec![reply(200, r#"{"name":"Rex"}"#)]);
    assert_eq!(counts(&run(&flow, &server).0), (1, 0, 0));
}

#[test]
fn a_regex_criterion_matches_its_context() {
    let flow = one_step(
        "listJobs",
        "        successCriteria:\n          - context: $response.body#/id\n            condition: '^job_[0-9]+$'\n            type: regex\n          - context: $statusCode\n            condition: '^2..$'\n            type: regex\n",
    );
    let good = Script::default().on("GET /jobs", vec![reply(200, r#"{"id":"job_42"}"#)]);
    assert_eq!(counts(&run(&flow, &good).0), (1, 0, 0));
    let bad = Script::default().on("GET /jobs", vec![reply(200, r#"{"id":"task_42"}"#)]);
    let (summary, events) = run(&flow, &bad);
    assert_eq!(counts(&summary), (0, 1, 0));
    let failures = failures(&events);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].2.contains("task_42"), "{failures:?}");
}

#[test]
fn a_jsonpath_criterion_holds_when_it_selects_a_node() {
    let flow = one_step(
        "listJobs",
        "        successCriteria:\n          - context: $response.body\n            condition: $.items[?@.state == 'done']\n            type: jsonpath\n          - context: $response.body\n            condition: $[?count(@.items) > 0]\n            type: jsonpath\n",
    );
    let done = Script::default().on(
        "GET /jobs",
        vec![reply(
            200,
            r#"{"items":[{"state":"queued"},{"state":"done"}]}"#,
        )],
    );
    let (summary, events) = run(&flow, &done);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");

    let none_done = Script::default().on(
        "GET /jobs",
        vec![reply(200, r#"{"items":[{"state":"queued"}]}"#)],
    );
    let (summary, events) = run(&flow, &none_done);
    assert_eq!(counts(&summary), (0, 1, 0));
    let failures = failures(&events);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].0.contains("$.items[?@.state == 'done']"),
        "{failures:?}"
    );
}

#[test]
fn criteria_that_cannot_be_evaluated_do_not_compile() {
    for (extra, needle) in [
        (
            "        successCriteria:\n          - context: $response.body\n            condition: //job\n            type: xpath\n",
            "xpath",
        ),
        (
            "        successCriteria:\n          - context: $response.body\n            condition: '$.items['\n            type: jsonpath\n",
            "JSONPath",
        ),
        (
            "        successCriteria:\n          - context: $response.body#/id\n            condition: '(['\n            type: regex\n",
            "regular expression",
        ),
        (
            "        successCriteria:\n          - condition: '$statusCode == 200 &&'\n",
            "unsupported success criterion",
        ),
    ] {
        let error = compile(&one_step("listJobs", extra)).expect_err("must not compile");
        assert!(error.0.contains(needle), "`{needle}` in: {error}");
        assert!(error.0.contains("step 's'"), "the step is named: {error}");
    }
}

// ---------------------------------------------------------------- actions

const POLL: &str = "workflows:
  - workflowId: w
    steps:
      - stepId: poll
        operationId: showJob
        parameters:
          - { name: jobId, in: path, value: j1 }
        successCriteria:
          - condition: $response.body#/state == 'done'
        onFailure:
          - { name: wait, type: retry, retryAfter: 0, retryLimit: 3 }
";

fn retried(events: &[TestEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, TestEvent::StepRetried { .. }))
        .count()
}

#[test]
fn retry_reruns_the_step_until_it_passes() {
    let server = Script::default().on(
        "GET /jobs/j1",
        vec![
            reply(200, r#"{"state":"queued"}"#),
            reply(200, r#"{"state":"running"}"#),
            reply(200, r#"{"state":"done"}"#),
        ],
    );
    let plan = compile(POLL).expect("compiles");
    let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
    assert_eq!(
        counts(&summary),
        (1, 0, 0),
        "a step that passes on a retry passed: {events:#?}"
    );
    assert_eq!(server.requests().len(), 3);
    assert_eq!(retried(&events), 2);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TestEvent::WfDone { passed: true, .. }))
    );

    // Reports judge the step by its last attempt.
    let cases = reporters::workflow_cases(&plan, &events);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].outcome, reporters::Outcome::Passed, "{cases:#?}");
    assert_eq!(cases[0].requests.len(), 3, "every attempt is on the record");
    let console = reporters::console(&summary, &events);
    assert!(
        console.contains("PASS") && !console.contains("FAIL"),
        "{console}"
    );
}

#[test]
fn retry_gives_up_after_its_limit() {
    let server = Script::default().on("GET /jobs/j1", vec![reply(200, r#"{"state":"queued"}"#)]);
    let plan = compile(POLL).expect("compiles");
    let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
    assert_eq!(counts(&summary), (0, 1, 0), "{events:#?}");
    assert_eq!(
        server.requests().len(),
        4,
        "the first attempt plus three retries"
    );
    assert_eq!(retried(&events), 3);
    let cases = reporters::workflow_cases(&plan, &events);
    assert!(cases[0].is_failed());
}

#[test]
fn a_retry_without_a_limit_retries_once() {
    let flow = POLL.replace(", retryLimit: 3", "");
    let server = Script::default().on("GET /jobs/j1", vec![reply(200, r#"{"state":"queued"}"#)]);
    let (summary, _) = run(&flow, &server);
    assert_eq!(counts(&summary), (0, 1, 0));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn a_retry_also_covers_a_request_that_got_no_response() {
    // Nothing listens: every attempt is a transport failure.
    let server = Script::default();
    let (summary, events) = run(POLL, &server);
    assert_eq!(counts(&summary), (0, 1, 0), "{events:#?}");
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn action_criteria_select_the_action() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: start
        operationId: startJob
        successCriteria:
          - condition: $statusCode == 202
        onFailure:
          - name: busy
            type: retry
            retryAfter: 0
            retryLimit: 5
            criteria:
              - condition: $statusCode == 503
          - { name: stop, type: end }
";
    // 503 twice, then accepted: the retry applies.
    let busy = Script::default().on(
        "POST /jobs",
        vec![reply(503, ""), reply(503, ""), reply(202, "{}")],
    );
    let (summary, events) = run(flow, &busy);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");
    assert_eq!(busy.requests().len(), 3);

    // A 400 is not what the retry is for: the workflow ends at once.
    let rejected = Script::default().on("POST /jobs", vec![reply(400, "")]);
    let (summary, _) = run(flow, &rejected);
    assert_eq!(counts(&summary), (0, 1, 0));
    assert_eq!(rejected.requests().len(), 1);
}

#[test]
fn an_exhausted_retry_falls_through_to_the_next_action() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: start
        operationId: startJob
        successCriteria:
          - condition: $statusCode == 202
        onFailure:
          - { name: again, type: retry, retryAfter: 0, retryLimit: 1 }
          - { name: tidy, type: goto, stepId: tidy }
      - stepId: never
        operationId: listJobs
      - stepId: tidy
        operationId: cleanup
";
    let server = Script::default()
        .on("POST /jobs", vec![reply(503, "")])
        .on("POST /cleanup", vec![reply(200, "")]);
    let (summary, events) = run(flow, &server);
    assert_eq!(
        server.requests(),
        ["POST /jobs", "POST /jobs", "POST /cleanup"],
        "{events:#?}"
    );
    assert_eq!(
        summary.failed, 1,
        "the failed step stays failed: {events:#?}"
    );
    assert_eq!(summary.passed, 1, "the goto target ran");
}

#[test]
fn on_success_end_stops_the_workflow_as_passed() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: probe
        operationId: listJobs
        onSuccess:
          - name: nothing-to-do
            type: end
            criteria:
              - condition: $response.body#/total == 0
      - stepId: start
        operationId: startJob
";
    let empty = Script::default()
        .on("GET /jobs", vec![reply(200, r#"{"total":0}"#)])
        .on("POST /jobs", vec![reply(200, "{}")]);
    let (summary, events) = run(flow, &empty);
    assert_eq!(counts(&summary), (1, 0, 1), "{events:#?}");
    assert_eq!(empty.requests(), ["GET /jobs"]);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TestEvent::WfDone { passed: true, .. }))
    );

    // The criteria do not hold: the workflow carries on.
    let busy = Script::default()
        .on("GET /jobs", vec![reply(200, r#"{"total":2}"#)])
        .on("POST /jobs", vec![reply(200, "{}")]);
    let (summary, _) = run(flow, &busy);
    assert_eq!(counts(&summary), (2, 0, 0));
    assert_eq!(busy.requests(), ["GET /jobs", "POST /jobs"]);
}

#[test]
fn on_success_goto_continues_at_the_named_step() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: probe
        operationId: listJobs
        onSuccess:
          - { name: skip-start, type: goto, stepId: tidy }
      - stepId: start
        operationId: startJob
      - stepId: tidy
        operationId: cleanup
";
    let server = Script::default()
        .on("GET /jobs", vec![reply(200, "{}")])
        .on("POST /jobs", vec![reply(200, "{}")])
        .on("POST /cleanup", vec![reply(200, "")]);
    let (summary, events) = run(flow, &server);
    assert_eq!(
        server.requests(),
        ["GET /jobs", "POST /cleanup"],
        "{events:#?}"
    );
    assert_eq!(counts(&summary), (2, 0, 1));
}

#[test]
fn a_reusable_action_resolves_through_components() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: poll
        operationId: showJob
        parameters:
          - { name: jobId, in: path, value: j1 }
        successCriteria:
          - condition: $response.body#/state == 'done'
        onFailure:
          - reference: $components.failureActions.keepTrying
components:
  failureActions:
    keepTrying: { name: keepTrying, type: retry, retryAfter: 0, retryLimit: 2 }
";
    let server = Script::default().on(
        "GET /jobs/j1",
        vec![
            reply(200, r#"{"state":"queued"}"#),
            reply(200, r#"{"state":"done"}"#),
        ],
    );
    let (summary, events) = run(flow, &server);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn actions_that_cannot_run_do_not_compile() {
    let step = |actions: &str| {
        format!(
            "workflows:\n  - workflowId: w\n    steps:\n      - stepId: s\n        operationId: listJobs\n{actions}"
        )
    };
    for (actions, needle) in [
        (
            "        onFailure:\n          - { name: a, type: goto, stepId: nowhere }\n",
            "matches no step",
        ),
        (
            "        onFailure:\n          - { name: a, type: goto, workflowId: other }\n",
            "workflowId",
        ),
        (
            "        onSuccess:\n          - { name: a, type: retry }\n",
            "retry",
        ),
        (
            "        onFailure:\n          - { name: a, type: explode }\n",
            "explode",
        ),
        (
            "        onFailure:\n          - reference: $components.failureActions.missing\n",
            "missing",
        ),
    ] {
        let error = compile(&step(actions)).expect_err("must not compile");
        assert!(error.0.contains(needle), "`{needle}` in: {error}");
    }
}

#[test]
fn the_retry_delay_prefers_the_servers_retry_after() {
    use std::time::Duration;
    let declared = Duration::from_millis(250);
    assert_eq!(crate::exec::retry_delay(declared, &[]), declared);
    let header = [("Retry-After".to_owned(), "2".to_owned())];
    assert_eq!(
        crate::exec::retry_delay(declared, &header),
        Duration::from_secs(2)
    );
    // A date or garbage is not a number of seconds: the declared delay stands.
    let date = [(
        "retry-after".to_owned(),
        "Wed, 21 Oct 2026 07:28:00 GMT".to_owned(),
    )];
    assert_eq!(crate::exec::retry_delay(declared, &date), declared);
}

#[test]
fn workflow_level_failure_actions_apply_to_every_step() {
    let flow = "workflows:
  - workflowId: w
    failureActions:
      - { name: again, type: retry, retryAfter: 0, retryLimit: 2 }
    steps:
      - stepId: list
        operationId: listJobs
        successCriteria:
          - condition: $statusCode == 200
      - stepId: start
        operationId: startJob
        successCriteria:
          - condition: $statusCode == 202
        # A step's action of the same name replaces the workflow's.
        onFailure:
          - { name: again, type: end }
";
    let server = Script::default()
        .on(
            "GET /jobs",
            vec![reply(503, ""), reply(503, ""), reply(200, "{}")],
        )
        .on("POST /jobs", vec![reply(503, "")]);
    let (summary, events) = run(flow, &server);
    assert_eq!(
        server.requests(),
        ["GET /jobs", "GET /jobs", "GET /jobs", "POST /jobs"],
        "{events:#?}"
    );
    assert_eq!(counts(&summary), (1, 1, 0));
}

#[test]
fn workflow_level_success_actions_apply_to_every_step() {
    let flow = "workflows:
  - workflowId: w
    successActions:
      - name: done-early
        type: end
        criteria:
          - condition: $response.body#/done == true
    steps:
      - stepId: list
        operationId: listJobs
      - stepId: start
        operationId: startJob
";
    let server = Script::default()
        .on("GET /jobs", vec![reply(200, r#"{"done": true}"#)])
        .on("POST /jobs", vec![reply(200, "{}")]);
    let (summary, _) = run(flow, &server);
    assert_eq!(server.requests(), ["GET /jobs"]);
    assert_eq!(counts(&summary), (1, 0, 1));
}

#[test]
fn request_body_replacements_are_applied() {
    let flow = "workflows:
  - workflowId: w
    inputs:
      type: object
      properties:
        jobName: { type: string, default: nightly }
    steps:
      - stepId: start
        operationId: startJob
        requestBody:
          contentType: application/json
          payload:
            name: placeholder
            options: { retries: 0, tags: [a, b] }
          replacements:
            - { target: /name, value: $inputs.jobName }
            - { target: /options/retries, value: 3 }
            - { target: /options/tags/1, value: z }
            - { target: /options/added, value: true }
";
    let server = Script::default().on("POST /jobs", vec![reply(200, "{}")]);
    let (summary, events) = run(flow, &server);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");
    let sent: serde_json::Value =
        serde_json::from_str(&server.bodies.lock().expect("lock")[0]).expect("a JSON body");
    assert_eq!(
        sent,
        serde_json::json!({
            "name": "nightly",
            "options": {"retries": 3, "tags": ["a", "z"], "added": true}
        })
    );
}

#[test]
fn steps_and_replacements_that_cannot_run_do_not_compile() {
    for (step, needle) in [
        (
            "      - stepId: s\n        workflowId: other\n",
            "runs workflow 'other'",
        ),
        (
            "      - stepId: s\n        operationId: startJob\n        requestBody:\n          payload: { a: 1 }\n          replacements:\n            - { target: //a, value: 2 }\n",
            "replacement",
        ),
        (
            "      - stepId: s\n        operationId: startJob\n        requestBody:\n          payload: { a: 1 }\n          replacements:\n            - { target: /missing/deep, value: 2 }\n",
            "/missing/deep",
        ),
    ] {
        let flow = format!("workflows:\n  - workflowId: w\n    steps:\n{step}");
        let error = compile(&flow).expect_err("must not compile");
        assert!(error.0.contains(needle), "`{needle}` in: {error}");
    }
}
