//! Emitted-only typed SSE events for the PHP HTTP backend: per-operation
//! `<op>Events` generator methods plus per-kind event, unknown-event and
//! completion classes on the generated `Client.php`, `php -l` lint of every
//! emitted file, and native behavior over a stubbed stream transport. Static
//! runtime files are never modified; the typed decode lives entirely inside
//! the generated package, and operations without a discriminated stream schema
//! emit no new bytes at all.
#![cfg(all(feature = "php-sdk", feature = "http-protocol"))]

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
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const NAMESPACE: &str = "StreamFixture";

/// Two discriminated SSE streams (one with a declared `[DONE]` sentinel via its
/// description, one with declared envelope metadata) and two controls: an SSE
/// envelope without discrimination evidence and a JSON-lines item schema.
fn document() -> Value {
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

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.streams.test/php-streams.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document()).unwrap(),
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

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::PhpHttp,
        package_name: "streams/php-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(NAMESPACE.into()),
    }
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "php/src/Client.php")
        .expect("generated client")
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[test]
fn discriminated_sse_operations_emit_typed_event_generators() {
    let files = generate(&GenerationOptions::default());
    let client = client_file(&files).content.clone();
    for expected in [
        // The per-operation generator and its shared request preparation.
        "public function streamChatEvents(StreamChatInput $input, ?RequestOptions $options = null): \\Generator",
        "public function streamTranscriptionEvents(StreamTranscriptionInput $input = new StreamTranscriptionInput(), ?RequestOptions $options = null): \\Generator",
        "$metadata = ProtocolData::streamChat();",
        "Protocol::exchange($metadata, $this->credentials, $this->transport, $this->options, $options, $call, $values, $payload, true)",
        // The lenient envelope reader: the existing ItemStream with a pass-through codec.
        "new ItemStream($response, 'server-sent-events', static fn (JsonValue $item): JsonValue => $item, $call,",
        // Per-kind readonly event classes with the declared metadata member.
        "final readonly class StreamChatEventMessage {",
        "final readonly class StreamChatEventDone {",
        "public readonly StreamChatResponse200Item $data,",
        "public readonly Absent|string $id,",
        "final readonly class StreamTranscriptionEventMessage {",
        // The typed unknown alternative and the completion.
        "final readonly class StreamChatEventUnknown {",
        "final readonly class StreamChatCompletion {",
        "final readonly class StreamTranscriptionCompletion {",
        // The generator return value carries the terminal metadata.
        "return new StreamChatCompletion('eof', null);",
        // The sentinel completes the stream before any payload decoding and
        // preserves the final usage frame.
        "if ($data === \"[DONE]\") { return new StreamTranscriptionCompletion('sentinel', $held); }",
        "return new StreamTranscriptionCompletion('eof', $held);",
        "if ($held !== null) { yield $held; }",
        // Recognized kinds decode through the operation's existing stream item codec.
        "$item = Codecs::fromStreamChatResponse200Item($envelope, $context);",
        // The untyped direct call and its item stream stay exactly as before.
        "$decoded = new ItemStream($response, \"server-sent-events\", static fn (JsonValue $item): StreamChatResponse200Item => Codecs::fromStreamChatResponse200Item($item, new CodecContext($call->control)), $call,",
    ] {
        assert!(
            client.contains(expected),
            "Client.php is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }
    // Declared controls and JSON lines never emit typed events.
    assert!(!client.contains("streamLogsEvents"));
    assert!(!client.contains("streamRowsEvents"));
    assert!(!client.contains("StreamLogsEvent"));

    // The plan carries the compiled stream semantics for every stream media.
    let plan = plan();
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
    let entry = Uri::parse("https://source.streams.test/php-streams.json").unwrap();
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
    let contract = Arc::new(Contract::from_workspace(&workspace, &entry).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = backend::generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    let typed = generate(&GenerationOptions::default());
    // The typed document adds no package files at all: the events live inside
    // the generated Client.php only.
    assert_eq!(sorted(&control), sorted(&typed));
    let plain = client_file(&control).content.clone();
    assert!(!plain.contains("Events"));
    assert!(!plain.contains("EventUnknown"));
    assert!(!plain.contains("Completion"));
    // The typed stream plan is still compiled and stored on the plan, with the
    // documented conservative single default events.
    assert!(
        !plain.contains("server-sent-events', static fn (JsonValue $item): JsonValue => $item")
    );
}

fn plan() -> suspect_codegen::php_sdk::Plan {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    suspect_codegen::php_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::php_sdk::PhpConfig {
            package_name: "streams/php-sdk".into(),
            package_version: "1.0.0".into(),
            namespace: NAMESPACE.into(),
            ..Default::default()
        },
    )
    .unwrap()
}

/// The sdk defaults gate must not affect typed stream emission: it is
/// conditional only on the compiled stream plan.
#[test]
fn sdk_defaults_neither_enable_nor_disable_typed_stream_emission() {
    let with_defaults = generate(&configured());
    let without_defaults = generate(&GenerationOptions::default());
    assert_eq!(
        client_file(&with_defaults).content,
        client_file(&without_defaults).content
    );
    assert!(
        client_file(&with_defaults)
            .content
            .contains("streamChatEvents")
    );
}

/// The repository's verified PHP 8.3 interpreter, when available.
fn php() -> Option<PathBuf> {
    let candidate = std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-tools/php-8.3.32/php")
        });
    Command::new(&candidate)
        .arg("-v")
        .output()
        .is_ok()
        .then_some(candidate)
}

#[test]
fn emitted_php_files_lint() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; skipping the lint check");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&GenerationOptions::default()), root.path()).unwrap();
    let mut linted = 0usize;
    for entry in fs::read_dir(root.path().join("php/src")).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("php") {
            continue;
        }
        let output = Command::new(&php).arg("-l").arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "{} failed to lint:\n{}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        linted += 1;
    }
    assert!(
        linted > 10,
        "expected the generated package to lint: {linted}"
    );
}

#[test]
fn typed_stream_events_drive_stubbed_streams_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&GenerationOptions::default());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    fs::write(root.path().join("behavior.php"), BEHAVIOR).unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("behavior.php"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("behavior.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("behavior.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const BEHAVIOR: &str = r#"<?php
declare(strict_types=1);

foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }

use StreamFixture\Client;
use StreamFixture\Credentials;
use StreamFixture\ClientOptions;
use StreamFixture\HttpRequest;
use StreamFixture\HttpResponse;
use StreamFixture\StreamResponse;
use StreamFixture\StreamTransport;
use StreamFixture\BodyReader;
use StreamFixture\JsonValue;
use StreamFixture\SdkError;
use StreamFixture\StreamChatInput;
use StreamFixture\StreamTranscriptionInput;
use StreamFixture\StreamChatEventMessage;
use StreamFixture\StreamChatEventDone;
use StreamFixture\StreamChatEventUnknown;
use StreamFixture\StreamChatCompletion;
use StreamFixture\StreamTranscriptionCompletion;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

/** A byte reader over one scripted SSE body; closing is observable. */
final class StubReader implements BodyReader
{
    private int $offset = 0;
    public function __construct(private StreamStub $stub) {}
    public function read(): ?string
    {
        if ($this->stub->closed) { return null; }
        $this->stub->reads++;
        if ($this->offset >= strlen($this->stub->frames)) { return null; }
        $chunk = substr($this->stub->frames, $this->offset, 4);
        $this->offset += strlen($chunk);
        return $chunk;
    }
    public function close(): void { $this->stub->closed = true; }
}

/** A stubbed stream transport serving one scripted event-stream body. */
final class StreamStub implements StreamTransport
{
    /** @var list<HttpRequest> */
    public array $requests = [];
    public int $reads = 0;
    public bool $closed = false;
    public function __construct(public string $frames) {}
    public function send(HttpRequest $request): HttpResponse
    {
        throw new LogicException('this probe only opens streams');
    }
    public function open(HttpRequest $request): StreamResponse
    {
        $this->requests[] = $request;
        return new StreamResponse(200, ['content-type' => ['text/event-stream']], new StubReader($this));
    }
}

$body = JsonValue::fromObject(['prompt' => JsonValue::fromString('hi')]);

// A declared message event decodes to the typed model with its declared
// metadata; a declared done event follows; the completion reports end-of-body.
$stub = new StreamStub("event: message\nid: 42\ndata: hello\n\nevent: done\ndata: {}\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$generator = $client->streamChatEvents(new StreamChatInput(body: $body));
$seen = [];
foreach ($generator as $event) { $seen[] = $event; }
check(count($seen) === 2, 'two typed events: ' . count($seen));
check($seen[0] instanceof StreamChatEventMessage, 'message kind: ' . get_class($seen[0]));
check($seen[0]->kind === 'message', 'message kind name');
check($seen[0]->data->data === 'hello', 'typed payload: ' . $seen[0]->data->data);
check($seen[0]->id === '42', 'declared id metadata: ' . $seen[0]->id);
check($seen[1] instanceof StreamChatEventDone, 'done kind: ' . get_class($seen[1]));
$completion = $generator->getReturn();
check($completion instanceof StreamChatCompletion, 'completion class: ' . get_class($completion));
check($completion->reason === 'eof', 'eof reason: ' . $completion->reason);
check($completion->usage === null, 'eof preserves no usage without the policy');
check(count($stub->requests) === 1, 'one request: ' . count($stub->requests));
check($stub->closed, 'exhaustion releases the reader');

// An undeclared event kind surfaces through the typed unknown alternative
// without failing the stream, and later declared events still decode.
$stub = new StreamStub("event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$seen = iterator_to_array($client->streamChatEvents(new StreamChatInput(body: JsonValue::fromObject([]))));
check(count($seen) === 2, 'unknown kinds never fail the stream: ' . count($seen));
check($seen[0] instanceof StreamChatEventUnknown, 'unknown kind: ' . get_class($seen[0]));
check($seen[0]->kind === 'unknown' && $seen[0]->event === 'surprise' && $seen[0]->data === 'hello', 'unknown carries the raw frame');
check($seen[1] instanceof StreamChatEventMessage && $seen[1]->data->data === 'ok', 'later declared events still decode');

// An invalid payload for a recognized kind remains a decoding error.
$stub = new StreamStub("data: hello\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
try {
    iterator_to_array($client->streamChatEvents(new StreamChatInput(body: JsonValue::fromObject([]))));
    throw new LogicException('expected a decoding failure');
} catch (SdkError $error) {
    check($error->kind === 'response_validation', 'invalid payload kind: ' . $error->kind);
}

// The declared [DONE] sentinel completes the stream before any payload
// decoding, preserves the final usage frame, and issues no further reads.
$stub = new StreamStub("event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\nevent: message\ndata: never\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$seen = iterator_to_array($client->streamTranscriptionEvents());
check(count($seen) === 0, 'the final usage frame is preserved, not yielded: ' . count($seen));
check($stub->closed, 'the sentinel releases the reader');
check(count($stub->requests) === 1, 'the sentinel issues one request');

// Earlier frames are still yielded when a sentinel follows them, and the
// last frame before it becomes the terminal usage.
$stub = new StreamStub("event: message\ndata: a\n\nevent: message\ndata: b\n\ndata: [DONE]\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$generator = $client->streamTranscriptionEvents();
$seen = [];
foreach ($generator as $event) { $seen[] = $event; }
check(count($seen) === 1 && $seen[0]->data->data === 'a', 'earlier frames are yielded: ' . count($seen));
$completion = $generator->getReturn();
check($completion->reason === 'sentinel', 'sentinel reason: ' . $completion->reason);
check($completion->usage !== null && $completion->usage->data->data === 'b', 'the last frame before the sentinel is the usage: ' . ($completion->usage !== null ? $completion->usage->data->data : 'null'));

// Early break stops consumption: one event, one request, released reader.
$stub = new StreamStub("event: message\ndata: a\n\nevent: message\ndata: b\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$generator = $client->streamChatEvents(new StreamChatInput(body: JsonValue::fromObject([])));
$seen = [];
foreach ($generator as $event) { $seen[] = $event; break; }
check(count($seen) === 1, 'one event before the break');
unset($generator);
check($stub->closed, 'an early break releases the reader');
$reads = $stub->reads;
unset($generator);
check($stub->reads === $reads, 'an early break issues no further reads');

// The untyped direct call keeps its exact previous behavior: strictly decoded
// envelope items for well-formed frames.
$stub = new StreamStub("event: message\ndata: a\n\nevent: message\ndata: b\n\n");
$client = new Client(new Credentials([]), $stub, new ClientOptions(serverUrl: 'https://api.streams.test/v1'));
$response = $client->streamChat(new StreamChatInput(body: JsonValue::fromObject([])));
$items = [];
foreach ($response->body as $item) { $items[] = $item; }
check(count($items) === 2 && $items[0]->data === 'a' && $items[1]->data === 'b', 'the untyped item stream is unchanged');

echo 'typed stream behavior verified', PHP_EOL;
"#;
