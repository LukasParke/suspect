//! Emitted-only incoming receipt helpers for the Swift package:
//! `Sources/<module>/Incoming.swift`, strict native compilation, and native
//! behavior over the generated decoders and reply constructors. Plans without
//! incoming declarations emit no file at all, so receipt-less output stays
//! byte-identical.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    swift_sdk,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// Two webhooks (a signed delivery with a declared 204 reply and a ping with
/// a declared JSON reply plus an optional reply header) and one callback with
/// a runtime-expression route.
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
                    "properties": {"pong": {"type": "string", "maxLength": 3}},
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
                    "responses": {"204": {"description": "Accepted"}}
                }
            },
            "ping": {
                "post": {
                    "operationId": "onPing",
                    "responses": {"200": {"description": "ok", "headers": {"x-trace-id": {"description": "trace", "schema": {"type": "string"}}}, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
                }
            }
        }
    })
}

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.incoming.test/swift-incoming.json").unwrap();
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

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "IncomingSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn file<'a>(files: &'a [OutFile], suffix: &str) -> &'a OutFile {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate(document());
    let incoming = file(&files, "Incoming.swift").content.clone();
    for expected in [
        // The branded failure and the shared case-insensitive header read.
        "public struct IncomingRequestError: Error, Sendable, CustomStringConvertible",
        "func incomingHeaderValue(_ headers: [String: String], _ name: String) -> String?",
        "for (key, value) in headers where key.caseInsensitiveCompare(name) == .orderedSame",
        // The webhook payload binds its declared model codec.
        "public typealias NewIssuePayload = IssueEvent",
        // The frozen route constant: method/path/expression.
        "public struct NewIssueWebhookRoute: Sendable, Equatable {",
        "public static let method = \"POST\"",
        "public static let path = \"newIssue\"",
        "public static let expression = false",
        // The decoder presence-checks the declared required header and decodes
        // through the package's own codec machinery.
        "public func decodeNewIssueWebhook(headers: [String: String], body: Data) throws -> NewIssuePayload {",
        "if incomingHeaderValue(headers, \"x-signature\") == nil {",
        "throw IncomingRequestError(\"required receipt header x-signature is absent\")",
        "if body.isEmpty {",
        "throw IncomingRequestError(\"the declared required receipt body is absent\")",
        "try Codecs.onNewIssueRequest.decode(body)",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "public func constructNewIssueResponse() throws -> (status: Int, headers: [String: String], body: Data) {",
        "return (status: 204, headers: [:], body: Data())",
        // The declared 200 reply encodes through the response codec, with the
        // declared reply header applied from the optional argument.
        "public func constructPingResponse(result: Pong, headers: [String: String]? = nil) throws -> (status: Int, headers: [String: String], body: Data) {",
        "body = try Codecs.onPingResponse200.encode(result)",
        "for name in [\"x-trace-id\"] {",
        // The callback receipt carries its runtime expression verbatim.
        "public typealias SubscribeOnEventPayload = Event",
        "public static let path = \"{$request.body#/callbackUrl}\"",
        "public static let expression = true",
        "public func decodeSubscribeOnEventWebhook(headers: [String: String], body: Data) throws -> SubscribeOnEventPayload {",
        // The frozen compiled descriptor constants.
        "public enum IncomingDescriptors {",
        "public struct Descriptor: Sendable, Equatable {",
        "public static let newIssue = Descriptor(",
        "kind: \"webhook\", method: \"POST\", route: \"newIssue\", expression: false,",
        "requiredHeaders: [\"x-signature\"], payload: \"json\", payloadCodec: \"onNewIssueRequest\"",
        "reply: \"none\", replyStatus: 204, replyCodec: nil, replyHeaders: []",
        "public static let subscribeOnEvent = Descriptor(",
        "kind: \"callback\", method: \"POST\", route: \"{$request.body#/callbackUrl}\", expression: true,",
        "public static let ping = Descriptor(",
        "reply: \"json\", replyStatus: 200, replyCodec: \"onPingResponse200\", replyHeaders: [\"x-trace-id\"]",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.swift lacks:\n{expected}\n--- emitted: ---\n{incoming}"
        );
    }
    // The client surface stays untouched: no receipt member leaks into it.
    let client = file(&files, "Client.swift").content.clone();
    let operations = file(&files, "Operations.swift").content.clone();
    assert!(!client.contains("decodeNewIssueWebhook"));
    assert!(!operations.contains("decodeNewIssueWebhook"));
}

#[test]
fn receipt_less_documents_emit_no_incoming_bytes() {
    let without = document();
    let mut control = without.clone();
    control
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    control["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok"}}
    }});
    let files = generate(control.clone());
    assert!(
        !files
            .iter()
            .any(|file| file.path.ends_with("Incoming.swift")),
        "a receipt-less contract must emit no Incoming.swift"
    );
    for file in &files {
        assert!(
            !file.content.contains("IncomingRequestError"),
            "{} carries incoming bytes",
            file.path
        );
        assert!(
            !file.content.contains("WebhookRoute"),
            "{} carries incoming bytes",
            file.path
        );
    }
    // The compiled plan carries no receipts either.
    let contract = contract_with_document(control);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap();
    assert!(plan.incoming().is_empty());
}

#[test]
fn plan_carries_the_compiled_receipts_with_allocated_names() {
    let contract = contract_with_document(document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap();
    let receipts = plan.incoming();
    assert_eq!(receipts.len(), 3, "{receipts:?}");
    let new_issue = receipts.iter().find(|r| r.name == "newIssue").unwrap();
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
    assert_eq!(new_issue.route_type, "NewIssueWebhookRoute");
    assert_eq!(new_issue.descriptor, "newIssue");
    assert_eq!(new_issue.required_headers, vec!["x-signature"]);
    assert!(new_issue.required_body);
    let response = new_issue.response.as_ref().unwrap();
    assert_eq!(response.status, 204);
    assert!(response.body.is_none());
    let ping = receipts.iter().find(|r| r.name == "ping").unwrap();
    assert_eq!(ping.construct.as_deref(), Some("constructPingResponse"));
    let ping_response = ping.response.as_ref().unwrap();
    assert_eq!(ping_response.status, 200);
    assert!(ping_response.body.is_some());
    assert_eq!(ping_response.headers, vec!["x-trace-id"]);
    assert!(ping_response.required_headers.is_empty());
    let callback = receipts
        .iter()
        .find(|r| r.name == "subscribe.onEvent")
        .unwrap();
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);
    assert_eq!(callback.decode, "decodeSubscribeOnEventWebhook");
    assert!(callback.required_body);
}

#[test]
fn non_json_receipt_media_refuse_the_plan() {
    let mut text = document();
    text["webhooks"]["ping"]["post"]["responses"] = json!({"204": {"description": "Accepted"}});
    text["webhooks"]["ping"]["post"]["requestBody"] = json!({
        "required": true,
        "content": {"text/plain": {"schema": {"type": "string"}}}
    });
    let contract = contract_with_document(text);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let error =
        swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap_err();
    assert!(
        error
            .iter()
            .any(|item| item.code == "sdk-incoming-receipt-unrepresentable"),
        "{error:?}"
    );
}

fn swiftc() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_SWIFTC_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let found = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .ok()?;
    if !found.status.success() {
        return None;
    }
    let text = String::from_utf8(found.stdout).ok()?;
    let path = std::path::PathBuf::from(text.trim());
    Command::new(&path).arg("--version").output().ok()?;
    Some(path)
}

fn swift() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin/swift"))
}

fn swift_command(root: &Path, action: &str) -> Command {
    let mut command = Command::new(swift());
    command.arg(action);
    if action == "test" {
        command.arg("--disable-swift-testing");
    }
    if let Some(sdkroot) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("--sdk").arg(&sdkroot).env("SDKROOT", sdkroot);
    }
    command.env("SWIFT_EXEC", swiftc().unwrap_or_else(|| "swiftc".into()));
    command.current_dir(root);
    command
}

const BEHAVIOR: &str = r##"import Foundation
import XCTest
import IncomingSDK

final class IncomingTests: XCTestCase {
    func testValidWebhookPostDecodesThroughTheDeclaredModelCodec() throws {
        let payload = try decodeNewIssueWebhook(
            headers: ["X-Signature": "abc"],
            body: Data(#"{"id":"i1","title":"t"}"#.utf8))
        XCTAssertEqual(payload.id, "i1")
        XCTAssertEqual(payload.title, "t")
    }

    func testMissingRequiredHeaderIsTheBrandedFailure() {
        do {
            _ = try decodeNewIssueWebhook(
                headers: [:],
                body: Data(#"{"id":"i1","title":"t"}"#.utf8))
            XCTFail("a missing required header must fail")
        } catch let error as IncomingRequestError {
            XCTAssertEqual(error.message, "required receipt header x-signature is absent")
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testHeaderReadsAreCaseInsensitive() throws {
        _ = try decodeNewIssueWebhook(
            headers: ["x-SIGNATURE": "abc"],
            body: Data(#"{"id":"i1","title":"t"}"#.utf8))
    }

    func testInvalidPayloadIsTheBrandedFailure() {
        do {
            _ = try decodeNewIssueWebhook(
                headers: ["x-signature": "abc"],
                body: Data(#"{"id":"i1"}"#.utf8))
            XCTFail("an invalid payload must fail")
        } catch let error as IncomingRequestError {
            XCTAssertTrue(
                error.message.hasPrefix("the received payload does not satisfy its declared schema"),
                error.message)
            XCTAssertNotNil(error.cause)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testRequiredBodyRefusesAnEmptyDelivery() {
        do {
            _ = try decodeNewIssueWebhook(headers: ["x-signature": "abc"], body: Data())
            XCTFail("an empty delivery of a required body must fail")
        } catch let error as IncomingRequestError {
            XCTAssertEqual(error.message, "the declared required receipt body is absent")
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testTheCallbackReceiptDecodesItsOwnSchema() throws {
        let event = try decodeSubscribeOnEventWebhook(
            headers: [:], body: Data(#"{"kind":"open"}"#.utf8))
        XCTAssertEqual(event.kind, "open")
    }

    func testTheDeclaredReplyIsPinnedToItsStatusAndBody() throws {
        let accepted = try constructNewIssueResponse()
        XCTAssertEqual(accepted.status, 204)
        XCTAssertTrue(accepted.headers.isEmpty)
        XCTAssertTrue(accepted.body.isEmpty)

        let pong = try Codecs.onPingResponse200.decode(Data(#"{"pong":"ok"}"#.utf8))
        let reply = try constructPingResponse(result: pong)
        XCTAssertEqual(reply.status, 200)
        XCTAssertEqual(reply.headers, [:])
        XCTAssertEqual(String(decoding: reply.body, as: UTF8.self), #"{"pong":"ok"}"#)

        // A declared reply value that violates its schema is the branded
        // failure, with the underlying codec failure as its cause.
        let invalid = Pong(pong: "toolong")
        do {
            _ = try constructPingResponse(result: invalid)
            XCTFail("an invalid reply value must fail")
        } catch let error as IncomingRequestError {
            XCTAssertTrue(
                error.message.hasPrefix("the declared reply value does not satisfy its declared schema"),
                error.message)
        }
    }

    func testDeclaredReplyHeadersAreAppliedWhenGiven() throws {
        let pong = try Codecs.onPingResponse200.decode(Data(#"{"pong":"ok"}"#.utf8))
        let traced = try constructPingResponse(result: pong, headers: ["x-trace-id": "t1"])
        XCTAssertEqual(traced.headers["x-trace-id"], "t1")
        let untraced = try constructPingResponse(result: pong)
        XCTAssertTrue(untraced.headers.isEmpty)
    }

    func testRouteConstantsCarryTheDeclaredRouteVerbatim() {
        XCTAssertEqual(NewIssueWebhookRoute.method, "POST")
        XCTAssertEqual(NewIssueWebhookRoute.path, "newIssue")
        XCTAssertFalse(NewIssueWebhookRoute.expression)
        XCTAssertEqual(PingWebhookRoute.path, "ping")
        XCTAssertEqual(SubscribeOnEventWebhookRoute.path, "{$request.body#/callbackUrl}")
        XCTAssertTrue(SubscribeOnEventWebhookRoute.expression)
    }

    func testCompiledDescriptorsAreFrozenData() {
        XCTAssertEqual(IncomingDescriptors.newIssue.kind, "webhook")
        XCTAssertEqual(IncomingDescriptors.newIssue.payload, "json")
        XCTAssertEqual(IncomingDescriptors.newIssue.payloadCodec, "onNewIssueRequest")
        XCTAssertEqual(IncomingDescriptors.newIssue.replyStatus, 204)
        XCTAssertEqual(IncomingDescriptors.newIssue.reply, "none")
        XCTAssertNil(IncomingDescriptors.newIssue.replyCodec)
        XCTAssertEqual(IncomingDescriptors.newIssue.requiredHeaders, ["x-signature"])
        XCTAssertEqual(IncomingDescriptors.subscribeOnEvent.expression, true)
        XCTAssertEqual(IncomingDescriptors.ping.reply, "json")
        XCTAssertEqual(IncomingDescriptors.ping.replyCodec, "onPingResponse200")
        XCTAssertEqual(IncomingDescriptors.ping.replyHeaders, ["x-trace-id"])
        XCTAssertEqual(IncomingDescriptors.ping.source.pointer, "/webhooks/ping/post")
    }
}
"##;

#[test]
fn native_receipts_decode_and_construct() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_incoming: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_incoming: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    let files = generate(document());
    suspect_codegen::write_files(&files, &root.path().join("sdk")).unwrap();

    // The generated package must build cleanly with warnings as errors.
    let build = swift_command(&root.path().join("sdk/swift"), "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/sdk"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "generated package build failed\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Behavioral consumer: fake webhook posts drive the decoder, the branded
    // failures and the declared reply constructors without any loopback
    // socket.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/IncomingConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"IncomingConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"IncomingConsumer\", dependencies: [.product(name: \"IncomingSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/IncomingConsumer/IncomingTests.swift"),
        BEHAVIOR,
    )
    .unwrap();
    let test = swift_command(&consumer, "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "native incoming behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_incoming: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}
