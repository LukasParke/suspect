//! Emitted-only typed SSE events for the C# HTTP backend: the per-operation
//! `<Op>EventsAsync` iterators and `<Op>EventsCompletionAsync` accessors, the
//! generated `StreamEvents.g.cs` shapes, byte-identity for operations without a
//! discriminated stream schema, and native .NET behavior over a stubbed
//! streaming HttpMessageHandler. Static runtime files are never modified; the
//! typed decode lives entirely in the generated package.

#![cfg(feature = "csharp-sdk")]

use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
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
        backend: Backend::CsharpHttp,
        package_name: "acme.typed-streams-sdk".into(),
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

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn discriminated_sse_operations_emit_typed_event_members() {
    let files = generate(stream_document());
    let client = source(&files, "csharp/src/Client.g.cs");
    let events = source(&files, "csharp/src/StreamEvents.g.cs");
    for expected in [
        // The typed events iterator on the generated Client, with the exact
        // documented signature.
        "public async global::System.Collections.Generic.IAsyncEnumerable<StreamChatEvent> StreamChatEventsAsync(StreamChatInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        "public async global::System.Collections.Generic.IAsyncEnumerable<StreamTranscriptionEvent> StreamTranscriptionEventsAsync(StreamTranscriptionInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        // The documented completion accessor.
        "public async Task<StreamChatCompletion> StreamChatEventsCompletionAsync(StreamChatInput input, CancellationToken cancellationToken = default)",
        // The shared core iteration over the lenient frame transport.
        "var stream = await StreamChatEventsSourceAsync(input, null, cancellationToken).ConfigureAwait(false);",
        "var frames = stream.GetAsyncEnumerator(cancellationToken);",
        "while (await frames.MoveNextAsync().ConfigureAwait(false))",
        // The lenient wire call reuses the operation's own request preparation
        // and streams raw framed envelopes.
        "private Task<HttpStream<StreamTranscriptionEvent?>> StreamTranscriptionEventsSourceAsync(StreamTranscriptionInput input, RequestOptions? requestOptions, CancellationToken cancellationToken)",
        "return _runtime.CallAsync<HttpStream<StreamChatEvent>>(0, requestOptions, cancellationToken, () =>",
        "return raw.Stream(StreamEvents.DecodeStreamChatFrame);",
    ] {
        assert!(client.contains(expected), "Client.g.cs lacks {expected}");
    }
    for expected in [
        // The discriminated event union with typed per-kind records and the
        // typed unknown alternative.
        "public abstract record StreamChatEvent",
        "public sealed record Message : StreamChatEvent",
        "public sealed record Done : StreamChatEvent",
        "public required StreamChatItem Data { get; init; }",
        "public override string Kind => \"message\";",
        "public sealed record UnknownRecord : StreamChatEvent",
        "public required string Event { get; init; }",
        "public required string Data { get; init; }",
        // The declared envelope metadata (id) is an event member.
        "public string? Id { get; init; }",
        // The completion carrier and its reason vocabulary.
        "public enum StreamChatCompletionReason",
        "public sealed record StreamChatCompletion(StreamChatCompletionReason Reason, StreamChatEvent? Usage);",
        // The per-frame decoder: sentinel first, then the per-kind decode
        // through the operation's existing stream item codec.
        "internal static StreamChatEvent DecodeStreamChatFrame(byte[] frame)",
        "var item = Codecs.DecodeStreamChatItem(frame);",
        "return new StreamChatEvent.Message { Data = item, Id = item.Id.HasValue ? item.Id.Value : null };",
        "return new StreamChatEvent.UnknownRecord { Event = kind, Data = data };",
    ] {
        assert!(
            events.contains(expected),
            "StreamEvents.g.cs lacks {expected}"
        );
    }
    // The declared [DONE] sentinel is matched on frame data before any payload
    // decoding and the final usage frame is held back for the completion.
    for expected in [
        "// The declared [DONE] sentinel completes the stream before any payload decoding.",
        "if (data == \"[DONE]\") { return null; }",
        "internal static StreamTranscriptionEvent? DecodeStreamTranscriptionFrame(byte[] frame)",
        "completion?.Invoke(new StreamTranscriptionCompletion(StreamTranscriptionCompletionReason.Sentinel, held));",
        "if (held is not null)",
        "completion?.Invoke(new StreamTranscriptionCompletion(StreamTranscriptionCompletionReason.Eof, held));",
    ] {
        assert!(
            client.contains(expected) || events.contains(expected),
            "emitted package lacks {expected}"
        );
    }
    // The declared sentinel enables the compiled keep-final-usage holding
    // policy; the un-sentinel operation yields every frame.
    assert!(client.contains("yield return frame;\n            }\n            completion?.Invoke(new StreamChatCompletion(StreamChatCompletionReason.Eof, null));"));

    // Controls without discrimination emit nothing.
    assert!(!client.contains("StreamLogsEvents"));
    assert!(!client.contains("StreamRowsEvents"));
    assert!(!events.contains("StreamLogsEvent"));
    assert!(!events.contains("StreamRowsEvent"));
    // The untyped direct calls and their item codecs stay exactly as before.
    assert!(client.contains("public Task<StreamChatResult> StreamChatAsync(StreamChatInput input, RequestOptions? requestOptions = null, CancellationToken cancellationToken = default)"));
    assert!(client.contains("raw.Stream(static bytes => Codecs.DecodeStreamChatItem(bytes))"));
}

#[ignore = "requires the .NET SDK on the test host"]
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
            .any(|file| file.path == "csharp/src/StreamEvents.g.cs")
    );
    let plain_client = source(&plain, "csharp/src/Client.g.cs");
    assert!(!plain_client.contains("EventsAsync"));
    assert!(!plain_client.contains("StreamChatEvent"));
    assert!(!plain_client.contains("Completion"));
    // The typed package adds exactly one file, and the generated client gains
    // only the appended typed events members.
    assert_eq!(
        typed.len(),
        plain.len() + 1,
        "the typed stream plan may add exactly one file"
    );
    let suffix = "}\n\ninternal static class HttpCodecs\n{\n}\n";
    let plain_body = plain_client
        .strip_suffix(suffix)
        .expect("client closing shape");
    let typed_client = source(&typed, "csharp/src/Client.g.cs");
    assert!(
        typed_client.starts_with(plain_body) && typed_client.ends_with(suffix),
        "the generated client may only gain the appended typed events members"
    );
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn plan_carries_the_compiled_stream_semantics() {
    use suspect_codegen::csharp_sdk;
    let contract = contract_with_document(stream_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = csharp_sdk::plan_sdk(contract, &selected, csharp_sdk::SdkConfig::default()).unwrap();
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
    assert!(chat.terminal.keep_final_usage == chat.sentinel.enabled);
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
    assert_eq!(
        rows.framing,
        suspect_codegen::http_protocol::StreamFraming::JsonLines
    );
}

fn dotnet() -> Option<String> {
    let path = std::env::var_os("SUSPECT_DOTNET_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/.local/share/mise/dotnet-root/dotnet"));
    let output = Command::new(&path).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| path.to_string_lossy().into_owned())
}

const DRIVER: &str = r#"
using System.Net;
using System.Text;
using Acme.TypedStreamsSdk;

var stub = new Stub();
using var http = new HttpClient(stub);
using var client = new Client(new Credentials(), httpClient: http);

// A declared message event decodes to the typed model, a declared done event
// follows, and end of body completes with an end-of-body completion.
{
    stub.Frames = "event: message\ndata: {\"text\":\"hello\"}\n\nevent: done\ndata: {}\n\n";
    stub.Sends = 0;
    var events = new List<StreamChatEvent>();
    await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput())) { events.Add(item); }
    if (events.Count != 2) throw new Exception($"expected 2 events, got {events.Count}");
    if (events[0] is not StreamChatEvent.Message message) throw new Exception("the first event is not the declared message kind");
    if (message.Data.Event != StreamChatItemEvent.Message) throw new Exception("the decoded envelope event differs");
    if (message.Data.Data != "{\"text\":\"hello\"}") throw new Exception($"the decoded envelope data differs: {message.Data.Data}");
    if (message.Kind != "message") throw new Exception("the declared event name differs");
    if (events[1] is not StreamChatEvent.Done) throw new Exception("the second event is not the declared done kind");
    if (stub.Sends != 1) throw new Exception($"typed iteration issued {stub.Sends} requests");
    var terminal = await client.StreamChatEventsCompletionAsync(new StreamChatInput());
    if (terminal.Reason != StreamChatCompletionReason.Eof) throw new Exception($"expected an end-of-body completion, got {terminal.Reason}");
    if (terminal.Usage is not null) throw new Exception("an end-of-body completion must preserve no usage without a declared sentinel");
    if (stub.Sends != 2) throw new Exception($"the completion accessor must drain its own stream; sends={stub.Sends}");
}

// Per-item metadata: the declared id field is exposed on the typed event.
{
    stub.Frames = "event: message\nid: 42\ndata: hello\n\n";
    stub.Sends = 0;
    await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput()))
    {
        if (item is not StreamChatEvent.Message message) throw new Exception("expected the declared message kind");
        if (message.Id != "42") throw new Exception($"the last-event id metadata differs: {message.Id}");
        if (message.Data.Id.Value != "42") throw new Exception("the decoded envelope id differs");
        break;
    }
}

// An undeclared event kind surfaces through the typed unknown alternative
// without failing the stream, and later declared events still decode.
{
    stub.Frames = "event: surprise\ndata: hello\n\nevent: message\ndata: ok\n\n";
    stub.Sends = 0;
    var kinds = new List<string>();
    await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput()))
    {
        if (item is StreamChatEvent.UnknownRecord unknown)
        {
            if (unknown.Event != "surprise" || unknown.Data != "hello") throw new Exception("the unknown event lost the raw frame");
            if (unknown.Kind != "unknown") throw new Exception("the unknown marker differs");
        }
        else if (item is StreamChatEvent.Message message)
        {
            if (message.Data.Data != "ok") throw new Exception("a declared event after an undeclared one failed to decode");
        }
        else
        {
            throw new Exception("unexpected event variant");
        }
        kinds.Add(item.Kind);
    }
    if (!kinds.SequenceEqual(new[] { "unknown", "message" })) throw new Exception($"event order differs: {string.Join(",", kinds)}");
}

// An invalid payload for a recognized kind remains a branded decoding error,
// and the response body is closed instead of read further.
{
    stub.Frames = "data: hello\n\n";
    stub.Sends = 0;
    stub.Endless = true;
    var thrown = false;
    try
    {
        await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput())) { _ = item; }
    }
    catch (SdkException error)
    {
        thrown = true;
        if (error.Kind != SdkErrorKind.ResponseDecode) throw new Exception($"expected a response-decoding failure, got {error.Kind}");
    }
    if (!thrown) throw new Exception("an invalid envelope payload did not fail the stream");
    await Task.Delay(50);
    if (!stub.Disposed) throw new Exception("a decoding failure must close the response body");
    var reads = stub.Reads;
    await Task.Delay(50);
    if (stub.Reads != reads) throw new Exception("a decoding failure kept reading the body");
    stub.Endless = false;
}

// The declared [DONE] sentinel completes the stream before any payload
// decoding, preserves the final usage frame as terminal metadata, and issues
// no further reads.
{
    stub.Frames = "event: message\ndata: {\"tokens\": 42}\n\ndata: [DONE]\n\n";
    stub.Sends = 0;
    stub.Endless = true;
    var terminal = await client.StreamTranscriptionEventsCompletionAsync(new StreamTranscriptionInput());
    if (terminal.Reason != StreamTranscriptionCompletionReason.Sentinel) throw new Exception($"expected a sentinel completion, got {terminal.Reason}");
    if (terminal.Usage is not StreamTranscriptionEvent.Message usage) throw new Exception("the sentinel completion lost the final usage frame");
    if (usage.Data.Data != "{\"tokens\": 42}") throw new Exception($"the preserved usage differs: {usage.Data.Data}");
    if (stub.Sends != 1) throw new Exception($"the completion accessor issued {stub.Sends} requests");
    await Task.Delay(50);
    if (!stub.Disposed) throw new Exception("the sentinel must close the response body");
    var reads = stub.Reads;
    await Task.Delay(50);
    if (stub.Reads != reads) throw new Exception("the sentinel kept reading the body");
    stub.Endless = false;
}

// Earlier frames are still yielded when a sentinel follows them, and the last
// data frame is held back as the completion's usage instead of being yielded.
{
    stub.Frames = "event: message\ndata: a\n\nevent: message\ndata: {\"usage\": true}\n\ndata: [DONE]\n\n";
    stub.Sends = 0;
    var collected = new List<string>();
    await foreach (var item in client.StreamTranscriptionEventsAsync(new StreamTranscriptionInput())) { collected.Add(((StreamTranscriptionEvent.Message)item).Data.Data); }
    if (!collected.SequenceEqual(new[] { "a" })) throw new Exception($"frames before the sentinel differ: {string.Join(",", collected)}");
    var terminal = await client.StreamTranscriptionEventsCompletionAsync(new StreamTranscriptionInput());
    if (terminal.Reason != StreamTranscriptionCompletionReason.Sentinel) throw new Exception($"expected a sentinel completion, got {terminal.Reason}");
    if (terminal.Usage is not StreamTranscriptionEvent.Message usage) throw new Exception("the sentinel completion lost the held final usage frame");
    if (usage.Data.Data != "{\"usage\": true}") throw new Exception($"the held usage differs: {usage.Data.Data}");
    if (stub.Sends != 2) throw new Exception($"each stream issues its own request: {stub.Sends}");
}

// Early break stops consumption: one event, one request, closed body.
{
    stub.Frames = "event: message\ndata: a\n\nevent: message\ndata: b\n\n";
    stub.Sends = 0;
    stub.Endless = true;
    var collected = new List<string>();
    await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput()))
    {
        collected.Add(((StreamChatEvent.Message)item).Data.Data);
        break;
    }
    if (!collected.SequenceEqual(new[] { "a" })) throw new Exception("early break collected the wrong frames");
    if (stub.Sends != 1) throw new Exception($"early break issued another request: {stub.Sends}");
    await Task.Delay(50);
    if (!stub.Disposed) throw new Exception("an early break must close the response body");
    var reads = stub.Reads;
    await Task.Delay(50);
    if (stub.Reads != reads) throw new Exception("an early break kept reading the body");
    stub.Endless = false;
}

// Cancellation before enumeration issues no request.
{
    stub.Frames = "event: message\ndata: a\n\n";
    stub.Sends = 0;
    using var cancelled = new CancellationTokenSource();
    cancelled.Cancel();
    try
    {
        await foreach (var item in client.StreamChatEventsAsync(new StreamChatInput(), cancelled.Token)) { _ = item; }
        throw new Exception("a cancelled iteration must not complete");
    }
    catch (Exception error) when (error is OperationCanceledException or SdkException) { }
    if (stub.Sends != 0) throw new Exception($"a cancelled iteration reached the transport: {stub.Sends}");
}

// The untyped direct call keeps its exact previous behavior: strictly decoded
// envelope items for well-formed frames.
{
    stub.Frames = "event: message\ndata: a\n\nevent: message\ndata: b\n\n";
    var response = await client.StreamChatAsync(new StreamChatInput());
    var items = new List<string>();
    await foreach (var item in response.Data) { items.Add(item.Data); }
    if (!items.SequenceEqual(new[] { "a", "b" })) throw new Exception($"the untyped stream differs: {string.Join(",", items)}");
}

Console.WriteLine("typed stream behavior verified: " + stub.Sends + " requests, " + stub.Reads + " reads");
return 0;

sealed class Stub : HttpMessageHandler
{
    internal string Frames = "";
    internal bool Endless;
    internal int Sends;
    internal int Reads;
    internal bool Disposed;

    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        Sends++;
        Disposed = false;
        var content = new SseContent(this);
        content.Headers.ContentType = new System.Net.Http.Headers.MediaTypeHeaderValue("text/event-stream");
        return Task.FromResult(new HttpResponseMessage(System.Net.HttpStatusCode.OK) { Content = content });
    }

    private sealed class SseStream(string frames, Stub stub) : Stream
    {
        private readonly byte[] _bytes = Encoding.UTF8.GetBytes(frames);
        private int _offset;

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => _bytes.Length - _offset;
        public override long Position { get => _offset; set => throw new NotSupportedException(); }
        public override void Flush() { }
        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken)
        {
            stub.Reads++;
            var count = Math.Min(buffer.Length, _bytes.Length - _offset);
            if (count > 0)
            {
                _bytes.AsMemory(_offset, count).CopyTo(buffer);
                _offset += count;
                return count;
            }
            if (!stub.Endless) return 0;
            // An endless body never terminates on its own: the runtime must close it.
            await Task.Delay(Timeout.InfiniteTimeSpan, cancellationToken);
            return 0;
        }

        protected override void Dispose(bool disposing)
        {
            stub.Disposed = true;
            base.Dispose(disposing);
        }
    }

    private sealed class SseContent(Stub stub) : HttpContent
    {
        protected override Task SerializeToStreamAsync(Stream stream, TransportContext? context) =>
            stream.WriteAsync(Encoding.UTF8.GetBytes(stub.Frames)).AsTask();

        protected override Task<Stream> CreateContentReadStreamAsync()
        {
            return Task.FromResult<Stream>(new SseStream(stub.Frames, stub));
        }

        protected override bool TryComputeLength(out long length)
        {
            length = 0;
            return false;
        }
    }
}
"#;

fn project(config: &str, framework: &str) -> String {
    format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{framework}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"{config}\" Version=\"[0.1.0]\" /></ItemGroup></Project>\n"
    )
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn typed_events_drive_stubbed_streams_in_dotnet() {
    let Some(dotnet) = dotnet() else {
        eprintln!("csharp_stream_typed: dotnet is not installed; degrading to static assertions");
        return;
    };
    eprintln!("csharp_stream_typed: {dotnet}");
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-stream-typed");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("stream-typed-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(stream_document()), &root).unwrap();
    fs::write(
        root.join("global.json"),
        b"{\"sdk\":{\"version\":\"8.0.424\",\"rollForward\":\"disable\"}}",
    )
    .unwrap();
    fs::create_dir_all(root.join("feed")).unwrap();
    fs::write(
        root.join("NuGet.Config"),
        b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    let run = |label: &str, directory: &str, arguments: &[&str]| {
        let output = Command::new(&dotnet)
            .args(arguments)
            .current_dir(root.join(directory))
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_HOME", root.join("dotnet-home"))
            .env("NUGET_PACKAGES", root.join("nuget-cache"))
            .output()
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        fs::write(
            root.join(format!("logs/{label}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            root.join(format!("logs/{label}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{label}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(
        "restore",
        "csharp",
        &[
            "restore",
            "Suspect.csproj",
            "--configfile",
            "../NuGet.Config",
        ],
    );
    run(
        "build",
        "csharp",
        &[
            "build",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-restore",
            "-m:1",
        ],
    );
    run(
        "pack",
        "csharp",
        &[
            "pack",
            "Suspect.csproj",
            "-c",
            "Release",
            "--no-build",
            "-o",
            "../feed",
        ],
    );
    let package = root.join("feed/acme.typed-streams-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.typed-streams-sdk", "net8.0"),
    )
    .unwrap();
    fs::write(root.join("consumer/Program.cs"), DRIVER).unwrap();
    run(
        "consumer-restore",
        "consumer",
        &["restore", "--configfile", "../NuGet.Config"],
    );
    run(
        "consumer-build",
        "consumer",
        &["build", "-c", "Release", "--no-restore", "-m:1"],
    );
    run(
        "consumer-run",
        "consumer",
        &["run", "-c", "Release", "--no-build"],
    );
    let assets: Value =
        serde_json::from_slice(&fs::read(root.join("consumer/obj/project.assets.json")).unwrap())
            .unwrap();
    assert_eq!(
        assets["libraries"]["acme.typed-streams-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}
