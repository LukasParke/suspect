//! Generated typed SSE events for the Swift package: the per-operation event
//! sequences, strict compilation, and native behavior over a scripted
//! event-stream transport. Static runtime files are never modified;
//! operations without a discriminated stream schema emit nothing at all, and
//! the direct operation calls plus their untyped HTTPEventStream iteration
//! stay unchanged.
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

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.streams.test/swift-typed-streams.json").unwrap();
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
            package_name: "TypedStreamsSDK".into(),
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
fn discriminated_sse_operations_emit_typed_event_sequences() {
    let files = generate(stream_document());
    let events = file(&files, "StreamEvents.swift").content.clone();
    let operations = file(&files, "Operations.swift").content.clone();
    let client = file(&files, "Client.swift").content.clone();

    // The typed event enum, completion carrier and pull-based sequence for the
    // discriminated streamChat operation, with its declared metadata member.
    for expected in [
        "public enum StreamChatEvent: Sendable, Equatable {",
        "case message(data: ",
        "case done(data: ",
        "case unknown(event: String, data: String)",
        "public struct StreamChatCompletion: Sendable {",
        "public enum Reason: Sendable, Equatable {",
        "case sentinel",
        "case eof",
        "public let usage: StreamChatEvent?",
        "public struct StreamChatEventSequence: AsyncSequence, AsyncIteratorProtocol, Sendable {",
        "public var completion: StreamChatCompletion? { terminal.value }",
        "func streamChatEventsRequest(_ input: StreamChatInput, options requestOptions: RequestOptions) async throws -> (HTTPStreamResponse, Int)",
        "public func streamChatEvents(_ input: StreamChatInput = .init(), options requestOptions: RequestOptions = .init()) -> StreamChatEventSequence",
        // Declared envelope metadata (id) is exposed per event.
        "id: item.id.valueIfPresent",
        // The declared sentinel completes the stream before any payload
        // decoding and preserves the final usage frame.
        "terminal.complete(StreamChatCompletion(reason: .sentinel, usage: held))",
        "terminal.complete(StreamChatCompletion(reason: .eof, usage: keepFinalUsage ? held : nil))",
        // The framed envelope decodes through the operation's existing stream
        // item codec.
        "try Codecs.",
        ".decodeValue(value, limits: JsonLimits(maxBytes: itemLimit))",
        // The runtime's own framer, lease and byte ceilings are reused.
        "HTTPFramer(framing: .serverSentEvents, limit: ",
        "HTTPStreamLease { response.close() }",
    ] {
        assert!(
            events.contains(expected),
            "StreamEvents.swift lacks:\n{expected}\n--- emitted: ---\n{events}"
        );
    }
    // The sequence emits exactly two typed stream operations.
    assert!(events.contains("StreamTranscriptionEventSequence"));
    assert!(!events.contains("StreamLogsEvent"));
    assert!(!events.contains("StreamRowsEvent"));

    // The direct operation calls and their untyped HTTPEventStream iteration
    // stay exported and unchanged: no event member leaks into them.
    assert!(operations.contains("public func streamChat(_ input: StreamChatInput"));
    assert!(operations.contains("HTTPBuild.eventStream(stream, codec: Codecs."));
    assert!(!operations.contains("streamChatEvents"));
    assert!(!client.contains("streamChatEvents"));
}

#[test]
fn control_operations_without_discrimination_keep_the_untyped_package() {
    let document = json!({
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
    });
    let files = generate(document.clone());
    assert!(
        !files
            .iter()
            .any(|file| file.path.ends_with("StreamEvents.swift")),
        "no-policy output must not carry typed stream helpers"
    );
    for file in &files {
        assert!(
            !file.content.contains("EventSequence"),
            "{} changed",
            file.path
        );
        assert!(
            !file.content.contains("StreamEventTerminal"),
            "{} changed",
            file.path
        );
    }
    // The compiled stream plan is still carried, with the documented
    // conservative single default events, but no operation is emittable.
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap();
    let events = plan.stream_events().expect("stream media are declared");
    assert_eq!(events.outcome.streams.len(), 2);
    assert!(events.operations.is_empty());
}

#[test]
fn plan_carries_the_compiled_stream_semantics_and_emittable_operations() {
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap();
    let events = plan.stream_events().expect("stream media are declared");
    assert_eq!(events.outcome.streams.len(), 4);
    let chat = events
        .outcome
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
    let transcribe = events
        .outcome
        .streams
        .iter()
        .find(|stream| stream.operation == "streamTranscription")
        .unwrap();
    assert!(transcribe.sentinel.enabled);
    assert_eq!(transcribe.sentinel.token, "[DONE]");
    assert!(transcribe.terminal.keep_final_usage);
    // Only the discriminated SSE operations are emittable, in plan order.
    assert_eq!(
        events
            .operations
            .iter()
            .map(|operation| operation.operation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["streamChat", "streamTranscription"]
    );
    assert_eq!(events.operations[0].events_method, "streamChatEvents");
    assert_eq!(events.operations[0].event_type, "StreamChatEvent");
    assert_eq!(
        events.operations[0].sequence_type,
        "StreamChatEventSequence"
    );
    assert_eq!(events.operations[0].completion_type, "StreamChatCompletion");
    assert!(events.operations[1].sentinel.is_some());
    assert!(events.operations[1].keep_final_usage);
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
import TypedStreamsSDK

/// Scripted transport: records requests and replays one framed SSE body in
/// bounded chunks. An endless body never terminates on its own, so only the
/// runtime's cancellation can stop it.
final class BodyState: @unchecked Sendable {
    let chunks: [Data]
    let endless: Bool
    var index = 0
    var reads = 0
    var closed = false

    init(_ frames: String, endless: Bool) {
        self.endless = endless
        let bytes = Array(frames.utf8)
        var chunks: [Data] = []
        var at = 0
        while at < bytes.count {
            let end = min(at + 3, bytes.count)
            chunks.append(Data(bytes[at..<end]))
            at = end
        }
        self.chunks = chunks
    }
}

actor ScriptedTransport: HTTPTransport {
    let state: BodyState
    private(set) var requests = 0

    init(frames: String, endless: Bool = false) {
        state = BodyState(frames, endless: endless)
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests += 1
        throw TransportError.invalidURL
    }

    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        requests += 1
        let state = self.state
        return HTTPStreamResponse(
            status: 200,
            headers: [HTTPHeader("Content-Type", "text/event-stream")],
            body: HTTPByteStream(
                next: {
                    try Task.checkCancellation()
                    if state.index < state.chunks.count {
                        defer { state.index += 1 }
                        state.reads += 1
                        return state.chunks[state.index]
                    }
                    if state.endless {
                        while !Task.isCancelled {
                            try await Task.sleep(nanoseconds: 10_000_000)
                        }
                        throw CancellationError()
                    }
                    return nil
                },
                cancel: { state.closed = true }
            )
        )
    }

    func recorded() -> (Int, Int, Bool, Int) {
        (requests, state.reads, state.closed, state.index)
    }
}

final class TypedStreamTests: XCTestCase {
    func client(_ transport: ScriptedTransport) -> Client {
        Client(transport: transport)
    }

    func testDeclaredKindsDecodeToTheirTypedModels() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n"
        )
        var events: [StreamChatEvent] = []
        for try await event in client(transport).streamChatEvents() {
            events.append(event)
        }
        guard case .message(let first, _)? = events.first else {
            return XCTFail("\(events)")
        }
        XCTAssertEqual(first.data, "{\"text\":\"hello\"}")
        guard case .done(let second, _)? = events.dropFirst().first else {
            return XCTFail("\(events)")
        }
        XCTAssertEqual(second.data, "{}")
        XCTAssertEqual(events.count, 2, "the eof completion must not be yielded as an event")
        let (requests, _, _, _) = await transport.recorded()
        XCTAssertEqual(requests, 1)
    }

    func testDeclaredEnvelopeMetadataIsExposedPerEvent() async throws {
        let transport = ScriptedTransport(frames: "event: message\nid: 42\ndata: hello\n\n")
        var iterator = client(transport).streamChatEvents().makeAsyncIterator()
        guard case .message(_, let id)? = try await iterator.next() else {
            return XCTFail("expected a typed message event")
        }
        XCTAssertEqual(id, "42")
    }

    func testUnknownKindsSurfaceWithoutFailingTheStream() async throws {
        let transport = ScriptedTransport(
            frames: "event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n"
        )
        var events: [StreamChatEvent] = []
        for try await event in client(transport).streamChatEvents() {
            events.append(event)
        }
        guard case .unknown(let name, let data)? = events.first else {
            return XCTFail("\(events)")
        }
        XCTAssertEqual(name, "surprise")
        XCTAssertEqual(data, "hello")
        guard case .message(let known, _)? = events.dropFirst().first else {
            return XCTFail("\(events)")
        }
        XCTAssertEqual(known.data, "ok")
    }

    func testSentinelCompletesBeforeDecodingAndPreservesUsage() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\n",
            endless: true
        )
        let sequence = client(transport).streamTranscriptionEvents()
        var events: [StreamTranscriptionEvent] = []
        for try await event in sequence {
            events.append(event)
        }
        XCTAssertTrue(events.isEmpty, "\(events)")
        let completion = try XCTUnwrap(sequence.completion)
        XCTAssertEqual(completion.reason, .sentinel)
        guard case .message(let usage)? = completion.usage else {
            return XCTFail("\(String(describing: completion.usage))")
        }
        XCTAssertEqual(usage.data, "{\"tokens\": 42}")
        let (requests, reads, closed, _) = await transport.recorded()
        XCTAssertEqual(requests, 1)
        XCTAssertTrue(closed, "the sentinel completes and closes the body")
        let after = await transport.recorded()
        XCTAssertEqual(after.1, reads, "the sentinel issues no further reads")
    }

    func testSentinelAfterEarlierEventsStillYieldsThem() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n"
        )
        let sequence = client(transport).streamTranscriptionEvents()
        var events: [StreamTranscriptionEvent] = []
        for try await event in sequence {
            events.append(event)
        }
        XCTAssertEqual(events.count, 1)
        guard case .message(let first)? = events.first else {
            return XCTFail("\(events)")
        }
        XCTAssertEqual(first.data, "a")
        let completion = try XCTUnwrap(sequence.completion)
        XCTAssertEqual(completion.reason, .sentinel)
        guard case .message(let usage)? = completion.usage else {
            return XCTFail("\(String(describing: completion.usage))")
        }
        XCTAssertEqual(usage.data, "{\"usage\": true}")
    }

    func testEndOfBodyCompletesWithTheEofReason() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n"
        )
        let sequence = client(transport).streamChatEvents()
        for try await _ in sequence {}
        let completion = try XCTUnwrap(sequence.completion)
        XCTAssertEqual(completion.reason, .eof)
        XCTAssertNil(completion.usage, "the un-sentineled stream preserves no usage")
        let (_, _, closed, _) = await transport.recorded()
        XCTAssertTrue(closed)
    }

    func testEarlyBreakClosesTheTransferAndIssuesNoFurtherReads() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: a\n\nevent: message\ndata: b\n\n",
            endless: true
        )
        var collected: [String] = []
        for try await event in client(transport).streamChatEvents() {
            guard case .message(let payload, _) = event else {
                return XCTFail("\(event)")
            }
            collected.append(payload.data)
            break
        }
        XCTAssertEqual(collected, ["a"])
        let (requests, reads, closed, _) = await transport.recorded()
        XCTAssertEqual(requests, 1)
        XCTAssertTrue(closed, "an early break closes the body")
        let after = await transport.recorded()
        XCTAssertEqual(after.1, reads, "an early break issues no further reads")
    }

    func testInvalidPayloadsOfRecognizedKindsRemainDecodingErrors() async throws {
        // The frame's envelope lacks the required event member, so decoding it
        // as the declared model fails and the error is branded response
        // decoding with the transfer released.
        let transport = ScriptedTransport(frames: "data: hello\n\n", endless: true)
        do {
            for try await _ in client(transport).streamChatEvents() {}
            XCTFail("an invalid payload must fail the stream")
        } catch let error as SDKError {
            XCTAssertEqual(error.kind, .responseDecoding, "\(error)")
        }
        let (_, _, closed, _) = await transport.recorded()
        XCTAssertTrue(closed, "a decoding failure closes the body")
    }

    func testTheUntypedIterationIsUnchanged() async throws {
        let transport = ScriptedTransport(
            frames: "event: message\ndata: a\n\nevent: message\ndata: b\n\n"
        )
        let response = try await client(transport).streamChat()
        var items: [String] = []
        for try await item in response.data {
            items.append(item.data)
        }
        XCTAssertEqual(items, ["a", "b"])
    }
}
"##;

#[test]
fn native_typed_events_drive_stubbed_streams() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_stream_typed: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_stream_typed: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    let files = generate(stream_document());
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

    // Behavioral consumer: a scripted transport drives the typed decode,
    // unknown kinds, the sentinel completion and early break without any
    // loopback socket.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/TypedStreamsConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"TypedStreamsConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"TypedStreamsConsumer\", dependencies: [.product(name: \"TypedStreamsSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/TypedStreamsConsumer/TypedStreamsTests.swift"),
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
        "native typed stream behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_stream_typed: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}
