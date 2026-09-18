//! Emitted-only typed SSE events for the Go HTTP backend: the per-operation
//! `<Op>Events` iterators, generation-time emission plus native behavior over
//! a scripted transport. Operations without a discriminated stream schema emit
//! nothing at all, and the direct operation methods plus their untyped
//! `*Stream` iterators stay byte-identical.

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.streams.test/go-typed-streams.json";

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

/// The control document: the same SSE envelope without any discrimination
/// evidence, plus a JSON-lines stream. Nothing new may be emitted for it.
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

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::GoHttp,
            package_name: "example.com/typed-streams-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn generate(document: Value) -> std::collections::BTreeMap<String, String> {
    generate_document(document)
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

#[test]
fn discriminated_sse_operations_emit_typed_event_iterators() {
    let configured = generate(stream_document());
    let control = generate(untyped_document());

    // Exactly one new file: the typed events helpers.
    let events = configured
        .get("go/stream_events.go")
        .expect("typed events helpers emitted");
    assert!(!control.contains_key("go/stream_events.go"));
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the discriminated stream adds exactly one file"
    );
    // The control document emits no typed-event helpers anywhere: no framer,
    // no event iterator, no events methods.
    for (path, content) in &control {
        assert!(
            !content.contains("httpEventFramer") && !content.contains("EventIterator"),
            "{path} leaked typed-event helpers"
        );
    }

    // The typed event union, completion metadata and iterator for exactly the
    // discriminated operations, with the declared [DONE] sentinel completing
    // the stream before any payload decoding.
    for expected in [
        "type StreamChatEvent struct",
        "Kind string",
        "Data *PathsChatPostResponses200ContentTextEventStreamItemSchema",
        "UnknownEvent string",
        "UnknownData string",
        "type StreamChatCompletion struct",
        "Reason string",
        "Usage *StreamChatEvent",
        "type StreamChatEventIterator struct",
        "func (c *Client) StreamChatEvents(ctx context.Context, input StreamChatInput) *StreamChatEventIterator",
        "func (it *StreamChatEventIterator) Next() bool",
        "func (it *StreamChatEventIterator) Event() StreamChatEvent",
        "func (it *StreamChatEventIterator) Completion() StreamChatCompletion",
        "func (it *StreamChatEventIterator) Err() error",
        "func (it *StreamChatEventIterator) Close()",
        "case \"message\", \"done\":",
        "item, err := Codecs.PathsChatPostResponses200ContentTextEventStreamItemSchema.DecodeValue(envelope)",
        "typed = &StreamChatEvent{Kind: \"unknown\", UnknownEvent: kind, UnknownData: data}",
        "func (c *Client) StreamTranscriptionEvents(ctx context.Context, input StreamTranscriptionInput) *StreamTranscriptionEventIterator",
        "The declared [DONE] sentinel completes the stream before any payload decoding.",
        "if data == \"[DONE]\" {",
        "it.finish(\"sentinel\", it.held)",
        "it.finish(\"eof\", it.held)",
        "func (it *StreamTranscriptionEventIterator) Completion() StreamTranscriptionCompletion",
        // The untyped stream codec stays strictly decoded by the direct method;
        // only the events iterator frames leniently.
        "httpEventFramerOf[T any](stream *Stream[T]) *httpEventFramer",
    ] {
        assert!(
            events.contains(expected),
            "stream_events.go lacks {expected}"
        );
    }
    // Declared envelope metadata (id) is exposed on the typed event.
    assert!(events.contains("Id Optional[string]"), "metadata member");
    assert!(events.contains("Data: &item, Id: item.Id"));
    // The controls without discrimination emit nothing.
    assert!(!events.contains("StreamLogsEvent"));
    assert!(!events.contains("StreamRowsEvent"));
    // The eof completion without keep-final-usage preserves nothing.
    assert!(events.contains("it.finish(\"eof\", nil)"));
}

#[test]
fn plan_carries_the_typed_stream_outcome_only_when_emission_happens() {
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(contract, &selected, go_http::HttpConfig::default()).unwrap();
    let events = plan.stream_events().expect("typed stream plan carried");
    assert!(events.emits());
    let chat = events
        .operations
        .iter()
        .find(|operation| operation.method_name == "StreamChat")
        .expect("chat operation");
    assert_eq!(chat.events, vec!["message", "done"]);
    assert!(chat.sentinel.is_none());
    assert!(!chat.keep_final_usage);
    assert_eq!(chat.events_type, "StreamChatEvent");
    assert_eq!(chat.completion_type, "StreamChatCompletion");
    assert_eq!(chat.iterator_type, "StreamChatEventIterator");
    assert_eq!(chat.events_method, "StreamChatEvents");
    assert_eq!(
        chat.id_metadata.as_ref().map(|m| m.field.as_str()),
        Some("Id")
    );
    let transcribe = events
        .operations
        .iter()
        .find(|operation| operation.method_name == "StreamTranscription")
        .expect("transcription operation");
    assert_eq!(transcribe.sentinel.as_deref(), Some("[DONE]"));
    assert!(transcribe.keep_final_usage);
    assert!(transcribe.id_metadata.is_none());

    let control_contract = contract_with_document(untyped_document());
    let control_selected = control_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = go_http::plan_http(
        control_contract,
        &control_selected,
        go_http::HttpConfig::default(),
    )
    .unwrap();
    assert!(control.stream_events().is_none());
}

fn go_toolchain() -> Option<String> {
    let output = Command::new("go").arg("version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let version = text.split_whitespace().nth(2)?;
    let minor = version
        .strip_prefix("go1.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 23).then_some(text)
}

#[test]
fn native_typed_events_drive_scripted_streams() {
    let Some(version) = go_toolchain() else {
        eprintln!(
            "go_stream_typed: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    eprintln!("go_stream_typed: {version}");
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate_document(stream_document()), root.path()).unwrap();
    std::fs::write(
        root.path().join("go/stream_events_behavior_test.go"),
        BEHAVIOR,
    )
    .unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "."])
        .current_dir(root.path().join("go"))
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native typed events behavior failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_stream_typed: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}

const BEHAVIOR: &str = r#"package sdk_test

import (
	"context"
	"errors"
	"io"
	"net/http"
	"sync"
	"testing"
	"time"

	sdk "example.com/typed-streams-sdk"
)

// streamBody replays fixed SSE frames in bounded chunks and observes closure
// and reads. An endless body never terminates on its own, so only the
// runtime's cancellation can stop it.
type streamBody struct {
	mu      sync.Mutex
	chunks  []string
	offset  int
	endless bool
	closed  bool
	reads   int
}

func newStreamBody(frames string, endless bool) *streamBody {
	var chunks []string
	for at := 0; at < len(frames); at += 3 {
		end := at + 3
		if end > len(frames) {
			end = len(frames)
		}
		chunks = append(chunks, frames[at:end])
	}
	return &streamBody{chunks: chunks, endless: endless}
}

func (b *streamBody) Read(p []byte) (int, error) {
	for {
		b.mu.Lock()
		if b.closed {
			b.mu.Unlock()
			return 0, io.ErrClosedPipe
		}
		if b.offset < len(b.chunks) {
			chunk := b.chunks[b.offset]
			b.offset++
			b.reads++
			b.mu.Unlock()
			return copy(p, chunk), nil
		}
		if !b.endless {
			b.mu.Unlock()
			return 0, io.EOF
		}
		b.mu.Unlock()
		time.Sleep(time.Millisecond)
	}
}

func (b *streamBody) Close() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closed = true
	return nil
}

func (b *streamBody) count() int {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.reads
}

func (b *streamBody) isClosed() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.closed
}

type streamFake struct {
	mu       sync.Mutex
	requests int
	frames   string
	endless  bool
	body     *streamBody
}

func (f *streamFake) Do(request *http.Request) (*http.Response, error) {
	f.mu.Lock()
	f.requests++
	f.body = newStreamBody(f.frames, f.endless)
	body := f.body
	f.mu.Unlock()
	if err := request.Context().Err(); err != nil {
		return nil, err
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": {"text/event-stream"}}, Body: body}, nil
}

func (f *streamFake) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.requests
}

func client(transport *streamFake) *sdk.Client {
	created, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: transport})
	if err != nil {
		panic(err)
	}
	return created
}

const chatFrames = "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n"

func TestTypedDecodeAndEofCompletion(t *testing.T) {
	transport := &streamFake{frames: chatFrames}
	iterator := client(transport).StreamChatEvents(context.Background(), sdk.NewStreamChatInput())
	if !iterator.Next() {
		t.Fatal("first event missing", iterator.Err())
	}
	first := iterator.Event()
	if first.Kind != "message" {
		t.Fatal("unexpected kind", first.Kind)
	}
	if string(first.Data.Event) != "message" || first.Data.Data != `{"text":"hello"}` {
		t.Fatal("envelope not decoded to the declared model", first.Data)
	}
	if !iterator.Next() {
		t.Fatal("second event missing", iterator.Err())
	}
	if iterator.Event().Kind != "done" {
		t.Fatal("unexpected kind", iterator.Event().Kind)
	}
	if iterator.Next() {
		t.Fatal("exhausted stream yielded another event")
	}
	if iterator.Err() != nil {
		t.Fatal("clean completion must leave Err nil", iterator.Err())
	}
	completion := iterator.Completion()
	if completion.Reason != "eof" || completion.Usage != nil {
		t.Fatal("unexpected eof completion", completion)
	}
	if transport.count() != 1 {
		t.Fatal("typed events issued extra requests", transport.count())
	}
	if !transport.body.isClosed() {
		t.Fatal("a completed stream closes its body")
	}
}

func TestDeclaredIdMetadata(t *testing.T) {
	transport := &streamFake{frames: "event: message\nid: 42\ndata: hello\n\n"}
	iterator := client(transport).StreamChatEvents(context.Background(), sdk.NewStreamChatInput())
	if !iterator.Next() {
		t.Fatal("event missing", iterator.Err())
	}
	event := iterator.Event()
	if !event.Id.IsSet || event.Id.Value != "42" {
		t.Fatal("declared id metadata lost", event.Id)
	}
	iterator.Close()
}

func TestUnknownKindDoesNotFailTheStream(t *testing.T) {
	transport := &streamFake{frames: "event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n"}
	iterator := client(transport).StreamChatEvents(context.Background(), sdk.NewStreamChatInput())
	if !iterator.Next() {
		t.Fatal("first event missing", iterator.Err())
	}
	unknown := iterator.Event()
	if unknown.Kind != "unknown" || unknown.UnknownEvent != "surprise" || unknown.UnknownData != "hello" {
		t.Fatal("undeclared kind is not representable", unknown)
	}
	if unknown.Data != nil {
		t.Fatal("unknown events carry no decoded model", unknown.Data)
	}
	if !iterator.Next() {
		t.Fatal("declared kinds after an unknown kind must still decode", iterator.Err())
	}
	if iterator.Event().Kind != "message" || iterator.Event().Data.Data != "ok" {
		t.Fatal("later declared event did not decode", iterator.Event())
	}
	if iterator.Next() {
		t.Fatal("exhausted stream yielded another event")
	}
	if iterator.Err() != nil || iterator.Completion().Reason != "eof" {
		t.Fatal("unexpected completion", iterator.Err(), iterator.Completion())
	}
	if transport.count() != 1 {
		t.Fatal("unknown kinds issued extra requests", transport.count())
	}
}

func TestInvalidPayloadOfRecognizedKindIsADecodingFailure(t *testing.T) {
	// The frame's envelope lacks the required event field, so decoding it as
	// the declared model fails instead of surfacing an event.
	transport := &streamFake{frames: "data: hello\n\n", endless: true}
	iterator := client(transport).StreamChatEvents(context.Background(), sdk.NewStreamChatInput())
	if iterator.Next() {
		t.Fatal("an invalid payload produced an event")
	}
	var failure *sdk.SDKError
	if !errors.As(iterator.Err(), &failure) {
		t.Fatal("decoding failure lost", iterator.Err())
	}
	if failure.Kind != "response-decoding" {
		t.Fatal("unexpected failure kind", failure.Kind)
	}
	if transport.count() != 1 {
		t.Fatal("extra requests", transport.count())
	}
	if !transport.body.isClosed() {
		t.Fatal("a decoding failure closes the body")
	}
	reads := transport.body.count()
	time.Sleep(20 * time.Millisecond)
	if transport.body.count() != reads {
		t.Fatal("a decoding failure issued further reads")
	}
}

func TestSentinelCompletesBeforeDecodeAndPreservesUsage(t *testing.T) {
	transport := &streamFake{frames: "event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\n", endless: true}
	iterator := client(transport).StreamTranscriptionEvents(context.Background(), sdk.NewStreamTranscriptionInput())
	if iterator.Next() {
		t.Fatal("the sentinel frame must not surface as an event", iterator.Event())
	}
	if iterator.Err() != nil {
		t.Fatal("the sentinel is not a failure", iterator.Err())
	}
	completion := iterator.Completion()
	if completion.Reason != "sentinel" {
		t.Fatal("unexpected completion", completion)
	}
	if completion.Usage == nil || completion.Usage.Kind != "message" || completion.Usage.Data.Data != `{"tokens": 42}` {
		t.Fatal("the final usage frame was not preserved", completion.Usage)
	}
	if transport.count() != 1 {
		t.Fatal("extra requests", transport.count())
	}
	if !transport.body.isClosed() {
		t.Fatal("the sentinel completes and closes the body")
	}
	reads := transport.body.count()
	if reads == 0 {
		t.Fatal("the stream never read its frames")
	}
	time.Sleep(20 * time.Millisecond)
	if transport.body.count() != reads {
		t.Fatal("the sentinel issues further reads")
	}
}

func TestSentinelAfterEarlierEvents(t *testing.T) {
	transport := &streamFake{frames: "event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n"}
	iterator := client(transport).StreamTranscriptionEvents(context.Background(), sdk.NewStreamTranscriptionInput())
	if !iterator.Next() {
		t.Fatal("first event missing", iterator.Err())
	}
	if iterator.Event().Data.Data != "a" {
		t.Fatal("earlier frames are still yielded", iterator.Event())
	}
	if iterator.Next() {
		t.Fatal("the sentinel frame must not surface as an event")
	}
	completion := iterator.Completion()
	if completion.Reason != "sentinel" || completion.Usage == nil || completion.Usage.Data.Data != `{"usage": true}` {
		t.Fatal("unexpected sentinel completion", completion)
	}
}

func TestEarlyBreakStopsConsumption(t *testing.T) {
	transport := &streamFake{frames: "event: message\ndata: a\n\nevent: message\ndata: b\n\n", endless: true}
	iterator := client(transport).StreamChatEvents(context.Background(), sdk.NewStreamChatInput())
	if !iterator.Next() {
		t.Fatal("first event missing", iterator.Err())
	}
	iterator.Close()
	if transport.count() != 1 {
		t.Fatal("an early break issued another request", transport.count())
	}
	if !transport.body.isClosed() {
		t.Fatal("an early break closes the body")
	}
	reads := transport.body.count()
	time.Sleep(20 * time.Millisecond)
	if transport.body.count() != reads {
		t.Fatal("an early break issues further reads")
	}
}

func TestContextCancellationSurfacesAndNeverReachesTheTransport(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	transport := &streamFake{frames: chatFrames}
	iterator := client(transport).StreamChatEvents(ctx, sdk.NewStreamChatInput())
	if iterator.Next() {
		t.Fatal("cancelled iteration produced an event")
	}
	if !errors.Is(iterator.Err(), context.Canceled) {
		t.Fatal("cancellation lost", iterator.Err())
	}
	if transport.count() != 0 {
		t.Fatal("cancelled iteration reached the transport", transport.count())
	}
}

func TestUntypedStreamIteratorIsUnchanged(t *testing.T) {
	transport := &streamFake{frames: "event: message\ndata: a\n\nevent: message\ndata: b\n\n"}
	reply, err := client(transport).StreamChat(context.Background(), sdk.NewStreamChatInput())
	if err != nil {
		t.Fatal(err)
	}
	page := reply.(sdk.StreamChatStatus200)
	var items []string
	for page.Data.Next() {
		items = append(items, page.Data.Value().Data)
	}
	if err := page.Data.Err(); err != nil {
		t.Fatal(err)
	}
	if len(items) != 2 || items[0] != "a" || items[1] != "b" {
		t.Fatal("untyped stream items changed", items)
	}
}
"#;

#[test]
fn native_module_builds_with_the_events_file() {
    let Some(_) = go_toolchain() else {
        eprintln!(
            "go_stream_typed: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate_document(stream_document()), root.path()).unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
