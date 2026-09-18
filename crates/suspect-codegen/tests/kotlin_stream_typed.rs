//! M4 typed-SSE runtime emission for the Kotlin backend: generation-time
//! emission shape, the discrimination controls, and native compilation of the
//! emitted package when a JDK/Maven toolchain is available, with a scripted-
//! transport behavioral probe. Static runtime files are never modified; every
//! walker lives in the emitted `StreamEvents.kt` plus conditional `Client.kt`
//! members, and operations without a discriminated stream schema emit nothing
//! at all.

#![cfg(feature = "kotlin-sdk")]

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
        backend: Backend::KotlinHttp,
        package_name: "test.suspect:stream-events-kotlin".into(),
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
        .unwrap_or_else(|| panic!("missing {suffix}"))
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

#[test]
fn discriminated_sse_operations_emit_typed_event_flows() {
    let files = generate(stream_document());
    let types = file(&files, "StreamEvents.kt").content.clone();
    for expected in [
        // The sealed typed event union with per-kind data classes, the typed
        // unknown alternative and the terminal completion element.
        "public sealed interface StreamChatEvent {",
        "public data class StreamChatMessageEvent(",
        "public data class StreamChatDoneEvent(",
        "public data class StreamChatUnknownEvent(",
        "public data class StreamChatCompletion(",
        "public data class StreamChatCompletionEvent(",
        "public enum class StreamChatCompletionReason {",
        "public override val kind: String get() = \"message\"",
        "public override val kind: String get() = \"unknown\"",
        "public override val kind: String get() = \"completion\"",
        // The per-item metadata members the item schema declares.
        "public val id: String? = null",
        // The terminal marker that stops the exchange at the sentinel.
        "internal class StreamTranscriptionTerminal(",
        "public enum class StreamTranscriptionCompletionReason {",
    ] {
        assert!(types.contains(expected), "StreamEvents.kt lacks {expected}");
    }
    // The untyped controls emit nothing.
    assert!(!types.contains("StreamLogsEvent"));
    assert!(!types.contains("StreamRowsEvent"));

    // The per-operation cold flow members join the generated Client.
    let client = file(&files, "Client.kt").content.clone();
    for expected in [
        "public fun streamChatEvents(input: StreamChatInput = StreamChatInput(), requestOptions: RequestOptions = RequestOptions()): Flow<StreamChatEvent> = channelFlow {",
        "public fun streamTranscriptionEvents(input: StreamTranscriptionInput = StreamTranscriptionInput(), requestOptions: RequestOptions = RequestOptions()): Flow<StreamTranscriptionEvent> = channelFlow {",
        // The typed decode through the operation's stream item codec.
        "StreamChatMessageEvent(Codecs.streamChatResponse200.decodeUsing(envelope, frame, \"\")",
        // The declared id metadata is exposed per event.
        "id = (envelope.values[\"id\"] as? JsonString)?.value",
        // The declared sentinel completes the stream before any payload
        // decoding, preserving the final usage frame.
        "if (data == \"[DONE]\") throw StreamTranscriptionTerminal(StreamTranscriptionCompletion(StreamTranscriptionCompletionReason.Sentinel, held))",
        // The final completion element is the flow's documented terminal.
        "send(StreamChatCompletionEvent(terminal ?: StreamChatCompletion(StreamChatCompletionReason.Eof, held)))",
        // The direct call and its untyped item flow stay exactly as before.
        "public fun streamChat(input: StreamChatInput = StreamChatInput(), requestOptions: RequestOptions = RequestOptions()): Flow<StreamChatResult> = channelFlow {",
    ] {
        assert!(
            client.contains(expected),
            "Client.kt lacks:\n{expected}\n--- emitted: ---\n{client}"
        );
    }
    assert!(!client.contains("streamLogsEvents"));
    assert!(!client.contains("streamRowsEvents"));

    // The emitted package typechecks when the JVM toolchain is available.
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_stream_typed: no JDK 21 found; emission assertions only");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_stream_typed: no Maven found; emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("kotlin_stream_typed: maven test-compile succeeded");
}

#[test]
fn control_operations_without_discrimination_emit_nothing() {
    let files = generate(untyped_document());
    assert!(
        !files
            .iter()
            .any(|file| file.path.ends_with("StreamEvents.kt"))
    );
    let client = file(&files, "Client.kt").content.clone();
    for absent in [
        "streamLogsEvents",
        "streamRowsEvents",
        "StreamLogsEvent",
        "StreamRowsEvent",
        "CompletionEvent",
    ] {
        assert!(!client.contains(absent), "no-policy client gained {absent}");
    }
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
    let plan = suspect_codegen::kotlin_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::kotlin_sdk::SdkConfig {
            group_id: "test.suspect".into(),
            artifact_id: "stream-events-kotlin".into(),
            version: "0.1.0".into(),
            package_name: "test.suspect.stream_events_kotlin".into(),
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
        .find(|entry| entry.method == "streamChat")
        .expect("typed stream operation");
    assert_eq!(chat.kinds, vec!["message".to_owned(), "done".to_owned()]);
    assert!(chat.sentinel.is_none());
    assert!(chat.id_metadata);
    assert!(!chat.retry_metadata);
    assert_eq!(chat.events_method, "streamChatEvents");
    assert_eq!(chat.event_type, "StreamChatEvent");
    assert_eq!(chat.completion_type, "StreamChatCompletion");
    let transcribe = events
        .operations
        .iter()
        .find(|entry| entry.method == "streamTranscription")
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

/// Native behavioral verification of the emitted typed-events flows against a
/// scripted streaming transport: typed decode, unknown kind, branded decoding
/// failure, sentinel completion preserving usage with no further reads, early
/// collection end, and the untyped item flow unchanged.
#[test]
fn native_typed_events_drive_scripted_streams() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_stream_typed: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_stream_typed: no Maven found; degrading to static assertions");
        return;
    };
    let files = generate(stream_document());
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, root.path()).unwrap();
    std::fs::create_dir_all(
        root.path()
            .join("kotlin/src/test/kotlin/test/suspect/stream_events_kotlin"),
    )
    .unwrap();
    std::fs::write(
        root.path().join(
            "kotlin/src/test/kotlin/test/suspect/stream_events_kotlin/StreamEventsBehavior.kt",
        ),
        BEHAVIOR,
    )
    .unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the behavioral probe\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let executable = java_home.join("bin/java");
    let classpath = format!(
        "{}:{}",
        root.path().join("kotlin/target/classes").display(),
        root.path().join("kotlin/target/test-classes").display()
    );
    let dependencies = Command::new(&maven)
        .args([
            "-q",
            "-B",
            "dependency:build-classpath",
            "-Dmdep.outputFile=cp.txt",
        ])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        dependencies.status.success(),
        "classpath build failed\n{}{}",
        String::from_utf8_lossy(&dependencies.stdout),
        String::from_utf8_lossy(&dependencies.stderr)
    );
    let deps = std::fs::read_to_string(root.path().join("kotlin/cp.txt")).unwrap();
    let run = Command::new(&executable)
        .arg("-Xmx512m")
        .arg("-cp")
        .arg(format!("{classpath}:{deps}"))
        .arg("test.suspect.stream_events_kotlin.StreamEventsBehaviorKt")
        .output()
        .unwrap();
    std::fs::write(
        root.path().join("behavior.log"),
        format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        ),
    )
    .unwrap();
    assert!(
        run.status.success(),
        "behavioral probe failed\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

const BEHAVIOR: &str = r#"package test.suspect.stream_events_kotlin

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking

/** A scripted streaming transport counting requests, reads and closures in
 * three-byte chunks, so no further reads are observable. With `endless`, the
 * body keeps cycling its frames so a producer that keeps reading after the
 * collector stops is observable as a growing read count. */
private class ScriptedStream(private val frames: List<String>, private val endless: Boolean = false) : StreamingTransport {
    val requests = mutableListOf<String>()
    var reads = 0
    var closed = false

    override suspend fun open(request: HttpRequest): StreamingResponse {
        requests.add(request.url.toString())
        val bytes = frames[requests.size - 1].toByteArray()
        var offset = 0
        val body = object : BodyReader {
            override suspend fun read(): ByteArray? {
                if (!endless && offset >= bytes.size) return null
                reads++
                val end = minOf(offset + 3, bytes.size)
                val chunk = bytes.copyOfRange(offset, end)
                offset = if (endless && end == bytes.size) 0 else end
                return chunk
            }
            override fun close() { closed = true }
        }
        return StreamingResponse(200, mapOf("Content-Type" to listOf("text/event-stream")), body)
    }
}

private fun expect(condition: Boolean, message: String) {
    check(condition) { "failed: $message" }
}

/** A declared message event decodes to the typed model; a declared done event
 * follows; the completion is the flow's final element with an eof reason. */
private fun typedDecodeAndCompletion() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamChatEvents().toList()
        expect(events.size == 3, "two events plus the terminal completion")
        val message = events[0] as StreamChatMessageEvent
        expect(message.kind == "message", "declared message kind")
        expect(message.data.data == "{\"text\":\"hello\"}", "decoded envelope data")
        val done = events[1] as StreamChatDoneEvent
        expect(done.kind == "done", "declared done kind")
        val completion = events[2] as StreamChatCompletionEvent
        expect(completion.completion.reason == StreamChatCompletionReason.Eof, "eof completion")
        expect(completion.completion.usage == null, "no usage is preserved without the policy")
    }
    expect(transport.requests.size == 1, "one request")
    expect(transport.closed, "a completed stream closes its body")
}

/** Per-item metadata: the declared id field is exposed on the typed event. */
private fun metadataId() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\nid: 42\ndata: hello\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamChatEvents().toList()
        val message = events[0] as StreamChatMessageEvent
        expect(message.id == "42", "the id metadata is exposed")
    }
}

/** An undeclared event kind surfaces through the typed unknown alternative
 * without failing the stream, and later declared events still decode. */
private fun unknownKindDoesNotFail() = runBlocking {
    val transport = ScriptedStream(listOf("event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamChatEvents().toList()
        expect(events.size == 3, "unknown, declared, completion")
        val unknown = events[0] as StreamChatUnknownEvent
        expect(unknown.event == "surprise", "the undeclared wire kind is carried")
        expect(unknown.data == "hello", "the raw frame data is carried")
        val known = events[1] as StreamChatMessageEvent
        expect(known.data.data == "ok", "the declared kind still decodes")
    }
}

/** An invalid payload for a recognized kind remains a branded decoding error:
 * the frame's envelope lacks the required event field. */
private fun invalidPayloadIsBranded() = runBlocking {
    val transport = ScriptedStream(listOf("data: hello\n\n"))
    Client(transport = transport).use { client ->
        try {
            client.streamChatEvents().first()
            throw AssertionError("expected a decoding failure")
        } catch (error: SdkException) {
            expect(error.kind == FailureKind.RESPONSE_VALIDATION, "the failure is branded: ${error.kind}")
        }
    }
    expect(transport.closed, "a decoding failure closes the body")
}

/** The declared [DONE] sentinel completes the stream before any payload
 * decoding, preserves the final usage frame as terminal metadata, and issues
 * no further reads: the post-sentinel frame is never decoded. */
private fun sentinelCompletesAndPreservesUsage() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\nevent: message\ndata: never\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamTranscriptionEvents().toList()
        expect(events.size == 1, "the sentinel completes without yielding the held frame")
        val completion = events[0] as StreamTranscriptionCompletionEvent
        expect(completion.completion.reason == StreamTranscriptionCompletionReason.Sentinel, "sentinel reason")
        val usage = completion.completion.usage
        expect(usage is StreamTranscriptionMessageEvent && usage.data.data == "{\"tokens\": 42}", "the usage frame carries the decoded tokens")
    }
    expect(transport.closed, "the sentinel completes and closes the body")
    // 3-byte chunks: the sentinel frame dispatches exactly at byte 51, the
    // last byte of the 17th chunk, so reads stop there and never reach the
    // post-sentinel frame's later chunks.
    expect(transport.reads == 17, "reads stop at the sentinel frame: ${transport.reads}")
}

/** Earlier frames are still yielded when a sentinel follows them. */
private fun sentinelAfterEarlierEvents() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamTranscriptionEvents().toList()
        expect(events.size == 2, "one event plus the terminal completion")
        val first = events[0] as StreamTranscriptionMessageEvent
        expect(first.data.data == "a", "earlier frames are yielded")
        val completion = events[1] as StreamTranscriptionCompletionEvent
        val usage = completion.completion.usage
        expect(usage is StreamTranscriptionMessageEvent && usage.data.data == "{\"usage\": true}", "the last frame before the sentinel is the terminal usage")
    }
}

/** Early collection end stops consumption: one request, a closed body and no
 * further reads once the collector's cancellation reaches the producer. */
private fun earlyCollectionEnd() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\ndata: a\n\nevent: message\ndata: b\n\n"), endless = true)
    Client(transport = transport).use { client ->
        val taken = client.streamChatEvents().take(1).toList()
        expect(taken.size == 1, "early end delivered exactly one event")
    }
    expect(transport.requests.size == 1, "early end issued no second request")
    // The collector's cancellation reaches the flow's producer asynchronously,
    // so reads stop once the body closes and never resume afterwards.
    val deadline = System.nanoTime() + 2_000_000_000L
    while (!transport.closed && System.nanoTime() < deadline) Thread.sleep(5)
    expect(transport.closed, "early end closes the body")
    val settled = transport.reads
    Thread.sleep(50)
    expect(transport.reads == settled, "early end kept reading the body: ${transport.reads}")
}

/** The untyped direct call keeps its exact previous behavior. */
private fun untypedPathUnchanged() = runBlocking {
    val transport = ScriptedStream(listOf("event: message\ndata: a\n\nevent: message\ndata: b\n\n"))
    Client(transport = transport).use { client ->
        val events = client.streamChat().toList()
        expect(events.size == 2, "two untyped items")
        expect((events[0] as StreamChatResult.Status200).data.data == "a", "first untyped item")
        expect((events[1] as StreamChatResult.Status200).data.data == "b", "second untyped item")
    }
}

fun main() {
    typedDecodeAndCompletion()
    metadataId()
    unknownKindDoesNotFail()
    invalidPayloadIsBranded()
    sentinelCompletesAndPreservesUsage()
    sentinelAfterEarlierEvents()
    earlyCollectionEnd()
    untypedPathUnchanged()
    println("typed stream behavior verified")
}
"#;

fn java_home() -> Option<std::path::PathBuf> {
    if let Some(home) =
        std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME").or_else(|| std::env::var_os("JAVA_HOME"))
    {
        return Some(std::path::PathBuf::from(home));
    }
    // The mise-managed Temurin install used by this workspace.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/java/temurin-21");
    let executable = home.join("bin/java");
    Command::new(&executable)
        .arg("-version")
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(home))
}

fn maven() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_KOTLIN_MAVEN") {
        return Some(std::path::PathBuf::from(path));
    }
    if Command::new("mvn")
        .arg("--version")
        .output()
        .is_ok_and(|probe| probe.status.success())
    {
        return Some(std::path::PathBuf::from("mvn"));
    }
    // mise-managed Maven installs carry a nested distribution directory.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/maven");
    let mut candidates = match std::fs::read_dir(&home) {
        Ok(installs) => installs
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| {
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .filter_map(|distribution| distribution.ok())
                    .map(|distribution| distribution.path().join("bin/mvn"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    candidates.sort();
    candidates.into_iter().find(|path| path.is_file())
}
