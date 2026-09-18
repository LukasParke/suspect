//! M2 per-language elimination gate for the C# HTTP SDK (`csharp-http`):
//! real `dotnet pack` SDK packages, a NuGet-referencing single-operation
//! consumer, and System.Reflection.Metadata inspection of both the
//! consumer's compiled unit and a `PublishTrimmed` self-contained publish,
//! mirroring the TypeScript bundler gate's property — a single-operation
//! consumer artifact must not retain unrelated operations.
//!
//! Three gates exist by design and all run here:
//! - **Dependency gate:** the emitted `Suspect.csproj` declares zero
//!   `<PackageReference>` items, so no transitive retention is possible.
//! - **Consumer unit gate:** the one-operation consumer (it only calls
//!   `Client.GetGadgetAsync`) is built against the packed SDK and inspected
//!   with System.Reflection.Metadata: its assembly must not reference any
//!   unrelated operation type. The artifact-level composition shape is then
//!   documented from the SDK assembly itself: `Client` carries every
//!   operation, so the unrelated operation's methods remain in the artifact
//!   (the plan's M0 root-composition caveat, measured instead of assumed).
//! - **Trim gate:** `dotnet publish -p:PublishTrimmed=true` on the
//!   one-operation consumer rewrites the SDK assembly; the measured
//!   surviving-method set on `Client` records whether the trimmer eliminates
//!   unreferenced operations.
//!
//! Additivity: the trimmed publish directory size must not grow beyond a
//! small constant when the unrelated operation is added to the source.
//!
//! Retained artifacts live under `target/sdk-csharp-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/csharp-elimination-report.json`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

static NATIVE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Fixed logical source URI for both fixture documents: emitted source
/// references stay byte-comparable across the two generations.
const ENTRY: &str = "https://source.elimination.test/openapi.json";

/// Same-length retained-artifact labels keep emitted source paths
/// length-identical across generations.
const BASE_LABEL: &str = "base";
const EXTENDED_LABEL: &str = "extd";

const PACKAGE_ID: &str = "Elimination.Sdk";
const VERSION: &str = "0.0.0";
const DOTNET_SDK: &str = "8.0.424";
const RUNTIME_IDENTIFIER: &str = "osx-arm64";

/// Methods of the four operations the one-operation consumer never calls.
const ABSENT_METHODS: &[&str] = &[
    "ListWidgetsAsync",
    "StreamChatAsync",
    "CreateBannerAsync",
    "ListLicensesAsync",
];
/// The unrelated operation's type/method marker.
const UNRELATED_MARKER: &str = "ListGizmos";
const MARKER_OWN_METHOD: &str = "GetGadgetAsync";

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

/// The canonical elimination fixture (identical to the TypeScript elimination
/// fixture): one limit/offset paginated list, one discriminated SSE stream,
/// one plain JSON read, one OAuth2 client-credentials operation and one
/// device-flow operation, under an API-key document policy.
fn elimination_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Elimination fixture", "version": "1.0.0"},
        "servers": [{"url": "https://api.elimination.test/v1"}],
        "security": [{"apiKey": []}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "summary": "List widgets with limit/offset pagination.",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer", "minimum": 1}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer", "minimum": 0}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Widget page", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}
            }},
            "/chat": {"post": {
                "operationId": "streamChat",
                "summary": "Stream chat completion events.",
                "responses": {"200": {"description": "Chat events", "content": {"text/event-stream": {"itemSchema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    },
                    "required": ["event", "data"]
                }}}}}
            }},
            "/gadgets/{gadgetId}": {"get": {
                "operationId": "getGadget",
                "summary": "Fetch one gadget.",
                "parameters": [{"name": "gadgetId", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"200": {"description": "Gadget", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/Gadget"}
                }}}}
            }},
            "/banners": {"post": {
                "operationId": "createBanner",
                "summary": "Create a banner with OAuth2 client credentials.",
                "security": [{"serviceOAuth": ["read"]}],
                "requestBody": {"required": true, "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/BannerCreate"}
                }}},
                "responses": {"200": {"description": "Banner acknowledged", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]
                }}}}}
            }},
            "/licenses": {"get": {
                "operationId": "listLicenses",
                "summary": "List licenses with the device-authorization scheme.",
                "security": [{"deviceOAuth": []}],
                "responses": {"200": {"description": "Licenses", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"items": {"type": "array", "items": {"type": "string"}}},
                    "required": ["items"]
                }}}}}
            }}
        },
        "components": {
            "securitySchemes": {
                "apiKey": {"type": "http", "scheme": "bearer"},
                "serviceOAuth": {"type": "oauth2", "flows": {
                    "authorizationCode": {
                        "authorizationUrl": "https://auth.elimination.test/authorize",
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access", "write": "Write access"}
                    },
                    "clientCredentials": {
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access"}
                    }
                }},
                "deviceOAuth": {"type": "oauth2", "flows": {"deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.elimination.test/device",
                    "tokenUrl": "https://auth.elimination.test/token",
                    "scopes": {}
                }}}
            },
            "schemas": {
                "BannerCreate": {"type": "object", "additionalProperties": false,
                    "properties": {"text": {"type": "string"}, "weight": {"type": "integer"}},
                    "required": ["text"]},
                "Gadget": {"type": "object", "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string", "enum": ["standard", "compact"]},
                        "label": {"type": "string"}
                    },
                    "required": ["kind", "label"]}
            }
        }
    })
}

/// The same contract plus one operation that no consumer references. Its
/// pointers sort strictly between existing ones, so every shared source
/// pointer is stable and the two generations are comparable.
fn document_with_unrelated_operation() -> Value {
    let mut document = elimination_document();
    let root = document.as_object_mut().unwrap();
    root.get_mut("paths")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "/gizmos".to_owned(),
            json!({"get": {
                "operationId": "listGizmos",
                "summary": "Unrelated gizmo inventory probe.",
                "responses": {"200": {"description": "Gizmos", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/UnrelatedGizmo"}
                }}}}
            }}),
        );
    root.get_mut("components")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("schemas")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "UnrelatedGizmo".to_owned(),
            json!({"type": "object", "additionalProperties": false,
                "properties": {
                    "flavor": {"type": "string", "enum": ["zeta-quantum"]},
                    "serial": {"type": "string"}
                },
                "required": ["flavor", "serial"]}),
        );
    document
}

fn elimination_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_CLIENT_SECRET".into()),
                            revocation_endpoint: Some(
                                "https://auth.elimination.test/revoke".into(),
                            ),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "deviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_DEVICE_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_DEVICE_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
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
        &TargetConfig {
            backend: Backend::CsharpHttp,
            package_name: PACKAGE_ID.into(),
            package_version: VERSION.into(),
            import_name: None,
        },
        &elimination_options(),
    )
    .unwrap()
}

fn dotnet() -> PathBuf {
    std::env::var_os("SUSPECT_DOTNET_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into())
}

fn dotnet_command(directory: &Path, home: &Path) -> Command {
    let mut command = Command::new(dotnet());
    command
        .current_dir(directory)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE", "true")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env("DOTNET_CLI_HOME", home.join("dotnet-home"))
        .env("NUGET_PACKAGES", home.join("nuget-cache"));
    command
}

/// Consumer-side dotnet invocation: same environment isolation except the
/// NuGet cache, which stays machine-default so the self-contained publish can
/// resolve the runtime packs and ILLink tasks.
fn consumer_dotnet_command(directory: &Path, home: &Path) -> Command {
    let mut command = Command::new(dotnet());
    command
        .current_dir(directory)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE", "true")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env("DOTNET_CLI_HOME", home.join("dotnet-home"));
    command
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: required native tool is unavailable: {error}"));
    fs::create_dir_all(retained.join("logs")).unwrap();
    let text = format!(
        "$ {command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(retained.join(format!("logs/{label}.log")), &text).unwrap();
    assert!(
        output.status.success(),
        "{label} failed; artifacts retained at {}\n{text}",
        retained.display()
    );
    text
}

/// The one-operation consumer: it constructs the client and references
/// exactly one operation. It is built, published and inspected, never run
/// against a server.
const CONSUMER_CSPROJ: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <LangVersion>12.0</LangVersion>
    <Nullable>enable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
    <!-- IL2104: the emitted SDK uses reflection-shaped runtimes the trimmer
         cannot prove safe; the trim gate measures what survives regardless. -->
    <NoWarn>$(NoWarn);IL2104</NoWarn>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Elimination.Sdk" Version="0.0.0" />
  </ItemGroup>
</Project>
"#;

const CONSUMER_PROGRAM: &str = r#"using Elimination.Sdk;

public static class Program
{
    public static async System.Threading.Tasks.Task Main()
    {
        var client = new Client();
        var input = new GetGadgetInput { GadgetId = "g" };
        await using var result = await client.GetGadgetAsync(input);
        System.Console.WriteLine(result.Status);
    }
}
"#;

/// The System.Reflection.Metadata inspector: a zero-dependency net8.0 console
/// tool. `refs <dll>` prints every referenced type; `types <dll>` prints every
/// defined type and method. The gate greps its output.
const INSPECTOR_CSPROJ: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
  </PropertyGroup>
</Project>
"#;

const INSPECTOR_PROGRAM: &str = r#"using System;
using System.IO;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;

public static class Program
{
    public static void Main(string[] args)
    {
        var mode = args[0];
        using var stream = File.OpenRead(args[1]);
        using var pe = new PEReader(stream);
        var metadata = pe.GetMetadataReader();
        if (mode == "refs")
        {
            foreach (var handle in metadata.TypeReferences)
            {
                var reference = metadata.GetTypeReference(handle);
                var ns = metadata.GetString(reference.Namespace);
                var name = metadata.GetString(reference.Name);
                Console.WriteLine(string.IsNullOrEmpty(ns) ? $"TYPEREF {name}" : $"TYREF {ns}.{name}");
            }
            foreach (var handle in metadata.MemberReferences)
            {
                var member = metadata.GetMemberReference(handle);
                var name = metadata.GetString(member.Name);
                var parent = "";
                if (member.Parent.Kind == HandleKind.TypeReference)
                {
                    var typeReference = metadata.GetTypeReference((TypeReferenceHandle)member.Parent);
                    var ns = metadata.GetString(typeReference.Namespace);
                    var typeReferenceName = metadata.GetString(typeReference.Name);
                    parent = string.IsNullOrEmpty(ns) ? typeReferenceName : $"{ns}.{typeReferenceName}";
                }
                Console.WriteLine($"MEMBERREF {parent}::{name}");
            }
        }
        else
        {
            foreach (var handle in metadata.TypeDefinitions)
            {
                var definition = metadata.GetTypeDefinition(handle);
                var ns = metadata.GetString(definition.Namespace);
                var name = metadata.GetString(definition.Name);
                var full = string.IsNullOrEmpty(ns) ? name : $"{ns}.{name}";
                Console.WriteLine($"TYPE {full}");
                foreach (var method in definition.GetMethods())
                {
                    var methodDefinition = metadata.GetMethodDefinition(method);
                    Console.WriteLine($"METHOD {full}::{metadata.GetString(methodDefinition.Name)}");
                }
            }
        }
    }
}
"#;

fn write_inspector(root: &Path) -> PathBuf {
    let inspector = root.join("inspector");
    fs::create_dir_all(inspector.join("src")).unwrap();
    fs::write(inspector.join("Inspector.csproj"), INSPECTOR_CSPROJ).unwrap();
    fs::write(inspector.join("src/Program.cs"), INSPECTOR_PROGRAM).unwrap();
    fs::write(
        root.join("global.json"),
        format!("{{\"sdk\":{{\"version\":\"{DOTNET_SDK}\",\"rollForward\":\"disable\"}}}}"),
    )
    .unwrap();
    let home = root.join("dotnet");
    fs::create_dir_all(&home).unwrap();
    let mut command = dotnet_command(&inspector, &home);
    command.args(["build", "-v", "q", "-nologo"]);
    checked(&mut command, root, "inspector-build");
    inspector
}

fn inspector_binary(inspector: &Path) -> PathBuf {
    inspector.join("bin/Debug/net8.0/Inspector.dll")
}

fn inspect(root: &Path, inspector: &Path, mode: &str, dll: &Path) -> String {
    let mut command = Command::new(dotnet());
    command
        .arg(inspector_binary(inspector))
        .arg(mode)
        .arg(dll)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_HOME", root.join("dotnet/dotnet-home"));
    checked(&mut command, root, &format!("inspect-{mode}"))
}

fn directory_bytes(directory: &Path) -> u64 {
    let mut total = 0;
    for entry in walk(directory) {
        if entry.is_file() {
            total += entry.metadata().unwrap().len();
        }
    }
    total
}

fn walk(directory: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(current).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                paths.push(path);
            }
        }
    }
    paths
}

fn write_report(report: &Value) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("csharp-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn csharp_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-elimination");
    fs::create_dir_all(&root).unwrap();
    let inspector = write_inspector(&root);

    let mut measurements = BTreeMap::new();
    let mut publish_bytes = BTreeMap::new();
    for (label, document) in [
        (BASE_LABEL, elimination_document()),
        (EXTENDED_LABEL, document_with_unrelated_operation()),
    ] {
        let generation = root.join(label);
        fs::create_dir_all(&generation).unwrap();
        suspect_codegen::write_files(&generate(document), &generation).unwrap();
        fs::write(
            generation.join("NuGet.Config"),
            "<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/><add key=\"nuget.org\" value=\"https://api.nuget.org/v3/index.json\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource><packageSource key=\"nuget.org\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n",
        )
        .unwrap();
        fs::create_dir_all(generation.join("feed")).unwrap();

        // Dependency gate: the emitted csproj declares zero PackageReference.
        let csproj = fs::read_to_string(generation.join("csharp/Suspect.csproj")).unwrap();
        assert!(
            !csproj.contains("<PackageReference"),
            "csharp-http csproj must declare zero PackageReference items; found {csproj}"
        );
        measurements.insert(
            format!("{label}/csproj-dependencies"),
            json!("zero (no <PackageReference> items)"),
        );

        // Pack the SDK into the local feed.
        let mut command = dotnet_command(&generation.join("csharp"), &root.join("dotnet"));
        command.args(["pack", "Suspect.csproj", "-v", "q", "-nologo"]);
        checked(&mut command, &generation, "dotnet-pack");
        let packed = generation.join(format!("csharp/bin/Release/{PACKAGE_ID}.{VERSION}.nupkg"));
        assert!(packed.is_file(), "packed package at {}", packed.display());
        fs::copy(
            &packed,
            generation.join(format!("feed/{PACKAGE_ID}.{VERSION}.nupkg")),
        )
        .unwrap();

        // Build the single-operation consumer against the packed SDK. The
        // NuGet cache keys by id+version, so the previous generation's SDK
        // package must be purged before restoring this generation's.
        let cache = root.join("dotnet/dotnet-home/.nuget/packages/elimination.sdk");
        if cache.exists() {
            fs::remove_dir_all(&cache).unwrap();
        }
        let consumer = generation.join("consumer");
        fs::create_dir_all(&consumer).unwrap();
        fs::write(consumer.join("Consumer.csproj"), CONSUMER_CSPROJ).unwrap();
        fs::write(consumer.join("Program.cs"), CONSUMER_PROGRAM).unwrap();
        let mut command = consumer_dotnet_command(&consumer, &root.join("dotnet"));
        command.args(["build", "-v", "q", "-nologo"]);
        checked(&mut command, &generation, "dotnet-consumer-build");
        let consumer_dll = consumer.join("bin/Debug/net8.0/Consumer.dll");
        assert!(consumer_dll.is_file());
        let sdk_dll = consumer.join("bin/Debug/net8.0/Elimination.Sdk.dll");
        assert!(sdk_dll.is_file());

        // Consumer unit gate: its compiled assembly references exactly one
        // operation.
        let references = inspect(&generation, &inspector, "refs", &consumer_dll);
        for method in ABSENT_METHODS {
            assert!(
                !references.contains(method),
                "{label}: the single-operation consumer assembly references unrelated operation member {method}"
            );
        }
        assert!(
            !references.contains(UNRELATED_MARKER),
            "{label}: the single-operation consumer assembly references the unrelated operation"
        );
        assert!(
            references.contains(MARKER_OWN_METHOD),
            "{label}: the consumer assembly must reference {MARKER_OWN_METHOD}"
        );

        // Artifact composition shape (documented, not fixed): the SDK
        // assembly's Client carries every operation.
        let sdk_types = inspect(&generation, &inspector, "types", &sdk_dll);
        let client_methods: Vec<&str> = sdk_types
            .lines()
            .filter(|line| line.starts_with("METHOD Elimination.Sdk.Client::"))
            .filter_map(|line| line.rsplit("::").next())
            .collect();
        assert!(
            client_methods.contains(&MARKER_OWN_METHOD),
            "SDK Client must expose {MARKER_OWN_METHOD}"
        );
        if label == EXTENDED_LABEL {
            assert!(
                client_methods.contains(&"ListGizmosAsync"),
                "documented composition retention regressed: Client no longer exposes the unrelated operation"
            );
        } else {
            assert!(!client_methods.contains(&"ListGizmosAsync"));
        }

        // Trim gate: publish the consumer self-contained with IL trimming and
        // measure what survives in the rewritten SDK assembly.
        let mut command = consumer_dotnet_command(&consumer, &root.join("dotnet"));
        command.args([
            "publish",
            "-c",
            "Release",
            "-r",
            RUNTIME_IDENTIFIER,
            "--self-contained",
            "true",
            "-p:PublishTrimmed=true",
            "-v",
            "q",
            "-nologo",
        ]);
        checked(&mut command, &generation, "dotnet-publish-trimmed");
        let publish = consumer.join("bin/Release/net8.0/osx-arm64/publish");
        let trimmed_sdk = publish.join("Elimination.Sdk.dll");
        assert!(
            trimmed_sdk.is_file(),
            "trimmed SDK at {}",
            trimmed_sdk.display()
        );
        let trimmed_types = inspect(&generation, &inspector, "types", &trimmed_sdk);
        let trimmed_client_methods: Vec<&str> = trimmed_types
            .lines()
            .filter(|line| line.starts_with("METHOD Elimination.Sdk.Client::"))
            .filter_map(|line| line.rsplit("::").next())
            .collect();
        let bytes = directory_bytes(&publish);
        publish_bytes.insert(label, bytes);
        measurements.insert(
            format!("{label}/trimmed_client_methods"),
            json!(trimmed_client_methods),
        );
        measurements.insert(format!("{label}/publish_bytes"), json!(bytes));
        println!("{label}: publish {bytes} B; trimmed Client methods: {trimmed_client_methods:?}");
    }

    // Additivity: the trimmed publish must not grow beyond a small constant
    // when the unrelated operation is added to the source.
    let base_bytes = publish_bytes.get(BASE_LABEL).unwrap();
    let extended_bytes = publish_bytes.get(EXTENDED_LABEL).unwrap();
    let delta = extended_bytes.abs_diff(*base_bytes);
    const ADDITIVITY_SLACK_BYTES: u64 = 65_536;
    assert!(
        delta <= ADDITIVITY_SLACK_BYTES,
        "the trimmed publish grew {delta} B when the unrelated operation was added (base {base_bytes} B, extended {extended_bytes} B)"
    );

    write_report(&json!({
        "gate": "csharp-http consumer assembly refs (System.Reflection.Metadata) + PublishTrimmed gate",
        "toolchain": format!("dotnet SDK {DOTNET_SDK}, rid {RUNTIME_IDENTIFIER}"),
        "fixture": ENTRY,
        "recorded_composition_shape": "the emitted package is one artifact: Client carries every operation (measured via the untrimmed assembly); the consumer's own compiled assembly references only its operation",
        "measurements": measurements,
        "additivity": {
            "base_publish_bytes": base_bytes,
            "extended_publish_bytes": extended_bytes,
            "delta_bytes": delta,
            "slack_bytes": ADDITIVITY_SLACK_BYTES,
        },
    }));
}
