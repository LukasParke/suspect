//! Emitted-only typed SSE events for the TypeScript HTTP backend: the
//! per-operation `<op>Events` iterators, strict compilation, and native node
//! behavior over a stubbed event-stream response. Operations without a
//! discriminated stream schema emit nothing at all, and the direct operation
//! functions plus their untyped AsyncIterables stay unchanged.

#![cfg(feature = "http-protocol")]

use std::{process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
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

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@streams/fixture".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    }
}

fn selected(contract: &Arc<Contract>) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn generate(document: Value) -> Vec<suspect_codegen::OutFile> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn operations_source(files: &[suspect_codegen::OutFile]) -> String {
    files
        .iter()
        .find(|file| file.path == "typescript/operations.ts")
        .unwrap()
        .content
        .clone()
}

#[test]
fn discriminated_sse_operations_emit_typed_event_iterators() {
    let files = generate(stream_document());
    let operations = operations_source(&files);

    // The discriminated streamChat operation: event union, completion metadata,
    // lenient descriptor clone and the typed generator.
    for expected in [
        "export type StreamChatEvent =",
        "readonly kind: \"message\"",
        "readonly kind: \"done\"",
        "readonly kind: 'unknown'; readonly event: string; readonly data: string",
        "export interface StreamChatCompletion {",
        "readonly reason: 'sentinel' | 'eof';",
        "readonly usage: StreamChatEvent | null;",
        "const streamChatEventsDescriptor = { operationId: \"streamChat\", source: streamChatSource, wire: streamChatWire, limits:",
        "export async function* streamChatEvents(client: ClientOptions, input: StreamChatInput = {}, call?: CallOptions): AsyncGenerator<StreamChatEvent, StreamChatCompletion, undefined> {",
        "    const items = ((await executeOperation(streamChatEventsDescriptor, input, client, call)) as {",
        // Declared envelope metadata (id) is exposed per event.
        "typed = { kind: event, data: item, id: item.id };",
        // The untyped direct call and its item codec stay exactly as before.
        "export function streamChat(client: ClientOptions",
    ] {
        assert!(
            operations.contains(expected),
            "operations.ts lacks {expected}"
        );
    }
    // The stream item codec is read leniently by the events descriptor only.
    assert!(operations.contains("decode: (text: string) => text"));

    // The sentinel is compiled from the description evidence and completes the
    // stream before any payload decoding, preserving the final usage frame.
    for expected in [
        "if (data === \"[DONE]\") return { reason: 'sentinel', usage: held };",
        "if (held !== null) yield held;",
        "return { reason: 'eof', usage: held };",
        "export async function* streamTranscriptionEvents(",
    ] {
        assert!(
            operations.contains(expected),
            "operations.ts lacks {expected}"
        );
    }

    // Controls without discrimination emit nothing.
    assert!(!operations.contains("streamLogsEvents"));
    assert!(!operations.contains("streamRowsEvents"));
    let manifest = files
        .iter()
        .find(|file| file.path == "typescript/http-manifest.json")
        .unwrap();
    assert!(manifest.content.contains("\"streamChat\""));

    // The plan carries the compiled stream semantics for every stream media.
    let contract = contract_with_document(stream_document());
    let selected = selected(&contract);
    let plan = suspect_codegen::typescript::http::plan_http(
        contract,
        &selected,
        suspect_codegen::typescript::http::HttpConfig::expanded(),
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
    let operations = operations_source(&files);
    assert!(!operations.contains("EventsDescriptor"));
    assert!(!operations.contains("streamLogsEvents"));
    assert!(!operations.contains("streamRowsEvents"));
    assert!(!operations.contains("Event ="));
    assert!(!operations.contains("Completion"));
    assert!(!operations.contains("from './http/common.js'"));
    // The typed stream plan is still compiled and stored on the plan, with the
    // documented conservative single default events.
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    let plan = suspect_codegen::typescript::http::plan_http(
        contract,
        &selected,
        suspect_codegen::typescript::http::HttpConfig::expanded(),
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

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

#[test]
fn typed_stream_events_compile_strictly_with_the_package() {
    if !tool_available("tsc") {
        eprintln!("tsc is not on PATH; skipping the strict typed-events compile check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(stream_document()), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), TYPES).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--noEmit",
            "operations.ts",
            "consumer.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
}

const TYPES: &str = r#"
import { streamChatEvents, streamTranscriptionEvents, type StreamChatEvent, type ClientOptions } from './operations.js';
export async function consume(client: ClientOptions): Promise<void> {
    const events = streamChatEvents(client, {});
    for await (const event of events) {
        if (event.kind === 'message') {
            const envelope = event.data;
            const frame: string = envelope.data;
            void frame;
        } else if (event.kind === 'unknown') {
            const name: string = event.event;
            const raw: string = event.data;
            void name; void raw;
        }
    }
    const terminal = streamTranscriptionEvents(client, {});
    for (;;) {
        const next = await terminal.next();
        if (next.done) {
            const completion = next.value;
            const reason: 'sentinel' | 'eof' = completion.reason;
            void reason;
            return;
        }
    }
}
"#;

const DRIVER: &str = r#"
import assert from 'node:assert/strict';
import {
  streamChat,
  streamChatEvents,
  streamTranscriptionEvents,
} from './dist/operations.js';

let frames = '';
// The stubbed body terminates naturally unless a test opts into an endless
// one; only an endless body makes the runtime's cancellation observable, since
// an already-closed body cannot run its cancel callback.
let bodyEnds = true;
let requests = 0;
let bodyState = null;
globalThis.fetch = async () => {
  requests++;
  bodyState = { pulls: 0, cancelled: false };
  const bytes = new TextEncoder().encode(frames);
  let offset = 0;
  const stream = new ReadableStream({
    pull(controller) {
      bodyState.pulls++;
      if (offset < bytes.length) {
        const end = Math.min(offset + 3, bytes.length);
        controller.enqueue(bytes.subarray(offset, end));
        offset = end;
      } else if (bodyEnds) {
        controller.close();
      }
      // An endless body never terminates on its own: the runtime must cancel.
    },
    cancel() { bodyState.cancelled = true; },
  });
  return new Response(stream, { status: 200, headers: { 'content-type': 'text/event-stream' } });
};
const client = {};

// A declared message event decodes to the typed model; a declared done event
// follows; the completion value reports an end-of-body completion.
{
  frames = 'event: message\ndata: {"text":"hello"}\n\nevent: done\ndata: {}\n\n';
  requests = 0;
  const iterator = streamChatEvents(client, {});
  const first = await iterator.next();
  assert.equal(first.done, false);
  assert.equal(first.value.kind, 'message');
  assert.equal(first.value.data.event, 'message');
  assert.equal(first.value.data.data, '{"text":"hello"}');
  const second = await iterator.next();
  assert.equal(second.done, false);
  assert.equal(second.value.kind, 'done');
  assert.equal(second.value.data.event, 'done');
  const terminal = await iterator.next();
  assert.equal(terminal.done, true);
  assert.equal(terminal.value.reason, 'eof');
  assert.equal(terminal.value.usage, null);
  assert.equal(requests, 1);
}

// Per-item metadata: the declared id field is exposed on the typed event.
{
  frames = 'event: message\nid: 42\ndata: hello\n\n';
  const iterator = streamChatEvents(client, {});
  const event = await iterator.next();
  assert.equal(event.value.kind, 'message');
  assert.equal(event.value.id, '42');
  await iterator.return();
}

// An undeclared event kind surfaces through the typed unknown alternative
// without failing the stream, and later declared events still decode.
{
  frames = 'event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n';
  const iterator = streamChatEvents(client, {});
  const unknown = await iterator.next();
  assert.equal(unknown.value.kind, 'unknown');
  assert.equal(unknown.value.event, 'surprise');
  assert.equal(unknown.value.data, 'hello');
  const known = await iterator.next();
  assert.equal(known.value.kind, 'message');
  assert.equal(known.value.data.data, 'ok');
  const terminal = await iterator.next();
  assert.equal(terminal.done, true);
  assert.equal(terminal.value.reason, 'eof');
}

// An invalid payload for a recognized kind remains a decoding error, and the
// response body is cancelled instead of read further: the frame's envelope
// lacks the required event field, so decoding it as the declared model fails.
{
  frames = 'data: hello\n\n';
  bodyEnds = false;
  requests = 0;
  await assert.rejects(
    async () => { for await (const event of streamChatEvents(client, {})) void event; },
    (error) => error instanceof Error && error.name === 'SdkError' && error.kind === 'response-decoding',
  );
  assert.equal(requests, 1);
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(bodyState.cancelled, true, 'a decoding failure cancels the body');
}

// The declared [DONE] sentinel completes the stream before any payload
// decoding, preserves the final usage frame as terminal metadata, and issues
// no further reads.
{
  frames = 'event: message\ndata: {"tokens": 42}\n\ndata: [DONE]\n\n';
  bodyEnds = false;
  requests = 0;
  const iterator = streamTranscriptionEvents(client, {});
  const terminal = await iterator.next();
  assert.equal(terminal.done, true);
  assert.equal(terminal.value.reason, 'sentinel');
  assert.notEqual(terminal.value.usage, null);
  assert.equal(terminal.value.usage.kind, 'message');
  assert.equal(terminal.value.usage.data.data, '{"tokens": 42}');
  assert.equal(requests, 1);
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(bodyState.cancelled, true, 'the sentinel completes and cancels the body');
  const pulls = bodyState.pulls;
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(bodyState.pulls, pulls, 'the sentinel issues no further reads');
}

// Earlier frames are still yielded when a sentinel follows them.
{
  frames = 'event: message\ndata: a\n\nevent: message\ndata: {"usage": true}\n\ndata: [DONE]\n\n';
  const iterator = streamTranscriptionEvents(client, {});
  const first = await iterator.next();
  assert.equal(first.done, false);
  assert.equal(first.value.data.data, 'a');
  const terminal = await iterator.next();
  assert.equal(terminal.done, true);
  assert.equal(terminal.value.reason, 'sentinel');
  assert.equal(terminal.value.usage.data.data, '{"usage": true}');
}

// Early break stops consumption: one event, one request, cancelled body.
{
  frames = 'event: message\ndata: a\n\nevent: message\ndata: b\n\n';
  bodyEnds = false;
  requests = 0;
  const collected = [];
  for await (const event of streamChatEvents(client, {})) {
    collected.push(event.data.data);
    break;
  }
  assert.deepEqual(collected, ['a']);
  assert.equal(requests, 1);
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(bodyState.cancelled, true, 'an early break cancels the body');
  const pulls = bodyState.pulls;
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(bodyState.pulls, pulls, 'an early break issues no further reads');
}

// The untyped direct call keeps its exact previous behavior: strictly decoded
// envelope items for well-formed frames.
{
  frames = 'event: message\ndata: a\n\nevent: message\ndata: b\n\n';
  bodyEnds = true;
  const response = await streamChat(client, {});
  const items = [];
  for await (const item of response.data) items.push({ ...item });
  assert.deepEqual(items, [
    { event: 'message', data: 'a' },
    { event: 'message', data: 'b' },
  ]);
}
"#;

#[test]
fn typed_stream_events_drive_stubbed_streams_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the behavioral typed-events check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(stream_document()), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("driver.mjs")
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

/// The sdk defaults gate must not affect typed stream emission: it is
/// conditional only on the compiled stream plan.
#[test]
fn sdk_defaults_neither_enable_nor_disable_typed_stream_emission() {
    let with_defaults = GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    };
    let contract = contract_with_document(stream_document());
    let selected = selected(&contract);
    let files = generate_with_options(contract, &selected, &target(), &with_defaults).unwrap();
    let operations = operations_source(&files);
    assert!(operations.contains("streamChatEvents"));
    assert!(operations.contains("streamTranscriptionEvents"));
    assert!(!operations.contains("streamLogsEvents"));
}
