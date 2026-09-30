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
                    success: vec![CriterionPlan {
                        kind: CriterionKind::StatusInRange(2, 2),
                        range: 0..0,
                    }],
                    outputs: Vec::new(),
                    body_pointers: Vec::new(),
                    failure_goto: None,
                    timeout_ms: None,
                    message: None,
                    security: Vec::new(),
                    response_schemas: Vec::new(),
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
                    timeout_ms: None,
                    message: None,
                    security: Vec::new(),
                    response_schemas: Vec::new(),
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
