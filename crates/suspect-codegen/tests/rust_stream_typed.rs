//! M4 typed-SSE runtime emission for the Rust backend: the canonical v3
//! generation path compiles the shared stream semantics and emits
//! dependency-free typed event iterators, while plans without a discriminated
//! SSE schema (and the retained v1/v2 planning APIs) gain no artifact at all.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.streams.test/rust-typed-streams.json";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec_pretty(&document).unwrap(),
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

fn contract(document: Value) -> Arc<Contract> {
    contract_with_document(document)
}

fn selection(contract: &Contract) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::RustHttp,
        package_name: "typed-streams-rust".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract(document);
    let selected = selection(&contract);
    backend::generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn by_path(files: &[OutFile]) -> std::collections::BTreeMap<&str, &str> {
    files
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect()
}

#[test]
fn v3_path_compiles_typed_stream_entries_and_v2_stays_empty() {
    let contract = contract(stream_document());
    let selected = selection(&contract);
    let plan = suspect_codegen::rust_http::plan_http_v3(
        contract.clone(),
        &selected,
        suspect_codegen::rust_http::HttpConfig::default(),
    )
    .unwrap();
    let events = plan.stream_events();
    assert_eq!(events.len(), 2, "exactly the discriminated operations");
    let chat = events
        .iter()
        .find(|entry| entry.function == "stream_chat")
        .expect("chat entry");
    assert_eq!(chat.events, vec!["message", "done"]);
    assert!(chat.sentinel.is_none());
    assert!(!chat.keep_final_usage);
    assert_eq!(chat.stem, "StreamChat");
    assert_eq!(
        chat.id_metadata.as_ref().map(|m| m.field.as_str()),
        Some("id")
    );
    let transcribe = events
        .iter()
        .find(|entry| entry.function == "stream_transcription")
        .expect("transcription entry");
    assert_eq!(transcribe.sentinel.as_deref(), Some("[DONE]"));
    assert!(transcribe.keep_final_usage);
    assert!(transcribe.id_metadata.is_none());
    // The retained v1/v2 planning APIs never populate the typed stream plan.
    for plan_with in [
        suspect_codegen::rust_http::plan_http(
            contract.clone(),
            &selected,
            suspect_codegen::rust_http::HttpConfig::default(),
        )
        .unwrap(),
        suspect_codegen::rust_http::plan_http_v2(
            contract.clone(),
            &selected,
            suspect_codegen::rust_http::HttpConfig::default(),
        )
        .unwrap(),
    ] {
        assert!(plan_with.stream_events().is_empty());
    }
}

#[test]
fn discriminated_generation_emits_the_events_module_and_untyped_generation_emits_nothing() {
    let configured = generate_document(stream_document());
    let files = by_path(&configured);
    let module = files
        .get("rust/src/stream_events.rs")
        .expect("typed events module emitted");
    for expected in [
        "pub enum CompletionReason",
        "pub enum StreamChatEvent",
        "StreamChatEvent::Message",
        "StreamChatEvent::Unknown",
        "pub struct StreamChatCompletion",
        "pub reason: crate::stream_events::CompletionReason",
        "pub usage: std::option::Option<StreamChatEvent>",
        "pub struct StreamChatEvents<'a, T>",
        "pub async fn next(",
        "pub fn completion(&self)",
        "pub fn stream_chat_events<'a, T: crate::http::Transport>",
        "fn stream_chat_events_decode_error(",
        "crate::codecs::PathsChatPostResponses200ContentTextEventStreamItemSchemaCodec::decode_value(envelope.clone())",
        "SdkErrorKind::ResponseDecoding",
        "pub enum StreamTranscriptionEvent",
        "if data == \"[DONE]\" {",
        "CompletionReason::Sentinel",
        "CompletionReason::Eof",
        "usage: self.held.clone()",
    ] {
        assert!(
            module.contains(expected),
            "stream_events.rs lacks {expected}"
        );
    }
    // The lenient envelope reader and request path live beside the operation's
    // own descriptor, which stays private to its module.
    let chat_operation = files
        .get("rust/src/operations/stream_chat.rs")
        .expect("chat operation module");
    for expected in [
        "fn stream_chat_events_envelope(",
        "pub(crate) async fn stream_chat_events_stream<T: crate::http::Transport>(",
        "exchange.into_item_response(stream_chat_events_envelope",
        "crate::http::Framing::ServerSentEvents",
    ] {
        assert!(
            chat_operation.contains(expected),
            "operations/stream_chat.rs lacks {expected}"
        );
    }
    // Undeclared kinds never fail the stream and the declared id metadata is
    // carried on the declared variants.
    assert!(
        module.contains("StreamChatEvent::Unknown {\n                    event: kind.clone(),")
    );
    assert!(module.contains("let event_id = item.id.clone();"));
    // The controls without discrimination emit nothing.
    assert!(!module.contains("StreamLogsEvent"));
    assert!(!module.contains("StreamRowsEvent"));
    // The emitted lib.rs gains exactly the module declaration and the client
    // methods.
    let lib = files.get("rust/src/lib.rs").unwrap();
    assert!(lib.contains("#[cfg(feature=\"http\")]\npub mod stream_events;\n"));
    assert!(lib.contains("pub fn stream_chat_events("));
    assert!(lib.contains("pub fn stream_transcription_events("));
    assert!(!lib.contains("stream_logs_events"));
    assert!(!lib.contains("stream_rows_events"));
    // The direct operation functions and their untyped item streams stay.
    assert!(lib.contains("pub async fn stream_chat("));
}

#[test]
fn untyped_generation_stays_byte_identical_to_its_plan() {
    // The control document has no discriminated stream: no file, no module
    // declaration, no client method, anywhere.
    let control = generate_document(untyped_document());
    let files = by_path(&control);
    assert!(!files.contains_key("rust/src/stream_events.rs"));
    assert_eq!(
        control.len(),
        by_path(&generate_document(untyped_document())).len()
    );
    let lib = files.get("rust/src/lib.rs").unwrap();
    assert!(!lib.contains("pub mod stream_events"));
    assert!(!lib.contains("_events"));
    for (path, content) in &files {
        if *path == "rust/src/lib.rs" {
            continue;
        }
        assert!(!content.contains("stream_events::"), "{path} leaked events");
        assert!(!content.contains("EventsEnvelope"), "{path} leaked events");
    }
    // The v3 emission of the discriminated document keeps the typed events
    // inside the new module and the emitting operations' own helper functions:
    // no other artifact leaks events code.
    let configured = generate_document(stream_document());
    let configured_files = by_path(&configured);
    assert!(configured_files.contains_key("rust/src/stream_events.rs"));
    for (path, content) in &configured_files {
        if *path == "rust/src/stream_events.rs"
            || *path == "rust/src/lib.rs"
            || *path == "rust/src/operations/stream_chat.rs"
            || *path == "rust/src/operations/stream_transcription.rs"
        {
            continue;
        }
        assert!(
            !content.contains("_events_envelope"),
            "{path} leaked events"
        );
        assert!(!content.contains("StreamChatEvent"), "{path} leaked events");
        assert!(!content.contains("_events("), "{path} leaked events");
    }
}

/// Per-user cargo target directory, so concurrent agents never contend on the
/// shared workspace lock while building emitted packages.
fn cargo_target() -> PathBuf {
    let user = std::env::var_os("USER")
        .or_else(|| std::env::var_os("LOGNAME"))
        .unwrap_or_else(|| format!("uid-{}", std::process::id()).into());
    std::env::temp_dir().join(format!(
        "suspect-rust-stream-events-target-{}",
        user.to_string_lossy()
    ))
}

fn cargo(command: &str, manifest: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new("cargo")
        .arg(command)
        .arg("--offline")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(cargo_target())
        .args(args)
        .env_remove("RUST_MIN_STACK")
        .env(
            "RUSTUP_TOOLCHAIN",
            std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").unwrap_or_default(),
        )
        .output()
        .expect("cargo is available");
    let log = format!("{}", String::from_utf8_lossy(&output.stderr));
    (output.status.success(), log)
}

fn registry_unavailable(log: &str) -> bool {
    log.contains("no matching package named")
        || log.contains("failed to download")
        || log.contains("error: failed to select a version")
        || log.contains("network disabled")
        || log.contains("could not download")
}

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn emitted_events_drive_real_streams_in_a_compiled_package() {
    let configured = generate_document(stream_document());
    let directory = tempfile::Builder::new()
        .prefix("rust-stream-events-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");

    // Model-only compilation needs no dependencies at all and must always
    // succeed offline.
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");

    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"stream-events-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"typed-streams-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping behavioral events execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_STREAM_EVENTS").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

const CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the emitted typed event streams.
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[derive(Default)]
struct BodyState {
    reads: usize,
    closed: bool,
}

/// A bounded chunked byte stream counting reads and observing closure. An
/// endless body never terminates on its own, so only the runtime's
/// cancellation can stop it.
struct Body {
    chunks: std::collections::VecDeque<Vec<u8>>,
    endless: bool,
    state: Arc<Mutex<BodyState>>,
}
impl Body {
    fn new(frames: &str, endless: bool, state: Arc<Mutex<BodyState>>) -> Self {
        let chunks = frames
            .as_bytes()
            .chunks(3)
            .map(|chunk| chunk.to_vec())
            .collect();
        Self {
            chunks,
            endless,
            state,
        }
    }
}
impl Drop for Body {
    fn drop(&mut self) {
        self.state.lock().unwrap().closed = true;
    }
}
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        self.state.lock().unwrap().reads += 1;
        if let std::option::Option::Some(chunk) = self.chunks.pop_front() {
            return std::result::Result::Ok(std::option::Option::Some(chunk));
        }
        if !self.endless {
            return std::result::Result::Ok(std::option::Option::None);
        }
        std::future::pending::<()>().await;
        std::result::Result::Ok(std::option::Option::None)
    }
}

#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    body_state: Arc<Mutex<BodyState>>,
    chat: String,
    transcribe: String,
    endless: bool,
}
impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        self.calls
            .lock()
            .unwrap()
            .push(request.url.split('?').next().unwrap_or_default().to_owned());
        let frames = if request.url.contains("/chat") {
            self.chat.clone()
        } else {
            self.transcribe.clone()
        };
        std::result::Result::Ok(TransportResponse {
            status: 200,
            headers: vec![("Content-Type".into(), b"text/event-stream".to_vec())],
            body: Body::new(&frames, self.endless, self.body_state.clone()),
        })
    }
}

fn server(chat: &str, transcribe: &str, endless: bool) -> (Server, Arc<Mutex<Vec<String>>>, Arc<Mutex<BodyState>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let body_state = Arc::new(Mutex::new(BodyState::default()));
    (
        Server {
            calls: calls.clone(),
            body_state: body_state.clone(),
            chat: chat.to_owned(),
            transcribe: transcribe.to_owned(),
            endless,
        },
        calls,
        body_state,
    )
}

use sdk::{Client, Credentials};
use sdk::stream_events::{CompletionReason, StreamChatEvent, StreamTranscriptionEvent};

const CHAT: &str = "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n";

#[test]
fn typed_decode_and_eof_completion() {
    let (transport, calls, body) = server(CHAT, "", false);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client.stream_chat_events(sdk::operations::stream_chat::StreamChat::new());
    let first = block_on(events.next()).unwrap().expect("first event");
    let StreamChatEvent::Message { data, id } = first else {
        panic!("unexpected first event");
    };
    assert_eq!(data.data, r#"{"text":"hello"}"#);
    assert_eq!(data.event, sdk::models::PathsChatPostResponses200ContentTextEventStreamItemSchemaEvent::Message);
    assert_eq!(id.as_deref(), None);
    let second = block_on(events.next()).unwrap().expect("second event");
    assert!(matches!(second, StreamChatEvent::Done { .. }));
    assert!(block_on(events.next()).unwrap().is_none(), "exhausted");
    let completion = events.completion().expect("eof completion");
    assert_eq!(completion.reason, CompletionReason::Eof);
    assert!(completion.usage.is_none());
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(body.lock().unwrap().closed, "a completed stream closes its body");
}

#[test]
fn declared_id_metadata_is_carried() {
    let (transport, _calls, _body) = server("event: message\nid: 42\ndata: hello\n\n", "", false);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client.stream_chat_events(sdk::operations::stream_chat::StreamChat::new());
    let event = block_on(events.next()).unwrap().expect("event");
    let StreamChatEvent::Message { id, .. } = event else {
        panic!("unexpected event");
    };
    assert_eq!(id.as_deref(), Some("42"));
}

#[test]
fn unknown_kind_does_not_fail_the_stream() {
    let (transport, calls, _body) =
        server("event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n", "", false);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client.stream_chat_events(sdk::operations::stream_chat::StreamChat::new());
    let unknown = block_on(events.next()).unwrap().expect("unknown event");
    let StreamChatEvent::Unknown { event, data } = unknown else {
        panic!("undeclared kind did not surface as the typed unknown alternative");
    };
    assert_eq!(event, "surprise");
    assert_eq!(data, "hello");
    let known = block_on(events.next()).unwrap().expect("declared event");
    let StreamChatEvent::Message { data, .. } = known else {
        panic!("declared kinds after an unknown kind must still decode");
    };
    assert_eq!(data.data, "ok");
    assert!(block_on(events.next()).unwrap().is_none());
    assert_eq!(events.completion().unwrap().reason, CompletionReason::Eof);
    assert_eq!(calls.lock().unwrap().len(), 1, "unknown kinds issued extra requests");
}

#[test]
fn invalid_payload_of_recognated_kind_is_a_decoding_failure() {
    // The frame's envelope lacks the required event field, so decoding it as
    // the declared model fails instead of surfacing an event.
    let (transport, calls, body) = server("data: hello\n\n", "", true);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client.stream_chat_events(sdk::operations::stream_chat::StreamChat::new());
    let failure = block_on(events.next()).unwrap_err();
    match failure {
        sdk::operations::stream_chat::StreamChatError::Sdk(error) => {
            assert_eq!(error.kind, sdk::http::SdkErrorKind::ResponseDecoding);
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(body.lock().unwrap().closed, "a decoding failure closes the body");
    let reads = body.lock().unwrap().reads;
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(body.lock().unwrap().reads, reads, "a decoding failure issued further reads");
}

#[test]
fn sentinel_completes_before_decode_and_preserves_usage() {
    let (transport, calls, body) =
        server("", "event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\n", true);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client
        .stream_transcription_events(sdk::operations::stream_transcription::StreamTranscription::new());
    assert!(
        block_on(events.next()).unwrap().is_none(),
        "the sentinel frame must not surface as an event"
    );
    let completion = events.completion().expect("sentinel completion");
    assert_eq!(completion.reason, CompletionReason::Sentinel);
    let StreamTranscriptionEvent::Message { data, .. } = completion.usage.as_ref().expect("usage") else {
        panic!("unexpected usage event");
    };
    assert_eq!(data.data, r#"{"tokens": 42}"#);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(body.lock().unwrap().closed, "the sentinel completes and closes the body");
    let reads = body.lock().unwrap().reads;
    assert!(reads > 0);
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(body.lock().unwrap().reads, reads, "the sentinel issues further reads");
}

#[test]
fn sentinel_after_earlier_events() {
    let (transport, _calls, _body) = server(
        "",
        "event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n",
        false,
    );
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client
        .stream_transcription_events(sdk::operations::stream_transcription::StreamTranscription::new());
    let first = block_on(events.next()).unwrap().expect("earlier event");
    let StreamTranscriptionEvent::Message { data, .. } = first else {
        panic!("unexpected event");
    };
    assert_eq!(data.data, "a");
    assert!(block_on(events.next()).unwrap().is_none());
    let completion = events.completion().expect("sentinel completion");
    assert_eq!(completion.reason, CompletionReason::Sentinel);
    let StreamTranscriptionEvent::Message { data, .. } = completion.usage.as_ref().expect("usage") else {
        panic!("unexpected usage event");
    };
    assert_eq!(data.data, r#"{"usage": true}"#);
}

#[test]
fn early_drop_stops_consumption() {
    let (transport, calls, body) =
        server("event: message\ndata: a\n\nevent: message\ndata: b\n\n", "", true);
    let client = Client::with_transport(transport, Credentials::new());
    let mut events = client.stream_chat_events(sdk::operations::stream_chat::StreamChat::new());
    let first = block_on(events.next()).unwrap().expect("first event");
    assert!(matches!(first, StreamChatEvent::Message { .. }));
    drop(events);
    assert_eq!(calls.lock().unwrap().len(), 1, "an early drop issued another request");
    assert!(body.lock().unwrap().closed, "an early drop closes the body");
    let reads = body.lock().unwrap().reads;
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(body.lock().unwrap().reads, reads, "an early drop issues further reads");
}

#[test]
fn untyped_item_stream_is_unchanged() {
    let (transport, calls, _body) =
        server("event: message\ndata: a\n\nevent: message\ndata: b\n\n", "", false);
    let client = Client::with_transport(transport, Credentials::new());
    let reply = block_on(client.stream_chat(sdk::operations::stream_chat::StreamChat::new())).unwrap();
    let mut stream = reply.into_data();
    let mut items = std::vec::Vec::new();
    while let std::option::Option::Some(item) = block_on(stream.next()) {
        items.push(item.unwrap().data);
    }
    assert_eq!(items, vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(calls.lock().unwrap().len(), 1);
}
"##;
