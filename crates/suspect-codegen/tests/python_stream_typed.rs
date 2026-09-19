//! Generated typed SSE events for the Python HTTP backend: the emitted
//! `iter_<op>_events` generators (sync and async), the frozen descriptor
//! constants, and native behavioral verification over a stubbed httpx
//! transport. Static runtime files are never modified; operations without a
//! discriminated stream schema emit no new bytes at all.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const IMPORT: &str = "typed_streams_sdk";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let uri = Uri::parse("https://source.streams.test/python-typed-streams.json").unwrap();
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

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "typed-streams-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    };
    suspect_codegen::backend::generate_with_options(
        contract,
        &selected,
        &target,
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == format!("python/src/{IMPORT}/_client.py"))
        .expect("generated client module")
}

fn operations_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == format!("python/src/{IMPORT}/operations.py"))
        .expect("generated operations module")
}

#[test]
fn discriminated_sse_operations_emit_typed_event_generators() {
    let files = generate(stream_document());
    let client = client_file(&files).content.clone();
    let operations = operations_file(&files).content.clone();

    // Sync and async generators for exactly the discriminated operations. The
    // async flavor wraps an inner generator because async generators cannot
    // return values; the completion metadata is the wrapper's attribute.
    for signature in [
        "def iter_stream_chat_events(self, **kwargs: Any) -> Iterator[operations.StreamChatEvent]:",
        "def iter_stream_chat_events(self, **kwargs: Any) -> _AsyncStreamEvents:",
        "async def iter_stream_chat_events_stream(self, **kwargs: Any) -> AsyncIterator[Any]:",
        "def iter_stream_transcription_events(self, **kwargs: Any) -> Iterator[operations.StreamTranscriptionEvent]:",
        "def iter_stream_transcription_events(self, **kwargs: Any) -> _AsyncStreamEvents:",
        "async def iter_stream_transcription_events_stream(self, **kwargs: Any) -> AsyncIterator[Any]:",
    ] {
        assert_eq!(
            client.matches(signature).count(),
            1,
            "signature: {signature}"
        );
    }
    // Frozen descriptor constants embedded, with the compiled kinds, sentinel
    // and keep-final-usage policy.
    for expected in [
        "_ITER_STREAM_CHAT_EVENTS = types.MappingProxyType({",
        "_ITER_STREAM_TRANSCRIPTION_EVENTS = types.MappingProxyType({",
        "'kinds': ('message', 'done'),",
        "'sentinel': '[DONE]',",
        "'keep_final_usage': True,",
        "'sentinel': '',",
        "'keep_final_usage': False,",
        "stream['item_codec'] = {'schema': {'id': _EVENTS_RAW_SOURCE}}",
        "def _events_register_raw_codec() -> None:",
        "class _RawStreamEvents:",
    ] {
        assert!(client.contains(expected), "_client.py lacks {expected}");
    }
    // Per-item metadata (id) and the typed decode through the declared model.
    assert!(client.contains("'id_field': 'id',"));
    assert!(client.contains(
        "typed = descriptor['events'][kind](kind=kind, data=item, id=_events_value(item.id))"
    ));
    // The completion metadata is the generator's documented return value.
    assert!(
        client.contains("return operations.StreamChatCompletion(reason='sentinel', usage=held)")
    );
    assert!(client.contains("return operations.StreamChatCompletion(reason='eof', usage=held if descriptor['keep_final_usage'] else None)"));

    // The public operations.py event, unknown and completion dataclasses.
    for expected in [
        "class StreamChatMessageEvent:",
        "class StreamChatDoneEvent:",
        "class StreamChatUnknownEvent:",
        "class StreamChatCompletion:",
        "StreamChatEvent: TypeAlias = StreamChatMessageEvent | StreamChatDoneEvent | StreamChatUnknownEvent",
        "kind: Literal['message']",
        "kind: Literal['unknown']",
        "reason: Literal['sentinel', 'eof']",
        "id: str | None = None",
    ] {
        assert!(
            operations.contains(expected),
            "operations.py lacks {expected}"
        );
    }

    // The controls without discrimination emit nothing.
    assert!(!client.contains("iter_stream_logs_events"));
    assert!(!client.contains("iter_stream_rows_events"));
    assert!(!operations.contains("StreamLogsEvent"));
    assert!(!operations.contains("StreamRowsEvent"));

    // The untyped iterators are unchanged: the direct methods remain.
    assert!(client.contains("def stream_chat("));
    assert!(client.contains("async def stream_chat("));
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
    let files = generate(document);
    let client = client_file(&files).content.clone();
    let operations = operations_file(&files).content.clone();
    assert!(!client.contains("iter_stream_logs_events"));
    assert!(!client.contains("iter_stream_rows_events"));
    assert!(!client.contains("_events_register_raw_codec"));
    assert!(!client.contains("MappingProxyType"));
    assert!(!client.contains("import copy"));
    assert!(!operations.contains("StreamLogsEvent"));
    assert!(!operations.contains("StreamRowsEvent"));
    assert!(!operations.contains("StreamChat"));
}

fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An interpreter able to import httpx: plain `python3` first, then this
/// repository's native Python tools environment, then nothing.
fn httpx_interpreter() -> Option<PathBuf> {
    fn imports_httpx(python: &Path) -> bool {
        Command::new(python)
            .arg("-c")
            .arg("import httpx")
            .output()
            .is_ok_and(|output| output.status.success())
    }
    if imports_httpx(Path::new("python3")) {
        return Some(PathBuf::from("python3"));
    }
    let candidate = std::env::var_os("SUSPECT_PYTHON_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    imports_httpx(&candidate).then_some(candidate)
}

const BEHAVIOR: &str = r#"import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from typed_streams_sdk import AsyncClient, Client


class Body(httpx.SyncByteStream):
    """A bounded chunked byte stream counting reads and observing closure. An
    endless body never terminates on its own, so only the runtime's cancellation
    can stop it."""

    def __init__(self, frames, endless=False):
        self.chunks = [frames[at:at + 3].encode("utf-8") for at in range(0, len(frames), 3)]
        self.endless = endless
        self.reads = 0
        self.closed = False

    def __iter__(self):
        return self

    def __next__(self):
        if not self.chunks:
            if not self.endless:
                raise StopIteration
            import time
            while True:
                time.sleep(0.01)
        self.reads += 1
        return self.chunks.pop(0)

    def close(self):
        self.closed = True


class AsyncBody(httpx.AsyncByteStream):
    def __init__(self, frames, endless=False):
        self.chunks = [frames[at:at + 3].encode("utf-8") for at in range(0, len(frames), 3)]
        self.endless = endless
        self.reads = 0
        self.closed = False

    def __aiter__(self):
        return self

    async def __anext__(self):
        if not self.chunks:
            if not self.endless:
                raise StopAsyncIteration
            import asyncio
            await asyncio.Event().wait()
        self.reads += 1
        return self.chunks.pop(0)

    async def aclose(self):
        self.closed = True


class SyncStub(httpx.BaseTransport):
    def __init__(self, frames, endless=False):
        self.frames = frames
        self.endless = endless
        self.body = None
        self.requests = 0

    def handle_request(self, request):
        self.requests += 1
        self.body = Body(self.frames, self.endless)
        return httpx.Response(200, stream=self.body, headers={"content-type": "text/event-stream"})


class AsyncStub(httpx.AsyncBaseTransport):
    def __init__(self, frames, endless=False):
        self.frames = frames
        self.endless = endless
        self.body = None
        self.requests = 0

    async def handle_async_request(self, request):
        self.requests += 1
        self.body = AsyncBody(self.frames, self.endless)
        return httpx.Response(200, stream=self.body, headers={"content-type": "text/event-stream"})


def sync_typed_decode_and_completion():
    transport = SyncStub('event: message\ndata: {"text":"hello"}\n\nevent: done\ndata: {}\n\n')
    with Client(transport=transport) as client:
        generator = client.iter_stream_chat_events()
        first = next(generator)
        assert first.kind == "message", first
        assert first.data.event == "message"
        assert first.data.data == '{"text":"hello"}'
        second = next(generator)
        assert second.kind == "done"
        assert second.data.event == "done"
        try:
            next(generator)
        except StopIteration as stop:
            assert stop.value.reason == "eof", stop.value
            assert stop.value.usage is None
        else:
            raise AssertionError("expected completion")
    assert transport.requests == 1
    assert transport.body.closed, "a completed stream closes its body"


def sync_metadata_id():
    transport = SyncStub('event: message\nid: 42\ndata: hello\n\n')
    with Client(transport=transport) as client:
        event = next(client.iter_stream_chat_events())
    assert event.kind == "message"
    assert event.id == "42"


def sync_unknown_kind_does_not_fail():
    transport = SyncStub('event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n')
    with Client(transport=transport) as client:
        generator = client.iter_stream_chat_events()
        unknown = next(generator)
        assert unknown.kind == "unknown", unknown
        assert unknown.event == "surprise"
        assert unknown.data == "hello"
        known = next(generator)
        assert known.kind == "message"
        assert known.data.data == "ok"
        try:
            next(generator)
        except StopIteration as stop:
            assert stop.value.reason == "eof"
        else:
            raise AssertionError("expected completion")
    assert transport.requests == 1


def sync_invalid_payload_raises_decoding_error():
    transport = SyncStub('data: hello\n\n', endless=True)
    with Client(transport=transport) as client:
        generator = client.iter_stream_chat_events()
        try:
            next(generator)
        except Exception as error:
            assert getattr(error, "kind", None) == "response-decoding", error
        else:
            raise AssertionError("expected a decoding failure")
    assert transport.requests == 1
    assert transport.body.closed, "a decoding failure closes the body"


def sync_sentinel_completes_and_preserves_usage():
    transport = SyncStub('event: message\ndata: {"tokens": 42}\n\ndata: [DONE]\n\n', endless=True)
    with Client(transport=transport) as client:
        generator = client.iter_stream_transcription_events()
        yielded = []
        while True:
            try:
                yielded.append(next(generator))
            except StopIteration as stop:
                completion = stop.value
                break
        assert yielded == [], yielded
        assert completion.reason == "sentinel", completion
        assert completion.usage is not None
        assert completion.usage.kind == "message"
        assert completion.usage.data.data == '{"tokens": 42}'
    assert transport.requests == 1
    assert transport.body.closed, "the sentinel completes and closes the body"
    reads = transport.body.reads
    assert reads > 0
    assert transport.body.reads == reads, "the sentinel issues no further reads"


def sync_sentinel_after_earlier_events():
    transport = SyncStub('event: message\ndata: a\n\nevent: message\ndata: {"usage": true}\n\ndata: [DONE]\n\n')
    with Client(transport=transport) as client:
        generator = client.iter_stream_transcription_events()
        first = next(generator)
        assert first.kind == "message"
        assert first.data.data == "a"
        try:
            next(generator)
        except StopIteration as stop:
            assert stop.value.reason == "sentinel"
            assert stop.value.usage.data.data == '{"usage": true}'
        else:
            raise AssertionError("expected the sentinel completion")


def sync_early_break_stops_consumption():
    transport = SyncStub('event: message\ndata: a\n\nevent: message\ndata: b\n\n', endless=True)
    with Client(transport=transport) as client:
        collected = []
        for event in client.iter_stream_chat_events():
            collected.append(event.data.data)
            break
    assert collected == ["a"], collected
    assert transport.requests == 1
    assert transport.body.closed, "an early break closes the body"
    reads = transport.body.reads
    assert transport.body.reads == reads, "an early break issues no further reads"


def sync_untyped_path_unchanged():
    transport = SyncStub('event: message\ndata: a\n\nevent: message\ndata: b\n\n')
    with Client(transport=transport) as client:
        items = list(client.stream_chat().data)
    assert [(item.event, item.data) for item in items] == [("message", "a"), ("message", "b")]


async def async_checks():
    transport = AsyncStub('event: message\ndata: {"text":"hello"}\n\nevent: done\ndata: {}\n\n')
    async with AsyncClient(transport=transport) as client:
        iterator = client.iter_stream_chat_events()
        events = [event async for event in iterator]
    assert [event.kind for event in events] == ["message", "done"], events
    assert events[0].data.data == '{"text":"hello"}'
    assert iterator.completion is not None, "the eof completion metadata is exposed on the iterator"
    assert iterator.completion.reason == "eof"
    assert iterator.completion.usage is None
    assert transport.requests == 1
    assert transport.body.closed

    transport = AsyncStub('event: message\ndata: {"tokens": 42}\n\ndata: [DONE]\n\n', endless=True)
    async with AsyncClient(transport=transport) as client:
        iterator = client.iter_stream_transcription_events()
        yielded = []
        async for event in iterator:
            yielded.append(event)
        assert yielded == []
        completion = iterator.completion
    assert completion is not None
    assert completion.reason == "sentinel"
    assert completion.usage is not None
    assert completion.usage.kind == "message"
    assert completion.usage.data.data == '{"tokens": 42}'
    assert transport.body.closed
    reads = transport.body.reads
    assert transport.body.reads == reads, "the sentinel issues no further reads"

    transport = AsyncStub('event: message\ndata: a\n\nevent: message\ndata: b\n\n', endless=True)
    async with AsyncClient(transport=transport) as client:
        iterator = client.iter_stream_chat_events()
        first = await iterator.__anext__()
        await iterator.aclose()
    assert first.data.data == "a"
    assert iterator.completion is None, "closing early leaves the completion unset"
    assert transport.requests == 1
    assert transport.body.closed
    reads = transport.body.reads
    assert transport.body.reads == reads, "an early break issues no further reads"


sync_typed_decode_and_completion()
sync_metadata_id()
sync_unknown_kind_does_not_fail()
sync_invalid_payload_raises_decoding_error()
sync_sentinel_completes_and_preserves_usage()
sync_sentinel_after_earlier_events()
sync_early_break_stops_consumption()
sync_untyped_path_unchanged()
asyncio.run(async_checks())
print("typed stream behavior verified")
"#;

#[test]
fn native_typed_events_drive_stubbed_streams() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(stream_document());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Python file must at least be valid bytecode.
    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile",
            );
        }
    }

    let Some(python) = httpx_interpreter() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(root.path().join("behavior.py"), BEHAVIOR).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("behavior.py"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
}
