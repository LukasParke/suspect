//! Emitted-only incoming receipt helpers for the C# HTTP backend: the
//! generated `src/Incoming.g.cs` decoders, reply constructors and frozen
//! receipt descriptors, native .NET behavior against fake webhook deliveries,
//! and byte-identity for contracts without incoming declarations. Static
//! runtime files are never modified; the receipt helpers live entirely in the
//! generated package.

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
    let uri = Uri::parse("https://source.incoming.test/csharp-incoming.json").unwrap();
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

/// One webhook with a required header and required JSON body plus a declared
/// reply header, one schema-free JSON webhook, one webhook with a JSON reply,
/// and one operation-attached callback receipt with a runtime expression
/// route. The same shape the shared planner and the TypeScript/Python backends
/// pin.
fn incoming_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"],
                    "additionalProperties": false
                },
                "Event": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}},
                    "required": ["kind"],
                    "additionalProperties": false
                },
                "Pong": {
                    "type": "object",
                    "properties": {"pong": {"type": "boolean"}},
                    "required": ["pong"],
                    "additionalProperties": false
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"], "additionalProperties": false}}}}},
                "callbacks": {
                    "onEvent": {
                        "{$request.body#/callbackUrl}": {
                            "post": {
                                "operationId": "eventReceived",
                                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Event"}}}},
                                "responses": {"200": {"description": "ok"}}
                            }
                        }
                    }
                }
            }}
        },
        "webhooks": {
            "newIssue": {"post": {
                "operationId": "onNewIssue",
                "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                "responses": {"204": {"description": "Accepted", "headers": {"x-received": {"description": "when", "schema": {"type": "string"}}}}}
            }},
            "ping": {"post": {
                "operationId": "onPing",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
            }},
            "note": {"post": {
                "operationId": "onNote",
                "requestBody": {"content": {"application/json": {}}},
                "responses": {"204": {"description": "Accepted"}}
            }}
        }
    })
}

/// The control document: no webhooks and no callback. Nothing new may be
/// emitted for it.
fn control_document() -> Value {
    let mut document = incoming_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    document["paths"]["/subscribe"].as_object_mut().unwrap()["post"]
        .as_object_mut()
        .unwrap()
        .remove("callbacks")
        .expect("callbacks key");
    document
}

/// The control document with an empty webhooks map: still receipt-less.
fn emptied_document() -> Value {
    let mut document = control_document();
    document["webhooks"] = json!({});
    document
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "acme.incoming-sdk".into(),
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

fn files_map(files: &[OutFile]) -> std::collections::BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect()
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let configured = files_map(&generate(incoming_document()));
    let control = files_map(&generate(control_document()));
    let emptied = files_map(&generate(emptied_document()));

    // Exactly one new file: the incoming receipt helpers.
    let incoming = configured
        .get("csharp/src/Incoming.g.cs")
        .expect("the incoming helpers are emitted for a webhook/callback contract");
    assert!(
        configured.len() == control.len() + 1,
        "the declared receipts add exactly one file ({} vs {})",
        configured.len(),
        control.len()
    );
    assert!(!control.contains_key("csharp/src/Incoming.g.cs"));
    assert!(!emptied.contains_key("csharp/src/Incoming.g.cs"));
    // The receipt-less documents agree on their file lists and emit no
    // incoming helpers anywhere. (Their manifests embed document spans, so the
    // two documents' bytes legitimately differ; the receipt-less byte shape is
    // pinned by the incoming-free credential-env fixtures.)
    assert_eq!(
        control.keys().collect::<Vec<_>>(),
        emptied.keys().collect::<Vec<_>>(),
        "receipt-less documents must agree on their file lists"
    );
    for (path, content) in &control {
        assert!(
            !content.contains("DecodeNewIssueWebhook") && !content.contains("IncomingDescriptor"),
            "{path} leaked incoming helpers"
        );
    }
    assert!(
        !control.keys().any(|path| path.contains("incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );

    // The shared receipt types: the frozen route record, the typed failure,
    // the source locator and the compiled descriptor record.
    for expected in [
        "public sealed record IncomingRoute(string Method, string Path, bool Expression);",
        "public sealed class IncomingRequestException : Exception",
        "public IncomingRequestException(string message, Exception? cause = null) : base(message, cause) { }",
        "public sealed record IncomingSource(string Document, string Pointer);",
        "public sealed record IncomingDescriptor(string Kind, string Method, string Path, bool Expression, IncomingSource Source, IReadOnlyList<string> RequiredHeaders, string Payload, string? PayloadCodec, string Reply, int? ReplyStatus, string? ReplyCodec, IReadOnlyList<string> ReplyHeaders);",
        "public static class Incoming",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.g.cs lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The newIssue webhook receipt: frozen route, typed payload, header
    // presence check, required-body refusal and codec decode; the declared 204
    // reply pins the status and applies the declared reply header.
    for expected in [
        "public static readonly IncomingRoute NewIssueWebhookRoute = new(\"POST\", \"newIssue\", false);",
        "public static IssueEvent DecodeNewIssueWebhook(IReadOnlyDictionary<string, string> headers, ReadOnlyMemory<byte> body)",
        "RequireHeader(headers, \"x-signature\");",
        "if (body.Length == 0) throw new IncomingRequestException(\"the declared required receipt body is absent\");",
        "return Codecs.DecodeWebhooksNewIssuePostRequestBody(body.Span);",
        "throw new IncomingRequestException(\"the received payload does not satisfy its declared schema (https://source.incoming.test/csharp-incoming.json#/webhooks/newIssue/post)\", error);",
        "public static (int Status, Dictionary<string, string> Headers, byte[] Body) ConstructNewIssueResponse(IReadOnlyDictionary<string, string>? typedHeaders = null)",
        "foreach (var name in new[] { \"x-received\" })",
        "var value = HeaderValue(typedHeaders, name);",
        "if (value is not null) headers[name] = value;",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.g.cs lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The ping receipt decodes nothing and its declared 200 reply encodes
    // through the response codec; the note receipt decodes schema-free JSON.
    for expected in [
        "public static void DecodePingWebhook(IReadOnlyDictionary<string, string> headers, ReadOnlyMemory<byte> body)",
        "public static (int Status, Dictionary<string, string> Headers, byte[] Body) ConstructPingResponse(Pong result)",
        "return (200, headers, Codecs.EncodeWebhooksPingPostResponsesValue200(result));",
        "public static global::System.Text.Json.JsonElement DecodeNoteWebhook(IReadOnlyDictionary<string, string> headers, ReadOnlyMemory<byte> body)",
        "return JsonRuntime.Parse(body.Span);",
        "throw new IncomingRequestException(\"the received payload is not valid JSON (https://source.incoming.test/csharp-incoming.json#/webhooks/note/post)\", error);",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.g.cs lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The callback receipt carries its runtime expression verbatim and decodes
    // through its own declared schema.
    for expected in [
        "public static readonly IncomingRoute SubscribeOnEventWebhookRoute = new(\"POST\", \"{$request.body#/callbackUrl}\", true);",
        "public static Event DecodeSubscribeOnEventWebhook(IReadOnlyDictionary<string, string> headers, ReadOnlyMemory<byte> body)",
        "return Codecs.DecodeEventReceivedRequest(body.Span);",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.g.cs lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The frozen compiled descriptors: one entry per declared receipt.
    for expected in [
        "public static readonly IReadOnlyDictionary<string, IncomingDescriptor> Receipts = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, IncomingDescriptor>(new Dictionary<string, IncomingDescriptor>(StringComparer.Ordinal)",
        "[\"newIssue\"] = new IncomingDescriptor(\"webhook\", \"POST\", \"newIssue\", false, new IncomingSource(\"https://source.incoming.test/csharp-incoming.json\", \"/webhooks/newIssue/post\"), new[] { \"x-signature\" }, \"json\", \"WebhooksNewIssuePostRequestBody\", \"none\", 204, null, new[] { \"x-received\" }),",
        "[\"note\"] = new IncomingDescriptor(\"webhook\", \"POST\", \"note\", false, new IncomingSource(\"https://source.incoming.test/csharp-incoming.json\", \"/webhooks/note/post\"), global::System.Array.Empty<string>(), \"schema-free-json\", null, \"none\", 204, null, global::System.Array.Empty<string>()),",
        "[\"ping\"] = new IncomingDescriptor(\"webhook\", \"POST\", \"ping\", false, new IncomingSource(\"https://source.incoming.test/csharp-incoming.json\", \"/webhooks/ping/post\"), global::System.Array.Empty<string>(), \"none\", null, \"json\", 200, \"WebhooksPingPostResponsesValue200\", global::System.Array.Empty<string>()),",
        "[\"subscribe_onEvent\"] = new IncomingDescriptor(\"callback\", \"POST\", \"{$request.body#/callbackUrl}\", true, new IncomingSource(\"https://source.incoming.test/csharp-incoming.json\", \"/paths/~1subscribe/post/callbacks/onEvent/{$request.body#~1callbackUrl}/post\"), global::System.Array.Empty<string>(), \"json\", \"EventReceivedRequest\", \"none\", 200, null, global::System.Array.Empty<string>()),",
        "internal static string? HeaderValue(IReadOnlyDictionary<string, string>? headers, string name)",
        "internal static void RequireHeader(IReadOnlyDictionary<string, string> headers, string name)",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.g.cs lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn plan_carries_the_compiled_incoming_receipts_only_when_emission_happens() {
    use suspect_codegen::csharp_sdk;
    let contract = contract_with_document(incoming_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = csharp_sdk::plan_sdk(contract, &selected, csharp_sdk::SdkConfig::default()).unwrap();
    let incoming = plan.incoming();
    assert_eq!(incoming.operations().len(), 4);
    let new_issue = incoming
        .operations()
        .iter()
        .find(|operation| operation.name() == "newIssue")
        .expect("the newIssue webhook plan");
    assert_eq!(
        new_issue.kind(),
        suspect_codegen::http_protocol::IncomingKind::Webhook
    );
    assert_eq!(new_issue.method().as_str(), "POST");
    assert_eq!(new_issue.route().route(), "newIssue");
    assert!(!new_issue.route().expression());
    assert!(!incoming.codec_roots().is_empty());
    let callback = incoming
        .operations()
        .iter()
        .find(|operation| operation.name() == "subscribe.onEvent")
        .expect("the callback plan");
    assert_eq!(
        callback.kind(),
        suspect_codegen::http_protocol::IncomingKind::Callback
    );
    assert_eq!(callback.route().route(), "{$request.body#/callbackUrl}");
    assert!(callback.route().expression());

    let control_contract = contract_with_document(control_document());
    let control_selected = control_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = csharp_sdk::plan_sdk(
        control_contract,
        &control_selected,
        csharp_sdk::SdkConfig::default(),
    )
    .unwrap();
    assert!(control.incoming().is_empty());
    assert!(control.incoming().codec_roots().is_empty());
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
using System.Text;
using System.Text.Json;
using Acme.IncomingSdk;

// Route constants are registration hints: the declared method is what the
// provider sends and the route is carried verbatim.
if (Incoming.NewIssueWebhookRoute != new IncomingRoute("POST", "newIssue", false)) throw new Exception("the newIssue route differs");
if (Incoming.PingWebhookRoute.Method != "POST" || Incoming.PingWebhookRoute.Path != "ping" || Incoming.PingWebhookRoute.Expression) throw new Exception("the ping route differs");
if (Incoming.SubscribeOnEventWebhookRoute != new IncomingRoute("POST", "{$request.body#/callbackUrl}", true)) throw new Exception("the callback route differs");

// A valid fake webhook POST decodes into the declared model, regardless of
// header-name casing.
var payload = Incoming.DecodeNewIssueWebhook(new Dictionary<string, string> { ["X-Signature"] = "abc" }, Encoding.UTF8.GetBytes("""{"id":"i1","title":"t"}"""));
if (payload.Id != "i1" || payload.Title != "t") throw new Exception($"the decoded payload differs: {payload}");

// A missing required header is the typed incoming failure.
try
{
    Incoming.DecodeNewIssueWebhook(new Dictionary<string, string>(), Encoding.UTF8.GetBytes("""{"id":"i1","title":"t"}"""));
    throw new Exception("a missing required header must fail");
}
catch (IncomingRequestException) { }

// An invalid payload is a typed failure carrying the codec failure as its cause.
try
{
    Incoming.DecodeNewIssueWebhook(new Dictionary<string, string> { ["x-signature"] = "abc" }, Encoding.UTF8.GetBytes("""{"id":"i1"}"""));
    throw new Exception("an invalid payload must fail");
}
catch (IncomingRequestException error)
{
    if (error.InnerException is not CodecException) throw new Exception($"the codec cause differs: {error.InnerException?.GetType().FullName}");
}

// A required declared body refuses an empty delivery.
try
{
    Incoming.DecodeNewIssueWebhook(new Dictionary<string, string> { ["x-signature"] = "abc" }, ReadOnlyMemory<byte>.Empty);
    throw new Exception("an empty required body must fail");
}
catch (IncomingRequestException) { }

// A schema-free JSON body decodes as the parsed JSON element.
var note = Incoming.DecodeNoteWebhook(new Dictionary<string, string>(), Encoding.UTF8.GetBytes("""{"seen":true}"""));
if (note.ValueKind != JsonValueKind.Object || !note.GetProperty("seen").GetBoolean()) throw new Exception($"the schema-free payload differs: {note}");

// Body-less receipts validate their headers and return.
Incoming.DecodePingWebhook(new Dictionary<string, string>(), ReadOnlyMemory<byte>.Empty);

// The declared 204 reply constructs with the pinned status and the declared
// reply header applied from the caller-provided values.
var accepted = Incoming.ConstructNewIssueResponse(new Dictionary<string, string> { ["X-Received"] = "now" });
if (accepted.Status != 204) throw new Exception($"the reply status differs: {accepted.Status}");
if (accepted.Headers.Count != 1 || accepted.Headers["x-received"] != "now") throw new Exception($"the reply headers differ: {string.Join(",", accepted.Headers)}");
if (accepted.Body.Length != 0) throw new Exception("a body-less reply must carry no bytes");

// The declared 200 reply encodes its body through the response codec.
var pong = Codecs.DecodePong("""{"pong":true}""");
var reply = Incoming.ConstructPingResponse(pong);
if (reply.Status != 200) throw new Exception($"the reply status differs: {reply.Status}");
if (!reply.Body.SequenceEqual(Codecs.EncodePong(pong))) throw new Exception("the reply body differs");
if (reply.Headers.Count != 0) throw new Exception("an undeclared reply header must not appear");

// The callback reply constructs its declared 200 with no body.
var callback = Incoming.ConstructSubscribeOnEventResponse();
if (callback.Status != 200 || callback.Body.Length != 0 || callback.Headers.Count != 0) throw new Exception("the callback reply differs");

// The frozen compiled descriptors carry the receipt facts.
if (Incoming.Receipts["newIssue"].Payload != "json" || Incoming.Receipts["newIssue"].PayloadCodec != "WebhooksNewIssuePostRequestBody") throw new Exception("the newIssue payload descriptor differs");
if (Incoming.Receipts["newIssue"].Reply != "none" || Incoming.Receipts["newIssue"].ReplyStatus != 204) throw new Exception("the newIssue reply descriptor differs");
if (Incoming.Receipts["newIssue"].RequiredHeaders.SequenceEqual(new[] { "x-signature" }) != true) throw new Exception("the newIssue header descriptor differs");
if (Incoming.Receipts["subscribe_onEvent"].Path != "{$request.body#/callbackUrl}" || !Incoming.Receipts["subscribe_onEvent"].Expression) throw new Exception("the callback descriptor differs");
if (Incoming.Receipts["ping"].Reply != "json" || Incoming.Receipts["ping"].ReplyCodec != "WebhooksPingPostResponsesValue200" || Incoming.Receipts["ping"].ReplyStatus != 200) throw new Exception("the ping reply descriptor differs");
if (Incoming.Receipts["note"].Payload != "schema-free-json") throw new Exception("the note payload descriptor differs");

Console.WriteLine("incoming behavior verified");
return 0;
"#;

fn project(config: &str, framework: &str) -> String {
    format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{framework}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"{config}\" Version=\"[0.1.0]\" /></ItemGroup></Project>\n"
    )
}

fn incoming_run(label: &str, directory: &str, arguments: &[&str], root: &Path, dotnet: &str) {
    let output = Command::new(dotnet)
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
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn receipts_drive_the_decoder_and_constructor_in_dotnet() {
    let Some(dotnet) = dotnet() else {
        eprintln!("csharp_incoming: dotnet is not installed; degrading to static assertions");
        return;
    };
    eprintln!("csharp_incoming: {dotnet}");
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-incoming");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("incoming-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(incoming_document()), &root).unwrap();
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
    incoming_run(
        "restore",
        "csharp",
        &[
            "restore",
            "Suspect.csproj",
            "--configfile",
            "../NuGet.Config",
        ],
        &root,
        &dotnet,
    );
    incoming_run(
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
        &root,
        &dotnet,
    );
    incoming_run(
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
        &root,
        &dotnet,
    );
    let package = root.join("feed/acme.incoming-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.incoming-sdk", "net8.0"),
    )
    .unwrap();
    fs::write(root.join("consumer/Program.cs"), DRIVER).unwrap();
    incoming_run(
        "consumer-restore",
        "consumer",
        &["restore", "--configfile", "../NuGet.Config"],
        &root,
        &dotnet,
    );
    incoming_run(
        "consumer-build",
        "consumer",
        &["build", "-c", "Release", "--no-restore", "-m:1"],
        &root,
        &dotnet,
    );
    incoming_run(
        "consumer-run",
        "consumer",
        &["run", "-c", "Release", "--no-build"],
        &root,
        &dotnet,
    );
    let assets: Value =
        serde_json::from_slice(&fs::read(root.join("consumer/obj/project.assets.json")).unwrap())
            .unwrap();
    assert_eq!(
        assets["libraries"]["acme.incoming-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}
