//! M4 typed-SSE runtime emission for the C++ backend: generation-time
//! emission shape, the discrimination controls, and native behavioral
//! verification of the generated typed-event pagers against a scripted
//! transport. Static runtime files are never modified; every pager lives in
//! the emitted `include/<package>/stream_events.hpp` plus conditional
//! `client.hpp` members, and operations without a discriminated stream schema
//! emit nothing at all.

#![cfg(feature = "cpp-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.streams.test/openapi.json").unwrap();
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

/// Two discriminated SSE streams (one with a declared `[DONE]` sentinel via
/// its description) and two controls: an SSE envelope without discrimination
/// evidence and a plain-string JSON-lines item schema.
fn stream_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Typed streams", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/chat": {"post": {
                "operationId": "streamChat",
                "responses": {"200": {"description": "chat events", "content": {"text/event-stream": {
                    "itemSchema": {
                        "type": "object",
                        "properties": {
                            "event": {"type": "string", "enum": ["message", "done"]},
                            "data": {"type": "string"},
                            "id": {"type": "string"}
                        },
                        "required": ["event", "data"]
                    }
                }}}}
            }},
            "/transcribe": {"post": {
                "operationId": "streamTranscription",
                "description": "Audio chunks arrive as JSON objects. [DONE] terminates the stream.",
                "responses": {"200": {"description": "transcript events", "content": {"text/event-stream": {
                    "itemSchema": {
                        "type": "object",
                        "properties": {
                            "event": {"type": "string", "enum": ["message", "done"]},
                            "data": {"type": "string"}
                        },
                        "required": ["event", "data"]
                    }
                }}}}
            }},
            "/logs": {"get": {
                "operationId": "streamLogs",
                "responses": {"200": {"description": "log lines", "content": {"text/event-stream": {
                    "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                }}}}
            }},
            "/rows": {"get": {
                "operationId": "streamRows",
                "responses": {"200": {"description": "rows", "content": {"application/x-ndjson": {
                    "itemSchema": {"type": "string"}
                }}}}
            }}
        }
    })
}

fn untyped_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Untyped streams", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/logs": {"get": {
                "operationId": "streamLogs",
                "responses": {"200": {"description": "log lines", "content": {"text/event-stream": {
                    "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                }}}}
            }},
            "/rows": {"get": {
                "operationId": "streamRows",
                "responses": {"200": {"description": "rows", "content": {"application/x-ndjson": {
                    "itemSchema": {"type": "string"}
                }}}}
            }}
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CppHttp,
        package_name: "stream_events_cpp".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate_document(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn generate(document: Value) -> Vec<OutFile> {
    generate_document(document, &GenerationOptions::default())
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
fn discriminated_sse_operations_emit_typed_event_pagers() {
    let files = generate(stream_document());
    let events = file(&files, "stream_events.hpp").content.clone();
    for expected in [
        // The per-kind payload structs, the typed unknown alternative, the
        // event variant, the completion metadata and the RAII pager surface.
        "struct StreamChatMessageEventData {",
        "struct StreamChatDoneEventData {",
        "struct StreamChatUnknownEventData {",
        "using Payload = std::variant<StreamChatMessageEventData, StreamChatDoneEventData, StreamChatUnknownEventData>;",
        "struct StreamChatEvent {",
        "struct StreamChatCompletion {",
        "enum class Reason { Sentinel, Eof };",
        "class StreamChatEventsPager {",
        "bool next(StreamChatEvent& out)",
        "const StreamChatEvent& value() const",
        "const StreamChatCompletion& completion() const",
        "inline StreamChatEventsPager Client::stream_chat_events(const StreamChatInput& input, CallOptions options) const",
        // The typed decode through the operation's stream item codec, with
        // the branded decoding failure path.
        "state_->context->validation.require(",
        "state_->fail(std::move(failure.error));",
        // The declared id metadata is exposed per event.
        "typed.id = found->second.as<std::string>();",
        // The declared sentinel completes the stream before any payload
        // decoding, preserving the final usage frame.
        "if (data == std::string(\"[DONE]\", 6)) {",
        "completion_.reason = StreamTranscriptionCompletion::Reason::Sentinel;",
        "if (held_) value_ = std::move(held_);",
        // The second typed stream: same surface, no id metadata, sentinel.
        "class StreamTranscriptionEventsPager {",
        "inline StreamTranscriptionEventsPager Client::stream_transcription_events(",
        // The exchange opener is defined on the generated client.
        "Result<std::unique_ptr<detail::ItemState>, StreamChatError> Client::stream_chat_events_stream(const StreamChatInput& input, CallOptions options) const {",
    ] {
        assert!(
            events.contains(expected),
            "stream_events.hpp lacks:\n{expected}\n--- emitted: ---\n{events}"
        );
    }

    // The controls without discrimination emit nothing at all.
    assert!(!events.contains("StreamLogs"));
    assert!(!events.contains("stream_rows_events"));

    // The paginated-style client declarations join `client.hpp` only when
    // pagers exist, and the exchange opener is declared on the client.
    let client = file(&files, "client.hpp").content.clone();
    for expected in [
        "class StreamChatEventsPager;",
        "[[nodiscard]] StreamChatEventsPager stream_chat_events(const StreamChatInput& input, CallOptions options = {}) const;",
        "Result<std::unique_ptr<detail::ItemState>, StreamChatError> stream_chat_events_stream(const StreamChatInput& input, CallOptions options = {}) const;",
        "[[nodiscard]] StreamTranscriptionEventsPager stream_transcription_events(const StreamTranscriptionInput& input, CallOptions options = {}) const;",
    ] {
        assert!(client.contains(expected), "client.hpp lacks {expected}");
    }
    assert!(!client.contains("stream_logs_events"));

    // The direct call and its untyped item stream stay exactly as before.
    let client_source = file(&files, "src/client.cpp").content.clone();
    assert!(
        client_source.contains(
            "Result<StreamChatSuccess, StreamChatError> Client::stream_chat(const StreamChatInput& input, CallOptions options) const {"
        ),
        "the untyped direct call is unchanged"
    );
    assert!(
        client_source
            .contains("StreamChatStatus200BodyItem> data(std::move(state), detail::decode_"),
        "the untyped item stream keeps its strict decode"
    );

    // The emitted package compiles: the generated header typechecks inside a
    // compiled consumer package.
    if let Some((cmake, cxx)) = toolchain() {
        let root = tempfile::tempdir().unwrap();
        suspect_codegen::write_files(&files, root.path()).unwrap();
        let consumer = root.path().join("consumer");
        std::fs::create_dir_all(&consumer).unwrap();
        std::fs::write(
            consumer.join("main.cpp"),
            "#include <stream_events_cpp/sdk.hpp>\n#include <stream_events_cpp/stream_events.hpp>\n\nint main() { return 0; }\n",
        )
        .unwrap();
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
    } else {
        eprintln!("cpp_stream_typed: cmake/clang++ not available; emission assertions only");
    }
}

#[test]
fn control_operations_without_discrimination_emit_nothing() {
    let files = generate(untyped_document());
    assert!(
        !files
            .iter()
            .any(|file| file.path.ends_with("stream_events.hpp"))
    );
    let client = file(&files, "client.hpp").content.clone();
    for absent in [
        "stream_logs_events",
        "stream_rows_events",
        "StreamLogsEvent",
        "StreamRowsEvent",
        "stream_events.hpp",
    ] {
        assert!(!client.contains(absent), "no-policy client gained {absent}");
    }
    let client_source = file(&files, "src/client.cpp").content.clone();
    assert!(!client_source.contains("_events_stream"));
}

/// SDK defaults neither enable nor disable typed stream emission: it is
/// conditional only on the compiled stream plan.
#[test]
fn sdk_defaults_neither_enable_nor_disable_typed_stream_emission() {
    let with_defaults = GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    };
    let mut configured = generate_document(stream_document(), &with_defaults);
    let mut control = generate(stream_document());
    sorted(&mut configured);
    sorted(&mut control);
    assert_eq!(
        configured
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
        control
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
    );
    for (configured, control) in configured.iter().zip(control.iter()) {
        assert_eq!(configured.path, control.path);
        assert_eq!(
            configured.content, control.content,
            "{} changed",
            control.path
        );
    }
}

#[test]
fn plan_carries_the_compiled_stream_semantics() {
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::cpp_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::cpp_sdk::SdkConfig {
            name: "stream_events_cpp".into(),
            namespace: "stream_events_cpp".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let events = plan.stream_events();
    // The shared semantics are compiled for every stream media; the lowered
    // emission covers exactly the discriminated SSE operations.
    assert_eq!(events.semantics.streams.len(), 4);
    assert_eq!(events.operations.len(), 2);
    let chat = events
        .operations
        .iter()
        .find(|entry| entry.method == "stream_chat")
        .expect("typed stream operation");
    assert_eq!(chat.kinds, vec!["message".to_owned(), "done".to_owned()]);
    assert!(chat.sentinel.is_none());
    assert!(chat.id_metadata);
    assert!(!chat.retry_metadata);
    assert_eq!(chat.pager, "StreamChatEventsPager");
    assert_eq!(chat.events_method, "stream_chat_events");
    assert_eq!(chat.open_method, "stream_chat_events_stream");
    assert_eq!(chat.event, "StreamChatEvent");
    assert_eq!(chat.completion, "StreamChatCompletion");
    let transcribe = events
        .operations
        .iter()
        .find(|entry| entry.method == "stream_transcription")
        .expect("typed stream operation");
    assert!(
        transcribe.sentinel.is_some(),
        "the sentinel is compiled from the description evidence"
    );
    assert_eq!(transcribe.sentinel.as_deref(), Some("[DONE]"));
    assert!(transcribe.keep_final_usage);
    assert!(!transcribe.id_metadata);
    // The shared plan still records the untyped controls conservatively.
    for identity in ["streamLogs", "streamRows"] {
        let compiled = events
            .semantics
            .streams
            .iter()
            .find(|stream| stream.operation == identity)
            .unwrap_or_else(|| panic!("{identity} carries a compiled stream entry"));
        assert_eq!(compiled.events.len(), 1);
    }
}

const CONSUMER_CMAKE: &str = r#"cmake_minimum_required(VERSION 3.24)
project(StreamEventsConsumer LANGUAGES CXX)
set(SUSPECT_SDK_WITH_CURL OFF CACHE BOOL "" FORCE)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../cpp stream-events-build)
add_executable(consumer main.cpp)
target_link_libraries(consumer PRIVATE stream_events_cpp::stream_events_cpp)
set_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)
target_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)
enable_testing()
add_test(NAME stream_events COMMAND consumer)
"#;

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
        "native stream-events gate retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const CONSUMER: &str = r#"// Scripted-transport consumer asserting the generated typed event pagers.
#include <stream_events_cpp/sdk.hpp>
#include <stream_events_cpp/stream_events.hpp>

#include <iostream>
#include <memory>
#include <string>
#include <utility>
#include <vector>

using namespace stream_events_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

class ScriptedTransport final : public Transport {
public:
    explicit ScriptedTransport(std::vector<std::string> bodies) : bodies_(std::move(bodies)) {}
    Result<HttpResponse, TransportError> send(const HttpRequest&, const TransportOptions&) const override {
        ++requests_;
        const auto index = static_cast<std::size_t>(requests_ - 1);
        if (index >= bodies_.size()) {
            TransportError error;
            error.kind = TransportError::Kind::Protocol;
            error.message = "unexpected request " + std::to_string(requests_);
            return Result<HttpResponse, TransportError>::failure(std::move(error));
        }
        HttpResponse response;
        response.status = 200;
        response.headers = Headers{{"Content-Type", "text/event-stream"}};
        response.body = bodies_[index];
        return Result<HttpResponse, TransportError>::success(std::move(response));
    }
    int requests() const { return requests_; }

private:
    std::vector<std::string> bodies_;
    mutable int requests_ = 0;
};

// A declared message event decodes to the typed model; a declared done event
// follows; the completion reports an end-of-body completion with no usage.
void typed_decode_and_completion() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n",
    });
    Client client(transport);
    auto pager = client.stream_chat_events(StreamChatInput{});
    StreamChatEvent event;
    expect(pager.next(event), "first event delivered");
    const auto* message = std::get_if<0>(&event.payload);
    expect(message != nullptr, "the declared message kind decodes to the declared model");
    expect(message && message->data.data == "{\"text\":\"hello\"}", "decoded envelope data");
    expect(event.kind == "message", "the event kind is the declared name");
    expect(pager.next(event), "second event delivered");
    const auto* done = std::get_if<1>(&event.payload);
    expect(done != nullptr, "the declared done kind decodes to the declared model");
    expect(done && done->data.data == "{}", "decoded envelope data");
    expect(!pager.next(event), "the stream completed");
    expect(!pager.error().has_value(), "ordinary completion carries no failure");
    expect(pager.completion().reason == StreamChatCompletion::Reason::Eof, "eof completion reason");
    expect(!pager.completion().usage.has_value(), "no usage is preserved without the policy");
    expect(transport->requests() == 1, "one request");
}

// Per-item metadata: the declared id field is exposed on the typed event.
void metadata_id() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\nid: 42\ndata: hello\n\n",
    });
    Client client(transport);
    auto pager = client.stream_chat_events(StreamChatInput{});
    StreamChatEvent event;
    expect(pager.next(event), "event delivered");
    expect(event.id.has_value() && event.id.value() == "42", "the id metadata is exposed");
    expect(!pager.next(event), "the stream completed");
    expect(!pager.completion().usage.has_value(), "no usage without the policy");
}

// An undeclared event kind surfaces through the typed unknown alternative
// without failing the stream, and later declared events still decode.
void unknown_kind_does_not_fail() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n",
    });
    Client client(transport);
    auto pager = client.stream_chat_events(StreamChatInput{});
    StreamChatEvent event;
    expect(pager.next(event), "unknown event delivered");
    const auto* unknown = std::get_if<2>(&event.payload);
    expect(unknown != nullptr, "the undeclared kind surfaces through the unknown alternative");
    expect(unknown && unknown->kind == "surprise", "the undeclared wire kind is carried");
    expect(unknown && unknown->data == "hello", "the raw frame data is carried");
    expect(event.kind == "surprise", "the event name is the wire kind");
    expect(pager.next(event), "declared event after an unknown one");
    expect(std::get_if<0>(&event.payload) != nullptr, "the declared kind still decodes");
    expect(!pager.next(event), "the stream completed");
}

// An invalid payload for a recognized kind remains a branded decoding error:
// the frame's envelope lacks the required event field.
void invalid_payload_is_branded() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "data: hello\n\n",
    });
    Client client(transport);
    auto pager = client.stream_chat_events(StreamChatInput{});
    StreamChatEvent event;
    expect(!pager.next(event), "the decoding failure stops the walk");
    expect(pager.error().has_value(), "the terminal failure is engaged");
    expect(
        pager.error().has_value() && pager.error().value().kind == SdkError::Kind::ResponseDecoding,
        "the failure is branded as response decoding"
    );
    expect(transport->requests() == 1, "one request");
}

// The declared [DONE] sentinel completes the stream before any payload
// decoding, preserves the final usage frame as terminal metadata, and issues
// no further reads: the post-sentinel frame is never decoded.
void sentinel_completes_and_preserves_usage() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\nevent: message\ndata: never\n\n",
    });
    Client client(transport);
    auto pager = client.stream_transcription_events(StreamTranscriptionInput{});
    StreamTranscriptionEvent event;
    expect(!pager.next(event), "the sentinel completes without yielding the held frame");
    expect(!pager.error().has_value(), "an ordinary completion carries no failure");
    expect(pager.completion().reason == StreamTranscriptionCompletion::Reason::Sentinel, "sentinel reason");
    expect(pager.completion().usage.has_value(), "the final usage frame is preserved");
    expect(
        pager.completion().usage.has_value() && pager.completion().usage->kind == "message",
        "the usage frame is the decoded final event"
    );
    expect(
        pager.completion().usage.has_value() &&
            std::get_if<0>(&pager.completion().usage->payload) != nullptr &&
            std::get<0>(pager.completion().usage->payload).data.data == "{\"tokens\": 42}",
        "the usage frame carries the decoded tokens"
    );
    expect(!pager.next(event), "the completed pager stays completed");
    expect(transport->requests() == 1, "one request");
}

// Earlier frames are still yielded when a sentinel follows them.
void sentinel_after_earlier_events() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n",
    });
    Client client(transport);
    auto pager = client.stream_transcription_events(StreamTranscriptionInput{});
    StreamTranscriptionEvent event;
    expect(pager.next(event), "earlier frames are yielded");
    expect(std::get_if<0>(&event.payload) != nullptr && std::get<0>(event.payload).data.data == "a", "first frame");
    expect(!pager.next(event), "the sentinel completes");
    expect(
        pager.completion().usage.has_value() &&
            std::get<0>(pager.completion().usage->payload).data.data == "{\"usage\": true}",
        "the last frame before the sentinel is the terminal usage"
    );
}

// Early break stops consumption: one event, one request, and no further
// exchange after the pager is destroyed.
void early_break() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\ndata: a\n\nevent: message\ndata: b\n\n",
    });
    Client client(transport);
    StreamChatEvent event;
    int events = 0;
    for (auto pager = client.stream_chat_events(StreamChatInput{}); pager.next(event);) {
        if (++events == 1) break;
    }
    expect(events == 1, "early break delivered exactly one event");
    expect(transport->requests() == 1, "early break issued no second request");
}

// The untyped direct call keeps its exact previous behavior: strictly decoded
// envelope items for well-formed frames.
void untyped_path_unchanged() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        "event: message\ndata: a\n\nevent: message\ndata: b\n\n",
    });
    Client client(transport);
    auto result = client.stream_chat(StreamChatInput{});
    expect(result.ok(), "the direct call succeeds");
    auto* page = result.ok() ? std::get_if<StreamChatStatus200>(&result.value()) : nullptr;
    expect(page != nullptr, "the stream success variant is selected");
    if (page) {
        auto first = page->data.next();
        expect(first.ok() && first.value().has_value() && first.value()->data == "a", "first untyped item");
        auto second = page->data.next();
        expect(second.ok() && second.value().has_value() && second.value()->data == "b", "second untyped item");
        auto end = page->data.next();
        expect(end.ok() && !end.value().has_value(), "the untyped stream ends");
    }
}

} // namespace

int main() {
    typed_decode_and_completion();
    metadata_id();
    unknown_kind_does_not_fail();
    invalid_payload_is_branded();
    sentinel_completes_and_preserves_usage();
    sentinel_after_earlier_events();
    early_break();
    untyped_path_unchanged();
    return failures == 0 ? 0 : 1;
}
"#;

#[test]
fn native_typed_events_drive_scripted_streams() {
    let Some((cmake, cxx)) = toolchain() else {
        eprintln!("cpp_stream_typed: cmake/clang++ not available; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(stream_document()), root.path()).unwrap();
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
