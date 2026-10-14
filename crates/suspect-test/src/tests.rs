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

use std::collections::BTreeMap;
use suspect_ir::{Method, ParamIn};

pub(crate) const OAS: &str = r#"
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
pub(crate) fn workspace() -> Arc<suspect_ref::Workspace> {
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

pub(crate) fn compile_fixture() -> crate::plan::Plan {
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
            expected: crate::plan::Expected::Literal(serde_json::json!(201))
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
            expected: crate::plan::Expected::Literal(serde_json::json!(204))
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

pub(crate) fn canned_fixture_http() -> CannedTransport {
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
        components: BTreeMap::new(),
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
                    request_content_type: None,
                    success: vec![CriterionPlan {
                        kind: CriterionKind::StatusInRange(2, 2),
                        range: 0..0,
                    }],
                    outputs: Vec::new(),
                    body_pointers: Vec::new(),
                    failure_goto: None,
                    timeout_ms: None,
                    message: None,
                    depends_on: Vec::new(),
                    workflow_depends_on: Vec::new(),
                    security: Vec::new(),
                    response_schemas: Vec::new(),
                    think: None,
                },
                crate::plan::StepPlan {
                    step_id: "never-runs".to_owned(),
                    operation: OpKey {
                        method: Method::Get,
                        path: "/pets".to_owned(),
                    },
                    parameters: Vec::new(),
                    request_body: None,
                    request_content_type: None,
                    success: Vec::new(),
                    outputs: Vec::new(),
                    body_pointers: Vec::new(),
                    failure_goto: None,
                    timeout_ms: None,
                    message: None,
                    depends_on: Vec::new(),
                    workflow_depends_on: Vec::new(),
                    security: Vec::new(),
                    response_schemas: Vec::new(),
                    think: None,
                },
            ],
            depends_on: Vec::new(),
            outputs: Vec::new(),
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
    assert_eq!(payload["tag"], Value::String("sample text".into()));
    assert_eq!(
        payload["owner"]["email"],
        Value::String("ada@example.org".into())
    );

    // Non-targeted runs keep everything benign.
    let benign = fuzz::payload(
        &fields,
        &fuzz::Mutant {
            field: String::new(),
            kind: fuzz::MutantKind::Empty,
            value: Value::Null,
        },
    );
    assert_eq!(benign["id"], Value::String("1".into()));
}

// ------------------------------------------- Arazzo 1.1 AsyncAPI message steps

const ASYNCAPI: &str = r#"
asyncapi: 3.0.0
info: {title: Orders bus, version: '1'}
channels:
  orders:
    address: orders
    send:
      - name: orderPlaced
        payload:
          type: object
          required: [orderId]
          properties: {orderId: {type: string}}
    receive:
      - name: orderConfirmed
        payload: {$ref: '#/components/schemas/Confirmation'}
components:
  schemas:
    Confirmation:
      type: object
      required: [orderId, status]
      properties:
        orderId: {type: string}
        status: {type: string}
"#;

const MIXED_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Order flow, version: '1'}
sourceDescriptions:
  - {name: api, url: spec.yaml, type: openapi}
  - {name: bus, url: asyncapi.yaml, type: asyncapi}
workflows:
  - workflowId: place-and-confirm
    inputs:
      type: object
      properties:
        orderId: {type: string, default: order-42}
    steps:
      - stepId: place
        operationId: createPet
        parameters:
          - {name: name, in: query, value: $inputs.orderId}
      - stepId: announce
        channelPath: '$sourceDescriptions.bus.orders'
        action: send
        correlationId: $inputs.orderId
        requestBody:
          payload:
            orderId: $inputs.orderId
      - stepId: await-confirmation
        channelPath: '$sourceDescriptions.bus.orders'
        action: receive
        correlationId: $inputs.orderId
        timeout: 1000
        successCriteria:
          - condition: '$message.payload#/status == "confirmed"'
        outputs:
          status: $message.payload#/status
"#;

/// Workspace with the OpenAPI spec, the AsyncAPI bus, and the flow.
fn mixed_workspace() -> Arc<suspect_ref::Workspace> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), OAS).expect("spec");
    std::fs::write(dir.path().join("asyncapi.yaml"), ASYNCAPI).expect("asyncapi");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("spec");
    ws.load_all("asyncapi.yaml").expect("asyncapi");
    std::mem::forget(dir);
    Arc::new(ws)
}

fn mixed_plan() -> crate::plan::Plan {
    let ws = mixed_workspace();
    let doc = LowDoc::parse(
        "mem://mixed.arazzo.yaml".into(),
        Source::from_vec(MIXED_ARAZZO.as_bytes().to_vec()),
    );
    compile_plan(&doc, &ws).expect("mixed flow compiles")
}

#[test]
fn asyncapi_steps_compile_into_message_plans() {
    let plan = mixed_plan();
    let steps = &plan.workflows[0].steps;
    assert_eq!(steps.len(), 3);
    assert!(steps[0].message.is_none(), "HTTP step stays HTTP");
    let send = steps[1].message.as_ref().expect("send step");
    assert_eq!(send.direction, crate::plan::MessageDirection::Send);
    assert_eq!(send.source, "bus");
    assert_eq!(send.channel, "orders");
    assert_eq!(send.message_type.as_deref(), Some("orderPlaced"));
    let receive = steps[2].message.as_ref().expect("receive step");
    assert_eq!(receive.direction, crate::plan::MessageDirection::Receive);
    // The AsyncAPI payload schema resolved through its `$ref`.
    let schema = receive
        .payload_schema
        .as_ref()
        .expect("payload schema from the asyncapi document");
    assert_eq!(schema["required"], serde_json::json!(["orderId", "status"]));
}

#[test]
fn mixed_http_and_message_workflow_executes() {
    use crate::exec::run_plan_with_messages;
    use crate::messaging::{LoopbackBroker, Message};

    let plan = mixed_plan();
    let broker = LoopbackBroker::with_inbox(BTreeMap::from([(
        "orders".to_owned(),
        vec![Message {
            channel: "orders".to_owned(),
            message_type: Some("orderConfirmed".to_owned()),
            correlation_id: Some("order-42".to_owned()),
            payload: serde_json::json!({"orderId": "order-42", "status": "confirmed"}),
        }],
    )]));
    let transport = crate::transports::CannedTransport::new().route(
        Match {
            method: Some("POST".to_owned()),
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: Bytes::from_static(br#"{"id":1}"#),
        },
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            run_plan_with_messages(
                &plan,
                "http://api.example.com",
                &transport,
                Some(&broker),
                tx,
            )
            .await
        });

    assert_eq!(summary.failed, 0, "no step failed: {summary:?}");
    assert_eq!(summary.passed, 3, "{summary:?}");

    // The send step published the evaluated payload.
    let published = broker.published();
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0].channel, "orders");
    assert_eq!(published[0].payload["orderId"], "order-42");
    assert_eq!(published[0].correlation_id.as_deref(), Some("order-42"));

    // The receive step's output came from `$message.payload`.
    let mut saw_output = false;
    while let Ok(event) = rx.try_recv() {
        if let TestEvent::OutputSet { key, value, .. } = event
            && key == "status"
        {
            assert_eq!(value, Value::String("confirmed".to_owned()));
            saw_output = true;
        }
    }
    assert!(saw_output, "the receive step captured its output");
}

#[test]
fn received_payload_must_match_the_declared_message_schema() {
    use crate::exec::run_plan_with_messages;
    use crate::messaging::{LoopbackBroker, Message};

    let plan = mixed_plan();
    // A confirmation missing the required `status` field.
    let broker = LoopbackBroker::with_inbox(BTreeMap::from([(
        "orders".to_owned(),
        vec![Message {
            channel: "orders".to_owned(),
            message_type: Some("orderConfirmed".to_owned()),
            correlation_id: Some("order-42".to_owned()),
            payload: serde_json::json!({"orderId": "order-42"}),
        }],
    )]));
    let transport = crate::transports::CannedTransport::new().route(
        Match {
            method: Some("POST".to_owned()),
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: Bytes::from_static(br#"{"id":1}"#),
        },
    );

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            run_plan_with_messages(
                &plan,
                "http://api.example.com",
                &transport,
                Some(&broker),
                tx,
            )
            .await
        });
    assert_eq!(
        summary.failed, 1,
        "the failing step stops its workflow: {summary:?}"
    );
    assert_eq!(summary.passed, 2, "{summary:?}");
}

#[test]
fn message_steps_fail_cleanly_without_a_broker() {
    use crate::exec::run_plan_with_messages;

    let plan = mixed_plan();
    let transport = crate::transports::CannedTransport::new().route(
        Match {
            method: Some("POST".to_owned()),
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: Bytes::from_static(br#"{"id":1}"#),
        },
    );
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            run_plan_with_messages(&plan, "http://api.example.com", &transport, None, tx).await
        });
    assert_eq!(
        summary.failed, 1,
        "the failing step stops its workflow: {summary:?}"
    );
    assert_eq!(summary.passed, 1, "{summary:?}");
}

// --------------------------- response validation through recursive components

/// A recursive response schema: the old depth-capped inliner collapsed this
/// to permissive past depth 8, so a violation nested through the recursion
/// silently passed. The compiler's document-root fallback resolves it.
const RECURSIVE_OAS: &str = r#"
openapi: 3.1.0
info:
  title: Recursive
  version: "1.0"
servers:
  - url: http://api.example.com
paths:
  /trees:
    get:
      operationId: getTree
      responses:
        '200':
          description: A tree
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Node'}
components:
  schemas:
    Node:
      type: object
      required: [name]
      properties:
        name: {type: string}
        child: {$ref: '#/components/schemas/Node'}
"#;

const RECURSIVE_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Recursive, version: '1'}
sourceDescriptions:
  - {name: api, url: spec.yaml, type: openapi}
workflows:
  - workflowId: fetch-tree
    steps:
      - stepId: tree
        operationId: getTree
"#;

#[test]
fn recursive_response_schema_violates_at_depth() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), RECURSIVE_OAS).expect("spec");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("load");
    std::mem::forget(dir);
    let ws = Arc::new(ws);
    let doc = LowDoc::parse(
        "mem://recursive.arazzo.yaml".into(),
        Source::from_vec(RECURSIVE_ARAZZO.as_bytes().to_vec()),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");
    assert!(
        !plan.components.is_empty(),
        "the response schema must be carried on the step"
    );

    // A violation nine levels deep: the old inliner went permissive at
    // depth 8, so this is exactly the case that used to pass.
    let mut body = String::new();
    for level in 0..9 {
        body.push_str(&format!("{{\"name\": \"n{level}\", \"child\": "));
    }
    body.push_str("{\"name\": 7}");
    body.push_str(&"}".repeat(9));

    let transport = crate::transports::CannedTransport::new().route(
        Match {
            method: Some("GET".to_owned()),
            path_suffix: "/trees".to_owned(),
        },
        HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from(body.into_bytes()),
        },
    );
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async { run_plan(&plan, "http://api.example.com", &transport, tx).await });

    assert_eq!(
        summary.passed, 0,
        "a type violation nine levels deep must fail the step: {summary:?}"
    );
    assert_eq!(summary.failed, 1, "{summary:?}");
}

#[test]
fn conforming_recursive_response_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), RECURSIVE_OAS).expect("spec");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("load");
    std::mem::forget(dir);
    let ws = Arc::new(ws);
    let doc = LowDoc::parse(
        "mem://recursive-ok.arazzo.yaml".into(),
        Source::from_vec(RECURSIVE_ARAZZO.as_bytes().to_vec()),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");

    let mut body = String::new();
    for level in 0..9 {
        body.push_str(&format!("{{\"name\": \"n{level}\", \"child\": "));
    }
    body.push_str("{\"name\": \"leaf\"}");
    body.push_str(&"}".repeat(9));

    let transport = crate::transports::CannedTransport::new().route(
        Match {
            method: Some("GET".to_owned()),
            path_suffix: "/trees".to_owned(),
        },
        HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from(body.into_bytes()),
        },
    );
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async { run_plan(&plan, "http://api.example.com", &transport, tx).await });
    assert_eq!(
        summary.passed, 1,
        "a conforming deep tree passes: {summary:?}"
    );
}

// ------------------------------------------------- Arazzo 1.1 step dependencies

/// A workflow whose steps are declared out of dependency order: `report`
/// is listed first but depends on the data step listed last.
const ORDERED_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Ordered, version: '1'}
sourceDescriptions:
  - {name: api, url: spec.yaml, type: openapi}
workflows:
  - workflowId: ordered
    steps:
      - stepId: report
        operationId: createPet
        dependsOn: [collect]
        parameters:
          - {name: name, in: query, value: $steps.collect.outputs.collected}
      - stepId: collect
        operationId: listPets
        outputs:
          collected: $response.body
"#;

/// Records the execution order the scheduler chose.
#[derive(Default)]
struct OrderRecorder {
    paths: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HttpClient for OrderRecorder {
    async fn execute(
        &self,
        req: crate::exec::HttpRequest,
    ) -> Result<crate::exec::HttpResponse, crate::exec::TransportError> {
        self.paths.lock().unwrap().push(req.url.clone());
        Ok(crate::exec::HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from_static(b"[]"),
        })
    }
}

fn compile_ordered() -> crate::plan::Plan {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), OAS).expect("spec");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("load");
    std::mem::forget(dir);
    let ws = Arc::new(ws);
    let doc = LowDoc::parse(
        "mem://ordered.arazzo.yaml".into(),
        Source::from_vec(ORDERED_ARAZZO.as_bytes().to_vec()),
    );
    compile_plan(&doc, &ws).expect("compiles")
}

#[test]
fn dependencies_are_compiled_onto_the_plan() {
    let plan = compile_ordered();
    let steps = &plan.workflows[0].steps;
    let report = steps.iter().find(|s| s.step_id == "report").unwrap();
    // Explicit dependsOn.
    assert!(report.depends_on.contains(&"collect".to_owned()));
    // Implicit: the parameter reads $steps.collect.outputs.collected.
    assert_eq!(report.depends_on, vec!["collect".to_owned()]);
    let collect = steps.iter().find(|s| s.step_id == "collect").unwrap();
    assert!(collect.depends_on.is_empty());
}

#[test]
fn the_scheduler_runs_dependencies_first() {
    let plan = compile_ordered();
    let recorder = OrderRecorder::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async { run_plan(&plan, "http://api.example.com", &recorder, tx).await });
    assert_eq!(summary.failed, 0, "{summary:?}");

    // The dependent step ran second even though it is declared first.
    let started: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|event| match event {
            crate::exec::TestEvent::StepStarted { step, .. } => Some(step),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec!["collect".to_owned(), "report".to_owned()]);
}

#[test]
fn an_unsatisfiable_graph_fails_with_a_diagnosis() {
    // `b` depends on `c` and `c` depends on `b`: no step can ever run.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), OAS).expect("spec");
    let ws = WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("load");
    std::mem::forget(dir);
    let ws = Arc::new(ws);
    let doc = LowDoc::parse(
        "mem://cycle.arazzo.yaml".into(),
        Source::from_vec(
            b"arazzo: 1.1.0
info: {title: Cycle, version: '1'}
sourceDescriptions:
  - {name: api, url: spec.yaml, type: openapi}
workflows:
  - workflowId: cyclic
    steps:
      - stepId: b
        operationId: listPets
        dependsOn: [c]
      - stepId: c
        operationId: listPets
        dependsOn: [b]
"
            .to_vec(),
        ),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let summary = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            run_plan(&plan, "http://api.example.com", &CannedTransport::new(), tx).await
        });
    assert!(
        summary.passed == 0,
        "a cyclic graph runs nothing: {summary:?}"
    );
    // The blocked graph must fail the run, not pass as "0 failed".
    assert_eq!((summary.failed, summary.skipped), (1, 1), "{summary:?}");
    let mut diagnosed = false;
    while let Ok(event) = rx.try_recv() {
        if let crate::exec::TestEvent::CriterionFail { crit, actual, .. } = event
            && crit == "depends-on"
        {
            assert!(actual.contains("unsatisfied dependencies"), "{actual}");
            diagnosed = true;
        }
    }
    assert!(diagnosed, "a blocked graph must say why");
}

// --- credentials: grant encoding, caching, files, discovery ---

/// A token endpoint that records the form bodies it receives, serving the
/// same access token a real one would.
#[derive(Default)]
struct TokenEndpoint {
    bodies: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HttpClient for TokenEndpoint {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, crate::exec::TransportError> {
        self.bodies
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&req.body).into_owned());
        Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::from_static(br#"{"access_token":"tok-xyz","expires_in":3600}"#),
        })
    }
}

/// Resolves one credential against a fresh state and a canned endpoint.
fn resolve_credential(
    credential: &crate::auth::Credential,
    http: &TokenEndpoint,
) -> crate::auth::Injected {
    let state = crate::auth::AuthState::default();
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            state
                .resolve(http, "scheme", credential)
                .await
                .expect("resolves")
                .expect("places a header")
        })
}

fn credential(json: Value) -> crate::auth::Credential {
    serde_json::from_value(json).expect("credential parses")
}

#[test]
fn oauth_grant_encodes_reserved_characters_in_the_form_body() {
    // A secret containing `&` and `=` is a value, not form structure: it
    // must arrive percent-encoded or it would corrupt the grant.
    let credential = credential(serde_json::json!({
        "kind": "clientCredentials",
        "token_url": "https://sso.example.com/token",
        "client_id": "cid",
        "client_secret": "p&a=ss",
    }));
    let endpoint = TokenEndpoint::default();
    let placed = resolve_credential(&credential, &endpoint);
    assert_eq!(
        placed,
        crate::auth::Injected::Header("Authorization".to_owned(), "Bearer tok-xyz".to_owned()),
        "the grant's access token becomes the bearer"
    );
    assert_eq!(
        endpoint.bodies.lock().unwrap().clone(),
        ["grant_type=client_credentials&client_id=cid&client_secret=p%26a%3Dss".to_owned()],
        "unreserved marks stay literal, reserved ones are encoded"
    );
}

#[test]
fn refresh_token_grant_uses_the_refresh_grant_and_encodes() {
    let credential = credential(serde_json::json!({
        "kind": "refreshToken",
        "token_url": "https://sso.example.com/token",
        "client_id": "cid",
        "client_secret": "s&e",
        "refresh_token": "r&t",
        "scope": "read write",
    }));
    let endpoint = TokenEndpoint::default();
    let placed = resolve_credential(&credential, &endpoint);
    assert_eq!(
        placed,
        crate::auth::Injected::Header("Authorization".to_owned(), "Bearer tok-xyz".to_owned())
    );
    let bodies = endpoint.bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        1,
        "one grant per resolve, the refresh token goes nowhere else"
    );
    assert!(
        bodies[0].starts_with("grant_type=refresh_token&"),
        "the grant type is refresh: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("refresh_token=r%26t"),
        "the refresh token is a value: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("client_secret=s%26e"),
        "the client secret is a value: {}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("scope=read%20write"),
        "spaces encode: {}",
        bodies[0]
    );
}

#[test]
fn oauth_tokens_are_cached_across_resolves() {
    let credential = credential(serde_json::json!({
        "kind": "clientCredentials",
        "token_url": "https://sso.example.com/token",
        "client_id": "cid",
        "client_secret": "sec",
    }));
    let endpoint = TokenEndpoint::default();
    let state = crate::auth::AuthState::default();
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            for _ in 0..2 {
                let placed = state
                    .resolve(&endpoint, "scheme", &credential)
                    .await
                    .expect("resolves")
                    .expect("places a header");
                assert_eq!(
                    placed,
                    crate::auth::Injected::Header(
                        "Authorization".to_owned(),
                        "Bearer tok-xyz".to_owned()
                    )
                );
            }
        });
    assert_eq!(
        endpoint.bodies.lock().unwrap().len(),
        1,
        "the second resolve reuses the cached token"
    );
}

#[test]
fn interpolation_errors_name_the_missing_variable() {
    let error = crate::auth::interpolate_env(&serde_json::json!({
        "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "${SUSPECT_ABSENT_VAR_9183}"}}}
    }))
    .expect_err("the variable is not set");
    assert!(
        error.contains("SUSPECT_ABSENT_VAR_9183"),
        "the error names the variable: {error}"
    );
    assert!(
        error.contains("not set in the environment"),
        "the error says what happened: {error}"
    );
}

#[test]
fn interpolation_rejects_unterminated_references() {
    let error = crate::auth::interpolate_env(&serde_json::json!("${never-closed"))
        .expect_err("no closing brace");
    assert!(
        error.contains("unterminated"),
        "the error says the reference never closed: {error}"
    );
}

#[test]
fn interpolation_traverses_nested_structures_and_passes_non_strings_through() {
    // `${SUSPECT_ABSENT_VAR_9184}` inside a nested array still resolves to a
    // named error; a number survives untouched.
    let error = crate::auth::interpolate_env(&serde_json::json!({
        "schemes": {"x": {"y": ["${SUSPECT_ABSENT_VAR_9184}", 7, true]}}
    }))
    .expect_err("the variable is not set");
    assert!(
        error.contains("SUSPECT_ABSENT_VAR_9184"),
        "nested strings interpolate: {error}"
    );
    assert_eq!(
        crate::auth::interpolate_env(&serde_json::json!({"n": 7, "b": false})).unwrap(),
        serde_json::json!({"n": 7, "b": false}),
        "non-strings pass through unchanged"
    );
}

#[test]
fn credentials_file_round_trips_and_requires_an_auth_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("credentials.json");
    std::fs::write(
        &path,
        r#"{"auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "tok"}}}}"#,
    )
    .expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("tighten");
    }
    let config = crate::auth::load_credentials_file(&path).expect("loads");
    assert_eq!(
        config.schemes.len(),
        1,
        "the one configured scheme round-trips"
    );

    std::fs::write(&path, r#"{"nope": 1}"#).expect("write");
    let error = crate::auth::load_credentials_file(&path).expect_err("no auth section");
    assert!(
        error.contains("no `auth` section"),
        "the error says what is missing: {error}"
    );

    std::fs::write(&path, "{not json").expect("write");
    assert!(
        crate::auth::load_credentials_file(&path).is_err(),
        "invalid JSON is an error, not a silent empty config"
    );
}

#[test]
fn discover_credentials_walks_up_to_the_nearest_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("a/.suspect")).expect("mkdir");
    std::fs::create_dir_all(root.join("a/b/c")).expect("mkdir");
    std::fs::write(
        root.join("a/.suspect/credentials.json"),
        r#"{"auth": {"schemes": {}}}"#,
    )
    .expect("write");
    let found = crate::auth::discover_credentials(&root.join("a/b/c")).expect("found");
    assert_eq!(found, root.join("a/.suspect/credentials.json"));
    assert!(
        crate::auth::discover_credentials(root).is_none(),
        "no file above the start directory means no credentials"
    );
}

#[test]
fn injection_fills_empty_headers_but_never_overrides_values() {
    let mut request = HttpRequest {
        method: "GET".to_owned(),
        url: "http://api.example.com/models".to_owned(),
        headers: vec![
            ("X-Plex-Token".to_owned(), String::new()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ],
        body: Bytes::new(),
    };
    crate::auth::inject(
        &mut request,
        &[crate::auth::Injected::Header(
            "x-plex-token".to_owned(),
            "tok-123".to_owned(),
        )],
    );
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(k, _)| k == "X-Plex-Token")
            .map(|(_, v)| v.clone()),
        Some("tok-123".to_owned()),
        "a header left empty by a missing input reference is filled by the credential"
    );
    crate::auth::inject(
        &mut request,
        &[crate::auth::Injected::Header(
            "x-plex-token".to_owned(),
            "other".to_owned(),
        )],
    );
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(k, _)| k == "X-Plex-Token")
            .map(|(_, v)| v.clone()),
        Some("tok-123".to_owned()),
        "a real value always wins over injection"
    );
    crate::auth::inject(
        &mut request,
        &[crate::auth::Injected::Header(
            "x-api-key".to_owned(),
            "k".to_owned(),
        )],
    );
    assert!(
        request.headers.iter().any(|(k, _)| k == "x-api-key"),
        "absent headers are added"
    );
}

/// Caller-provided input overrides beat the workflow's static
/// `parameters`, and reach the wire.
#[tokio::test(flavor = "multi_thread")]
async fn input_overrides_beat_workflow_parameters() {
    use crate::exec::run_plan_full;

    let plan = compile_fixture();

    // Baseline: the static parameter `userName: world` reaches the wire,
    // the canned DELETE /users/world matches, all three steps pass.
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan_full(
        &plan,
        "http://api.test",
        &canned_fixture_http(),
        &crate::auth::AuthState::default(),
        &crate::auth::AuthConfig::default(),
        &serde_json::Map::new(),
        None,
        tx,
    )
    .await;
    drop(drain(rx).await);
    assert_eq!(summary.failed, 0, "static parameter satisfies the route");

    // Override: `userName=override-user` must win over the static
    // `world`, sending DELETE /users/override-user — which the canned
    // transport names in its unmatched-request error.
    let mut overrides = serde_json::Map::new();
    overrides.insert(
        "userName".to_owned(),
        serde_json::Value::String("override-user".to_owned()),
    );
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan_full(
        &plan,
        "http://api.test",
        &canned_fixture_http(),
        &crate::auth::AuthState::default(),
        &crate::auth::AuthConfig::default(),
        &overrides,
        None,
        tx,
    )
    .await;
    let events = drain(rx).await;
    assert_eq!(summary.failed, 1, "the overridden request misses the route");
    assert!(
        events
            .iter()
            .any(|e| format!("{e:?}").contains("override-user")),
        "the override reached the wire: {events:?}"
    );
}

/// A success criterion may compare a response field against an earlier
/// step's output (`$steps.<id>.outputs.<name>`) or an input
/// (`$inputs.<name>`) — resolved at run time, not treated as a literal.
#[tokio::test(flavor = "multi_thread")]
async fn criteria_reference_step_outputs_and_inputs() {
    use crate::exec::run_plan_full;

    const ARAZZO_REFS: &str = r#"
arazzo: 1.0.0
info:
  title: refs
  version: "1.0"
sourceDescriptions:
  - name: petstore
    url: spec.yaml
workflows:
  - workflowId: ref-check
    steps:
      - stepId: create-pet
        operationId: createPet
        requestBody:
          name: Rex
        successCriteria:
          - condition: '{$statusCode} == 201'
        outputs:
          petId: $response.body#/id
      - stepId: verify-roundtrip
        operationPath: 'GET /pets/{petId}'
        parameters:
          - name: petId
            in: path
            value: $steps.create-pet.outputs.petId
        successCriteria:
          - condition: '$response.body#/id == $steps.create-pet.outputs.petId'
          - condition: '$statusCode == $inputs.expectedStatus'
"#;

    let ws = workspace();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("refs.arazzo.yaml"), ARAZZO_REFS).expect("write arazzo");
    let arazzo = LowDoc::parse(
        format!("file://{}", dir.path().join("refs.arazzo.yaml").display()).into(),
        Source::from_vec(ARAZZO_REFS.as_bytes().to_vec()),
    );
    let plan = compile_plan(&arazzo, &ws).expect("compiles");

    let http = CannedTransport::new()
        .route(
            Match {
                method: Some("POST".to_owned()),
                path_suffix: "/pets".to_owned(),
            },
            HttpResponse {
                status: 201,
                headers: Vec::new(),
                body: Bytes::from(r#"{"id":"7","name":"Rex"}"#.to_owned()),
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
                body: Bytes::from(r#"{"id":"7","name":"Rex"}"#.to_owned()),
            },
        );

    let mut inputs = serde_json::Map::new();
    inputs.insert(
        "expectedStatus".to_owned(),
        serde_json::Value::Number(200.into()),
    );
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan_full(
        &plan,
        "http://api.test",
        &http,
        &crate::auth::AuthState::default(),
        &crate::auth::AuthConfig::default(),
        &inputs,
        None,
        tx,
    )
    .await;
    drop(drain(rx).await);
    assert_eq!(
        summary.passed, 2,
        "step-output and input references resolve at run time"
    );
    assert_eq!(summary.failed, 0);

    // And the input reference is live: a wrong expectation fails the
    // same criterion that just passed.
    let mut wrong = serde_json::Map::new();
    wrong.insert(
        "expectedStatus".to_owned(),
        serde_json::Value::Number(418.into()),
    );
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan_full(
        &plan,
        "http://api.test",
        &http,
        &crate::auth::AuthState::default(),
        &crate::auth::AuthConfig::default(),
        &wrong,
        None,
        tx,
    )
    .await;
    drop(drain(rx).await);
    assert_eq!(summary.failed, 1, "the input value reached the criterion");
}

/// Arazzo success criteria support ordered comparisons
/// (`>`, `>=`, `<`, `<=`) over body fields; the classic
/// `$response.body#/MediaContainer/size > 0` pattern.
#[test]
fn ordered_comparisons_compile_and_classify() {
    use crate::plan::parse_condition;

    let crit = parse_condition("$response.body#/MediaContainer/size > 0").unwrap();
    assert!(matches!(
        crit,
        crate::plan::CriterionKind::Compare {
            comparison: crate::plan::Comparison::Greater,
            bound: serde_json::Value::Number(_),
            ..
        }
    ));

    let crit = parse_condition("$response.body#/count >= 2").unwrap();
    assert!(matches!(
        crit,
        crate::plan::CriterionKind::Compare {
            comparison: crate::plan::Comparison::GreaterOrEqual,
            ..
        }
    ));

    // `>=` wins over `>` at the same position; `>` alone still parses.
    let crit = parse_condition("$response.body#/x > 5").unwrap();
    assert!(matches!(
        crit,
        crate::plan::CriterionKind::Compare {
            comparison: crate::plan::Comparison::Greater,
            ..
        }
    ));

    // Operators inside the pointer fragment never split.
    let crit = parse_condition("$response.body#/a>b == 1").unwrap();
    assert!(matches!(
        &crit,
        crate::plan::CriterionKind::Equals { pointer, .. } if pointer.as_deref() == Some("/a>b")
    ));
}

// ---------------------------------------------------------------------------
// Executor seams (observer, owned runs, inputs, dependsOn, think, errors)
// ---------------------------------------------------------------------------

use crate::exec::{
    ErrorClass, ExecObserver, NullObserver, RunOpts, StepCx, StepSample, StepVerdict, ThinkPolicy,
    TransportError, VuId, WfOutcome, run_plan_observed, run_workflow_owned,
};
use crate::plan::ThinkSpec;
use crate::transports::DelayTransport;
use std::sync::Mutex;
use std::time::Duration;

/// Golden for the classic event stream: the exact sequence of event kinds a
/// single-workflow run produces. Guards the `EventObserver` refactor —
/// editors parse this stream.
#[tokio::test(flavor = "multi_thread")]
async fn event_stream_order_is_stable() {
    let mut plan = compile_fixture();
    plan.workflows
        .retain(|wf| wf.workflow_id == "create-and-fetch");
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let _ = run_plan(&plan, "http://api.test", &canned_fixture_http(), tx).await;
    let kinds: Vec<&str> = drain(rx)
        .await
        .iter()
        .map(|ev| match ev {
            TestEvent::WfStarted { .. } => "wf_started",
            TestEvent::StepStarted { .. } => "step_started",
            TestEvent::RequestSent { .. } => "request_sent",
            TestEvent::ResponseGot { .. } => "response_got",
            TestEvent::ResponseValidated { .. } => "response_validated",
            TestEvent::CriterionOk { .. } => "criterion_ok",
            TestEvent::CriterionFail { .. } => "criterion_fail",
            TestEvent::OutputSet { .. } => "output_set",
            TestEvent::WfDone { .. } => "wf_done",
            TestEvent::RunDone { .. } => "run_done",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "wf_started",
            "step_started",
            "request_sent",
            "response_got",
            "criterion_ok",
            "output_set",
            "step_started",
            "request_sent",
            "response_got",
            "criterion_ok",
            "criterion_ok",
            "wf_done",
            "run_done",
        ]
    );
}

/// Observer that records step samples and workflow outcomes only.
#[derive(Default)]
struct Recorder {
    samples: Mutex<Vec<(String, VuId, StepSample)>>,
    done: Mutex<Vec<(String, VuId, WfOutcome)>>,
}

#[async_trait::async_trait]
impl ExecObserver for Recorder {
    async fn on_step_done(&self, cx: &StepCx<'_>, sample: &StepSample) {
        self.samples
            .lock()
            .unwrap()
            .push((cx.step.to_owned(), cx.vu, sample.clone()));
    }
    async fn on_workflow_done(&self, wf: &crate::plan::WfPlan, vu: VuId, outcome: &WfOutcome) {
        self.done
            .lock()
            .unwrap()
            .push((wf.workflow_id.clone(), vu, outcome.clone()));
    }
}

/// Owned runs are `'static`: many virtual users spawn over one shared plan,
/// each with its own inputs, and the observer sees per-VU samples.
#[tokio::test(flavor = "multi_thread")]
async fn owned_workflow_runs_spawn_with_per_vu_inputs() {
    let plan = Arc::new(compile_fixture());
    let wf_index = plan
        .workflows
        .iter()
        .position(|wf| wf.workflow_id == "cleanup-user")
        .unwrap();
    let http: Arc<dyn HttpClient> = Arc::new(
        CannedTransport::new()
            .route(
                Match {
                    method: Some("DELETE".to_owned()),
                    path_suffix: "/users/alice".to_owned(),
                },
                HttpResponse {
                    status: 204,
                    ..Default::default()
                },
            )
            .route(
                Match {
                    method: Some("DELETE".to_owned()),
                    path_suffix: "/users/bob".to_owned(),
                },
                HttpResponse {
                    status: 204,
                    ..Default::default()
                },
            ),
    );
    let recorder = Arc::new(Recorder::default());
    let observer: Arc<dyn ExecObserver> = recorder.clone();

    let mut set = tokio::task::JoinSet::new();
    for (vu, name) in [(1u64, "alice"), (2u64, "bob"), (3u64, "carol")] {
        let mut opts = RunOpts::new("http://api.test/");
        opts.vu = VuId(vu);
        // Overrides the Arazzo `parameters` value (`world`).
        opts.inputs
            .insert("userName".to_owned(), serde_json::json!(name));
        set.spawn(run_workflow_owned(
            Arc::clone(&plan),
            wf_index,
            opts,
            Arc::clone(&http),
            Arc::clone(&observer),
        ));
    }
    let mut outcomes = Vec::new();
    while let Some(res) = set.join_next().await {
        outcomes.push(res.unwrap());
    }
    assert_eq!(outcomes.iter().filter(|o| o.ok).count(), 2);
    assert_eq!(
        outcomes.iter().filter(|o| !o.ok).count(),
        1,
        "carol has no route"
    );

    let samples = recorder.samples.lock().unwrap();
    assert_eq!(samples.len(), 3);
    let carol = samples.iter().find(|(_, vu, _)| *vu == VuId(3)).unwrap();
    assert_eq!(carol.2.status, None);
    assert!(matches!(
        carol.2.verdict,
        StepVerdict::Transport(ErrorClass::Other)
    ));
    assert!(
        samples
            .iter()
            .filter(|(_, vu, _)| *vu != VuId(3))
            .all(|(_, _, s)| s.ok() && s.status == Some(204))
    );
}

pub(crate) const DEPENDS_ARAZZO: &str = r#"
arazzo: 1.0.0
info: { title: deps, version: "1" }
sourceDescriptions:
  - name: petstore
    url: spec.yaml
workflows:
  - workflowId: use-pet
    dependsOn: [seed]
    steps:
      - stepId: fetch
        operationId: showPetById
        parameters:
          - name: petId
            in: path
            value: $workflows.seed.outputs.createdId
          - name: via
            in: query
            value: $workflows.seed.steps.make.outputs.petId
  - workflowId: seed
    steps:
      - stepId: make
        operationId: createPet
        requestBody: { name: Rex }
        successCriteria:
          - condition: '{$statusCode} == 201'
        outputs:
          petId: $response.body#/id
    outputs:
      createdId: $steps.make.outputs.petId
"#;

/// A cross-document `dependsOn` (`$sourceDescriptions.<name>.<id>`) is a
/// valid Arazzo reference this plan cannot schedule; it must not fail
/// compilation, while an unknown local id still does.
#[test]
fn cross_document_depends_on_compiles_and_unknown_local_fails() {
    let ws = workspace();
    let compile = |depends: &str| {
        let text = format!(
            "arazzo: 1.0.0\ninfo: {{ title: x, version: '1' }}\nsourceDescriptions:\n  - {{ name: petstore, url: spec.yaml }}\n  - {{ name: other, url: other.arazzo.yaml, type: arazzo }}\nworkflows:\n  - workflowId: w\n    dependsOn: [{depends}]\n    steps:\n      - {{ stepId: s, operationId: listPets }}\n"
        );
        let doc = LowDoc::parse(
            "mem://cross.arazzo.yaml".into(),
            Source::from_vec(text.into_bytes()),
        );
        compile_plan(&doc, &ws)
    };
    let plan = compile("'$sourceDescriptions.other.setup'").expect("compiles");
    assert!(plan.workflows[0].depends_on.is_empty());
    let err = compile("ghost").expect_err("unknown local workflow");
    assert!(err.0.contains("ghost"), "{err}");
}

/// `dependsOn` schedules `seed` before `use-pet` even though it is declared
/// second, and both `$workflows.<id>.outputs.*` and
/// `$workflows.<id>.steps.<step>.outputs.*` resolve in the dependent.
#[tokio::test(flavor = "multi_thread")]
async fn depends_on_runs_prerequisites_and_exposes_outputs() {
    let ws = workspace();
    let doc = LowDoc::parse(
        "mem://deps.arazzo.yaml".into(),
        Source::from_vec(DEPENDS_ARAZZO.as_bytes().to_vec()),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");
    assert_eq!(
        crate::plan::dependency_waves(&plan.workflows).unwrap(),
        vec![vec![1], vec![0]]
    );

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan(&plan, "http://api.test", &canned_fixture_http(), tx).await;
    let events = drain(rx).await;
    assert_eq!((summary.passed, summary.failed), (2, 0), "{events:#?}");

    let started: Vec<&str> = events
        .iter()
        .filter_map(|ev| match ev {
            TestEvent::WfStarted { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec!["seed", "use-pet"]);
    assert!(events.iter().any(|ev| matches!(
        ev,
        TestEvent::RequestSent { url, .. } if url.ends_with("/pets/7?via=7")
    )));
}

#[test]
fn depends_on_rejects_unknown_targets_and_cycles() {
    let ws = workspace();
    let unknown = DEPENDS_ARAZZO.replace("dependsOn: [seed]", "dependsOn: [ghost]");
    let doc = LowDoc::parse(
        "mem://x.arazzo.yaml".into(),
        Source::from_vec(unknown.into_bytes()),
    );
    let err = compile_plan(&doc, &ws).expect_err("unknown dep");
    assert!(err.to_string().contains("ghost"), "{err}");

    let cyclic = DEPENDS_ARAZZO.replace(
        "  - workflowId: seed\n",
        "  - workflowId: seed\n    dependsOn: [use-pet]\n",
    );
    let doc = LowDoc::parse(
        "mem://y.arazzo.yaml".into(),
        Source::from_vec(cyclic.into_bytes()),
    );
    let err = compile_plan(&doc, &ws).expect_err("cycle");
    assert!(err.to_string().contains("cycle"), "{err}");
}

const THINK_ARAZZO: &str = r#"
arazzo: 1.0.0
info: { title: think, version: "1" }
sourceDescriptions:
  - name: petstore
    url: spec.yaml
workflows:
  - workflowId: slow
    steps:
      - stepId: first
        operationId: listPets
        x-suspect-think: 2s
      - stepId: second
        operationId: listPets
        x-suspect-think: 10s
"#;

/// Think time is compiled from `x-suspect-think`, ignored by default, and
/// honoured (after non-final steps only) under `ThinkPolicy::Honour`.
#[tokio::test(start_paused = true)]
async fn think_time_is_ignored_by_default_and_honoured_on_request() {
    let ws = workspace();
    let doc = LowDoc::parse(
        "mem://think.arazzo.yaml".into(),
        Source::from_vec(THINK_ARAZZO.as_bytes().to_vec()),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");
    assert_eq!(
        plan.workflows[0].steps[0].think,
        Some(ThinkSpec::fixed(Duration::from_secs(2)))
    );
    let http = CannedTransport::new().route(
        Match {
            method: None,
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 200,
            ..Default::default()
        },
    );

    let t0 = tokio::time::Instant::now();
    let summary = run_plan_observed(
        &plan,
        &RunOpts::new("http://api.test"),
        &http,
        &NullObserver,
    )
    .await;
    assert_eq!(summary.passed, 2);
    assert_eq!(t0.elapsed(), Duration::ZERO, "ignore policy never sleeps");

    let t1 = tokio::time::Instant::now();
    let mut honour = RunOpts::new("http://api.test");
    honour.think = ThinkPolicy::Honour { default: None };
    let summary = run_plan_observed(&plan, &honour, &http, &NullObserver).await;
    assert_eq!(summary.passed, 2);
    // Only the first step's 2s pause applies: the final step never thinks.
    assert_eq!(t1.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn delay_transport_sleeps_on_virtual_clock() {
    let inner = CannedTransport::new().route(
        Match {
            method: None,
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 200,
            ..Default::default()
        },
    );
    let delayed = DelayTransport::new(inner, Duration::from_millis(300));
    let t0 = tokio::time::Instant::now();
    let resp = delayed
        .execute(HttpRequest {
            method: "GET".into(),
            url: "http://x/pets".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(t0.elapsed(), Duration::from_millis(300));
    assert_eq!(delayed.calls(), 1);
}

#[test]
fn transport_errors_classify_by_prefix_then_heuristics() {
    assert_eq!(
        TransportError::classified(ErrorClass::Timeout, "deadline").class(),
        ErrorClass::Timeout
    );
    assert_eq!(
        TransportError("operation timed out".into()).class(),
        ErrorClass::Timeout
    );
    assert_eq!(
        TransportError("connection refused (os error 61)".into()).class(),
        ErrorClass::Connect
    );
    assert_eq!(
        TransportError("invalid peer certificate".into()).class(),
        ErrorClass::Tls
    );
    assert_eq!(
        TransportError("connection reset by peer".into()).class(),
        ErrorClass::Reset
    );
    assert_eq!(
        TransportError("no canned response for GET /x".into()).class(),
        ErrorClass::Other
    );
}

#[test]
fn rex_parses_workflow_output_expressions() {
    use suspect_rex::parse_rex;
    assert_eq!(
        parse_rex("$workflows.seed.outputs.token").unwrap(),
        Rex::Workflows {
            workflow: "seed".into(),
            step: None,
            outputs_key: "token".into()
        }
    );
    assert_eq!(
        parse_rex("$workflows.seed.steps.login.outputs.token").unwrap(),
        Rex::Workflows {
            workflow: "seed".into(),
            step: Some("login".into()),
            outputs_key: "token".into()
        }
    );
    assert!(parse_rex("$workflows.seed").is_err());
    assert!(parse_rex("$workflows.seed.outputs.").is_err());
    assert!(parse_rex("$workflows.seed.inputs.x").is_err());
}

const BODY_ARAZZO: &str = r#"
arazzo: 1.0.0
info: { title: body, version: "1" }
sourceDescriptions:
  - name: petstore
    url: spec.yaml
workflows:
  - workflowId: typed-body
    inputs:
      type: object
      properties:
        qty: { type: integer, default: 3 }
    parameters:
      - name: owner
        value: ada
    steps:
      - stepId: create
        operationId: createPet
        requestBody:
          contentType: application/vnd.pet+json
          payload:
            name: "pet of {$inputs.owner}"
            qty: $inputs.qty
            tags: ["${$inputs.owner}", literal]
            note: "price is $5"
        successCriteria:
          - condition: '{$statusCode} == 201'
"#;

/// Spec-form `requestBody.payload` is unwrapped, `contentType` is sent,
/// whole-leaf expressions keep their JSON type and embedded expressions
/// (both `{$x}` and `${$x}` spellings) render as text. A lone `$` that is
/// not an expression is left alone.
#[tokio::test(flavor = "multi_thread")]
async fn request_body_payload_resolves_runtime_expressions() {
    let ws = workspace();
    let doc = LowDoc::parse(
        "mem://body.arazzo.yaml".into(),
        Source::from_vec(BODY_ARAZZO.as_bytes().to_vec()),
    );
    let plan = compile_plan(&doc, &ws).expect("compiles");
    assert_eq!(
        plan.workflows[0].steps[0].request_content_type.as_deref(),
        Some("application/vnd.pet+json")
    );

    struct Capture(Mutex<Vec<HttpRequest>>);
    #[async_trait::async_trait]
    impl HttpClient for Capture {
        async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.0.lock().unwrap().push(req);
            Ok(HttpResponse {
                status: 201,
                ..Default::default()
            })
        }
    }
    let http = Capture(Mutex::new(Vec::new()));
    let mut opts = RunOpts::new("http://api.test");
    opts.inputs.insert("qty".into(), serde_json::json!(9));
    let summary = run_plan_observed(&plan, &opts, &http, &NullObserver).await;
    assert_eq!(summary.passed, 1);

    let reqs = http.0.lock().unwrap();
    let req = &reqs[0];
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/vnd.pet+json")
    );
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "name": "pet of ada",
            "qty": 9,
            "tags": ["ada", "literal"],
            "note": "price is $5"
        })
    );
}

// ------------------------------- step dependsOn: qualified and cross-workflow

/// Compiles an inline Arazzo 1.1 document against the petstore fixture.
fn compile_text(text: &str) -> Result<crate::plan::Plan, CompileError> {
    let doc = LowDoc::parse(
        "mem://steps.arazzo.yaml".into(),
        Source::from_vec(text.as_bytes().to_vec()),
    );
    compile_plan(&doc, &workspace())
}

/// Step `b` of workflow `w` depends on `a` through the spec's qualified
/// form; `c` is independent and declared last.
const QUALIFIED_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Qualified, version: '1'}
sourceDescriptions:
  - {name: petstore, url: spec.yaml, type: openapi}
workflows:
  - workflowId: w
    steps:
      - stepId: a
        operationId: createPet
        requestBody: { name: Rex }
      - stepId: b
        operationId: listPets
        dependsOn: ['$workflows.w.steps.a']
      - stepId: c
        operationId: listPets
"#;

fn started_steps(events: &[TestEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            TestEvent::StepStarted { step, .. } => Some(step.as_str()),
            _ => None,
        })
        .collect()
}

/// `$workflows.<this>.steps.<id>` is the current workflow's step `<id>`:
/// `b` runs right after `a`, in document order, and the run passes.
/// Before the fix the entry stayed unresolved, so `b` never ran: `c` ran
/// first, then `b` was reported blocked and counted *skipped*, leaving
/// `0 failed` and a green exit code.
#[tokio::test(flavor = "multi_thread")]
async fn qualified_same_workflow_depends_on_runs_the_step() {
    let plan = compile_text(QUALIFIED_ARAZZO).expect("compiles");
    let b = &plan.workflows[0].steps[1];
    assert_eq!(b.depends_on, vec!["a".to_owned()]);
    assert!(b.workflow_depends_on.is_empty());
    assert!(plan.workflows[0].depends_on.is_empty());

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan(&plan, "http://api.test", &OrderRecorder::default(), tx).await;
    let events = drain(rx).await;
    assert_eq!(
        (summary.passed, summary.failed, summary.skipped),
        (3, 0, 0),
        "{events:#?}"
    );
    assert_eq!(started_steps(&events), vec!["a", "b", "c"]);
}

/// `fetch` (declared in the first workflow) waits on a step of `seed`.
const CROSS_STEP_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Cross, version: '1'}
sourceDescriptions:
  - {name: petstore, url: spec.yaml, type: openapi}
workflows:
  - workflowId: use-pet
    steps:
      - stepId: fetch
        operationId: showPetById
        dependsOn: ['$workflows.seed.steps.make']
        parameters:
          - name: petId
            in: path
            value: $workflows.seed.steps.make.outputs.petId
  - workflowId: seed
    steps:
      - stepId: make
        operationId: createPet
        requestBody: { name: Rex }
        successCriteria:
          - condition: '{$statusCode} == 201'
        outputs:
          petId: $response.body#/id
"#;

/// Arazzo 1.1 Step Object: a step of a different workflow is referenced as
/// `$workflows.<workflowId>.steps.<stepId>` and MUST complete first. The
/// plan orders `seed` before `use-pet` (implied workflow dependency) and
/// the dependent step reads the prerequisite step's outputs.
#[tokio::test(flavor = "multi_thread")]
async fn cross_workflow_step_depends_on_orders_the_workflows() {
    let plan = compile_text(CROSS_STEP_ARAZZO).expect("compiles");
    assert_eq!(plan.workflows[0].depends_on, vec!["seed".to_owned()]);
    assert_eq!(
        plan.workflows[0].steps[0].workflow_depends_on,
        vec![crate::plan::WorkflowStepRef {
            workflow: "seed".to_owned(),
            step: "make".to_owned(),
        }]
    );
    assert!(plan.workflows[0].steps[0].depends_on.is_empty());
    assert_eq!(
        crate::plan::dependency_waves(&plan.workflows).unwrap(),
        vec![vec![1], vec![0]]
    );

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan(&plan, "http://api.test", &canned_fixture_http(), tx).await;
    let events = drain(rx).await;
    assert_eq!((summary.passed, summary.failed), (2, 0), "{events:#?}");
    assert_eq!(started_steps(&events), vec!["make", "fetch"]);
    assert!(events.iter().any(|event| matches!(
        event,
        TestEvent::RequestSent { url, .. } if url.ends_with("/pets/7")
    )));
}

/// When the prerequisite step fails, or its workflow is not part of the
/// run (`suspect test --workflow use-pet`), the dependent step can never
/// become ready: it fails with a diagnosis naming the missing step instead
/// of being skipped silently or hanging.
#[tokio::test(flavor = "multi_thread")]
async fn unmet_cross_workflow_step_dependency_fails_the_run() {
    let assert_blocked = |summary: &RunSummary, events: &[TestEvent]| {
        assert!(summary.failed >= 1, "{summary:?}");
        assert!(
            events.iter().any(|event| matches!(
                event,
                TestEvent::CriterionFail { step, crit, actual, .. }
                    if step == "fetch"
                        && crit == "depends-on"
                        && actual.contains("$workflows.seed.steps.make")
            )),
            "{events:#?}"
        );
        assert!(!started_steps(events).contains(&"fetch"));
    };

    // `make` answers 200 where its criterion wants 201.
    let plan = compile_text(CROSS_STEP_ARAZZO).expect("compiles");
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan(&plan, "http://api.test", &OrderRecorder::default(), tx).await;
    let events = drain(rx).await;
    assert_eq!((summary.passed, summary.failed), (0, 2), "{events:#?}");
    assert_blocked(&summary, &events);

    // The prerequisite workflow was filtered out of the run.
    let mut plan = compile_text(CROSS_STEP_ARAZZO).expect("compiles");
    plan.workflows.retain(|wf| wf.workflow_id == "use-pet");
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = run_plan(&plan, "http://api.test", &canned_fixture_http(), tx).await;
    let events = drain(rx).await;
    assert_eq!((summary.passed, summary.failed), (0, 1), "{events:#?}");
    assert_blocked(&summary, &events);
}

/// A step that could never become ready is a compile error naming the
/// workflow, the step, and the offending entry, like an unknown
/// workflow-level `dependsOn`.
#[test]
fn unresolvable_step_dependencies_fail_compilation() {
    let with_dep = |dep: &str| {
        QUALIFIED_ARAZZO.replace(
            "dependsOn: ['$workflows.w.steps.a']",
            &format!("dependsOn: ['{dep}']"),
        )
    };
    for (dep, needle) in [
        ("ghost", "dependsOn 'ghost' matches no step of workflow 'w'"),
        (
            "$workflows.w.steps.ghost",
            "dependsOn '$workflows.w.steps.ghost' matches no step of workflow 'w'",
        ),
        (
            "$workflows.nowhere.steps.a",
            "dependsOn '$workflows.nowhere.steps.a' matches no workflow",
        ),
        ("$workflows.w.steps.b", "names the step itself"),
        ("b", "names the step itself"),
        ("$workflows.w.a", "is not a stepId"),
        ("$workflows.w.steps.a.outputs.x", "is not a stepId"),
    ] {
        let err = compile_text(&with_dep(dep)).expect_err(dep);
        assert!(
            err.0.starts_with("workflow 'w', step 'b': "),
            "{dep}: {err}"
        );
        assert!(err.0.contains(needle), "{dep}: {err}");
    }

    let unknown_foreign_step = CROSS_STEP_ARAZZO.replace(
        "dependsOn: ['$workflows.seed.steps.make']",
        "dependsOn: ['$workflows.seed.steps.ghost']",
    );
    let err = compile_text(&unknown_foreign_step).expect_err("unknown step");
    assert!(
        err.0
            .contains("'$workflows.seed.steps.ghost' matches no step of workflow 'seed'"),
        "{err}"
    );

    // An implicit dependency must resolve too.
    let implicit = QUALIFIED_ARAZZO.replace(
        "      - stepId: c\n        operationId: listPets\n",
        "      - stepId: c\n        operationId: listPets\n        parameters:\n          - {name: q, in: query, value: $steps.ghost.outputs.x}\n",
    );
    let err = compile_text(&implicit).expect_err("implicit unknown");
    assert!(err.0.contains("step 'c': reads '$steps.ghost'"), "{err}");

    // A cross-workflow step dependency orders workflows, so it closes a
    // cycle with a workflow-level dependsOn pointing back.
    let cyclic = CROSS_STEP_ARAZZO.replace(
        "  - workflowId: seed\n",
        "  - workflowId: seed\n    dependsOn: [use-pet]\n",
    );
    let err = compile_text(&cyclic).expect_err("cycle");
    assert!(err.0.contains("cycle"), "{err}");
}

/// A step in another Arazzo document is a valid reference this plan never
/// executes; like a cross-document workflow `dependsOn`, it compiles and
/// does not block the step.
#[test]
fn cross_document_step_depends_on_compiles() {
    let text = QUALIFIED_ARAZZO
        .replace(
            "  - {name: petstore, url: spec.yaml, type: openapi}\n",
            "  - {name: petstore, url: spec.yaml, type: openapi}\n  - {name: other, url: other.arazzo.yaml, type: arazzo}\n",
        )
        .replace(
            "dependsOn: ['$workflows.w.steps.a']",
            "dependsOn: ['$sourceDescriptions.other.setup.steps.login', a]",
        );
    let plan = compile_text(&text).expect("compiles");
    let b = &plan.workflows[0].steps[1];
    assert_eq!(b.depends_on, vec!["a".to_owned()]);
    assert!(b.workflow_depends_on.is_empty());
}

/// `createPet` fails (500) on its first call and passes (201) after.
#[derive(Default)]
struct FailFirstCreate {
    creates: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl HttpClient for FailFirstCreate {
    async fn execute(
        &self,
        req: crate::exec::HttpRequest,
    ) -> Result<crate::exec::HttpResponse, crate::exec::TransportError> {
        let status = if req.method == "POST" {
            let prior = self
                .creates
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if prior == 0 { 500 } else { 201 }
        } else {
            200
        };
        Ok(crate::exec::HttpResponse {
            status,
            headers: Vec::new(),
            body: Bytes::from_static(b"{}"),
        })
    }
}

/// `make` fails once and recovers by jumping back to `prepare`.
const GOTO_RECOVERY_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Goto, version: '1'}
sourceDescriptions:
  - {name: petstore, url: spec.yaml, type: openapi}
workflows:
  - workflowId: w
    steps:
      - stepId: prepare
        operationId: listPets
      - stepId: check
        operationId: listPets
        dependsOn: [prepare]
      - stepId: make
        operationId: createPet
        requestBody: { name: Rex }
        successCriteria:
          - condition: '{$statusCode} == 201'
        onFailure:
          - {name: recover, type: goto, stepId: prepare}
"#;

/// Arazzo `goto` jumps to the target step and execution continues from
/// there in order: `prepare` and its dependent `check` re-run against the
/// fresh state, then the failed `make` is retried. Before the fix the
/// re-run invalidated nothing (`check` kept its stale completion) and the
/// failed step was dropped, so the recovery path never retried it.
#[tokio::test(flavor = "multi_thread")]
async fn on_failure_goto_reruns_from_the_target_step() {
    let plan = compile_text(GOTO_RECOVERY_ARAZZO).expect("compiles");
    assert_eq!(
        plan.workflows[0].steps[2].failure_goto.as_deref(),
        Some("prepare")
    );

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_plan(&plan, "http://api.test", &FailFirstCreate::default(), tx),
    )
    .await
    .expect("the run terminates");
    let events = drain(rx).await;
    assert_eq!(
        started_steps(&events),
        vec!["prepare", "check", "make", "prepare", "check", "make"],
        "{events:#?}"
    );
    // The first `make` failure stays on the record; the re-runs replace,
    // rather than add to, the earlier passes.
    assert_eq!((summary.passed, summary.failed), (3, 1), "{events:#?}");
}

/// `make` always fails and its `onFailure` jumps back to itself.
const GOTO_LOOP_ARAZZO: &str = r#"
arazzo: 1.1.0
info: {title: Loop, version: '1'}
sourceDescriptions:
  - {name: petstore, url: spec.yaml, type: openapi}
workflows:
  - workflowId: w
    steps:
      - stepId: make
        operationId: createPet
        requestBody: { name: Rex }
        successCriteria:
          - condition: '{$statusCode} == 201'
        onFailure:
          - {name: again, type: goto, stepId: make}
"#;

/// A `goto` cycle that never recovers ends after a bounded number of jumps
/// with a diagnosis. Before the fix it re-sent the request forever.
#[tokio::test(flavor = "multi_thread")]
async fn on_failure_goto_loops_are_bounded() {
    let plan = compile_text(GOTO_LOOP_ARAZZO).expect("compiles");
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    let drained = tokio::spawn(drain(rx));
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_plan(&plan, "http://api.test", &OrderRecorder::default(), tx),
    )
    .await
    .expect("a goto loop terminates");
    let events = drained.await.expect("drain");
    assert_eq!(
        started_steps(&events).len(),
        crate::exec::MAX_GOTO_JUMPS + 1,
        "one initial attempt plus one per allowed jump"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            TestEvent::CriterionFail { crit, .. } if crit == "on-failure-goto"
        )),
        "{events:#?}"
    );
    assert!(summary.failed >= 1);
}
