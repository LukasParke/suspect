//! Emitted-only incoming receipt helpers for the Go HTTP backend: the
//! `go/incoming.go` decoders, reply constructors and compiled receipt
//! descriptors, generation-time emission plus native behavior over fake
//! webhook deliveries. Contracts without incoming declarations emit nothing at
//! all, and every other emitted file keeps its receipt-less shape.

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.incoming.test/go-incoming.json";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

/// One webhook with a required header and required JSON body plus a 204 reply,
/// one webhook with a JSON reply, and one operation-attached callback receipt
/// with a runtime expression route. The same shape the shared planner and the
/// TypeScript/Python backends pin.
fn incoming_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
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

/// The control document: no webhooks and no callback. Nothing new may be
/// emitted for it.
fn control_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"]
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}}
            }}
        }
    })
}

/// The control document with an empty webhooks map: still receipt-less.
fn emptied_document() -> Value {
    let mut document = control_document();
    document["webhooks"] = json!({});
    document
}

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::GoHttp,
            package_name: "example.com/incoming-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn generate(document: Value) -> std::collections::BTreeMap<String, String> {
    generate_document(document)
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let configured = generate(incoming_document());
    let control = generate(control_document());
    let emptied = generate(emptied_document());

    // Exactly one new file: the incoming receipt helpers.
    let incoming = configured
        .get("go/incoming.go")
        .expect("the incoming helpers are emitted for a webhook/callback contract");
    assert!(!control.contains_key("go/incoming.go"));
    assert!(
        configured.len() == control.len() + 1,
        "the declared receipts add exactly one file ({} vs {})",
        configured.len(),
        control.len()
    );
    // The receipt-less documents agree with each other on every file and emit
    // no incoming helpers anywhere.
    for (path, content) in &control {
        assert!(
            !content.contains("DecodeNewIssueWebhook") && !content.contains("IncomingDescriptors"),
            "{path} leaked incoming helpers"
        );
    }
    for (path, content) in &emptied {
        assert!(
            !content.contains("DecodeNewIssueWebhook") && !content.contains("IncomingDescriptors"),
            "{path} leaked incoming helpers"
        );
    }
    assert!(!control.keys().any(|path| path.contains("incoming")));
    assert!(!emptied.keys().any(|path| path.contains("incoming")));
    assert_eq!(
        control.keys().collect::<Vec<_>>(),
        emptied.keys().collect::<Vec<_>>(),
        "receipt-less documents must agree on the emitted file list"
    );

    // The shared receipt library: the route type, the compiled descriptor
    // types and the branded failure helper.
    for expected in [
        "type IncomingRoute struct",
        "Method string",
        "Path string",
        "Expression bool",
        "type IncomingDescriptor struct",
        "Kind string",
        "RequiredHeaders []string",
        "Payload string",
        "ReplyStatus int",
        "func httpIncomingError(at HTTPSource, cause error) *SDKError",
        "func httpIncomingHeaderValue(headers map[string]string, name string) (string, bool)",
        "func httpIncomingRequireHeaders(headers map[string]string, required []string, at HTTPSource) error",
        "func httpIncomingBodyText(body []byte, at HTTPSource) error",
    ] {
        assert!(
            incoming.contains(expected),
            "incoming.go lacks {expected}\n{incoming}"
        );
    }
    // The newIssue webhook receipt: typed payload, route, source locator,
    // header presence check and codec decode.
    for expected in [
        "type NewIssuePayload = OnNewIssueRequest",
        "var NewIssueWebhookRoute = IncomingRoute{Method: \"POST\", Path: \"newIssue\", Expression: false}",
        "var incomingNewIssueSource = HTTPSource{Document: \"https://source.incoming.test/go-incoming.json\", Pointer: \"/webhooks/newIssue/post\"}",
        "func DecodeNewIssueWebhook(headers map[string]string, body []byte) (*NewIssuePayload, error) {",
        "httpIncomingRequireHeaders(headers, []string{\"x-signature\"}, incomingNewIssueSource)",
        "if len(body) == 0 {",
        "value, err := httpJSONParse(body, 8388608)",
        "model, err := Codecs.OnNewIssueRequest.DecodeValue(value)",
        "return &model, nil",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "func ConstructNewIssueResponse() (int, map[string]string, []byte, error) {",
        "return 204, map[string]string{}, nil, nil",
        // The declared 200 reply encodes through the response codec.
        "func ConstructPingResponse(result OnPingResponse200) (int, map[string]string, []byte, error) {",
        "data, err := Codecs.OnPingResponse200.Encode(result)",
        "return 200, map[string]string{}, data, nil",
        // The body-less receipt payload returns nil.
        "type PingPayload = struct{}",
        // The callback receipt carries its runtime expression verbatim.
        "var SubscribeOnEventWebhookRoute = IncomingRoute{Method: \"POST\", Path: \"{$request.body#/callbackUrl}\", Expression: true}",
        "func DecodeSubscribeOnEventWebhook(headers map[string]string, body []byte) (*SubscribeOnEventPayload, error) {",
        // The compiled descriptors as generated data.
        "var IncomingDescriptors = map[string]IncomingDescriptor{",
        "\"newIssue\": {",
        "Kind: \"webhook\"",
        "Payload: \"json\",",
        "RequiredHeaders: []string{\"x-signature\"}",
        "Reply: \"none\",",
        "ReplyStatus: 204",
        "\"subscribe.onEvent\": {",
        "Kind: \"callback\"",
        "Route: \"{$request.body#/callbackUrl}\",",
        "Expression: true,",
    ] {
        assert!(
            incoming.contains(expected),
            "incoming.go lacks {expected}\n{incoming}"
        );
    }
}

#[test]
fn plan_carries_the_incoming_receipts_only_when_emission_happens() {
    let contract = contract_with_document(incoming_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(contract, &selected, go_http::HttpConfig::default()).unwrap();
    assert!(!plan.incoming().is_empty());
    let receipts = plan.incoming_receipts();
    assert_eq!(receipts.len(), 3);
    let new_issue = receipts
        .iter()
        .find(|receipt| receipt.name == "newIssue")
        .expect("the declared webhook compiles");
    assert_eq!(new_issue.kind, "webhook");
    assert_eq!(new_issue.method, "POST");
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.payload_type, "NewIssuePayload");
    assert_eq!(new_issue.route_const, "NewIssueWebhookRoute");
    assert_eq!(new_issue.decode, "DecodeNewIssueWebhook");
    assert_eq!(
        new_issue.construct.as_deref(),
        Some("ConstructNewIssueResponse")
    );
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    assert_eq!(
        new_issue.payload,
        go_http::incoming::Payload::Json {
            codec: "OnNewIssueRequest".into()
        }
    );
    let response = new_issue.response.as_ref().expect("the 204 reply compiles");
    assert_eq!(response.status, 204);
    assert!(response.body.is_none());
    assert!(response.headers.is_empty());
    let ping = receipts
        .iter()
        .find(|receipt| receipt.name == "ping")
        .expect("the ping webhook compiles");
    assert!(!ping.required_body);
    assert!(ping.required_headers.is_empty());
    let reply = ping.response.as_ref().expect("the 200 reply compiles");
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body,
        Some(go_http::incoming::ConstructedBody::Json {
            codec: "OnPingResponse200".into()
        })
    );
    let callback = receipts
        .iter()
        .find(|receipt| receipt.name == "subscribe.onEvent")
        .expect("the declared callback compiles under its parent operation");
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);
    assert!(callback.payload_type.starts_with("SubscribeOnEvent"));

    let control_contract = contract_with_document(control_document());
    let control_selected = control_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = go_http::plan_http(
        control_contract,
        &control_selected,
        go_http::HttpConfig::default(),
    )
    .unwrap();
    assert!(control.incoming().is_empty());
    assert!(control.incoming_receipts().is_empty());
}

fn go_toolchain() -> Option<String> {
    // The manifest carries the version command, regex, minimum, and install
    // guidance; the skip message below quotes it verbatim.
    let (status, _) = suspect_codegen::toolchain::probe_status("go");
    match status {
        suspect_codegen::toolchain::ToolStatus::Available { version } => Some(version),
        _ => None,
    }
}

#[test]
fn native_module_builds_with_the_incoming_file() {
    let Some(_) = go_toolchain() else {
        eprintln!(
            "go_incoming: degrading to static assertions; {}",
            suspect_codegen::toolchain::guidance("go")
        );
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate_document(incoming_document()), root.path()).unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn fake_webhook_deliveries_drive_the_decoder_and_constructor() {
    let Some(version) = go_toolchain() else {
        eprintln!(
            "go_incoming: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    eprintln!("go_incoming: {version}");
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate_document(incoming_document()), root.path()).unwrap();
    std::fs::write(root.path().join("go/incoming_behavior_test.go"), BEHAVIOR).unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "."])
        .current_dir(root.path().join("go"))
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native incoming behavior failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_incoming: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}

const BEHAVIOR: &str = r#"package sdk_test

import (
	"errors"
	"testing"

	sdk "example.com/incoming-sdk"
)

// A valid fake webhook delivery decodes into the declared model, regardless of
// header-name casing.
func TestValidDeliveryDecodesIntoTheDeclaredModel(t *testing.T) {
	payload, err := sdk.DecodeNewIssueWebhook(map[string]string{"X-Signature": "abc"}, []byte(`{"id":"i1","title":"t"}`))
	if err != nil {
		t.Fatal(err)
	}
	if payload.Id != "i1" || payload.Title != "t" {
		t.Fatal("payload not decoded to the declared model", payload)
	}
}

// A missing required header is the branded incoming-request failure.
func TestMissingRequiredHeaderIsATypedFailure(t *testing.T) {
	_, err := sdk.DecodeNewIssueWebhook(map[string]string{}, []byte(`{"id":"i1","title":"t"}`))
	var failure *sdk.SDKError
	if !errors.As(err, &failure) {
		t.Fatal("not a typed SDKError", err)
	}
	if failure.Kind != "incoming-request" {
		t.Fatal("unexpected failure kind", failure.Kind)
	}
}

// An invalid payload is a branded failure too.
func TestInvalidPayloadIsATypedFailure(t *testing.T) {
	_, err := sdk.DecodeNewIssueWebhook(map[string]string{"x-signature": "abc"}, []byte(`{"id":"i1"}`))
	var failure *sdk.SDKError
	if !errors.As(err, &failure) {
		t.Fatal("not a typed SDKError", err)
	}
	if failure.Kind != "incoming-request" {
		t.Fatal("unexpected failure kind", failure.Kind)
	}
	if failure.Source.Pointer != "/webhooks/newIssue/post" {
		t.Fatal("the failure lost its source", failure.Source)
	}
}

// A required declared body refuses an empty delivery.
func TestEmptyRequiredBodyIsATypedFailure(t *testing.T) {
	_, err := sdk.DecodeNewIssueWebhook(map[string]string{"x-signature": "abc"}, nil)
	var failure *sdk.SDKError
	if !errors.As(err, &failure) || failure.Kind != "incoming-request" {
		t.Fatal("the empty required body must fail", err)
	}
}

// The declared 204 reply constructs with the pinned status and no body.
func TestBodyLessReplyConstructs(t *testing.T) {
	status, headers, body, err := sdk.ConstructNewIssueResponse()
	if err != nil {
		t.Fatal(err)
	}
	if status != 204 || len(headers) != 0 || body != nil {
		t.Fatal("unexpected declared reply", status, headers, body)
	}
}

// The declared 200 reply encodes its body through the response codec.
func TestJsonReplyConstructsThroughTheResponseCodec(t *testing.T) {
	pong, err := sdk.Codecs.OnPingResponse200.Decode([]byte(`{"pong":true}`))
	if err != nil {
		t.Fatal(err)
	}
	status, headers, body, err := sdk.ConstructPingResponse(pong)
	if err != nil {
		t.Fatal(err)
	}
	if status != 200 {
		t.Fatal("unexpected declared status", status)
	}
	encoded, err := sdk.Codecs.OnPingResponse200.Encode(pong)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != string(encoded) {
		t.Fatal("the reply body is not the codec encoding", string(body))
	}
	if len(headers) != 0 {
		t.Fatal("unexpected declared headers", headers)
	}
}

// The callback receipt decodes through its own declared schema and carries its
// route expression verbatim.
func TestCallbackReceiptDecodesAndCarriesItsExpressionRoute(t *testing.T) {
	event, err := sdk.DecodeSubscribeOnEventWebhook(nil, []byte(`{"kind":"open"}`))
	if err != nil {
		t.Fatal(err)
	}
	if event.Kind != "open" {
		t.Fatal("callback payload not decoded", event)
	}
	descriptor, ok := sdk.IncomingDescriptors["subscribe.onEvent"]
	if !ok {
		t.Fatal("callback descriptor missing", sdk.IncomingDescriptors)
	}
	if descriptor.Route != "{$request.body#/callbackUrl}" || !descriptor.Expression {
		t.Fatal("callback route lost", descriptor.Route, descriptor.Expression)
	}
	if descriptor.Payload != "json" {
		t.Fatal("unexpected payload representation", descriptor.Payload)
	}
	reply, ok := sdk.IncomingDescriptors["newIssue"]
	if !ok || reply.ReplyStatus != 204 || reply.Reply != "none" {
		t.Fatal("webhook reply descriptor lost", reply)
	}
	route := sdk.SubscribeOnEventWebhookRoute
	if route.Method != "POST" || route.Expression != true {
		t.Fatal("route constant lost", route)
	}
}
"#;
