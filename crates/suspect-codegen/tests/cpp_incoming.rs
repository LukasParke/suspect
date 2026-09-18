//! Incoming receipt emission for the C++ backend: generation-time emission
//! shape, the receipt-less controls, and native behavioral verification of
//! the generated decoders and reply constructors against a fake webhook POST
//! through a compiled consumer. Static runtime files are never modified; the
//! whole surface lives in the emitted `include/<package>/incoming.hpp`, and
//! contracts without any incoming declaration emit nothing at all.

#![cfg(feature = "cpp-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.incoming.test/openapi.json").unwrap();
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

/// One outgoing operation plus declared incoming receipts: a webhook with a
/// required signed header and required JSON body answering 204, a bodyless
/// ping webhook answering a JSON 200, and a callback whose route carries an
/// RFC 6570 runtime expression.
fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
            "securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"],
                    "additionalProperties": false
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
            "/widgets": {"get": {"operationId": "listWidgets",
                "security": [{"apiKey": []}],
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
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
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
                }
            },
            "freeform": {
                "post": {
                    "operationId": "onFreeform",
                    "requestBody": {"content": {"application/json": {}}},
                    "responses": {"204": {"description": "Accepted"}}
                }
            }
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CppHttp,
        package_name: "incoming_cpp".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
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
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn generate() -> Vec<OutFile> {
    generate_document(document())
}

fn file<'a>(files: &'a [OutFile], suffix: &str) -> &'a OutFile {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| {
            panic!(
                "missing {suffix} in {:?}",
                files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        })
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

#[test]
fn incoming_receipts_emit_the_helpers_header() {
    let files = generate();
    let incoming = file(&files, "incoming.hpp").content.clone();
    for expected in [
        // The shared surface: the route record, the constructed reply record
        // and the branded failure type.
        "struct IncomingRoute {",
        "struct ConstructedResponse {",
        "class IncomingRequestError {",
        "enum class Kind {",
        "Kind::MissingHeader: return \"missing-header\";",
        "Kind::MissingBody: return \"missing-body\";",
        "Kind::InvalidPayload: return \"invalid-payload\";",
        "Kind::InvalidReply: return \"invalid-reply\";",
        // The frozen compiled descriptor table and its lookup.
        "struct IncomingDescriptor {",
        "inline const IncomingDescriptor incoming_descriptors[] = {",
        "inline const IncomingDescriptor* incoming_descriptor(std::string_view name)",
        // The newIssue webhook: the decoded payload alias, the frozen route
        // constant, the header-presence check and the compiled decode.
        "using NewIssuePayload = ::incoming_cpp::IssueEvent;",
        "inline constexpr IncomingRoute new_issue_webhook_route{std::string_view(\"POST\", 4), std::string_view(\"newIssue\", 8), false};",
        "Result<NewIssuePayload, IncomingRequestError> decode_new_issue_webhook(const Headers& headers, const Bytes& body)",
        "detail::incoming_header_value(headers, std::string_view(\"x-signature\", 11))",
        "Codec::decode(detail::incoming_body_view(body))",
        "IncomingRequestError::Kind::MissingHeader;",
        "IncomingRequestError::Kind::MissingBody;",
        "IncomingRequestError::Kind::InvalidPayload;",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "Result<ConstructedResponse, IncomingRequestError> construct_new_issue_response()",
        "reply.status = 204;",
        // The ping webhook has no declared request body; its 200 reply
        // encodes through the reply codec.
        "using PingPayload = Unit;",
        "inline constexpr IncomingRoute ping_webhook_route{std::string_view(\"POST\", 4), std::string_view(\"ping\", 4), false};",
        "Result<ConstructedResponse, IncomingRequestError> construct_ping_response(const ::incoming_cpp::Pong& result)",
        "reply.status = 200;",
        "Codec::encode(result)",
        // The callback receipt carries its runtime expression verbatim.
        "inline constexpr IncomingRoute subscribe_on_event_webhook_route{std::string_view(\"POST\", 4), std::string_view(\"{$request.body#/callbackUrl}\", 28), true};",
        "Result<SubscribeOnEventPayload, IncomingRequestError> decode_subscribe_on_event_webhook(const Headers& headers, const Bytes& body)",
        // The schema-free receipt decodes through the bounded runtime parser.
        "using FreeformPayload = JsonValue;",
        "parse_json(detail::incoming_body_view(body), JsonLimits{})",
    ] {
        assert!(
            incoming.contains(expected),
            "incoming.hpp lacks:\n{expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The outgoing package is unchanged: no client seams were added.
    let client = file(&files, "client.hpp").content.clone();
    for absent in [
        "decode_new_issue_webhook",
        "construct_ping_response",
        "NewIssuePayload",
        "IncomingRequestError",
        "IncomingRoute",
    ] {
        assert!(
            !client.contains(absent),
            "client.hpp gained the receipt seam {absent}"
        );
    }
}

#[test]
fn receipt_less_contracts_emit_nothing_and_stay_byte_identical() {
    let mut without = document();
    without
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    without["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok"}}
    }});
    let mut emptied = without.clone();
    emptied["webhooks"] = json!({});
    let mut control = generate_document(without);
    let mut emptied = generate_document(emptied);
    sorted(&mut control);
    sorted(&mut emptied);
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("incoming.hpp")),
        "a receipt-less contract must emit no incoming header"
    );
    // Identical file lists, whatever the receipt-less document spelling.
    assert_eq!(control.len(), emptied.len());
    for (control, emptied) in control.iter().zip(emptied.iter()) {
        assert_eq!(control.path, emptied.path);
    }
    let client = file(&control, "client.hpp").content.clone();
    let models = file(&control, "models.hpp").content.clone();
    for absent in [
        "decode_new_issue_webhook",
        "IncomingRequestError",
        "incoming.hpp",
        "NewIssuePayload",
    ] {
        assert!(
            !client.contains(absent),
            "receipt-less client gained {absent}"
        );
        assert!(
            !models.contains(absent),
            "receipt-less models gained {absent}"
        );
    }
}

#[test]
fn plan_carries_the_compiled_receipts() {
    let contract = contract_with_document(document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::cpp_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::cpp_sdk::SdkConfig {
            name: "incoming_cpp".into(),
            namespace: "incoming_cpp".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let incoming = plan.incoming().expect("the receipts compile into the plan");
    assert_eq!(incoming.plan.operations().len(), 4);
    assert_eq!(incoming.receipts.len(), 4);
    let new_issue = incoming
        .receipts
        .iter()
        .find(|receipt| receipt.name == "newIssue")
        .expect("the webhook receipt");
    assert_eq!(new_issue.kind, "webhook");
    assert_eq!(new_issue.method, "POST");
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.payload_alias, "NewIssuePayload");
    assert_eq!(new_issue.decode, "decode_new_issue_webhook");
    assert_eq!(
        new_issue.construct.as_deref(),
        Some("construct_new_issue_response")
    );
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    assert!(matches!(
        new_issue.payload,
        suspect_codegen::cpp_sdk::IncomingPayload::Json { .. }
    ));
    assert_eq!(
        new_issue.payload.label(),
        "json",
        "the declared JSON body decodes through its model codec"
    );
    let response = new_issue.response.as_ref().expect("the 204 reply");
    assert_eq!(response.status, 204);
    assert!(
        response.body.is_none(),
        "the declared 204 reply has no body"
    );
    let ping = incoming
        .receipts
        .iter()
        .find(|receipt| receipt.name == "ping")
        .expect("the ping receipt");
    let response = ping.response.as_ref().expect("the 200 reply");
    assert_eq!(response.status, 200);
    assert!(
        response.body.is_some(),
        "the declared 200 reply carries a body"
    );
    let callback = incoming
        .receipts
        .iter()
        .find(|receipt| receipt.name == "subscribe.onEvent")
        .expect("the callback receipt");
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);

    // The receipt-less contract carries no incoming emission at all.
    let mut receipt_less = document();
    receipt_less
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    receipt_less["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok"}}
    }});
    let control = contract_with_document(receipt_less);
    let selected = control
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::cpp_sdk::plan_sdk(
        control,
        &selected,
        suspect_codegen::cpp_sdk::SdkConfig::default(),
    )
    .unwrap();
    assert!(plan.incoming().is_none());
}

fn toolchain() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let cmake = tool("SUSPECT_CPP_CMAKE", "cmake")?;
    let cxx = tool("SUSPECT_CPP_CXX", "clang++")?;
    Some((cmake, cxx))
}

fn tool(variable: &str, fallback: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Some(std::path::PathBuf::from(path));
    }
    let probe = Command::new(fallback).arg("--version").output().ok()?;
    probe
        .status
        .success()
        .then(|| std::path::PathBuf::from(fallback))
}

/// Runs the command and fails the test with the retained log on failure.
fn checked(command: &mut Command, retained: &std::path::Path) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("required native tool {command:?}: {error}"));
    let log = retained.join("commands.log");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    let _ = writeln!(
        file,
        "\n{command:?}\nstatus: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "native incoming gate retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const CONSUMER_CMAKE: &str = r#"cmake_minimum_required(VERSION 3.24)
project(IncomingConsumer LANGUAGES CXX)
set(SUSPECT_SDK_WITH_CURL OFF CACHE BOOL "" FORCE)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../cpp incoming-build)
add_executable(consumer main.cpp)
target_link_libraries(consumer PRIVATE incoming_cpp::incoming_cpp)
set_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)
target_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)
enable_testing()
add_test(NAME incoming COMMAND consumer)
"#;

const CONSUMER: &str = r#"// Fake-webhook-POST consumer asserting the generated receipt helpers.
#include <incoming_cpp/sdk.hpp>
#include <incoming_cpp/incoming.hpp>

#include <iostream>
#include <string>
#include <utility>
#include <vector>

using namespace incoming_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

Bytes bytes(const std::string& text) {
    return Bytes(text.begin(), text.end());
}

// A valid fake webhook POST decodes into the declared model, regardless of
// header-name casing.
void valid_decode() {
    const Headers headers{{"X-Signature", "abc"}, {"Content-Type", "application/json"}};
    auto decoded = decode_new_issue_webhook(headers, bytes(R"({"id":"i1","title":"t"})"));
    expect(decoded.ok(), "the valid receipt decodes");
    if (decoded.ok()) {
        expect(decoded.value().id == "i1", "decoded id");
        expect(decoded.value().title == "t", "decoded title");
    }
    // The callback receipt decodes through its own declared schema.
    auto event = decode_subscribe_on_event_webhook({}, bytes(R"({"kind":"open"})"));
    expect(event.ok(), "the callback receipt decodes");
    if (event.ok()) expect(event.value().kind == "open", "decoded kind");
    // A schema-free JSON receipt parses through the bounded runtime parser.
    auto freeform = decode_freeform_webhook({}, bytes(R"({"anything":["goes", 1]})"));
    expect(freeform.ok() && freeform.value().is<JsonValue::Object>(),
        "the schema-free receipt parses");
}

// A missing required declared header is the branded typed failure, and the
// error names the declared header.
void missing_header_is_typed() {
    auto decoded = decode_new_issue_webhook({}, bytes(R"({"id":"i1","title":"t"})"));
    expect(!decoded.ok(), "the missing header fails");
    expect(decoded.error().kind == IncomingRequestError::Kind::MissingHeader,
        "the missing header is typed");
    expect(decoded.error().header == "x-signature", "the error names the declared header");
    expect(decoded.error().message().find("x-signature") != std::string::npos,
        "the message names the declared header");
}

// Invalid and schema-violating payloads are branded failures; a required
// body refuses an empty delivery.
void invalid_payload_is_typed() {
    auto schema_violation = decode_new_issue_webhook(
        Headers{{"x-signature", "abc"}}, bytes(R"({"id":"i1"})"));
    expect(!schema_violation.ok(), "a schema-violating payload fails");
    expect(schema_violation.error().kind == IncomingRequestError::Kind::InvalidPayload,
        "the schema violation is typed");
    auto not_json = decode_new_issue_webhook(Headers{{"x-signature", "abc"}}, bytes("{"));
    expect(!not_json.ok() && not_json.error().kind == IncomingRequestError::Kind::InvalidPayload,
        "invalid JSON is typed");
    auto empty = decode_new_issue_webhook(Headers{{"x-signature", "abc"}}, bytes(""));
    expect(!empty.ok() && empty.error().kind == IncomingRequestError::Kind::MissingBody,
        "an absent required body is typed");
    // The parse_json schema-free path still bounds its documents: a callback
    // receipt refusing invalid JSON is the same typed failure, whichever
    // decode path compiles for it.
    auto callback = decode_subscribe_on_event_webhook({}, bytes("{"));
    expect(!callback.ok() && callback.error().kind == IncomingRequestError::Kind::InvalidPayload,
        "invalid JSON on a codec receipt is typed");
    auto freeform = decode_freeform_webhook({}, bytes("not json"));
    expect(!freeform.ok() && freeform.error().kind == IncomingRequestError::Kind::InvalidPayload,
        "invalid JSON on a schema-free receipt is typed");
}

// The declared 204 reply constructs with the pinned status and no body.
void constructed_status_204() {
    auto reply = construct_new_issue_response();
    expect(reply.ok(), "the declared 204 reply constructs");
    if (reply.ok()) {
        expect(reply.value().status == 204, "the declared status is pinned");
        expect(reply.value().body.empty(), "a body-less reply carries no body");
        expect(reply.value().headers.empty(), "no declared headers apply");
    }
}

// The declared 200 reply encodes its body through the reply codec.
void constructed_body_encodes() {
    const ::incoming_cpp::Pong pong{true};
    auto reply = construct_ping_response(pong);
    expect(reply.ok(), "the declared 200 reply constructs");
    if (reply.ok()) {
        expect(reply.value().status == 200, "the declared status is pinned");
        expect(reply.value().body == R"({"pong":true})", "the body encodes through the reply codec");
        expect(reply.value().headers.empty(), "no declared headers apply");
    }
    // Route constants are the registration hints; the callback expression is
    // carried verbatim and flagged.
    expect(std::string(ping_webhook_route.method) == "POST", "the route method is the provider verb");
    expect(std::string(new_issue_webhook_route.route) == "newIssue", "the webhook route");
    expect(!new_issue_webhook_route.expression, "a fixed webhook route carries no expression");
    expect(std::string(subscribe_on_event_webhook_route.route) == "{$request.body#/callbackUrl}",
        "the callback route is verbatim");
    expect(subscribe_on_event_webhook_route.expression, "the expression is flagged");
    // The frozen compiled descriptors carry the receipt data.
    const auto* descriptor = detail::incoming_descriptor("newIssue");
    expect(descriptor != nullptr, "the newIssue descriptor is compiled");
    expect(descriptor && descriptor->payload == "json", "the payload representation");
    expect(descriptor && !descriptor->payload_codec.empty(), "the payload codec is named");
    expect(descriptor && descriptor->reply == "none", "the body-less reply representation");
    expect(descriptor && descriptor->reply_status == 204, "the reply status");
    expect(descriptor && descriptor->required_header_count == 1
            && descriptor->required_headers[0] == "x-signature",
        "the required headers");
    const auto* ping = detail::incoming_descriptor("ping");
    expect(ping && ping->payload == "none" && ping->reply == "json" && ping->reply_status == 200,
        "the ping descriptor");
    expect(detail::incoming_descriptor("missing") == nullptr, "an unknown receipt finds nothing");
}

} // namespace

int main() {
    valid_decode();
    missing_header_is_typed();
    invalid_payload_is_typed();
    constructed_status_204();
    constructed_body_encodes();
    return failures == 0 ? 0 : 1;
}
"#;

/// Native behavior of the generated receipt helpers, when the C++ toolchain
/// is available. Degrades to the static assertions above otherwise.
#[test]
fn native_incoming_receipts_decode_and_construct() {
    let Some((cmake, cxx)) = toolchain() else {
        eprintln!("cpp_incoming: cmake/clang++ not available; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(), root.path()).unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(consumer.join("main.cpp"), CONSUMER).unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
    let ctest = cmake.with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.path().join("build"))
            .arg("--output-on-failure"),
        root.path(),
    );
}
