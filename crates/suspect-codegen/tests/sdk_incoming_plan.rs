//! The shared incoming plan: webhooks and callbacks compile from the whole
//! Contract independent of operation selection, carry their routes verbatim,
//! and refuse broken declarations instead of skipping them.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::http_protocol as protocol;
use suspect_codegen::http_protocol::{Capabilities, Capability, plan, plan_incoming};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "security": [{"apiKey": []}],
        "components": {
            "securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"]
                },
                "Event": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}},
                    "required": ["kind"]
                },
                "Pong": {
                    "type": "object",
                    "properties": {"pong": {"type": "boolean"}},
                    "required": ["pong"]
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}},
                "callbacks": {
                    "onEvent": {
                        "{$request.body#/callbackUrl}": {
                            "post": {
                                "operationId": "eventReceived",
                                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Event"}}}},
                                "responses": {"200": {"description": "ok"}}
                            }
                        }
                    }
                }
            }}
        },
        "webhooks": {
            "newIssue": {
                "post": {
                    "operationId": "onNewIssue",
                    "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                    "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                    "responses": {"204": {"description": "Accepted"}}
                }
            },
            "ping": {
                "post": {
                    "operationId": "onPing",
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
                }
            }
        }
    })
}

#[test]
fn webhooks_compile_with_declared_request_and_response_semantics() {
    let contract = contract_with_document(document());
    let incoming = plan_incoming(&contract).unwrap();
    let webhook = incoming
        .operations()
        .iter()
        .find(|operation| operation.name() == "newIssue")
        .expect("the declared webhook compiles");
    assert_eq!(webhook.kind(), protocol::IncomingKind::Webhook);
    assert_eq!(webhook.method().as_str(), "POST");
    // The webhook key is the receipt route and carries no runtime expression.
    assert_eq!(webhook.route().route(), "newIssue");
    assert!(!webhook.route().expression());
    // The provider sends the declared header; presence-checked by the helpers.
    let headers = webhook
        .request()
        .parameters()
        .iter()
        .filter(|parameter| {
            parameter.required() && parameter.location() == protocol::ParameterLocation::Header
        })
        .map(|parameter| parameter.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(headers, vec!["x-signature".to_owned()]);
    // The declared body compiles through the same media machinery, and its
    // schema becomes an actual codec root.
    let body = webhook
        .request()
        .body()
        .as_ref()
        .expect("declared request body");
    assert!(body.required());
    assert_eq!(body.media().len(), 1);
    let protocol::Representation::Json { codec: Some(codec) } = body.media()[0].representation()
    else {
        panic!("JSON body representation");
    };
    // The codec id is the $ref use-site, exactly like the outbound plan.
    assert_eq!(
        codec.schema().id().pointer(),
        "/webhooks/newIssue/post/requestBody/content/application~1json/schema"
    );
    assert!(
        incoming
            .codec_roots()
            .iter()
            .any(|root| root == codec.schema().id())
    );
    // The declared responses are what OUR handler returns; a 204 pins the
    // status with no body media.
    assert_eq!(webhook.responses().len(), 1);
    assert_eq!(webhook.responses()[0].status_key(), "204");
    assert_eq!(
        webhook.responses()[0].status(),
        protocol::ResponseStatus::Exact(204)
    );
    assert!(webhook.responses()[0].media().is_empty());
}

#[test]
fn callbacks_carry_runtime_expression_routes_verbatim() {
    let contract = contract_with_document(document());
    let incoming = plan_incoming(&contract).unwrap();
    let callback = incoming
        .operations()
        .iter()
        .find(|operation| operation.name() == "subscribe.onEvent")
        .expect("the declared callback compiles under its parent operation");
    assert_eq!(callback.kind(), protocol::IncomingKind::Callback);
    assert_eq!(callback.method().as_str(), "POST");
    assert_eq!(callback.route().route(), "{$request.body#/callbackUrl}");
    assert!(callback.route().expression());
    assert!(
        callback
            .explanations()
            .iter()
            .any(|explanation| explanation.contains("runtime expression")),
        "the plan must explain that substitution is a runtime/framework concern: {:?}",
        callback.explanations()
    );
    let body = callback
        .request()
        .body()
        .as_ref()
        .expect("declared request body");
    let protocol::Representation::Json { codec: Some(codec) } = body.media()[0].representation()
    else {
        panic!("JSON body representation");
    };
    assert_eq!(
        codec.schema().id().pointer(),
        "/paths/~1subscribe/post/callbacks/onEvent/{$request.body#~1callbackUrl}/post/requestBody/content/application~1json/schema"
    );
}

#[test]
fn webhooks_in_a_30_document_are_diagnosed_not_skipped() {
    let document = json!({
        "openapi": "3.0.3",
        "info": {"title": "Legacy", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "paths": {},
        "webhooks": {
            "newIssue": {"post": {
                "operationId": "onNewIssue",
                "responses": {"204": {"description": "Accepted"}}
            }}
        }
    });
    let contract = contract_with_document(document);
    let error = plan_incoming(&contract).expect_err("3.0 webhooks are refused");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "sdk-incoming-version"),
        "expected an sdk-incoming-version diagnostic: {error:?}"
    );
}

#[test]
fn callbacks_compile_even_when_their_operation_is_unselected() {
    let contract = contract_with_document(document());
    // Select only the unrelated /widgets operation; /subscribe owns the
    // callback and is deliberately not selected.
    let selected: Vec<_> = contract
        .operations()
        .filter(|operation| {
            operation
                .operation_id()
                .is_some_and(|id| id == "listWidgets")
        })
        .map(|operation| operation.source().clone())
        .collect();
    assert_eq!(selected.len(), 1);
    let wire = plan(
        &contract,
        &selected,
        Capabilities::for_adapter("incoming-plan-test", [Capability::UnnamedOperations]),
    )
    .into_result()
    .expect("the selected operation plans");
    assert_eq!(wire.operations().len(), 1);
    // The callback still compiles: selection has no say over incoming receipts.
    let incoming = plan_incoming(&contract).unwrap();
    assert!(
        incoming
            .operations()
            .iter()
            .any(|operation| operation.name() == "subscribe.onEvent")
    );
    assert!(
        incoming
            .operations()
            .iter()
            .any(|operation| operation.name() == "newIssue")
    );
}

#[test]
fn broken_incoming_declarations_refuse_the_plan_instead_of_skipping() {
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "Broken", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "webhooks": {
            "newIssue": {"post": {
                "operationId": "onNewIssue",
                "requestBody": {"required": true, "content": {}},
                "responses": {"204": {"description": "Accepted"}}
            }}
        }
    });
    let contract = contract_with_document(document);
    let error = plan_incoming(&contract).expect_err("a broken body declaration refuses");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "http-content-empty"),
        "expected the shared content refusal: {error:?}"
    );
}

#[test]
fn a_contract_without_incoming_declarations_plans_to_an_empty_result() {
    let mut document = document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    // The callback lives under /subscribe; remove the whole operation too.
    document["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok"}}
    }});
    let contract = contract_with_document(document);
    let incoming = plan_incoming(&contract).unwrap();
    assert!(incoming.is_empty());
    assert!(incoming.codec_roots().is_empty());
}
