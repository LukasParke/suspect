//! Generated typed SSE events for the Ruby gem: emission shape, plan
//! carriage, and native behavioral verification of the emitted typed event
//! enumerators over a scripted transport. Static runtime files are never
//! modified; operations without a discriminated stream schema emit no new
//! bytes at all, and the direct operation calls plus their untyped
//! ItemStream iteration stay unchanged.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    ruby_sdk,
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
    let entry = Uri::parse("https://source.streams.test/ruby-typed-streams.json").unwrap();
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
            backend: Backend::RubyHttp,
            package_name: "typed-streams-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn content<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn discriminated_sse_operations_emit_typed_event_methods() {
    let files = generate(stream_document());
    let client = content(&files, "ruby/lib/typed_streams_sdk/client.rb");
    let signatures = content(&files, "ruby/sig/typed_streams_sdk.rbs");

    // Frozen Data event classes per declared kind, the typed unknown
    // alternative, the completion carrier and the events wrapper.
    for expected in [
        "StreamChatMessageEvent = Data.define(:kind, :data, :id)",
        "StreamChatDoneEvent = Data.define(:kind, :data, :id)",
        "StreamChatUnknownEvent = Data.define(:kind, :event, :data)",
        "StreamChatCompletion = Data.define(:reason, :usage)",
        "StreamTranscriptionMessageEvent = Data.define(:kind, :data)",
        "class StreamChatEvents",
        "def completion",
        "def stream_chat_events(**kwargs)",
        "def stream_transcription_events(**kwargs)",
        // The typed decode through the operation's existing stream item codec.
        "Codecs::",
        ".decode(envelope)",
        // Per-item metadata (id) is exposed on the typed event.
        "StreamChatMessageEvent.new(kind: \"message\", data: item, id: envelope['id'])",
        // The declared sentinel completes the stream before any payload
        // decoding, preserving the final usage frame.
        "if data == \"[DONE]\"",
        "StreamTranscriptionCompletion.new(reason: :sentinel, usage: held)",
        "wrapper.complete!(StreamChatCompletion.new(reason: :eof, usage: nil))",
        "wrapper.complete!(StreamTranscriptionCompletion.new(reason: :eof, usage: held))",
        // The runtime's own framer is reused with raw envelope pass-through.
        "class TypedEventParser < Internal::ItemParser",
        "yield value",
        // The shared exchange mirrors the direct call's response handling.
        "def events_consume(op, response, context, limit, capture_limit)",
    ] {
        assert!(
            client.contains(expected),
            "client.rb is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }

    // The controls without discrimination emit nothing.
    assert!(!client.contains("stream_logs_events"));
    assert!(!client.contains("stream_rows_events"));
    assert!(!client.contains("StreamLogsEvent"));
    assert!(!client.contains("StreamRowsEvent"));

    // The direct calls and their untyped ItemStream iteration stay exported
    // and unchanged.
    assert!(client.contains("def stream_chat("));
    assert!(client.contains("streaming: true"));

    // RBS signatures cover the emitted methods and classes.
    for expected in [
        "class StreamChatMessageEvent < Data",
        "attr_accessor data: Types::",
        "attr_accessor id: String?",
        "attr_accessor reason: :sentinel | :eof",
        "attr_reader events: Enumerator[",
        "def stream_chat_events: (**untyped) -> StreamChatEvents",
        "def stream_transcription_events: (**untyped) -> StreamTranscriptionEvents",
    ] {
        assert!(
            signatures.contains(expected),
            "signatures are missing:\n{expected}\n--- emitted: ---\n{signatures}"
        );
    }
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn control_operations_without_discrimination_keep_the_untyped_gem() {
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
    let client = content(&files, "ruby/lib/typed_streams_sdk/client.rb");
    let signatures = content(&files, "ruby/sig/typed_streams_sdk.rbs");
    assert!(!client.contains("_events"));
    assert!(!client.contains("TypedEventParser"));
    assert!(!client.contains("Data.define"));
    assert!(!signatures.contains("< Data"));
    // The compiled stream plan is still carried, with the documented
    // conservative single default events, but no operation is emittable.
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = ruby_sdk::plan_sdk(contract, &selected, ruby_sdk::RubyConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let events = plan.stream_events().expect("stream media are declared");
    assert_eq!(events.outcome.streams.len(), 2);
    assert!(events.operations.is_empty());
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn plan_carries_the_compiled_stream_semantics_and_emittable_operations() {
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = ruby_sdk::plan_sdk(contract, &selected, ruby_sdk::RubyConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
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
            .map(|operation| operation.identity.as_str())
            .collect::<Vec<_>>(),
        vec!["streamChat", "streamTranscription"]
    );
    assert_eq!(events.operations[0].events_name, "stream_chat_events");
    assert_eq!(events.operations[0].events_class, "StreamChatEvents");
    assert_eq!(
        events.operations[0].completion_class,
        "StreamChatCompletion"
    );
    assert_eq!(
        events.operations[0].kind_classes[0].1,
        "StreamChatMessageEvent"
    );
    assert!(events.operations[1].sentinel.is_some());
    assert!(events.operations[1].keep_final_usage);
}

const BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'typed_streams_sdk'

class ScriptedTransport
  attr_reader :requests, :reads

  def initialize(frames, endless: false)
    @frames = frames
    @endless = endless
    @requests = 0
    @reads = 0
    @lock = Mutex.new
  end

  def exchange(request:, context:)
    context.check!
    @lock.synchronize do
      @requests += 1
      bytes = @frames.b
      chunks = Enumerator.new do |out|
        offset = 0
        while offset < bytes.bytesize
          @reads += 1
          out << bytes.byteslice(offset, 3)
          offset += 3
        end
        if @endless
          while true
            sleep 0.01
          end
        end
      end
      yield TypedStreamsSdk::WireResponse.new(
        status: 200,
        headers: { 'Content-Type' => 'text/event-stream' },
        body: chunks
      )
    end
  end
end

def client(transport)
  TypedStreamsSdk::Client.new(transport: transport, server_url: 'http://127.0.0.1')
end

# A declared message event decodes to the typed model; a declared done event
# follows; the completion reports an end-of-body completion with no usage.
transport = ScriptedTransport.new("event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n")
events = client(transport).stream_chat_events
collected = []
events.events.each { |event| collected << event }
raise 'expected two events' unless collected.length == 2
raise 'wrong first kind' unless collected[0].kind == 'message'
raise 'typed decode lost the payload' unless collected[0].data.data == '{"text":"hello"}'
raise 'wrong second kind' unless collected[1].kind == 'done'
raise 'eof completion missing' unless events.completion.reason == :eof
raise 'un-sentineled stream preserves no usage' unless events.completion.usage.nil?
raise 'request count changed' unless transport.requests == 1

# Per-item metadata: the declared id field is exposed on the typed event.
transport = ScriptedTransport.new("event: message\nid: 42\ndata: hello\n\n")
event = client(transport).stream_chat_events.events.first
raise 'metadata event changed' unless event.kind == 'message'
raise 'declared id lost' unless event.id == '42'

# An undeclared event kind surfaces through the typed unknown alternative
# without failing the stream, and later declared events still decode.
transport = ScriptedTransport.new("event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n")
collected = client(transport).stream_chat_events.events.to_a
raise 'expected two events' unless collected.length == 2
raise 'unknown kind changed' unless collected[0].kind == 'unknown'
raise 'unknown event name lost' unless collected[0].event == 'surprise'
raise 'unknown data lost' unless collected[0].data == 'hello'
raise 'later declared event lost' unless collected[1].kind == 'message' && collected[1].data.data == 'ok'

# An invalid payload for a recognized kind remains a branded decoding error,
# and the transfer is released.
transport = ScriptedTransport.new("data: hello\n\n", endless: true)
begin
  client(transport).stream_chat_events.events.to_a
  raise 'expected a decoding failure'
rescue TypedStreamsSdk::ResponseError => error
  raise 'wrong failure kind' unless error.kind == :response_decoding
end
raise 'request count changed' unless transport.requests == 1

# The declared [DONE] sentinel completes the stream before any payload
# decoding, preserves the final usage frame as terminal metadata, and issues
# no further reads.
transport = ScriptedTransport.new("event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\n", endless: true)
events = client(transport).stream_transcription_events
yielded = []
events.events.each { |event| yielded << event }
raise 'sentinel frames must not be yielded' unless yielded.empty?
raise 'sentinel completion missing' unless events.completion.reason == :sentinel
raise 'final usage lost' unless events.completion.usage.kind == 'message'
raise 'final usage payload lost' unless events.completion.usage.data.data == '{"tokens": 42}'
raise 'request count changed' unless transport.requests == 1
reads = transport.reads
sleep 0.05
raise 'the sentinel issues no further reads' unless transport.reads == reads

# Earlier frames are still yielded when a sentinel follows them.
transport = ScriptedTransport.new("event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n")
events = client(transport).stream_transcription_events
collected = events.events.to_a
raise 'earlier frames lost' unless collected.length == 1 && collected[0].data.data == 'a'
raise 'sentinel reason changed' unless events.completion.reason == :sentinel
raise 'later usage lost' unless events.completion.usage.data.data == '{"usage": true}'

# Early break stops consumption: one event, one request, released transfer,
# no further reads.
transport = ScriptedTransport.new("event: message\ndata: a\n\nevent: message\ndata: b\n\n", endless: true)
collected = []
client(transport).stream_chat_events.events.each do |event|
  collected << event.data.data
  break
end
raise 'early break collected changed' unless collected == ['a']
raise 'early break issued a request' unless transport.requests == 1
reads = transport.reads
sleep 0.05
raise 'an early break issues no further reads' unless transport.reads == reads

# The untyped direct call keeps its exact previous behavior: strictly decoded
# envelope items for well-formed frames.
transport = ScriptedTransport.new("event: message\ndata: a\n\nevent: message\ndata: b\n\n")
items = client(transport).stream_chat.data.to_a
raise 'untyped items changed' unless items.map { |item| item.data } == ['a', 'b']

puts 'typed stream behavior verified'
"#;

fn ruby_home() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}

/// The gem requires Ruby >= 3.3; older interpreters cannot even parse the
/// emitted runtime syntax, so discovery refuses them.
fn ruby() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_RUBY_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let ruby = ruby_home().join("bin/ruby");
    if !ruby.is_file() {
        return None;
    }
    let output = Command::new(&ruby).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let minor = text
        .strip_prefix("ruby ")
        .and_then(|rest| rest.split('.').nth(1))
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 3).then_some(ruby)
}

fn checked(command: &mut Command, root: &std::path::Path, label: &str) {
    let output = command.output().unwrap();
    std::fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    std::fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_typed_events_drive_stubbed_streams() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_stream_typed: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(stream_document());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "syntax",
            );
        }
    }

    std::fs::write(root.path().join("behavior.rb"), BEHAVIOR).unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
    eprintln!("ruby_stream_typed: native Ruby behavioral gate passed");
}
