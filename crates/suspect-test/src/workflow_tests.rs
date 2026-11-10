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
            "        onFailure:\n          - { name: a, type: retry, stepId: s }\n",
            "stepId 's'",
        ),
        (
            "        onFailure:\n          - { name: a, type: retry, workflowId: other }\n",
            "workflowId 'other'",
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

// ------------------------------------------------- review findings, pinned

/// `(passed, failed, skipped)` of the reported cases.
fn case_counts(cases: &[reporters::Case]) -> (usize, usize, usize) {
    let failed = cases.iter().filter(|case| case.is_failed()).count();
    let skipped = cases.iter().filter(|case| case.is_skipped()).count();
    (cases.len() - failed - skipped, failed, skipped)
}

#[test]
fn an_operation_path_in_the_specifications_own_form_keeps_its_contract() {
    for path in [
        "'{$sourceDescriptions.api.url}#/paths/~1jobs/get'",
        "$sourceDescriptions.api.url#/paths/~1jobs/get",
        "$sourceDescriptions.api#/paths/~1jobs/get",
    ] {
        let flow = format!(
            "workflows:\n  - workflowId: w\n    steps:\n      - stepId: s\n        operationPath: {path}\n"
        );
        let plan = compile(&flow).expect("compiles");
        assert!(
            plan.workflows[0].steps[0].contract.is_some(),
            "{path} names a declared operation"
        );
        let server = Script::default().on("GET /jobs", vec![reply(200, "{}")]);
        let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
        assert_eq!(counts(&summary), (1, 0, 0), "{path}: {events:#?}");
    }
}

#[test]
fn an_operand_that_decides_nothing_does_not_compile_wherever_it_stands() {
    for condition in [
        "$statusCode == 200 || 201",
        "$statusCode == 200 && 'ok'",
        "!(404)",
        "$statusCode == 200 || $statusCode",
        "($method) && $statusCode == 200",
    ] {
        let error =
            compile(&one_step("listJobs", &criteria(&[condition]))).expect_err("must not compile");
        assert!(error.0.contains("compares nothing"), "{condition}: {error}");
    }
    // An expression that may be absent is a check wherever it stands.
    compile(&one_step(
        "listJobs",
        &criteria(&[
            "$statusCode == 200 && $response.body#/id",
            "!$response.header.X-Error",
        ]),
    ))
    .expect("compiles");
}

const LOOP: &str = "workflows:
  - workflowId: w
    steps:
      - stepId: setup
        operationId: listJobs
      - stepId: work
        operationId: startJob
        successCriteria:
          - condition: $statusCode == 202
        onFailure:
          - { name: again, type: goto, stepId: setup }
";

#[test]
fn a_step_a_goto_runs_again_is_still_one_step() {
    let server = Script::default()
        .on("GET /jobs", vec![reply(200, "{}")])
        .on("POST /jobs", vec![reply(500, "")]);
    let plan = compile(LOOP).expect("compiles");
    let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
    assert!(server.requests().len() > 4, "the loop ran until it was cut");
    assert_eq!(
        counts(&summary),
        (1, 1, 0),
        "two steps, however often they ran: {events:#?}"
    );
    assert_eq!(
        case_counts(&reporters::workflow_cases(&plan, &events)),
        counts(&summary),
        "the totals are the totals of the reported cases"
    );
}

#[test]
fn a_step_that_never_ran_is_skipped_whatever_the_route() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: a
        operationId: listJobs
        onSuccess:
          - { name: jump, type: goto, stepId: c }
      - stepId: b
        operationId: startJob
      - stepId: c
        operationId: cleanup
        successCriteria:
          - condition: $statusCode == 200
      - stepId: d
        operationId: showJob
        parameters:
          - { name: jobId, in: path, value: j1 }
";
    let server = Script::default()
        .on("GET /jobs", vec![reply(200, "{}")])
        .on("POST /cleanup", vec![reply(500, "")]);
    let plan = compile(flow).expect("compiles");
    let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
    assert_eq!(server.requests(), ["GET /jobs", "POST /cleanup"]);
    assert_eq!(counts(&summary), (1, 1, 2), "{events:#?}");
    assert_eq!(
        case_counts(&reporters::workflow_cases(&plan, &events)),
        counts(&summary)
    );
}

#[test]
fn a_retry_delay_is_bounded() {
    use std::time::Duration;
    let declared = Duration::from_millis(250);
    let longest = Duration::from_secs(300);
    for seconds in ["86400", "1e20"] {
        let header = [("Retry-After".to_owned(), seconds.to_owned())];
        assert_eq!(
            crate::exec::retry_delay(declared, &header),
            longest,
            "Retry-After: {seconds}"
        );
    }
    for not_a_delay in ["inf", "NaN", "-1", "1e400"] {
        let header = [("Retry-After".to_owned(), not_a_delay.to_owned())];
        assert_eq!(
            crate::exec::retry_delay(declared, &header),
            declared,
            "Retry-After: {not_a_delay}"
        );
    }
    // A declared delay is held to the same bound instead of overflowing.
    for declared in ["86400", "1e20"] {
        let flow = POLL.replace("retryAfter: 0", &format!("retryAfter: {declared}"));
        let plan = compile(&flow).expect("compiles");
        let crate::plan::ActionKind::Retry { after, .. } =
            plan.workflows[0].steps[0].on_failure[0].kind
        else {
            panic!("a retry");
        };
        assert_eq!(after, longest, "retryAfter: {declared}");
    }
}

#[test]
fn a_condition_means_the_same_inside_parentheses() {
    let run_one = |condition: &str, body: &'static str| {
        let flow = one_step("listJobs", &criteria(&[condition]));
        let server = Script::default().on("GET /jobs", vec![reply(200, body)]);
        counts(&run(&flow, &server).0)
    };
    for condition in ["$response.body#/x == null", "($response.body#/x == null)"] {
        assert_eq!(run_one(condition, "{}"), (1, 0, 0), "{condition}: absent");
        assert_eq!(
            run_one(condition, r#"{"x":null}"#),
            (1, 0, 0),
            "{condition}: null"
        );
        assert_eq!(
            run_one(condition, r#"{"x":1}"#),
            (0, 1, 0),
            "{condition}: present"
        );
    }
    for condition in [
        "$response.body#/q == 'it''s'",
        "($response.body#/q == 'it''s')",
    ] {
        assert_eq!(
            run_one(condition, r#"{"q":"it's"}"#),
            (1, 0, 0),
            "{condition}"
        );
        assert_eq!(
            run_one(condition, r#"{"q":"it''s"}"#),
            (0, 1, 0),
            "{condition}"
        );
    }
    for condition in ["$response.body != null", "($response.body != null)"] {
        assert_eq!(
            run_one(condition, "{}"),
            (1, 0, 0),
            "{condition}: a document"
        );
        assert_eq!(run_one(condition, "null"), (0, 1, 0), "{condition}: null");
        assert_eq!(run_one(condition, ""), (0, 1, 0), "{condition}: no body");
    }
}

#[test]
fn a_regex_criterion_needs_its_context_to_exist() {
    let flow = one_step(
        "listJobs",
        "        successCriteria:\n          - context: $response.header.X-Request-Id\n            condition: '^[a-z]*$'\n            type: regex\n",
    );
    let with_id = Script::default().on(
        "GET /jobs",
        vec![Reply {
            status: 200,
            headers: vec![("x-request-id", "abc")],
            body: "{}",
        }],
    );
    assert_eq!(counts(&run(&flow, &with_id).0), (1, 0, 0));
    let without = Script::default().on("GET /jobs", vec![reply(200, "{}")]);
    let (summary, events) = run(&flow, &without);
    assert_eq!(
        counts(&summary),
        (0, 1, 0),
        "a header that is not there matches nothing: {events:#?}"
    );
    assert!(failures(&events)[0].2.contains("missing"), "{events:#?}");
}

#[test]
fn a_body_that_is_not_json_is_its_text() {
    let flow = one_step(
        "listJobs",
        "        successCriteria:\n          - context: $response.body\n            condition: '^OK$'\n            type: regex\n          - condition: $response.body == 'OK'\n          - condition: $response.body != null\n",
    );
    let ok = Script::default().on("GET /jobs", vec![reply(200, "OK")]);
    let (summary, events) = run(&flow, &ok);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");
    let not_ok = Script::default().on("GET /jobs", vec![reply(200, "FAILED")]);
    let (summary, events) = run(&flow, &not_ok);
    assert_eq!(counts(&summary), (0, 1, 0));
    assert_eq!(failures(&events).len(), 2, "{events:#?}");
}

#[test]
fn a_retry_replaces_only_the_attempt_it_retries() {
    let flow = "workflows:
  - workflowId: w
    steps:
      - stepId: setup
        operationId: listJobs
      - stepId: work
        operationId: startJob
        successCriteria:
          - condition: $statusCode == 202
        onFailure:
          - name: busy
            type: retry
            retryAfter: 0
            retryLimit: 2
            criteria:
              - condition: $statusCode == 503
          - { name: restart, type: goto, stepId: setup }
";
    // 500: back to setup. Then 503: retried. Then accepted.
    let server = Script::default()
        .on("GET /jobs", vec![reply(200, "{}")])
        .on(
            "POST /jobs",
            vec![reply(500, ""), reply(503, ""), reply(202, "{}")],
        );
    let plan = compile(flow).expect("compiles");
    let (summary, events) = run_with(&plan, &server, serde_json::json!({}));
    assert_eq!(
        server.requests(),
        [
            "GET /jobs",
            "POST /jobs",
            "GET /jobs",
            "POST /jobs",
            "POST /jobs"
        ],
        "{events:#?}"
    );
    assert_eq!(
        counts(&summary),
        (1, 1, 0),
        "the run that failed stays on the record: {events:#?}"
    );
    let cases = reporters::workflow_cases(&plan, &events);
    assert_eq!(case_counts(&cases), (1, 1, 0), "{cases:#?}");
    let reporters::Outcome::Failed { failures } = &cases[1].outcome else {
        panic!("`work` failed: {cases:#?}");
    };
    assert_eq!(
        failures.len(),
        1,
        "only the run no retry replaced: {failures:#?}"
    );
    assert!(failures[0].actual.contains("500"), "{failures:#?}");
    let console = reporters::console(&summary, &events);
    assert_eq!(console.matches("  FAIL ").count(), 1, "{console}");
    assert!(console.contains("500"), "{console}");
}

#[test]
fn a_qualified_operation_id_resolves_only_in_its_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("api.yaml"), API).expect("api");
    std::fs::write(
        dir.path().join("billing.yaml"),
        "openapi: 3.1.0\ninfo: { title: Billing, version: '1' }\npaths:\n  /invoices:\n    get:\n      operationId: listInvoices\n      responses:\n        default: { description: any }\n",
    )
    .expect("billing");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("api.yaml").expect("load");
    ws.load_all("billing.yaml").expect("load");
    let ws = Arc::new(ws);
    let compile_step = |operation: &str| {
        let text = format!(
            "arazzo: 1.0.0\ninfo: {{ title: t, version: '1' }}\nsourceDescriptions:\n  - {{ name: api, url: api.yaml, type: openapi }}\n  - {{ name: billing, url: billing.yaml, type: openapi }}\nworkflows:\n  - workflowId: w\n    steps:\n      - stepId: s\n        operationId: {operation}\n"
        );
        let doc = LowDoc::parse(
            "mem://flows.arazzo.yaml".into(),
            Source::from_vec(text.into_bytes()),
        );
        compile_plan(&doc, &ws)
    };
    let plan = compile_step("$sourceDescriptions.billing.listInvoices").expect("compiles");
    assert_eq!(plan.workflows[0].steps[0].contract, Some(1));
    compile_step("listJobs").expect("an unqualified id resolves anywhere");

    // `billing` exists and does not declare it: not another source's.
    let error = compile_step("$sourceDescriptions.billing.listJobs").expect_err("not in billing");
    assert!(
        error.0.contains("listJobs") && error.0.contains("'billing'"),
        "{error}"
    );
    let error = compile_step("$sourceDescriptions.nowhere.listJobs").expect_err("no such source");
    assert!(error.0.contains("'nowhere'"), "{error}");
}

#[test]
fn a_root_filter_reads_an_array_context_as_its_items() {
    let flow = one_step(
        "listJobs",
        "        successCriteria:\n          - context: $response.body\n            condition: $[?!@.error]\n            type: jsonpath\n",
    );
    let clean = Script::default().on("GET /jobs", vec![reply(200, r#"[{"id":1}]"#)]);
    assert_eq!(counts(&run(&flow, &clean).0), (1, 0, 0));
    let failed = Script::default().on("GET /jobs", vec![reply(200, r#"[{"error":"x"}]"#)]);
    let (summary, events) = run(&flow, &failed);
    assert_eq!(
        counts(&summary),
        (0, 1, 0),
        "no item is without an error: {events:#?}"
    );
}

#[test]
fn a_parenthesis_ends_a_pointer_and_a_pattern_keeps_its_groups() {
    let run_one = |condition: &str, body: &'static str| {
        let flow = one_step("listJobs", &criteria(&[condition]));
        let server = Script::default().on("GET /jobs", vec![reply(200, body)]);
        counts(&run(&flow, &server).0)
    };
    assert_eq!(
        run_one("!($response.body#/failed)", r#"{"failed":false}"#),
        (1, 0, 0)
    );
    assert_eq!(
        run_one("!($response.body#/failed)", r#"{"failed":true}"#),
        (0, 1, 0)
    );
    assert_eq!(
        run_one(
            "($response.body#/a == 1) && ($response.body#/b == 2)",
            r#"{"a":1,"b":2}"#
        ),
        (1, 0, 0)
    );
    // Parentheses inside a pointer belong to it.
    assert_eq!(
        run_one("($response.body#/f(x) == 1)", r#"{"f(x)":1}"#),
        (1, 0, 0)
    );

    assert_eq!(run_one("$response.body /= /(a|b)c/", "bc"), (1, 0, 0));
    assert_eq!(run_one("$response.body /= /(a|b)c/", "dc"), (0, 1, 0));
    // What follows a pattern is not dropped.
    let error = compile(&one_step(
        "listJobs",
        &criteria(&["$response.body /= /a/ && $statusCode == 500"]),
    ))
    .expect_err("must not compile");
    assert!(error.0.contains("unsupported success criterion"), "{error}");
}

#[test]
fn a_pattern_match_has_two_spellings() {
    for operator in ["/=", "=~"] {
        let flow = one_step(
            "listJobs",
            &criteria(&[
                &format!("$response.body#/name {operator} /^job-[0-9]+$/"),
                &format!("{{$response.body#/name}} {operator} /^job-/"),
                &format!("$response.body {operator} /job-7/"),
                &format!("$statusCode {operator} /^2../"),
                &format!("$statusCode {operator} /^20[01]$/"),
            ]),
        );
        let named = Script::default().on("GET /jobs", vec![reply(200, r#"{"name":"job-7"}"#)]);
        let (summary, events) = run(&flow, &named);
        assert_eq!(counts(&summary), (1, 0, 0), "{operator}: {events:#?}");

        // The pattern is held to the value it names, not to the whole body.
        let elsewhere = Script::default().on(
            "GET /jobs",
            vec![reply(204, r#"{"name":"task","note":"job-7 200"}"#)],
        );
        let (summary, events) = run(&flow, &elsewhere);
        assert_eq!(counts(&summary), (0, 1, 0), "{operator}");
        assert_eq!(
            failures(&events).len(),
            3,
            "the two patterns on the name and the one on the status: {events:#?}"
        );
    }
    // A pattern that is not a regular expression does not compile.
    let error = compile(&one_step(
        "listJobs",
        &criteria(&["$response.body#/name /= /([/"]),
    ))
    .expect_err("must not compile");
    assert!(error.0.contains("unsupported success criterion"), "{error}");
}

#[test]
fn an_expression_in_braces_reads_the_same_value() {
    let flow = one_step(
        "listJobs",
        &criteria(&[
            "{$response.body#/name} == 'job-7'",
            "{$response.body#/total} > 1",
            "{$response.body#/name} != null",
            "{$response.body#/name}",
        ]),
    );
    let server = Script::default().on(
        "GET /jobs",
        vec![reply(200, r#"{"name":"job-7","total":2}"#)],
    );
    let (summary, events) = run(&flow, &server);
    assert_eq!(counts(&summary), (1, 0, 0), "{events:#?}");
}

#[test]
fn text_after_an_expression_is_not_part_of_its_pointer() {
    let error = compile(&one_step(
        "listJobs",
        &criteria(&["$response.body#/name is 'rex'"]),
    ))
    .expect_err("must not compile");
    assert!(error.0.contains("unsupported success criterion"), "{error}");
}
