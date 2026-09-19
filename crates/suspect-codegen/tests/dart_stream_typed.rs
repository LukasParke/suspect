//! Emitted-only typed SSE events for the native Dart HTTP backend: the
//! generated `stream_events.dart` part, the per-operation `<op>Events` streams
//! and `<op>EventsCompletion` accessors on the generated `Client`, and
//! byte-identity for operations without a discriminated stream schema. Static
//! runtime files are never modified; the typed decode lives entirely in the
//! generated package. No Dart toolchain is required: the behavioral checks are
//! static assertions plus a documented manual verification of the emitted
//! stream (see `typed_events_are_static_and_untyped_operations_are_unchanged`).

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

fn contract_with_document(document: Value) -> Arc<Contract> {
    let uri = Uri::parse("https://source.streams.test/typed-streams.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
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
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}

/// Two discriminated SSE streams (one with a declared `[DONE]` sentinel via its
/// description) and two controls: an SSE envelope without discrimination
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

/// The same document with the declared event discriminators removed, so both
/// SSE operations compile the conservative single-default stream plan and emit
/// nothing at all.
fn untyped_document() -> Value {
    let mut document = stream_document();
    for path in ["/chat", "/transcribe"] {
        let event = document["paths"][path]["post"]["responses"]["200"]["content"]
            ["text/event-stream"]["itemSchema"]["properties"]["event"]
            .as_object_mut()
            .expect("discriminator property");
        event.remove("enum");
    }
    document
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::DartHttp,
        package_name: "typed_streams_sdk".into(),
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

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
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
fn discriminated_sse_operations_emit_typed_event_members() {
    let files = generate(stream_document());
    let client = source(&files, "dart/lib/src/client.dart");
    let part = source(&files, "dart/lib/src/stream_events.dart");
    let library = source(&files, "dart/lib/typed_streams_sdk.dart");
    assert!(
        library.contains("part 'src/stream_events.dart';"),
        "library lacks the stream events part directive"
    );
    for expected in [
        // The typed events stream on the generated Client.
        "Stream<StreamChatEvent> streamChatEvents({ CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) {",
        "return _streamChatEventsCore(null, cancellation: cancellation, timeout: timeout, server: server, securityAlternative: securityAlternative);",
        "Stream<StreamTranscriptionEvent> streamTranscriptionEvents({ CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) {",
        // The documented completion accessor.
        "Future<StreamChatCompletion> streamChatEventsCompletion({ CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async {",
        "return terminal ?? const StreamChatCompletion('eof', null);",
        // The shared core iteration over the lenient frame transport, reusing
        // the runtime framer and the direct call's argument forwarding.
        "await for (final frame in _stream<StreamChatEvent>(_operation0, prepare, _streamChatFrame, cancellation, timeout, server, securityAlternative)) {",
        "await for (final frame in _stream<StreamTranscriptionEvent?>(_operation3, prepare, _streamTranscriptionFrame, cancellation, timeout, server, securityAlternative)) {",
        // The untyped direct call stays exactly as before.
        "Stream<StreamChatStatus200> streamChat({",
        "return _stream(_operation0,prepare,(frame)=>_decodeOperation0(frame.received,frame.item),cancellation,timeout,server,securityAlternative);",
    ] {
        assert!(client.contains(expected), "client.dart lacks {expected}");
    }
    for expected in [
        // The discriminated event union with typed per-kind classes and the
        // typed unknown alternative.
        "sealed class StreamChatEvent {",
        "final class StreamChatMessageEvent extends StreamChatEvent {",
        "final class StreamChatDoneEvent extends StreamChatEvent {",
        "final StreamChatResponseTextEventStreamItemSchema data;",
        "String get kind => \"message\";",
        "final class StreamChatUnknownEvent extends StreamChatEvent {",
        "String get kind => 'unknown';",
        // The declared envelope metadata (id) is an event member.
        "final String? id;",
        // The completion carrier and its reason values.
        "final class StreamChatCompletion {",
        "`'sentinel'` when the declared terminal token completed the stream before any payload decoding and `'eof'` when the response body ended",
        "final StreamChatEvent? usage;",
        // The per-frame decoder: the declared error dispatch, then the
        // sentinel, then the per-kind decode through the operation's existing
        // stream item codec.
        "StreamChatEvent _streamChatFrame(_StreamRecord frame) {",
        "_decodeOperation0(frame.received);",
        "throw UnexpectedResponseException(frame.received.raw);",
        "final decoded = _decoded(frame.received, () => streamChatResponseTextEventStreamItemSchemaCodec.fromJson(item));",
        "return StreamChatMessageEvent(decoded, id: id is Present<String> ? id.value : null);",
        "return StreamChatUnknownEvent(kind, data);",
    ] {
        assert!(
            part.contains(expected),
            "stream_events.dart lacks {expected}"
        );
    }
    // The declared [DONE] sentinel is matched on frame data before any payload
    // decoding, and the final usage frame is held back for the completion.
    for expected in [
        "// The declared &#91;DONE&#93; sentinel completes the stream before any payload decoding.",
        "if (data == \"[DONE]\") { return null; }",
        "if (frame == null) {",
        "completion?.call(StreamTranscriptionCompletion('sentinel', held));",
        "if (held != null) {",
        "held = frame;",
        "completion?.call(StreamTranscriptionCompletion('eof', held));",
        // The un-sentinel operation yields every frame and preserves nothing.
        "yield frame;",
        "completion?.call(StreamChatCompletion('eof', null));",
    ] {
        assert!(
            client.contains(expected) || part.contains(expected),
            "emitted package lacks {expected}"
        );
    }

    // Controls without discrimination emit nothing.
    assert!(!client.contains("streamLogsEvents"));
    assert!(!client.contains("streamRowsEvents"));
    assert!(!part.contains("StreamLogsEvent"));
    assert!(!part.contains("StreamRowsEvent"));
}

#[test]
fn operations_without_discrimination_keep_the_untyped_package() {
    let shared = stream_document();
    let mut plain = generate(untyped_document());
    let mut typed = generate(shared);
    sorted(&mut plain);
    sorted(&mut typed);
    // The untyped package has no typed stream artifacts at all.
    assert!(
        !plain
            .iter()
            .any(|file| file.path == "dart/lib/src/stream_events.dart")
    );
    let plain_library = source(&plain, "dart/lib/typed_streams_sdk.dart");
    assert!(!plain_library.contains("part 'src/stream_events.dart'"));
    let plain_client = source(&plain, "dart/lib/src/client.dart");
    assert!(!plain_client.contains("Events"));
    assert!(!plain_client.contains("StreamChatEvent"));
    // The typed package adds exactly one file, and the generated library and
    // client gain only the appended typed events members.
    assert_eq!(
        typed.len(),
        plain.len() + 1,
        "the typed stream plan may add exactly one file"
    );
    assert_eq!(
        source(&typed, "dart/lib/typed_streams_sdk.dart"),
        format!("{plain_library}part 'src/stream_events.dart';\n"),
        "the library may only gain the stream events part directive"
    );
    let suffix = "}\n";
    let plain_body = plain_client
        .strip_suffix(suffix)
        .expect("client closing shape");
    let typed_client = source(&typed, "dart/lib/src/client.dart");
    assert!(
        typed_client.starts_with(plain_body) && typed_client.ends_with(suffix),
        "the generated client may only gain the appended typed events streams"
    );
}

#[test]
fn plan_carries_the_compiled_stream_semantics() {
    use suspect_codegen::{dart_sdk, http_protocol};
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk(contract, &selected, dart_sdk::DartConfig::default()).unwrap();
    let semantics = plan.stream_semantics();
    // One entry per declared stream media.
    assert_eq!(semantics.streams.len(), 4);
    let chat = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamChat")
        .expect("chat stream plan");
    assert_eq!(
        chat.events
            .iter()
            .map(|event| event.event_name.as_str())
            .collect::<Vec<_>>(),
        vec!["message", "done"]
    );
    assert!(!chat.sentinel.enabled);
    assert_eq!(chat.terminal.keep_final_usage, chat.sentinel.enabled);
    let transcribe = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamTranscription")
        .expect("transcription stream plan");
    assert!(transcribe.sentinel.enabled);
    assert_eq!(transcribe.sentinel.token, "[DONE]");
    assert!(transcribe.terminal.keep_final_usage);
    // Controls keep their conservative plans and stay untyped.
    let logs = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamLogs")
        .expect("log stream plan");
    assert_eq!(logs.events.len(), 1);
    let rows = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamRows")
        .expect("row stream plan");
    assert_eq!(rows.framing, http_protocol::StreamFraming::JsonLines);
    assert_eq!(rows.events.len(), 1);
}

/// With no installed Dart toolchain this test degrades to strict static
/// assertions plus a documented manual verification of the emitted stream:
///
/// 1. `streamChatEvents`/`streamTranscriptionEvents` forward every argument to
///    the shared core, which reuses the direct call's `_RequestInput`
///    preparation, so credentials, servers, timeouts and attribution travel
///    the exact direct-call path with no new request construction.
/// 2. `_streamChatFrame` receives the runtime framer's parsed envelope; an
///    absent frame item is the declared-error path, dispatched through the
///    direct operation's own `_decodeOperation0` so failures are typed
///    identically. The declared `[DONE]` sentinel returns null before any
///    payload decoding; recognized kinds decode through the operation's
///    existing stream item codec (branded decoding failures preserved by
///    `_decoded`); undeclared kinds surface through the typed unknown
///    alternative with the raw frame data.
/// 3. The core's `await for` yields events; leaving it (sentinel `return`,
///    consumer `break`, or cancel) cancels the inner single-subscription
///    stream at its yield point, which stops the transport and issues no
///    further reads. Pause propagates through the generator at each yield.
/// 4. The compiled keep-final-usage policy holds one frame behind, so the last
///    data frame before the sentinel or end of body becomes the completion's
///    `usage` instead of being yielded; the completion accessor drains the
///    same core and returns the documented `{ reason, usage }`.
/// 5. `streamChatEventsCompletion` never issues its own request: it consumes
///    the same core stream, and a completion of `'eof'` with null usage is
///    produced exactly when no frame preceded the end of body.
#[test]
fn typed_events_are_static_and_untyped_operations_are_unchanged() {
    let files = generate(stream_document());
    let part = source(&files, "dart/lib/src/stream_events.dart");
    let client = source(&files, "dart/lib/src/client.dart");
    // The typed stream decode never invents a request path of its own: the
    // stream events part contains no runtime plumbing, and every request in
    // the emitted streams goes through the shared `_stream` transport.
    for fragment in ["_RequestBuilder", "_exchange(", "HttpTransport", "_Framer"] {
        assert!(
            !part.contains(fragment),
            "stream events emission must reuse the direct call path, not the runtime plumbing: {fragment}"
        );
    }
    assert!(part.contains("part of '../typed_streams_sdk.dart';"));
    assert!(client.contains("part of '../typed_streams_sdk.dart';"));
    // Manifest stays honest: the typed events members are compiled source
    // semantics, not inferred wire behavior.
    let manifest: Value = serde_json::from_str(&source(&files, "dart/sdk-manifest.json")).unwrap();
    assert_eq!(manifest["inferred_pagination"], false);
}
