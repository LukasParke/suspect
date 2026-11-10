//! Response conformance through the runner: every way a reply can break
//! the description fails its step, and nothing unchecked is reported as
//! checked.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_runtime::Conformance;
use suspect_source::Source;

use crate::exec::{
    HttpClient, HttpRequest, HttpResponse, RunOpts, RunSummary, TestEvent, TransportError,
    run_plan_with,
};
use crate::plan::{Plan, compile_plan};

const PETS: &str = r##"
openapi: 3.1.0
info: { title: Pets, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: a page, declared inline
          content:
            application/json:
              schema:
                type: object
                required: [items]
                properties:
                  items:
                    type: array
                    items: { $ref: '#/components/schemas/Pet' }
    post:
      operationId: createPet
      responses:
        '201':
          description: created
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Pet' }
  /pets/{petId}:
    get:
      operationId: showPet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: string } }
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
            text/csv:
              schema: { type: string }
        '404':
          $ref: '#/components/responses/NotFound'
  /health:
    get:
      operationId: health
      responses:
        '200': { description: up }
components:
  responses:
    NotFound:
      description: no such pet
      content:
        application/problem+json:
          schema: { $ref: '#/components/schemas/Problem' }
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string }
"##;

/// A second description that reuses the name `Pet` for a different shape.
const CLINIC: &str = r##"
openapi: 3.0.3
info: { title: Clinic, version: "1" }
paths:
  /patients:
    get:
      operationId: listPets
      responses:
        '200':
          description: one patient
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
components:
  schemas:
    Pet:
      type: object
      required: [chip]
      properties:
        chip: { type: integer }
        owner: { type: string, nullable: true }
"##;

const FLOWS: &str = r#"
arazzo: 1.0.0
info: { title: flows, version: "1" }
sourceDescriptions:
  - { name: pets, url: pets.yaml, type: openapi }
  - { name: clinic, url: clinic.yaml, type: openapi }
workflows:
  - workflowId: list
    steps:
      - stepId: list
        operationId: $sourceDescriptions.pets.listPets
  - workflowId: show
    steps:
      - stepId: show
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: p1 }
  - workflowId: missing
    steps:
      - stepId: missing
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: nope }
        successCriteria:
          - condition: '{$statusCode} == 404'
  - workflowId: create
    steps:
      - stepId: create
        operationId: createPet
  - workflowId: health
    steps:
      - stepId: health
        operationId: health
  - workflowId: patients
    steps:
      - stepId: patients
        operationId: $sourceDescriptions.clinic.listPets
  - workflowId: off-spec
    steps:
      - stepId: reset
        operationPath: 'POST /test/reset'
"#;

fn plan() -> Plan {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("pets.yaml"), PETS).expect("pets");
    std::fs::write(dir.path().join("clinic.yaml"), CLINIC).expect("clinic");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("pets.yaml").expect("load pets");
    ws.load_all("clinic.yaml").expect("load clinic");
    std::mem::forget(dir);
    let doc = LowDoc::parse(
        "mem://flows.arazzo.yaml".into(),
        Source::from_vec(FLOWS.as_bytes().to_vec()),
    );
    compile_plan(&doc, &Arc::new(ws)).expect("compiles")
}

/// One canned reply for every request, recording what was sent.
struct Server {
    status: u16,
    content_type: Option<&'static str>,
    body: &'static str,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Server {
    fn replying(status: u16, content_type: Option<&'static str>, body: &'static str) -> Self {
        Self {
            status,
            content_type,
            body,
            seen: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl HttpClient for Server {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.seen.lock().expect("lock").push(req);
        Ok(HttpResponse {
            status: self.status,
            headers: self
                .content_type
                .map(|value| vec![("Content-Type".to_owned(), value.to_owned())])
                .unwrap_or_default(),
            body: Bytes::from(self.body),
        })
    }
}

/// Runs one workflow of the fixture against `server` at `level`.
fn run(workflow: &str, server: &Server, level: Conformance) -> (RunSummary, Vec<TestEvent>) {
    let mut plan = plan();
    plan.workflows.retain(|wf| wf.workflow_id == workflow);
    assert_eq!(plan.workflows.len(), 1, "workflow {workflow} exists");
    let opts = RunOpts {
        conformance: level,
        ..RunOpts::new("http://api.example.com")
    };
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::channel(256);
            let summary = run_plan_with(&plan, &opts, server, tx).await;
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            (summary, events)
        })
}

/// The `(check, actual)` of every failed check in the event log.
fn failures(events: &[TestEvent]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            TestEvent::CriterionFail { crit, actual, .. } => Some((crit.clone(), actual.clone())),
            _ => None,
        })
        .collect()
}

fn validated(events: &[TestEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, TestEvent::ResponseValidated { .. }))
}

const JSON: Option<&str> = Some("application/json");

#[test]
fn a_conforming_response_passes_and_is_reported_as_validated() {
    let server = Server::replying(200, JSON, r#"{"items": [{"id": "p1", "name": "Rex"}]}"#);
    let (summary, events) = run("list", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    assert!(validated(&events), "{events:?}");
}

#[test]
fn an_inline_response_schema_is_enforced() {
    let server = Server::replying(200, JSON, r#"{"items": [{"id": 42}]}"#);
    let (summary, events) = run("list", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    let failures = failures(&events);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].0, "response-schema");
    assert!(
        failures[0].1.contains("/items/0/id") && failures[0].1.contains("name"),
        "each violation names its location: {}",
        failures[0].1
    );
    assert!(
        !validated(&events),
        "a violating body is never reported as matching the contract"
    );
}

#[test]
fn an_undeclared_status_fails_the_step() {
    let server = Server::replying(202, JSON, r#"{"id": "p1", "name": "Rex"}"#);
    let (summary, events) = run("show", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    let failures = failures(&events);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].0, "response-status");
    assert_eq!(failures[0].1, "202");
    assert!(
        events.iter().any(|event| matches!(
            event,
            TestEvent::CriterionFail { expected, .. } if expected == "a declared status (200, 404)"
        )),
        "the declared statuses are named: {events:?}"
    );
    assert!(!validated(&events));
}

#[test]
fn an_undeclared_content_type_fails_the_step() {
    let server = Server::replying(200, Some("text/html"), "<html></html>");
    let (summary, events) = run("list", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    let failures = failures(&events);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        failures[0],
        ("response-content-type".to_owned(), "text/html".to_owned())
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            TestEvent::CriterionFail { expected, .. }
                if expected == "a declared content type (application/json)"
        )),
        "the declared content types are named: {events:?}"
    );
}

#[test]
fn a_structured_json_media_type_is_enforced() {
    let server = Server::replying(201, Some("application/vnd.api+json"), r#"{"id": 7}"#);
    let (summary, events) = run("create", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    assert_eq!(failures(&events)[0].0, "response-schema");
}

#[test]
fn a_shared_error_response_is_enforced_even_when_the_criteria_pass() {
    // The workflow expects the 404; the body still has to be the declared
    // problem document.
    let server = Server::replying(
        404,
        Some("application/problem+json"),
        r#"{"detail": "no such pet"}"#,
    );
    let (summary, events) = run("missing", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    let failures = failures(&events);
    assert_eq!(
        failures.len(),
        1,
        "the status criterion itself passes: {failures:?}"
    );
    assert_eq!(failures[0].0, "response-schema");
    assert!(failures[0].1.contains("title"), "{}", failures[0].1);

    let conforming = Server::replying(
        404,
        Some("application/problem+json"),
        r#"{"title": "no such pet"}"#,
    );
    let (summary, events) = run("missing", &conforming, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
}

#[test]
fn a_response_without_a_schema_is_not_reported_as_validated() {
    let server = Server::replying(200, None, "");
    let (summary, events) = run("health", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    assert!(
        !validated(&events),
        "nothing was compared, so nothing is claimed: {events:?}"
    );
}

#[test]
fn a_declared_non_json_body_passes_without_a_schema_claim() {
    let server = Server::replying(200, Some("text/csv"), "id,name\n");
    let (summary, events) = run("show", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    assert!(!validated(&events));
}

#[test]
fn the_schema_level_ignores_statuses_the_description_omits() {
    let server = Server::replying(202, JSON, "{}");
    let (summary, events) = run("show", &server, Conformance::Schema);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    // But a declared response is still held to its schema.
    let wrong = Server::replying(200, JSON, "{}");
    let (summary, _) = run("show", &wrong, Conformance::Schema);
    assert_eq!((summary.passed, summary.failed), (0, 1));
}

#[test]
fn conformance_off_judges_by_the_criteria_alone() {
    let server = Server::replying(200, Some("text/html"), "<html></html>");
    let (summary, events) = run("list", &server, Conformance::Off);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    assert!(!validated(&events));
}

#[test]
fn each_source_description_is_checked_against_its_own_schemas() {
    // `Pet` means `{chip}` in the clinic description, and that description
    // is OpenAPI 3.0, so `nullable` applies.
    let server = Server::replying(200, JSON, r#"{"chip": 7, "owner": null}"#);
    let (summary, events) = run("patients", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (1, 0), "{events:?}");
    assert!(validated(&events));

    // The same body is not a `Pet` of the pets description.
    let (summary, events) = run("show", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
}

#[test]
fn a_source_qualified_operation_id_resolves_in_that_source() {
    let plan = plan();
    let step = |workflow: &str| {
        plan.workflows
            .iter()
            .find(|wf| wf.workflow_id == workflow)
            .expect("workflow")
            .steps[0]
            .clone()
    };
    assert_eq!(step("list").operation.path, "/pets");
    assert_eq!(step("patients").operation.path, "/patients");
    assert_ne!(step("list").contract, step("patients").contract);
}

#[test]
fn an_operation_outside_the_description_fails_when_strict() {
    let server = Server::replying(200, None, "");
    let (summary, events) = run("off-spec", &server, Conformance::Strict);
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:?}");
    let failures = failures(&events);
    assert_eq!(failures[0].0, "operation");
    assert!(
        failures[0].1.contains("POST /test/reset"),
        "{}",
        failures[0].1
    );

    let (summary, _) = run("off-spec", &server, Conformance::Schema);
    assert_eq!((summary.passed, summary.failed), (1, 0));
}

#[test]
fn the_accept_header_asks_for_the_declared_media_types() {
    let server = Server::replying(200, JSON, r#"{"id": "p1", "name": "Rex"}"#);
    let _ = run("show", &server, Conformance::Strict);
    let seen = server.seen.lock().expect("lock");
    let accept = seen[0]
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
        .map(|(_, value)| value.as_str())
        .expect("an Accept header");
    assert_eq!(
        accept,
        "application/json, application/problem+json, text/csv;q=0.5"
    );
}

#[test]
fn an_operation_without_declared_content_still_asks_for_json() {
    let server = Server::replying(200, None, "");
    let _ = run("health", &server, Conformance::Strict);
    let seen = server.seen.lock().expect("lock");
    let accept = seen[0]
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
        .map(|(_, value)| value.as_str());
    assert_eq!(accept, Some("application/json"));
}
