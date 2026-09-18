//! Emitted-only incoming receipt helpers for the native Dart HTTP backend: the
//! generated `lib/src/incoming.dart` part with the typed
//! `IncomingException extends SdkException`, the per-receipt payload aliases,
//! frozen route constants, decoders and reply constructors, and byte-identity
//! for contracts without incoming declarations. Static runtime files are never
//! modified; the receipt decode lives entirely in the generated package. No
//! Dart toolchain is required: the behavioral checks are static assertions
//! plus a documented manual verification of the emitted part (see
//! `incoming_part_is_static_and_receipt_less_packages_are_unchanged`).

#![cfg(feature = "dart-sdk")]

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.incoming.test/dart-incoming.json";

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

/// One webhook with a required header and required JSON body plus a declared
/// 204 reply with a required reply header, one webhook with no request body
/// and a declared JSON reply with an optional reply header, and one
/// operation-attached callback receipt with a runtime expression route. The
/// same shape the shared planner and the other backends pin.
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
                "responses": {"200": {"description": "ok"}},
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
                    "responses": {"204": {"description": "Accepted", "headers": {
                        "x-request-id": {"required": true, "schema": {"type": "string"}}
                    }}}
                }
            },
            "ping": {
                "post": {
                    "operationId": "onPing",
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}, "headers": {
                        "x-trace-id": {"schema": {"type": "string"}}
                    }}}
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
                "responses": {"200": {"description": "ok"}}
            }}
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::DartHttp,
        package_name: "incoming_sdk".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn source(files: &[OutFile], path: &str) -> String {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} missing"))
        .content
        .clone()
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate(incoming_document());
    let part = source(&files, "dart/lib/src/incoming.dart");
    for expected in [
        // The typed failure extends the package's SdkException hierarchy and
        // never carries received payload text.
        "final class IncomingException extends SdkException {",
        "const IncomingException(this.message, {this.codecFailure});",
        "final CodecException? codecFailure;",
        "String toString() => 'IncomingException: $message';",
        // The shared case-insensitive header read and presence check.
        "String? _incomingHeaderValue(Map<String, String> headers, String name) {",
        "void _incomingRequireHeader(Map<String, String> headers, String name) {",
        // The webhook payload binds its declared model through the package's
        // own compiled codec.
        "typedef NewIssuePayload = IssueEvent;",
        // The frozen route constant: method/route/expression.
        "const ({String method, String route, bool expression}) newIssueWebhookRoute = (method: \"POST\", route: \"newIssue\", expression: false);",
        // The decoder presence-checks the declared required header, refuses a
        // missing required body and decodes through the compiled codec.
        "NewIssuePayload decodeNewIssueWebhook(Map<String, String> headers, Uint8List body) {",
        "_incomingRequireHeader(headers, \"x-signature\");",
        "if (body.isEmpty) {",
        "throw const IncomingException('the declared required receipt body is absent');",
        "return _incomingDecoded(() => webhooksNewIssuePostBodyCodec.decodeBytes(body));",
        // The declared 204 reply: pinned status, no body, required reply header.
        "({int status, Map<String, String> headers, Uint8List body}) constructNewIssueResponse({Map<String, String>? typedHeaders}) {",
        "throw IncomingException('the declared required reply header $name is absent');",
        "return (status: 204, headers: headers, body: Uint8List(0));",
        // A receipt with no request body decodes nothing and stays typed void.
        "typedef PingPayload = void;",
        "const ({String method, String route, bool expression}) pingWebhookRoute = (method: \"POST\", route: \"ping\", expression: false);",
        "PingPayload decodePingWebhook(Map<String, String> headers, Uint8List body) {",
        "// The receipt declares no request body; only the declared headers are checked.",
        // The declared 200 reply encodes through the response codec, with the
        // optional declared reply header applied from the optional argument.
        "({int status, Map<String, String> headers, Uint8List body}) constructPingResponse(Pong result, {Map<String, String>? typedHeaders}) {",
        "for (final name in const [\"x-trace-id\"]) {",
        "return (status: 200, headers: headers, body: webhooksPingPostResponse200Codec.encodeBytes(result));",
        // The callback receipt carries its runtime expression verbatim, with
        // the Dart string escape keeping the `$` literal.
        "typedef SubscribeOnEventPayload = Event;",
        "const ({String method, String route, bool expression}) subscribeOnEventWebhookRoute = (method: \"POST\", route: \"{\\$request.body#/callbackUrl}\", expression: true);",
        "SubscribeOnEventPayload decodeSubscribeOnEventWebhook(Map<String, String> headers, Uint8List body) {",
        "return _incomingDecoded(() => subscribeCallbacksOnEventRequestBodyCallbackUrlPostBodyCodec.decodeBytes(body));",
        "({int status, Map<String, String> headers, Uint8List body}) constructSubscribeOnEventResponse() {",
    ] {
        assert!(
            part.contains(expected),
            "incoming.dart lacks {expected}\n--- emitted: ---\n{part}"
        );
    }

    // The library registers the part, and the client surface stays untouched:
    // no receipt member leaks into the generated Client.
    let library = source(&files, "dart/lib/incoming_sdk.dart");
    assert!(library.contains("part 'src/incoming.dart';"), "{library}");
    let client = source(&files, "dart/lib/src/client.dart");
    assert!(!client.contains("decodeNewIssueWebhook"));
    assert!(!client.contains("IncomingException"));
    assert!(part.contains("part of '../incoming_sdk.dart';"));
}

#[test]
fn receipt_less_documents_keep_the_file_list_identical() {
    let configured = generate(incoming_document());
    let control = generate(control_document());

    // Exactly one new file: the incoming receipt helpers.
    assert!(
        !control
            .iter()
            .any(|file| file.path == "dart/lib/src/incoming.dart")
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the declared receipts add exactly one file"
    );
    let mut configured_paths = configured
        .iter()
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    let mut control_paths = control.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    configured_paths.sort();
    control_paths.sort();
    let added = configured_paths
        .iter()
        .filter(|path| !control_paths.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(added, vec!["dart/lib/src/incoming.dart".to_owned()]);

    // The generated library gains exactly the part directive.
    let control_library = source(&control, "dart/lib/incoming_sdk.dart");
    assert_eq!(
        source(&configured, "dart/lib/incoming_sdk.dart"),
        format!("{control_library}part 'src/incoming.dart';\n"),
        "the library may only gain the incoming part directive"
    );
    // The Client is untouched: receipt helpers are top-level library members.
    assert_eq!(
        source(&configured, "dart/lib/src/client.dart"),
        source(&control, "dart/lib/src/client.dart"),
        "the generated client may not change for receipts"
    );

    // No other file carries incoming bytes in the receipt-less control.
    for file in &control {
        for fragment in ["IncomingException", "WebhookRoute", "decodeNewIssueWebhook"] {
            assert!(
                !file.content.contains(fragment),
                "{} carries incoming bytes",
                file.path
            );
        }
    }
}

#[test]
fn plan_carries_the_compiled_incoming_receipts() {
    use suspect_codegen::dart_sdk;
    let contract = contract_with_document(incoming_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk(contract, &selected, dart_sdk::DartConfig::default()).unwrap();
    let incoming = plan.incoming();
    assert_eq!(incoming.operations().len(), 3);
    let receipts = plan.incoming_receipts();
    assert_eq!(receipts.exception_type, "IncomingException");
    assert_eq!(receipts.receipts.len(), 3);
    let new_issue = receipts
        .receipts
        .iter()
        .find(|receipt| receipt.name == "newIssue")
        .expect("the declared webhook receipt");
    assert_eq!(new_issue.kind, "webhook");
    assert_eq!(new_issue.method, "POST");
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.decode, "decodeNewIssueWebhook");
    assert_eq!(
        new_issue.construct.as_deref(),
        Some("constructNewIssueResponse")
    );
    assert_eq!(new_issue.payload_type, "NewIssuePayload");
    assert_eq!(new_issue.payload_annotation, "IssueEvent");
    assert_eq!(new_issue.route_const, "newIssueWebhookRoute");
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    let response = new_issue.response.as_ref().expect("the declared 204 reply");
    assert_eq!(response.status, 204);
    assert!(response.body.is_none());
    assert_eq!(response.required_headers, vec!["x-request-id".to_owned()]);
    let ping = receipts
        .receipts
        .iter()
        .find(|receipt| receipt.name == "ping")
        .expect("the body-less webhook receipt");
    assert_eq!(ping.payload, dart_sdk::incoming::Payload::None);
    assert!(!ping.required_body);
    assert!(ping.required_headers.is_empty());
    let callback = receipts
        .receipts
        .iter()
        .find(|receipt| receipt.name == "subscribe.onEvent")
        .expect("the declared callback receipt");
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);
    assert!(
        callback
            .explanations
            .iter()
            .any(|explanation| explanation.contains("runtime expression")),
        "the plan must explain that substitution is a runtime/framework concern: {:?}",
        callback.explanations
    );

    // The control plans to an empty result and reserves no receipt names.
    let control_contract = contract_with_document(control_document());
    let control_selected = control_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = dart_sdk::plan_sdk(
        control_contract,
        &control_selected,
        dart_sdk::DartConfig::default(),
    )
    .unwrap();
    assert!(control.incoming().is_empty());
    assert!(control.incoming_receipts().receipts.is_empty());
}

#[test]
fn unrepresentable_receipts_refuse_the_plan_instead_of_skipping() {
    let mut document = incoming_document();
    document["webhooks"]["formHook"] = json!({"post": {
        "operationId": "onForm",
        "requestBody": {"required": true, "content": {"application/x-www-form-urlencoded": {
            "schema": {"type": "object", "additionalProperties": false, "properties": {"name": {"type": "string"}}, "required": ["name"]}
        }}},
        "responses": {"204": {"description": "Accepted"}}
    }});
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let error = suspect_codegen::dart_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::dart_sdk::DartConfig::default(),
    )
    .expect_err("a form receipt has no v1 decode");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "sdk-incoming-receipt-unrepresentable"),
        "expected the shared receipt refusal: {error:?}"
    );
}

/// With no installed Dart toolchain this test degrades to strict static
/// assertions plus a documented manual verification of the emitted part:
///
/// 1. The receipt helpers are pure library-level functions over plain
///    `Map<String, String>` headers and `Uint8List` body carriers. The part
///    declares no transport, no HTTP client and no runtime plumbing of its
///    own: a framework routes the provider's call to `decode*Webhook`, and
///    `construct*Response` returns the exact `{status, headers, body}` record
///    the handler answers with.
/// 2. Decoders presence-check only the declared required headers
///    (case-insensitively), refuse a missing declared required body before
///    any decoding, and decode JSON strictly through the same compiled
///    `ModelCodec.decodeBytes` the client's own responses use; codec and
///    JSON-parse failures surface as the typed IncomingException with the
///    compiled `codecFailure` retained, never the received payload text.
/// 3. Constructors build exactly the first declared exact 2xx reply: the
///    status is pinned, the body encodes through the response codec's
///    `encodeBytes` (which revalidates the constructed value) or is empty for
///    a body-less reply, and declared reply headers are applied from the
///    optional `typedHeaders` argument with required-presence checks only.
/// 4. Route constants are frozen generated data: the provider's verb, the
///    declared route verbatim (runtime expressions unsubstituted) and the
///    expression flag. The runtime never parses OpenAPI.
/// 5. Receipt-less contracts emit no part and no bytes: the generated
///    library, client and every other file keep their exact pre-incoming
///    shape, and a broken incoming declaration refuses the whole plan with
///    source-linked diagnostics instead of skipping the receipt.
#[test]
fn incoming_part_is_static_and_receipt_less_packages_are_unchanged() {
    let files = generate(incoming_document());
    let part = source(&files, "dart/lib/src/incoming.dart");
    // The receipt decode never invents a request path of its own: the part
    // contains no transport plumbing and only reads its declared arguments.
    for fragment in ["HttpTransport", "HttpClient", "_exchange(", "IoTransport"] {
        assert!(
            !part.contains(fragment),
            "incoming emission must stay framework-routed, not transport-coupled: {fragment}"
        );
    }
    assert!(part.contains("part of '../incoming_sdk.dart';"));
    // Manifest stays honest: the receipt helpers are compiled source
    // semantics, not inferred wire behavior.
    let manifest: Value = serde_json::from_str(&source(&files, "dart/sdk-manifest.json")).unwrap();
    assert_eq!(manifest["inferred_pagination"], false);
    assert_eq!(manifest["automatic_retries"], false);
}
