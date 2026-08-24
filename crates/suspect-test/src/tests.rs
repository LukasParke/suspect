//! End-to-end coverage for plan compilation, execution, reporters, and
//! transports against an inline Arazzo + OpenAPI fixture.

use crate::exec::{HttpClient, HttpRequest, HttpResponse, RunSummary, TestEvent, run_plan};
use crate::fuzz;
use crate::plan::{CompileError, CriterionKind, CriterionPlan, OpKey, compile_plan};
use crate::reporters;
use crate::transports::{CannedTransport, Match, ReplayTransport};
use bytes::Bytes;
use serde_json::Value;
use std::sync::Arc;
use suspect_journal::{Body, CassetteEntry};
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_rex::Rex;
use suspect_source::Source;

use suspect_ir::{Method, ParamIn};

const OAS: &str = r#"
openapi: 3.1.0
info:
  title: Petstore
  version: "1.0"
servers:
  - url: http://api.example.com
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200': { description: ok }
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema: { type: object }
      responses:
        '201': { description: created }
  '/pets/{petId}':
    get:
      operationId: showPetById
      responses:
        '200': { description: ok }
  '/users/{userId}':
    delete:
      operationId: deleteUser
      parameters:
        - name: userId
          in: path
          required: true
          schema: { type: string }
      responses:
        '204': { description: gone }
"#;

const ARAZZO: &str = r#"
arazzo: 1.0.0
info:
  title: flows
  version: "1.0"
sourceDescriptions:
  - name: petstore
    url: spec.yaml
workflows:
  - workflowId: create-and-fetch
    parameters:
      - name: userName
        value: world
    steps:
      - stepId: create-pet
        operationId: createPet
        requestBody:
          name: Rex
          tag: happy
        successCriteria:
          - condition: '{$statusCode} == 201'
        outputs:
          petId: $response.body#/id
      - stepId: show-created
        operationPath: $sourceDescriptions.petstore#/paths/~1pets~1{petId}/get
        parameters:
          - name: petId
            in: path
            value: $steps.create-pet.outputs.petId
          - name: verbose
            in: query
            value: 'yes'
        successCriteria:
          - condition: '{$statusCode} /= /^2../'
          - condition: '$response.body#/name != null'
  - workflowId: cleanup-user
    parameters:
      - name: userName
        value: world
    steps:
      - stepId: delete-user
        operationPath: 'DELETE /users/{userId}'
        parameters:
          - name: userId
            in: path
            value: $inputs.userName
        successCriteria:
          - condition: '{$statusCode} == 204'
"#;

/// Parses the inline Arazzzo fixture into a `LowDoc`.
fn arazzo_doc() -> LowDoc {
    LowDoc::parse(
        "mem://flow.arazzo.yaml".into(),
        Source::from_vec(ARAZZO.as_bytes().to_vec()),
    )
}

/// Builds a workspace containing only `spec.yaml` and returns it shared.
fn workspace() -> Arc<suspect_ref::Workspace> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), OAS).expect("write spec");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    let _ = ws.load_all("spec.yaml").expect("load spec");
    // Keep the tempdir alive for process lifetime; tests are short-lived.
    std::mem::forget(dir);
    Arc::new(ws)
}

fn compile_fixture() -> crate::plan::Plan {
    let ws = workspace();
    compile_plan(&arazzo_doc(), &ws).expect("compiles")
}

#[test]
fn compiles_operations_parameters_and_criteria() {
    let plan = compile_fixture();
    assert_eq!(plan.workflows.len(), 2);

    let wf1 = &plan.workflows[0];
    assert_eq!(wf1.workflow_id, "create-and-fetch");
    assert_eq!(
        wf1.inputs.get("userName").and_then(|v| v.as_str()),
        Some("world")
    );
    assert_eq!(wf1.steps.len(), 2);

    let create = &wf1.steps[0];
    assert_eq!(
        create.operation,
        OpKey {
            method: Method::Post,
            path: "/pets".to_owned()
        }
    );
    // Object request body serializes to JSON text.
    assert!(matches!(&create.request_body, Some(Rex::Text(json)) if json.contains("\"name\"")));
    assert_eq!(
        create.success[0].kind,
        CriterionKind::Equals {
            pointer: None,
            expected: serde_json::json!(201)
        }
    );
    assert!(matches!(
        create.outputs.as_slice(),
        [(name, Rex::Response { .. })] if name == "petId"
    ));

    let show = &wf1.steps[1];
    assert_eq!(
        show.operation,
        OpKey {
            method: Method::Get,
            path: "/pets/{petId}".to_owned()
        }
    );
    assert_eq!(show.success[0].kind, CriterionKind::StatusInRange(2, 2));
    assert_eq!(
        show.success[1].kind,
        CriterionKind::NotNull {
            pointer: "/name".to_owned()
        }
    );
    assert_eq!(show.body_pointers, vec!["/name".to_owned()]);

    let chained = show
        .parameters
        .iter()
        .find(|p| p.name == "petId")
        .expect("path param");
    assert_eq!(chained.location, ParamIn::Path);
    assert!(matches!(&chained.value, Rex::Steps { step, .. } if step == "create-pet"));

    let delete = &plan.workflows[1].steps[0];
    assert_eq!(
        delete.operation,
        OpKey {
            method: Method::Delete,
            path: "/users/{userId}".to_owned()
        }
    );
    assert_eq!(
        delete.success[0].kind,
        CriterionKind::Equals {
            pointer: None,
            expected: serde_json::json!(204)
        }
    );
}

#[test]
fn missing_ids_and_unknown_sources_fail_compilation() {
    let ws = workspace();
    let bad = r#"
arazzo: 1.0.0
sourceDescriptions:
  - name: nowhere
    url: missing.yaml
workflows:
  - workflowId: w
    steps:
      - stepId: s
        operationPath: 'GET /pets'
"#;
    let doc = LowDoc::parse(
        "mem://bad.arazzo.yaml".into(),
        Source::from_vec(bad.as_bytes().to_vec()),
    );
    let err = compile_plan(&doc, &ws).expect_err("unknown source");
    assert!(err.0.contains("matches no loaded document"));

    let no_id = LowDoc::parse(
        "mem://noid.arazzo.yaml".into(),
        Source::from_vec(
            b"arazzo: 1.0.0\nworkflows:\n  - workflowId: w\n    steps:\n      - operationPath: 'GET /pets'\n".to_vec(),
        ),
    );
    assert_eq!(
        compile_plan(&no_id, &ws),
        Err(CompileError("step missing stepId".to_owned()))
    );
}

fn canned_fixture_http() -> CannedTransport {
    let body = |s: &str| Bytes::from(s.to_owned());
    CannedTransport::new()
        .route(
            Match {
                method: Some("POST".to_owned()),
                path_suffix: "/pets".to_owned(),
            },
            HttpResponse {
                status: 201,
                headers: Vec::new(),
                body: body(r#"{"id":"7","name":"Rex"}"#),
            },
        )
        .route(
            Match {
                method: Some("GET".to_owned()),
                path_suffix: "/pets/7".to_owned(),
            },
            HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: body(r#"{"id":"7","name":"Rex"}"#),
            },
        )
        .route(
            Match {
                method: Some("DELETE".to_owned()),
                path_suffix: "/users/world".to_owned(),
            },
            HttpResponse {
                status: 204,
                headers: Vec::new(),
                body: Bytes::new(),
            },
        )
}

async fn drain(mut rx: tokio::sync::mpsc::Receiver<TestEvent>) -> Vec<TestEvent> {
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test(flavor = "multi_thread")]
async fn canned_run_passes_and_chains_outputs() {
    let plan = compile_fixture();
    let (tx, rx) = tokio::sync::mpsc::channel(256);

    let summary = run_plan(&plan, "http://api.test", &canned_fixture_http(), tx).await;
    let events = drain(rx).await;

    assert_eq!(
        summary,
        RunSummary {
            passed: 3,
            failed: 0,
            skipped: 0,
            duration_ms: summary.duration_ms
        }
    );

    // Chained output flows into the second request URL.
    let urls: Vec<&str> = events
        .iter()
        .filter_map(|ev| match ev {
            TestEvent::RequestSent { url, .. } => Some(url.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        urls.iter().any(|u| u.ends_with("/pets/7?verbose=yes")),
        "chained URL missing from {urls:?}"
    );
    assert!(urls.iter().any(|u| u.ends_with("/users/world")));

    // Output capture event recorded with the created id.
    assert!(events.iter().any(|ev| matches!(
        ev,
        TestEvent::OutputSet { key, value, .. }
            if key == "petId" && value == &serde_json::json!("7")
    )));

    // Every criterion emitted an Ok; nothing failed.
    assert_eq!(
        events
            .iter()
            .filter(|ev| matches!(ev, TestEvent::CriterionFail { .. }))
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|ev| matches!(ev, TestEvent::WfDone { passed: true, .. }))
            .count(),
        2
    );
    assert!(events.iter().any(|ev| matches!(
        ev,
        TestEvent::RunDone {
            passed: 3,
            failed: 0
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn failing_criterion_fails_step_and_skips_rest() {
    // One workflow, two steps; the first fails its criterion so the second
    // is skipped.
    let plan = crate::plan::Plan {
        workflows: vec![crate::plan::WfPlan {
            workflow_id: "broken-flow".to_owned(),
            inputs: Default::default(),
            input_defaults: Default::default(),
            steps: vec![
                crate::plan::StepPlan {
                    step_id: "boom".to_owned(),
                    operation: OpKey {
                        method: Method::Get,
                        path: "/pets".to_owned(),
                    },
                    parameters: Vec::new(),
                    request_body: None,
                    success: vec![CriterionPlan {
                        kind: CriterionKind::StatusInRange(2, 2),
                        range: 0..0,
                    }],
                    outputs: Vec::new(),
                    body_pointers: Vec::new(),
                    failure_goto: None,
                },
                crate::plan::StepPlan {
                    step_id: "never-runs".to_owned(),
                    operation: OpKey {
                        method: Method::Get,
                        path: "/pets".to_owned(),
                    },
                    parameters: Vec::new(),
                    request_body: None,
                    success: Vec::new(),
                    outputs: Vec::new(),
                    body_pointers: Vec::new(),
                    failure_goto: None,
                },
            ],
        }],
    };
    let http = CannedTransport::new().route(
        Match {
            method: None,
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 500,
            headers: Vec::new(),
            body: Bytes::from_static(b"boom"),
        },
    );

    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let summary = run_plan(&plan, "http://api.test", &http, tx).await;
    let events = drain(rx).await;

    assert_eq!(summary.passed, 0);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.skipped, 1);

    assert!(events.iter().any(|ev| matches!(
        ev,
        TestEvent::CriterionFail { crit, expected, actual, .. }
            if crit == "2xx" && expected == "2xx" && actual == "500"
    )));
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, TestEvent::StepStarted { step, .. } if step == "never-runs"))
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, TestEvent::WfDone { passed: false, .. }))
    );
}

#[test]
fn reporters_render_junit_console_and_ndjson() {
    let passing = RunSummary {
        passed: 3,
        failed: 0,
        skipped: 0,
        duration_ms: 12,
    };
    let junit_ok = reporters::junit(&passing);
    assert!(junit_ok.contains("<testsuites"));
    assert!(junit_ok.contains("failures=\"0\""));
    assert!(junit_ok.contains("<testcase "));
    assert!(junit_ok.contains("skipped=\"0\""));

    let failing = RunSummary {
        passed: 1,
        failed: 1,
        skipped: 0,
        duration_ms: 5,
    };
    let junit_bad = reporters::junit(&failing);
    assert!(junit_bad.contains("failures=\"1\""));
    assert!(junit_bad.contains("<failure "));

    let events = vec![
        TestEvent::WfStarted {
            id: "create-and-fetch".to_owned(),
        },
        TestEvent::RequestSent {
            wf: "create-and-fetch".to_owned(),
            step: "create-pet".to_owned(),
            method: "POST".to_owned(),
            url: "http://api.test/pets".to_owned(),
        },
        TestEvent::ResponseGot {
            wf: "create-and-fetch".to_owned(),
            step: "create-pet".to_owned(),
            status: 500,
            duration_ms: 3,
        },
        TestEvent::CriterionFail {
            wf: "create-and-fetch".to_owned(),
            step: "create-pet".to_owned(),
            crit: "2xx".to_owned(),
            expected: "2xx".to_owned(),
            actual: "500".to_owned(),
        },
        TestEvent::WfDone {
            wf: "create-and-fetch".to_owned(),
            passed: false,
        },
        TestEvent::RunDone {
            passed: 0,
            failed: 1,
        },
    ];
    let console = reporters::console(&failing, &events);
    assert!(console.contains("create-and-fetch"));
    assert!(console.contains("FAIL"));
    assert!(console.contains("expected 2xx, got 500"));

    let ndjson = reporters::ndjson(&events);
    let lines: Vec<&str> = ndjson.lines().collect();
    assert_eq!(lines.len(), events.len());
    for line in lines {
        let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
        assert!(parsed.get("event").is_some(), "tagged event line");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_transport_serves_entries_in_order() {
    let entry = |id: u64, status: u16, body: &str| CassetteEntry {
        id,
        method: "GET".to_owned(),
        url: format!("http://api.test/item{id}"),
        status,
        request_headers: Vec::new(),
        request_body: Body::from_bytes(b""),
        response_headers: Vec::new(),
        response_body: Body::from_bytes(body.as_bytes()),
        duration_ms: 1.0,
    };
    let transport = ReplayTransport::new(vec![entry(1, 200, "first"), entry(2, 404, "second")]);

    let first = transport
        .execute(HttpRequest::default())
        .await
        .expect("entry 1");
    assert_eq!(first.status, 200);
    assert_eq!(first.body.as_ref(), b"first");

    let second = transport
        .execute(HttpRequest {
            method: "GET".to_owned(),
            url: "http://anything-else/".to_owned(),
            headers: Vec::new(),
            body: Bytes::new(),
        })
        .await
        .expect("entry 2");
    assert_eq!(second.status, 404);
    assert_eq!(second.body.as_ref(), b"second");

    // Exhausted cassette is a transport error regardless of the request.
    let third = transport.execute(HttpRequest::default()).await;
    assert!(third.is_err());
}

#[test]
fn fuzz_scalar_fields_recurse_one_level() {
    let schema = serde_json::json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "integer"},
            "name": {"type": "string"},
            "owner": {"type": "object", "properties": {
                "email": {"type": "string"},
                "deep": {"type": "object", "properties": {
                    "hidden": {"type": "string"}
                }}
            }},
            "tags": {"type": "array", "items": {"type": "string"}},
            "meta": {"type": "array", "items": {"type": "object", "properties": {
                "k": {"type": "string"}
            }}}
        }
    });
    let fields = fuzz::scalar_fields(&schema);
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["/id", "/meta/0/k", "/name", "/owner/email", "/tags/0"]
    );
    let id = fields.iter().find(|f| f.name == "/id").unwrap();
    assert!(id.required);
    let email = fields.iter().find(|f| f.name == "/owner/email").unwrap();
    assert!(!email.required, "nested props default to optional");
}

#[test]
fn fuzz_mutant_values_are_deterministic_and_typed() {
    let s = serde_json::json!({"type": "string"});
    let n = serde_json::json!({"type": "integer"});
    use fuzz::MutantKind as K;
    assert_eq!(fuzz::mutant_value(&s, K::WrongType), serde_json::json!(42));
    assert_eq!(
        fuzz::mutant_value(&n, K::WrongType),
        Value::String("fuzz".into())
    );
    assert_eq!(
        fuzz::mutant_value(&s, K::Empty),
        Value::String(String::new())
    );
    assert_eq!(fuzz::mutant_value(&n, K::Negative), serde_json::json!(-1));
    assert_eq!(
        fuzz::mutant_value(&s, K::Oversize),
        Value::String("a".repeat(512))
    );
    assert_eq!(
        fuzz::mutant_value(&s, K::UnicodeBomb)
            .as_str()
            .unwrap()
            .chars()
            .count(),
        64
    );
    assert_eq!(fuzz::mutant_value(&s, K::NullRequired), Value::Null);
}

#[test]
fn fuzz_generate_cycles_fields_then_kinds() {
    let field = |name: &str| fuzz::ScalarField {
        name: name.to_owned(),
        schema: serde_json::json!({"type": "string"}),
        required: true,
    };
    let fields = vec![field("a"), field("b")];
    let mutants = fuzz::generate_mutants(&fields, 5);
    // First N mutants are fully pinned by (field, kind) cycling.
    assert_eq!(mutants[0].field, "a");
    assert_eq!(mutants[0].kind, fuzz::MutantKind::WrongType);
    assert_eq!(mutants[1].field, "b");
    assert_eq!(mutants[1].kind, fuzz::MutantKind::WrongType);
    assert_eq!(mutants[2].field, "a");
    assert_eq!(mutants[2].kind, fuzz::MutantKind::Empty);
    assert_eq!(mutants[3].field, "b");
    assert_eq!(mutants[3].kind, fuzz::MutantKind::Empty);
    assert_eq!(mutants[4].field, "a");
    assert_eq!(mutants[4].kind, fuzz::MutantKind::Oversize);
    // Re-running reproduces byte-identical values.
    let again = fuzz::generate_mutants(&fields, 5);
    assert_eq!(mutants, again);
    assert!(fuzz::generate_mutants(&[], 10).is_empty());
}

#[test]
fn fuzz_payload_defaults_everything_but_target() {
    let f = |name: &str| fuzz::ScalarField {
        name: name.to_owned(),
        schema: serde_json::json!({"type": "string"}),
        required: true,
    };
    let nested = fuzz::ScalarField {
        name: "/owner/email".into(),
        schema: serde_json::json!({"type": "string"}),
        required: false,
    };
    let fields = vec![f("/id"), f("/tag"), nested];
    let m = fuzz::Mutant {
        field: "/id".into(),
        kind: fuzz::MutantKind::NullRequired,
        value: Value::Null,
    };
    let payload = fuzz::payload(&fields, &m);
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["tag"], Value::String("suspect".into()));
    assert_eq!(payload["owner"]["email"], Value::String("suspect".into()));

    // Non-targeted runs keep everything benign.
    let benign = fuzz::payload(
        &fields,
        &fuzz::Mutant {
            field: String::new(),
            kind: fuzz::MutantKind::Empty,
            value: Value::Null,
        },
    );
    assert_eq!(benign["id"], Value::String("suspect".into()));
}
