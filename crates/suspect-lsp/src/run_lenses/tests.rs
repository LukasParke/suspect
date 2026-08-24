//! Tests for run lenses and the workflow-run core against a canned
//! transport.

use super::*;
use suspect_low::LowDoc;
use suspect_source::Source;
use suspect_test::transports::{CannedTransport, Match};
use suspect_test::{HttpResponse, OpKey};

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
    steps:
      - stepId: create-pet
        operationId: createPet
        successCriteria:
          - condition: '{$statusCode} == 201'
  - workflowId: cleanup-user
    steps:
      - stepId: delete-user
        successCriteria:
          - condition: '{$statusCode} == 204'
"#;

fn arazzo_doc() -> LowDoc {
    LowDoc::parse(
        "mem://flow.arazzo.yaml".into(),
        Source::from_vec(ARAZZO.as_bytes().to_vec()),
    )
}

#[test]
fn one_run_lens_per_workflow_with_command_and_args() {
    let doc = arazzo_doc();
    let lenses = run_lenses(&doc);
    assert_eq!(lenses.len(), 2);

    assert_eq!(
        lenses[0].command.as_ref().unwrap().title,
        "▶ Run create-and-fetch"
    );
    let cmd = lenses[0].command.as_ref().unwrap();
    assert_eq!(cmd.command, RUN_WORKFLOW_COMMAND);
    let args = cmd.arguments.as_ref().unwrap();
    assert_eq!(args[0], serde_json::json!("mem://flow.arazzo.yaml"));
    assert_eq!(args[1], serde_json::json!("create-and-fetch"));

    // Anchored at the workflowId key line of each workflow.
    assert!(lenses[0].range.start.line < lenses[1].range.start.line);

    // Non-Arazzo documents get no run lenses.
    let oas = LowDoc::parse(
        "mem://spec.yaml".into(),
        Source::from_vec(b"openapi: 3.1.0\ninfo: {title: t, version: '1'}\n".to_vec()),
    );
    assert!(run_lenses(&oas).is_empty());
}

/// A single-step workflow whose criterion range points at the fixture's
/// condition string (`condition: '{$statusCode} == 201'`).
fn failing_plan(range: std::ops::Range<usize>) -> WfPlan {
    WfPlan {
        workflow_id: "create-and-fetch".to_owned(),
        inputs: Default::default(),
        steps: vec![suspect_test::StepPlan {
            step_id: "create-pet".to_owned(),
            operation: OpKey {
                method: suspect_ir::Method::Post,
                path: "/pets".to_owned(),
            },
            parameters: Vec::new(),
            request_body: None,
            success: vec![suspect_test::CriterionPlan {
                kind: suspect_test::CriterionKind::Equals {
                    pointer: None,
                    expected: serde_json::json!(201),
                },
                range,
            }],
            outputs: Vec::new(),
            body_pointers: Vec::new(),
        }],
    }
}

#[tokio::test]
async fn core_reports_failure_anchored_at_recorded_range() {
    let recorded = 210..236;
    let wf = failing_plan(recorded.clone());
    let http = CannedTransport::new().route(
        Match {
            method: None,
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 500,
            headers: Vec::new(),
            body: bytes::Bytes::from_static(b"boom"),
        },
    );

    let (summary, failures) = run_workflow_core(&wf, "http://api.test", &http, None).await;
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.failed, 1);
    assert_eq!(failures.len(), 1);
    let f = &failures[0];
    assert_eq!(f.step_id, "create-pet");
    assert_eq!(f.crit, "statusCode == 201");

    // The diagnostic anchors at the criterion's recorded source range.
    let doc = arazzo_doc();
    let diags = failures_to_diagnostics(
        &wf,
        &failures,
        doc.inner().bytes(),
        doc.inner().line_index(),
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].source.as_deref(), Some("suspect-test"));
    assert_eq!(
        diags[0].message,
        format!("expected {}, got {}", f.expected, f.actual)
    );
    let li = doc.inner().line_index();
    let want_start = super::super::state::lsp_range(doc.inner().bytes(), li, recorded).start;
    assert_eq!(diags[0].range.start, want_start);
}

#[tokio::test]
async fn core_passing_run_yields_no_diagnostics() {
    let wf = failing_plan(10..30);
    let http = CannedTransport::new().route(
        Match {
            method: Some("POST".to_owned()),
            path_suffix: "/pets".to_owned(),
        },
        HttpResponse {
            status: 201,
            headers: Vec::new(),
            body: bytes::Bytes::from_static(b"{\"id\":7}"),
        },
    );

    // Events mirror through to a second channel so progress streaming is
    // exercised too.
    let (mirror, mut mirror_rx) = tokio::sync::mpsc::channel::<TestEvent>(64);
    let drainer = tokio::spawn(async move {
        let mut seen = Vec::new();
        while let Some(ev) = mirror_rx.recv().await {
            seen.push(ev);
        }
        seen
    });
    let (summary, failures) = run_workflow_core(&wf, "http://api.test", &http, Some(&mirror)).await;
    drop(mirror);
    let mirrored = drainer.await.unwrap();

    assert_eq!(summary.failed, 0);
    assert_eq!(summary.passed, 1);
    assert!(failures.is_empty());

    let doc = arazzo_doc();
    let diags = failures_to_diagnostics(
        &wf,
        &failures,
        doc.inner().bytes(),
        doc.inner().line_index(),
    );
    assert!(diags.is_empty());
    assert!(
        mirrored
            .iter()
            .any(|ev| matches!(ev, TestEvent::RunDone { .. }))
    );
}

#[test]
fn failure_range_falls_back_for_transport_errors() {
    let wf = failing_plan(5..9);
    let f = CriterionFailure {
        step_id: "create-pet".to_owned(),
        crit: "transport".to_owned(),
        expected: "an HTTP response".to_owned(),
        actual: "connection refused".to_owned(),
    };
    assert_eq!(failure_range(&wf, &f), 0..0);
}

#[test]
fn base_url_precedence() {
    assert_eq!(
        base_url_from_options(Some(&serde_json::json!({
            "suspect": { "run": { "baseUrl": "http://init:1" } }
        }))),
        "http://init:1"
    );
    assert_eq!(
        base_url_from_options(Some(&serde_json::json!({ "baseUrl": "http://top:2" }))),
        "http://top:2"
    );
    // SAFETY: as above.
    unsafe { std::env::set_var("SUSPECT_BASE_URL", "http://env:3") };
    assert_eq!(base_url_from_options(None), "http://env:3");
    // SAFETY: as above.
    unsafe { std::env::remove_var("SUSPECT_BASE_URL") };
    assert_eq!(base_url_from_options(None), "http://localhost:8080");
}
