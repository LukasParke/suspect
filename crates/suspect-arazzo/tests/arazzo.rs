use suspect_arazzo::{
    ArazzoDoc, ComponentKind, Expr, ExprPart, HttpPart, RuntimeContext, SourceType, parse,
    parse_embedded, render_embedded, validate_arazzo,
};
use suspect_low::{LowDoc, SpecFamily};
use suspect_source::Source;

fn parse_doc(src: &str) -> LowDoc {
    LowDoc::parse(
        "mem://arazzo.yaml".into(),
        Source::from_vec(src.as_bytes().to_vec()),
    )
}

fn parse_yaml(src: &str) -> LowDoc {
    parse_doc(src)
}

const DOC: &str = r#"
arazzo: '1.0.1'
info:
  title: Pet walking
  summary: Walk the pets
sourceDescriptions:
  - name: petApi
    url: ./openapi.yaml
    type: openapi
workflows:
  - workflowId: walk-pet
    summary: Walk one pet
    parameters:
      - name: petId
        in: path
        value: 42
    steps:
      - stepId: fetch-pet
        operationId: getPet
        parameters:
          - name: petId
            in: path
            target: $url
            value: 42
        successCriteria:
          - condition: $statusCode == 200
        onSuccess:
          - name: goNext
            type: goto
            workflowId: walk-all
            stepId: prep
          - name: endIt
            type: end
      - stepId: report
        operationPath: $sourceDescriptions.petApi./pets/{petId}
        outputs:
          report: $response.body#/report
    outputs:
      walked: $workflows.walk-pet.steps.report.outputs.report
  - workflowId: walk-all
    steps:
      - stepId: prep
        operationId: listPets
"#;

#[test]
fn document_model_parses() {
    let doc = parse_doc(DOC);
    assert_eq!(doc.sniff_family(), SpecFamily::Arazzo10);
    let arazzo = ArazzoDoc::new(&doc);
    assert_eq!(arazzo.version(), Some("1.0.1"));
    assert_eq!(arazzo.info_title(), Some("Pet walking"));
    assert_eq!(arazzo.info_summary(), Some("Walk the pets"));
    assert_eq!(arazzo.source_descriptions().len(), 1);
    assert_eq!(arazzo.source_descriptions()[0].name, "petApi");
    assert_eq!(
        arazzo.source_descriptions()[0].kind,
        suspect_arazzo::SourceType::OpenApi
    );

    assert_eq!(arazzo.workflows().len(), 2);
    let wf = &arazzo.workflows()[0];
    assert_eq!(wf.workflow_id, "walk-pet");
    assert_eq!(wf.steps().len(), 2);
    assert_eq!(wf.steps()[0].step_id, "fetch-pet");
    assert_eq!(wf.steps()[0].operation_id(), Some("getPet"));
    assert_eq!(
        wf.steps()[1].operation_path(),
        Some("$sourceDescriptions.petApi./pets/{petId}")
    );
    // goto action carries workflow + step targets
    let on_success = wf.steps()[0].on_success();
    assert_eq!(on_success.len(), 2);
    assert_eq!(on_success[0].action_type(), Some("goto"));
    assert_eq!(on_success[0].workflow_id(), Some("walk-all"));
    assert_eq!(on_success[0].step_id(), Some("prep"));
}

#[test]
fn runtime_expression_grammar() {
    for (input, expected) in [
        ("$method", Expr::Method),
        ("$url", Expr::Url),
        ("$statusCode", Expr::StatusCode),
        (
            "$request.header.#X-Trace",
            Expr::Request {
                part: HttpPart::Header("X-Trace".into()),
            },
        ),
        (
            "$response.query.#page",
            Expr::Response {
                part: HttpPart::Query("page".into()),
            },
        ),
        (
            "$request.path.#petId",
            Expr::Request {
                part: HttpPart::Path("petId".into()),
            },
        ),
        (
            "$response.body",
            Expr::Response {
                part: HttpPart::Body(None),
            },
        ),
        (
            "$request.body#/user/id",
            Expr::Request {
                part: HttpPart::Body(Some(suspect_low::Pointer::parse("#/user/id").unwrap())),
            },
        ),
        (
            "$outputs.token",
            Expr::Outputs {
                name: "token".into(),
            },
        ),
        (
            "$inputs.limit",
            Expr::Inputs {
                name: "limit".into(),
            },
        ),
        (
            "$workflows.wf.steps.st.outputs.out",
            Expr::WorkflowOutput {
                workflow: "wf".into(),
                step: "st".into(),
                name: "out".into(),
            },
        ),
        (
            "$components.parameters.sharedKey",
            Expr::Component {
                kind: ComponentKind::Parameters,
                name: "sharedKey".into(),
            },
        ),
        (
            "$components.succeedOn.okCriteria",
            Expr::Component {
                kind: ComponentKind::SucceedOn,
                name: "okCriteria".into(),
            },
        ),
        (
            "$components.failureOn.badCriteria",
            Expr::Component {
                kind: ComponentKind::FailureOn,
                name: "badCriteria".into(),
            },
        ),
        (
            "$components.retryOn.retryCriteria",
            Expr::Component {
                kind: ComponentKind::RetryOn,
                name: "retryCriteria".into(),
            },
        ),
    ] {
        assert_eq!(parse(input).unwrap(), expected, "parsing {input}");
    }
}

#[test]
fn malformed_expressions_rejected() {
    for bad in [
        "$unknownRoot",
        "$request.body#/bad~pointer~9", // ~9 is not a valid escape
        "$workflows.wf.outputs.x",      // missing steps segment
        "$components.everything.x",     // unknown components kind
        "$statusCode extra",            // trailing garbage
        "statusCode",                   // missing $
    ] {
        assert!(parse(bad).is_err(), "{bad} must not parse");
    }
}

#[test]
fn embedded_expressions_split() {
    let parts = parse_embedded("created {$response.body#/id} at {$statusCode}!");
    assert_eq!(parts.len(), 5);
    assert_eq!(parts[0], ExprPart::Text("created ".into()));
    assert_eq!(
        parts[1],
        ExprPart::Expr(Expr::Response {
            part: HttpPart::Body(Some(suspect_low::Pointer::parse("#/id").unwrap()))
        })
    );
    assert_eq!(parts[2], ExprPart::Text(" at ".into()));
    assert_eq!(parts[4], ExprPart::Text("!".into()));

    // invalid brace content stays literal text
    let parts = parse_embedded("keep {$not valid} here");
    assert!(parts.iter().all(|p| matches!(p, ExprPart::Text(_))));
}

struct MockCtx<'d> {
    status: i64,
    req_id: Option<&'d str>,
    pet_id: Option<&'d str>,
    walked: Option<&'d str>,
    body: Option<&'d LowDoc>,
}

impl<'d> RuntimeContext<'d> for MockCtx<'d> {
    fn method(&self) -> Option<&str> {
        Some("GET")
    }
    fn url(&self) -> Option<&str> {
        Some("https://api.example.com/pets/42")
    }
    fn status_code(&self) -> Option<i64> {
        Some(self.status)
    }
    fn header(&self, response: bool, name: &str) -> Option<&'d str> {
        if response && name == "X-Req-Id" {
            self.req_id
        } else {
            None
        }
    }
    fn path_param(&self, _response: bool, name: &str) -> Option<&'d str> {
        if name == "petId" { self.pet_id } else { None }
    }
    fn body(&self, response: bool) -> Option<suspect_low::NodeRef<'d>> {
        match (self.body, response) {
            (Some(d), true) => Some(d.root()),
            _ => None,
        }
    }
    fn output(&self, name: &str) -> Option<&'d str> {
        if name == "walked" { self.walked } else { None }
    }
}

#[test]
fn evaluator_over_mock_context() {
    let payload = LowDoc::parse(
        "mem://body.json".into(),
        Source::from_vec(br#"{"id": "pet-7", "tags": ["a"]}"#.to_vec()),
    );
    let ctx = MockCtx {
        status: 201,
        req_id: Some("abc-123"),
        pet_id: Some("42"),
        walked: Some("yes"),
        body: Some(&payload),
    };

    let eval = |e| suspect_arazzo::evaluate(&e, &ctx);

    match eval(parse("$method").unwrap()).unwrap() {
        suspect_arazzo::Evaluated::Text(t) => assert_eq!(t, "GET"),
        other => panic!("expected text, got {other:?}"),
    }
    match eval(parse("$statusCode").unwrap()).unwrap() {
        suspect_arazzo::Evaluated::Text(t) => assert_eq!(t, "201"),
        other => panic!("expected text, got {other:?}"),
    }
    match eval(parse("$response.header.#X-Req-Id").unwrap()).unwrap() {
        suspect_arazzo::Evaluated::Text(t) => assert_eq!(t, "abc-123"),
        other => panic!("expected text, got {other:?}"),
    }
    match eval(parse("$response.body#/id").unwrap()).unwrap() {
        suspect_arazzo::Evaluated::Body(node) => {
            assert_eq!(node.as_str(), Some("pet-7"));
        }
        other => panic!("expected body node, got {other:?}"),
    }
    assert!(eval(parse("$outputs.nope").unwrap()).is_none());

    // unbraced $statusCode is literal text; braced expressions evaluate
    let rendered = render_embedded("status={$statusCode};id={$response.body#/id}", &ctx);
    assert_eq!(rendered, "status=201;id=pet-7");
}

#[test]
fn validation_catches_structural_problems() {
    let bad = parse_doc(
        r#"
arazzo: '1.0.1'
info: {}
sourceDescriptions:
  - name: api
    url: ./x.yaml
  - name: api
    url: ./y.yaml
workflows:
  - workflowId: a
    steps:
      - stepId: s1
        successCriteria:
          - condition: not-an-expression
        onFailure:
          - name: f
            type: goto
            workflowId: missing-workflow
      - stepId: s1
        operationId: someOp
  - workflowId: a
    steps:
      - stepId: no-op-step
"#,
    );
    let arazzo = ArazzoDoc::new(&bad);
    let diags = validate_arazzo(&arazzo);
    let codes: Vec<_> = diags.iter().map(|d| d.code).collect();
    assert!(codes.contains(&"arazzo-missing-info-title"), "{codes:?}");
    assert!(codes.contains(&"arazzo-duplicate-source-name"), "{codes:?}");
    assert!(codes.contains(&"arazzo-duplicate-workflow-id"), "{codes:?}");
    assert!(codes.contains(&"arazzo-duplicate-step-id"), "{codes:?}");
    assert!(codes.contains(&"arazzo-invalid-condition"), "{codes:?}");
    assert!(codes.contains(&"arazzo-goto-unknown-workflow"), "{codes:?}");
    assert!(
        codes.contains(&"arazzo-step-missing-operation"),
        "{codes:?}"
    );
}

#[test]
fn validation_passes_clean_document() {
    let doc = parse_doc(DOC);
    let arazzo = ArazzoDoc::new(&doc);
    let diags = validate_arazzo(&arazzo);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code != "arazzo-invalid-condition")
        .collect();
    assert!(errors.is_empty(), "clean doc produced: {errors:?}");
}

#[test]
fn output_cross_references_checked() {
    let bad = parse_doc(
        r#"
arazzo: '1.0.1'
info:
  title: t
sourceDescriptions:
  - name: api
    url: ./x.yaml
workflows:
  - workflowId: w1
    steps:
      - stepId: s1
        operationId: op
        outputs:
          real: $statusCode
  - workflowId: w2
    steps:
      - stepId: s2
        operationId: op2
        outputs:
          ghost: $workflows.w1.steps.s1.outputs.not-defined
          dead-wf: $workflows.nowhere.steps.s.outputs.x
"#,
    );
    let arazzo = ArazzoDoc::new(&bad);
    let codes: Vec<_> = validate_arazzo(&arazzo).iter().map(|d| d.code).collect();
    assert!(codes.contains(&"arazzo-output-unknown-name"), "{codes:?}");
    assert!(
        codes.contains(&"arazzo-output-unknown-workflow"),
        "{codes:?}"
    );
}

// ------------------------------------------------------------ Arazzo 1.1

const V11_HEADER: &str = "arazzo: 1.1.0\n$self: https://api.example.com/flows.arazzo.yaml\ninfo: {title: t, version: '1'}\nsourceDescriptions:\n  - {name: api, url: openapi.yaml, type: openapi}\n";

#[test]
fn arazzo_11_document_validates_clean() {
    let doc = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n        successCriteria:\n          - condition: $statusCode == 200\n      - stepId: b\n        operationId: getPet\n        dependsOn: [a]\n        timeout: 5000\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        codes.is_empty(),
        "a well-formed 1.1 document must validate clean: {codes:?}"
    );
    assert_eq!(
        parsed.self_uri(),
        Some("https://api.example.com/flows.arazzo.yaml")
    );
}

#[test]
fn arazzo_11_self_fragment_rejected() {
    let doc = parse_yaml(
        "arazzo: 1.1.0\n$self: https://api.example.com/flows.yaml#other\ninfo: {title: t, version: '1'}\nsourceDescriptions:\n  - {name: api, url: openapi.yaml, type: openapi}\nworkflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n",
    );
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(codes.contains(&"arazzo-self-fragment"), "{codes:?}");
}

#[test]
fn depends_on_unknown_or_later_steps_are_rejected() {
    let doc = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n        dependsOn: [ghost, b]\n      - stepId: b\n        operationId: getPet\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&"arazzo-depends-on-unknown-step"),
        "{codes:?}"
    );
}

#[test]
fn forward_output_reference_requires_depends_on() {
    // Arazzo 1.1 5.8.5.2.5: sequential execution cannot satisfy a forward
    // reference; dependsOn declares the join.
    let without = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n        parameters:\n          - {{name: q, in: query, value: '$steps.b.outputs.token'}}\n      - stepId: b\n        operationId: getPet\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&without);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&"arazzo-forward-output-reference"),
        "{codes:?}"
    );

    let with = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n        dependsOn: ['b']\n        parameters:\n          - {{name: q, in: query, value: '$steps.b.outputs.token'}}\n      - stepId: b\n        operationId: getPet\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&with);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        !codes.contains(&"arazzo-forward-output-reference"),
        "dependsOn legitimizes the join: {codes:?}"
    );
}

#[test]
fn selector_outputs_are_admission_diagnostics() {
    let doc = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    outputs:\n      picked:\n        type: jsonpath\n    steps:\n      - stepId: a\n        operationId: listPets\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&"arazzo-selector-output-unsupported"),
        "{codes:?}"
    );
}

#[test]
fn asyncapi_step_fields_are_validated() {
    let doc = parse_yaml(
        "arazzo: 1.1.0\ninfo: {title: t, version: '1'}\nsourceDescriptions:\n  - {name: bus, url: asyncapi.yaml, type: asyncapi}\n  - {name: api, url: openapi.yaml, type: openapi}\nworkflows:\n  - workflowId: w\n    steps:\n      - stepId: badSource\n        channelPath: '$sourceDescriptions.api.channels/orders'\n      - stepId: okSource\n        channelPath: '$sourceDescriptions.bus.channels/orders'\n        action: receive\n        correlationId: '$inputs.correlationId'\n      - stepId: httpWithAction\n        operationId: listPets\n        action: send\n",
    );
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&"arazzo-channel-on-non-asyncapi-source"),
        "{codes:?}"
    );
    assert!(
        codes.contains(&"arazzo-async-fields-on-http-step"),
        "{codes:?}"
    );
    assert_eq!(
        parsed
            .source_descriptions()
            .iter()
            .find(|s| s.name == "bus")
            .map(|s| s.kind),
        Some(SourceType::AsyncApi)
    );
}

#[test]
fn multiple_step_targets_and_bad_timeout_are_rejected() {
    let doc = parse_yaml(&format!(
        "{V11_HEADER}workflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n        operationPath: '$sourceDescriptions.api#/paths/~1pets/get'\n        timeout: -5\n"
    ));
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(codes.contains(&"arazzo-step-multiple-targets"), "{codes:?}");
    assert!(codes.contains(&"arazzo-invalid-timeout"), "{codes:?}");
}

#[test]
fn unknown_arazzo_major_is_rejected() {
    let doc = parse_yaml(
        "arazzo: 2.0.0\ninfo: {title: t, version: '1'}\nsourceDescriptions:\n  - {name: api, url: openapi.yaml}\nworkflows:\n  - workflowId: w\n    steps:\n      - stepId: a\n        operationId: listPets\n",
    );
    let parsed = suspect_arazzo::ArazzoDoc::new(&doc);
    let codes: Vec<&str> = validate_arazzo(&parsed).iter().map(|d| d.code).collect();
    assert!(codes.contains(&"arazzo-unsupported-version"), "{codes:?}");
}
