//! Emitted-only typed SSE events for the Java HTTP backend: one generated
//! `StreamEvents.java` with closeable per-operation event iterators, sealed
//! per-kind events, the typed unknown alternative and the terminal completion,
//! a javac compile gate over the whole package, and native behavior against a
//! stubbed `HttpClient`. Static runtime files are never modified, and plans
//! without a discriminated SSE operation emit no new file at all.
#![cfg(all(feature = "java-sdk", feature = "http-protocol"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// Two discriminated SSE streams (one with a declared `[DONE]` sentinel via its
/// description and declared envelope metadata) and two controls: an SSE
/// envelope without discrimination evidence and a JSON-lines item schema. The
/// chat request body carries an explicit empty schema, which the compiled
/// validation program keeps as a root that admits every framed envelope for
/// the lenient envelope reader.
fn stream_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Typed streams", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/chat": {"post": {
                "operationId": "streamChat",
                "requestBody": {"required": true, "content": {"application/json": {"schema": {}}}},
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

fn document_with_paths(paths: Value) -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Streams", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": paths
    })
}

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.streams.test/java-streams.json").unwrap();
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

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "test.suspect:stream-java".into(),
            package_version: "1.0.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn generate() -> Vec<OutFile> {
    generate_document(stream_document())
}

fn stream_events_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "java/src/main/java/test/suspect/StreamEvents.java")
        .expect("generated StreamEvents.java")
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "java/src/main/java/test/suspect/Client.java")
        .expect("generated client")
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn discriminated_sse_operations_emit_typed_event_iterators() {
    let files = generate();
    let source = stream_events_file(&files).content.clone();
    for expected in [
        // The lenient envelope reader: the only new codec, bound to a program
        // root that admits every framed envelope.
        "static final ModelCodec<JsonValue> RAW_ENVELOPE = new ModelCodec<>(",
        "(value,context)->value,(value,context)->value);",
        // Per-operation API: factory, closeable iterator over the sealed event
        // set, per-kind records with the declared metadata, the typed unknown
        // alternative and the terminal completion.
        "public static StreamChatEvents streamChatEvents(Client client,Client.StreamChatInput input)",
        "public static final class StreamChatEvents implements java.util.Iterator<StreamChatEvent>, AutoCloseable {",
        "public sealed interface StreamChatEvent {",
        "public record StreamChatMessageEvent(String kind,",
        ",String id) implements StreamChatEvent {}",
        "public record StreamChatDoneEvent(String kind,",
        "public record StreamChatUnknownEvent(String event,String data) implements StreamChatEvent {",
        "@Override public String kind() { return \"unknown\"; }",
        "public record StreamChatCompletion(String reason,StreamChatEvent usage) {}",
        "public static StreamTranscriptionEvents streamTranscriptionEvents(Client client,Client.StreamTranscriptionInput input)",
        "public static final class StreamTranscriptionEvents implements java.util.Iterator<StreamTranscriptionEvent>, AutoCloseable {",
        // Pull semantics: the request starts inside hasNext(); close stops
        // consumption; the completion carries the terminal metadata.
        "if (closed || done) return false;",
        "envelopes = client.streamChatEventsStream(input,RequestOptions.defaults());",
        "if (!hasNext()) throw new java.util.NoSuchElementException(\"the typed event stream has ended\");",
        "if (envelopes != null) envelopes.close();",
        "public StreamChatCompletion completion() {",
        // Recognized kinds decode through the operation's existing stream item codec.
        ".CODEC.decodeValue(envelope)",
        // The declared sentinel completes the stream before any payload
        // decoding and preserves the final usage frame.
        "if (data.equals(\"[DONE]\")) {",
        "completion = new StreamTranscriptionCompletion(\"sentinel\",held);",
        "if (held != null) {",
        "completion = new StreamTranscriptionCompletion(\"eof\",held);",
        "completion = new StreamChatCompletion(\"eof\",null);",
        // Undeclared kinds keep the raw frame representable.
        "typed = new StreamChatUnknownEvent(kind,data);",
    ] {
        assert!(
            source.contains(expected),
            "StreamEvents.java is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
    // The exchange seam shares the direct method's request preparation.
    let client = client_file(&files).content.clone();
    for expected in [
        "EventStream<JsonValue> streamChatEventsStream(Client.StreamChatInput input,RequestOptions options)",
        "if(raw.responseIndex!=0||raw.mediaIndex!=0)throw raw.failure(\"unexpected-response\");",
        "return raw.stream(Protocol.object(",
        "StreamEvents.RAW_ENVELOPE);",
    ] {
        assert!(
            client.contains(expected),
            "Client.java is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }
    // Controls without discrimination emit nothing.
    assert!(!client.contains("streamLogsEventsStream"));
    assert!(!client.contains("streamRowsEventsStream"));
    assert!(!source.contains("StreamLogsEvent"));

    // The plan carries the compiled stream semantics for every stream media.
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::java_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::java_sdk::PackageConfig::default(),
        &[],
    )
    .unwrap();
    let semantics = plan.stream_semantics();
    assert_eq!(semantics.streams.len(), 4);
    let chat = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamChat")
        .unwrap();
    assert_eq!(
        chat.events
            .iter()
            .map(|event| event.event_name.as_str())
            .collect::<Vec<_>>(),
        vec!["message", "done"]
    );
    assert!(!chat.sentinel.enabled);
    let transcribe = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == "streamTranscription")
        .unwrap();
    assert!(transcribe.sentinel.enabled);
    assert_eq!(transcribe.sentinel.token, "[DONE]");
    assert!(transcribe.terminal.keep_final_usage);
    assert_eq!(
        transcribe
            .item_metadata
            .as_ref()
            .unwrap()
            .event_field
            .as_deref(),
        Some("event")
    );
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn control_operations_without_discrimination_keep_the_untyped_package() {
    let paths = json!({
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
    });
    let control = generate_document(document_with_paths(paths.clone()));
    let typed = generate();
    // The control document never emits the typed-events file, and its client
    // carries no exchange seam.
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("StreamEvents.java"))
    );
    assert!(!client_file(&control).content.contains("EventsStream"));
    assert!(
        !client_file(&typed)
            .content
            .contains("streamLogsEventsStream")
    );
    assert!(
        !client_file(&typed)
            .content
            .contains("streamRowsEventsStream")
    );
    // The static runtime is untouched by typed emission: every shared runtime
    // file is byte-identical between the two documents.
    for name in [
        "JsonRuntime.java",
        "Presence.java",
        "ModelCodec.java",
        "CodecException.java",
        "HttpRuntime.java",
        "SdkException.java",
        "Protocol.java",
        "HttpWire.java",
        "WireValue.java",
        "WireCodec.java",
        "EventStream.java",
        "RequestOptions.java",
        "ExactHttp.java",
    ] {
        let path = format!("java/src/main/java/test/suspect/{name}");
        let control_file = control
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} missing"));
        let typed_file = typed
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} missing"));
        assert_eq!(typed_file.content, control_file.content, "{name} changed");
    }
    // The compiled stream plan still exists on the plan, conservative.
    let contract = contract_with_document(document_with_paths(paths.clone()));
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::java_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::java_sdk::PackageConfig::default(),
        &[],
    )
    .unwrap();
    let semantics = plan.stream_semantics();
    assert_eq!(semantics.streams.len(), 2);
    assert!(
        semantics
            .streams
            .iter()
            .all(|stream| stream.events.len() == 1)
    );
}

/// A JDK 21+ toolchain, mirroring the other Java acceptance tests.
fn java_home() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}

/// Compile the whole emitted package; the classes directory doubles as the
/// behavioral probe's classpath, including the generated resources.
fn compiled_package(root: &Path) -> PathBuf {
    let home = java_home();
    assert!(
        home.join("bin/javac").is_file(),
        "required JDK: {}",
        home.display()
    );
    let mut sources = fs::read_dir(root.join("java/src/main/java/test/suspect"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("java"))
        .collect::<Vec<_>>();
    sources.sort();
    let classes = root.join("classes");
    fs::create_dir_all(&classes).unwrap();
    let list = root.join("sources.txt");
    fs::write(
        &list,
        sources
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-d"])
        .arg(&classes)
        .arg(format!("@{}", list.display()))
        .current_dir(root)
        .output()
        .unwrap();
    fs::write(root.join("javac.stdout.log"), &output.stdout).unwrap();
    fs::write(root.join("javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for entry in fs::read_dir(root.join("java/src/main/resources/test/suspect"))
        .unwrap()
        .flatten()
    {
        fs::copy(
            entry.path(),
            classes.join("test/suspect").join(entry.file_name()),
        )
        .unwrap();
    }
    classes
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn generated_stream_events_compile_strictly_with_the_package() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(), root.path()).unwrap();
    compiled_package(root.path());
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn typed_stream_events_drive_stubbed_streams_in_java() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(), root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("StreamEventsProbe.java"), PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("StreamEventsProbe.java"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe-javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut classpath = classes.into_os_string();
    classpath.push(":");
    classpath.push(root.path().join("probe").into_os_string());
    let runtime = Command::new(home.join("bin/java"))
        .args(["-ea", "-cp"])
        .arg(&classpath)
        .arg("StreamEventsProbe")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe.stdout.log"), &runtime.stdout).unwrap();
    fs::write(root.path().join("probe.stderr.log"), &runtime.stderr).unwrap();
    assert!(
        runtime.status.success(),
        "{}{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const PROBE: &str = r#"
import java.net.*;
import java.net.http.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import javax.net.ssl.*;

import test.suspect.*;
import static test.suspect.JsonRuntime.*;

/** Independent typed-events acceptance over a stubbed JDK transport. */
public class StreamEventsProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static final class Stub extends HttpClient {
        final List<String> requests = Collections.synchronizedList(new ArrayList<>());
        final AtomicInteger cancellations = new AtomicInteger();
        private final List<String> chunks;
        Stub(String frames) {
            // Deliver in small chunks so the runtime framing is exercised.
            List<String> pieces = new ArrayList<>();
            for (int at = 0; at < frames.length(); at += 4) {
                pieces.add(frames.substring(at, Math.min(at + 4, frames.length())));
            }
            this.chunks = List.copyOf(pieces);
        }
        @Override public Optional<CookieHandler> cookieHandler() { return Optional.empty(); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return Redirect.NEVER; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (Exception error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.empty(); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) throws InterruptedException, java.io.IOException {
            try { return sendAsync(request, handler).get(); }
            catch (ExecutionException error) { throw new java.io.IOException("stub failure"); }
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) { return sendAsync(request, handler); }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            requests.add(request.uri().toString());
            HttpHeaders headers = HttpHeaders.of(Map.of("Content-Type", List.of("text/event-stream")), (key, value) -> true);
            HttpResponse.BodySubscriber<T> subscriber = handler.apply(new HttpResponse.ResponseInfo() {
                @Override public int statusCode() { return 200; }
                @Override public HttpHeaders headers() { return headers; }
                @Override public Version version() { return Version.HTTP_1_1; }
            });
            CompletableFuture<HttpResponse<T>> result = new CompletableFuture<>();
            subscriber.onSubscribe(new Flow.Subscription() {
                int delivered = 0;
                boolean cancelled;
                @Override public void request(long count) {
                    if (cancelled || delivered > 0) return;
                    delivered = 1;
                    List<ByteBuffer> buffers = new ArrayList<>();
                    for (String piece : chunks) buffers.add(ByteBuffer.wrap(piece.getBytes(StandardCharsets.UTF_8)));
                    subscriber.onNext(buffers);
                    subscriber.onComplete();
                }
                @Override public void cancel() { cancelled = true; cancellations.incrementAndGet(); }
            });
            subscriber.getBody().whenComplete((body, error) -> {
                if (error != null) result.completeExceptionally(error);
                else result.complete(new Response<>(request, 200, headers, body));
            });
            return result;
        }
        /** Waits for the runtime to cancel the response subscription. */
        void eventuallyCancelled() throws InterruptedException {
            long end = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
            while (cancellations.get() == 0 && System.nanoTime() < end) Thread.sleep(2);
            check(cancellations.get() > 0, "the response subscription was cancelled");
        }
    }

    private record Response<T>(HttpRequest request, int statusCode, HttpHeaders headers, T body) implements HttpResponse<T> {
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public Optional<SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }

    private static Client client(Stub stub) {
        return new Client(HttpRuntime.Options.builder()
            .httpClient(stub)
            .serverUrl(URI.create("https://api.streams.test/v1"))
            .build());
    }

    private static Client.StreamChatInput chatInput() {
        return Client.StreamChatInput.builder(new JsonObject(Map.of("prompt", new JsonString("hi"))))
            .build();
    }

    public static void main(String[] args) throws Exception {
        // A declared message event decodes to the typed model with its
        // declared metadata; a declared done event follows; the completion
        // reports end-of-body with no usage.
        Stub stub = new Stub("event: message\nid: 42\ndata: hello\n\nevent: done\ndata: {}\n\n");
        try (Client client = client(stub)) {
            try (StreamEvents.StreamChatEvents walk = StreamEvents.streamChatEvents(client, chatInput())) {
                List<StreamEvents.StreamChatEvent> events = new ArrayList<>();
                for (java.util.Iterator<StreamEvents.StreamChatEvent> iterator = walk; iterator.hasNext();) {
                    events.add(iterator.next());
                }
                check(events.size() == 2, "two typed events: " + events.size());
                check(events.get(0).kind().equals("message"), "message kind: " + events.get(0).kind());
                StreamEvents.StreamChatMessageEvent message = (StreamEvents.StreamChatMessageEvent) events.get(0);
                check(message.data().data().equals("hello"), "typed payload: " + message.data().data());
                check("42".equals(message.id()), "declared id metadata: " + message.id());
                check(events.get(1).kind().equals("done"), "done kind: " + events.get(1).kind());
                StreamEvents.StreamChatCompletion completion = walk.completion();
                check(completion != null, "completion present after the stream ends");
                check(completion.reason().equals("eof"), "eof reason: " + completion.reason());
                check(completion.usage() == null, "eof preserves no usage without the policy");
            }
            check(stub.requests.size() == 1, "one request: " + stub.requests.size());
        }

        // An undeclared event kind surfaces through the typed unknown
        // alternative without failing the stream, and later declared
        // events still decode.
        stub = new Stub("event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n");
        try (Client unknownClient = client(stub);
             StreamEvents.StreamChatEvents walk = StreamEvents.streamChatEvents(unknownClient, chatInput())) {
            StreamEvents.StreamChatEvent first = walk.next();
            check(first.kind().equals("unknown"), "unknown kind: " + first.kind());
            StreamEvents.StreamChatUnknownEvent unknown = (StreamEvents.StreamChatUnknownEvent) first;
            check(unknown.event().equals("surprise") && unknown.data().equals("hello"), "unknown carries the raw frame");
            StreamEvents.StreamChatEvent second = walk.next();
            check(second.kind().equals("message") && ((StreamEvents.StreamChatMessageEvent) second).data().data().equals("ok"),
                "later declared events still decode");
        }

        // An invalid payload for a recognized kind remains a decoding
        // error branded like the untyped stream path.
        stub = new Stub("data: hello\n\n");
        try (Client invalidClient = client(stub)) {
            try (StreamEvents.StreamChatEvents walk = StreamEvents.streamChatEvents(invalidClient, chatInput())) {
                walk.next();
                throw new AssertionError("expected a decoding failure");
            } catch (SdkException error) {
                check(error.kind().equals("invalid-stream-item"), "invalid payload kind: " + error.kind());
            }
        }

        // The declared [DONE] sentinel completes the stream before any
        // payload decoding, preserves the final usage frame, and issues
        // no further reads.
        stub = new Stub("event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\nevent: message\ndata: never\n\n");
        try (Client sentinelClient = client(stub)) {
            try (StreamEvents.StreamTranscriptionEvents walk = StreamEvents.streamTranscriptionEvents(sentinelClient,
                    Client.StreamTranscriptionInput.builder().build())) {
                List<StreamEvents.StreamTranscriptionEvent> events = new ArrayList<>();
                for (java.util.Iterator<StreamEvents.StreamTranscriptionEvent> iterator = walk; iterator.hasNext();) {
                    events.add(iterator.next());
                }
                check(events.isEmpty(), "the final usage frame is preserved, not yielded: " + events.size());
                StreamEvents.StreamTranscriptionCompletion completion = walk.completion();
                check(completion != null && completion.reason().equals("sentinel"), "sentinel reason");
                check(completion.usage() != null
                    && ((StreamEvents.StreamTranscriptionMessageEvent) completion.usage()).data().data().equals("{\"tokens\": 42}"),
                    "the last frame before the sentinel is the usage");
            }
            check(stub.requests.size() == 1, "the sentinel issues one request: " + stub.requests.size());
            stub.eventuallyCancelled();
        }

        // Earlier frames are still yielded when a sentinel follows them,
        // and the last frame before it becomes the terminal usage.
        stub = new Stub("event: message\ndata: a\n\nevent: message\ndata: b\n\ndata: [DONE]\n\n");
        try (Client yieldClient = client(stub)) {
            try (StreamEvents.StreamTranscriptionEvents walk = StreamEvents.streamTranscriptionEvents(yieldClient,
                    Client.StreamTranscriptionInput.builder().build())) {
                StreamEvents.StreamTranscriptionEvent first = walk.next();
                check(first.kind().equals("message")
                    && ((StreamEvents.StreamTranscriptionMessageEvent) first).data().data().equals("a"),
                    "earlier frames are yielded");
                check(!walk.hasNext(), "the sentinel ends the walk");
                StreamEvents.StreamTranscriptionCompletion completion = walk.completion();
                check(completion != null && completion.reason().equals("sentinel"), "sentinel reason");
                check(completion.usage() != null
                    && ((StreamEvents.StreamTranscriptionMessageEvent) completion.usage()).data().data().equals("b"),
                    "the usage is the final frame");
            }
        }

        // Early break stops consumption: one event, one request, and a
        // cancelled response; close() before the first hasNext() issues no
        // request at all.
        stub = new Stub("event: message\ndata: a\n\nevent: message\ndata: b\n\n");
        try (Client breakClient = client(stub)) {
            try (StreamEvents.StreamChatEvents walk = StreamEvents.streamChatEvents(breakClient, chatInput())) {
                for (java.util.Iterator<StreamEvents.StreamChatEvent> iterator = walk; iterator.hasNext();) {
                    iterator.next();
                    break;
                }
            }
            check(stub.requests.size() == 1, "an early break issues one request: " + stub.requests.size());
            breakClient2(stub);
        }

        // The untyped direct call keeps its exact previous behavior:
        // strictly decoded envelope items for well-formed frames.
        stub = new Stub("event: message\ndata: a\n\nevent: message\ndata: b\n\n");
        try (Client directClient = client(stub)) {
            List<String> items = new ArrayList<>();
            try (Client.StreamChatStatus200 response = directClient.streamChat(chatInput())) {
                for (var item : response.data()) items.add(item.data());
            }
            check(items.equals(List.of("a", "b")), "the untyped item stream is unchanged: " + items);
        }

        System.out.println("typed stream behavior verified");
    }

    /** close() before the first hasNext() issues no request at all. */
    private static void breakClient2(Stub stub) {
        try (Client closed = client(stub)) {
            StreamEvents.StreamChatEvents walk = StreamEvents.streamChatEvents(closed, chatInput());
            walk.close();
            check(stub.requests.size() == 1, "close before the first hasNext() issues no request");
        }
    }
}
"#;
