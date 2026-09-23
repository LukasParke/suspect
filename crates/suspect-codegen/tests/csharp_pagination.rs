//! Emitted-only pagination for the C# HTTP backend: descriptor map, generated
//! Client walkers, no-policy byte-identity, and native .NET behavior against a
//! stubbed HttpMessageHandler. Static runtime files are never modified; the
//! walk lives entirely in the generated package.

#![cfg(feature = "csharp-sdk")]

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract() -> Arc<Contract> {
    let uri = Uri::parse("https://source.pagination.test/csharp-pagination.json").unwrap();
    let provider = Arc::new(
        suspect_ref::DocumentProvider::new([suspect_ref::ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&pagination_document()).unwrap(),
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

/// One limit/offset list operation and one cursor list operation.
fn pagination_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Pagination", "version": "1"},
        "servers": [{"url": "https://api.pagination.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer"}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}}
            },
            "/events": {"get": {
                "operationId": "listEvents",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "items": {"type": "array", "items": {"type": "string"}},
                        "next_page_token": {"type": "string"}
                    },
                    "required": ["items"]
                }}}}}}
            }
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "acme.pagination-sdk".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate(contract: Arc<Contract>, options: &GenerationOptions) -> Vec<OutFile> {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
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
fn pagination_walkers_and_descriptors_emit_only_under_sdk_defaults() {
    let shared = contract();
    let files = generate(shared.clone(), &configured_options());
    let pagination = source(&files, "csharp/src/Pagination.g.cs");
    let client = source(&files, "csharp/src/Client.g.cs");
    for expected in [
        // Typed traversal error and frozen descriptors.
        "public sealed class PaginationException : Exception",
        "public sealed record PaginationDescriptor(string Operation, string Pattern, IReadOnlyDictionary<string, string> Request, IReadOnlyDictionary<string, string> Response, int? InitialOffset, string Advance)",
        "public static class PaginationDescriptors",
        "[\"listWidgets\"] = new PaginationDescriptor(\"listWidgets\", \"limit-offset\"",
        "[\"listEvents\"] = new PaginationDescriptor(\"listEvents\", \"cursor\"",
        "[\"limit\"] = \"Limit\"",
        "[\"offset\"] = \"Offset\"",
        "[\"cursor\"] = \"Cursor\"",
        "[\"items\"] = \"/data\"",
        "[\"total\"] = \"/total\"",
        "[\"items\"] = \"/items\"",
        "[\"nextCursor\"] = \"/next_page_token\"",
        "\"items-returned\"",
        "\"next-offset\"",
        // Pointer readers resolve with plain property chains.
        "internal static global::System.Collections.Generic.List<string>? ListWidgetsPageItems(ListWidgetsResult page)",
        "return page.Data.Data;",
        "internal static string? ListEventsPageCursor(ListEventsResult page)",
        // Continuation rules: counting advance and the cursor stall guard.
        "baseOffset = input.Offset.HasValue ? input.Offset.Value.ToBigInteger() : 0",
        "advanced = JsonInteger.FromInteger(baseOffset + count);",
        "if (count == 0)",
        "input with { Offset = Optional<JsonInteger>.Present(advanced) }",
        "input with { Cursor = Optional<string>.Present(token) }",
        "the source API returned an identical continuation value; the paginated walk would never terminate",
    ] {
        assert!(
            pagination.contains(expected),
            "Pagination.g.cs lacks {expected}\n--- emitted: ---\n{pagination}"
        );
    }
    for expected in [
        "public async global::System.Collections.Generic.IAsyncEnumerable<ListWidgetsResult> ListWidgetsPagesAsync(ListWidgetsInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        "public async global::System.Collections.Generic.IAsyncEnumerable<string> ListWidgetsItems(ListWidgetsInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        "public async Task<ListWidgetsInput?> ListWidgetsNextPageAsync(ListWidgetsInput input, CancellationToken cancellationToken = default)",
        "public async global::System.Collections.Generic.IAsyncEnumerable<ListEventsResult> ListEventsPagesAsync(ListEventsInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        "public async global::System.Collections.Generic.IAsyncEnumerable<string> ListEventsItems(ListEventsInput input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)",
        "public async Task<ListEventsInput?> ListEventsNextPageAsync(ListEventsInput input, CancellationToken cancellationToken = default)",
        "await ListWidgetsAsync(current, cancellationToken: cancellationToken).ConfigureAwait(false);",
        "yield return page;",
        // First-page fill of the configured initial offset; a caller-supplied
        // value still wins.
        "if (!current.Offset.HasValue)",
        "current = current with { Offset = Optional<JsonInteger>.Present(new JsonInteger(\"0\")) };",
    ] {
        assert!(client.contains(expected), "Client.g.cs lacks {expected}");
    }

    // Without SDK defaults nothing new is emitted at all. The generated client
    // gains only the appended walkers; every other file stays byte-identical.
    let mut control = generate(shared, &GenerationOptions::default());
    let mut configured = files;
    sorted(&mut control);
    sorted(&mut configured);
    assert!(
        !control
            .iter()
            .any(|file| file.path == "csharp/src/Pagination.g.cs")
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    let suffix = "}\n\ninternal static class HttpCodecs\n{\n}\n";
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if file.path == "csharp/src/Client.g.cs" {
            let plain_body = file
                .content
                .strip_suffix(suffix)
                .expect("client closing shape");
            assert!(
                emitted.content.starts_with(plain_body) && emitted.content.ends_with(suffix),
                "{} may only gain the appended pagination walkers",
                file.path
            );
            continue;
        }
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn disabled_pagination_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1", "pagination": "off"
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let shared = contract();
    let mut disabled = generate(shared.clone(), &off);
    let mut control = generate(shared, &GenerationOptions::default());
    sorted(&mut disabled);
    sorted(&mut control);
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[ignore = "requires the .NET SDK on the test host"]
#[test]
fn plan_carries_the_pagination_outcome_only_when_configured() {
    use suspect_codegen::csharp_sdk;
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = csharp_sdk::plan_sdk_with_options(
        contract.clone(),
        &selected,
        csharp_sdk::SdkConfig::default(),
        csharp_sdk::protocol::ProtocolOptions {
            sdk_defaults: Some(SdkDefaults::v1()),
            ..Default::default()
        },
    )
    .unwrap();
    let outcome = configured
        .pagination()
        .expect("configured policy is carried");
    assert_eq!(outcome.paginated.len(), 2);
    let widgets = outcome
        .paginated
        .iter()
        .find(|page| page.operation == "listWidgets")
        .expect("limit/offset operation");
    assert_eq!(widgets.initial_offset, Some(0));
    assert_eq!(
        widgets.advance,
        suspect_codegen::sdk_defaults::PaginationAdvance::ItemsReturned
    );
    let control = csharp_sdk::plan_sdk_with_options(
        contract,
        &selected,
        csharp_sdk::SdkConfig::default(),
        csharp_sdk::protocol::ProtocolOptions::default(),
    )
    .unwrap();
    assert!(control.pagination().is_none());
}

fn dotnet() -> Option<String> {
    // Resolution + version gating live in the toolchain manifest; the
    // `SUSPECT_DOTNET_BIN` override and the mise install are honored there.
    suspect_codegen::toolchain::gate("dotnet")
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

const DRIVER: &str = r#"
using System.Net;
using System.Text;
using Acme.PaginationSdk;

var sends = 0;
var queue = new Queue<string>();
var stub = new Stub(() =>
{
    sends++;
    if (queue.Count == 0) throw new Exception($"unexpected extra request {sends}");
    return queue.Dequeue();
});
using var http = new HttpClient(stub);
using var client = new Client(new Credentials(), httpClient: http);

static Dictionary<string, string> Query(string query) => query
    .Split('&', StringSplitOptions.RemoveEmptyEntries)
    .Select(pair => pair.Split('='))
    .ToDictionary(parts => Uri.UnescapeDataString(parts[0]), parts => Uri.UnescapeDataString(parts[1]));

// Two-page limit/offset walk: exactly 2 requests, offset 0 then 2, with the
// filter parameter preserved on every page.
{
    queue.Enqueue("""{"data":["a","b"],"total":4}""");
    queue.Enqueue("""{"data":["c","d"],"total":4}""");
    var pages = new List<ListWidgetsResult>();
    await foreach (var page in client.ListWidgetsPagesAsync(new ListWidgetsInput { Limit = new JsonInteger("2"), Filter = "active" }))
    {
        pages.Add(page);
        if (pages.Count == 2) break;
    }
    if (pages.Count != 2) throw new Exception($"expected 2 pages, got {pages.Count}");
    if (!pages[0].Data.Data.SequenceEqual(new[] { "a", "b" })) throw new Exception("page 1 data differs");
    if (!pages[1].Data.Data.SequenceEqual(new[] { "c", "d" })) throw new Exception("page 2 data differs");
    if (sends != 2) throw new Exception($"breaking after two pages must not start a third request; sends={sends}");
    var first = Query(stub.Requests[0]);
    var second = Query(stub.Requests[1]);
    if (first["limit"] != "2" || first["offset"] != "0" || first["filter"] != "active") throw new Exception($"page 1 wire differs: {stub.Requests[0]}");
    if (second["limit"] != "2" || second["offset"] != "2" || second["filter"] != "active") throw new Exception($"page 2 wire differs: {stub.Requests[1]}");
}

// Caller-supplied offset wins for page 1 and is replaced afterwards.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"data":["a"],"total":4}""");
    queue.Enqueue("""{"data":[],"total":4}""");
    var input = new ListWidgetsInput { Limit = new JsonInteger("2"), Offset = new JsonInteger("10") };
    var pages = 0;
    await foreach (var page in client.ListWidgetsPagesAsync(input)) { pages++; }
    // The empty page that ends the walk is still fetched and yielded once.
    if (pages != 2 || sends != 2) throw new Exception($"empty-page stop rule changed; pages={pages} sends={sends}");
    if (Query(stub.Requests[0])["offset"] != "10") throw new Exception("caller offset lost on page 1");
    if (Query(stub.Requests[1])["offset"] != "11") throw new Exception("offset did not advance by items returned");
}

// Full items walk: the empty third page stops the walk.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"data":["a","b"],"total":4}""");
    queue.Enqueue("""{"data":["c"],"total":4}""");
    queue.Enqueue("""{"data":[],"total":4}""");
    var items = new List<string>();
    await foreach (var item in client.ListWidgetsItems(new ListWidgetsInput { Limit = new JsonInteger("2") })) { items.Add(item); }
    if (!items.SequenceEqual(new[] { "a", "b", "c" })) throw new Exception("item walk flattened wrong: " + string.Join(",", items));
    if (sends != 3) throw new Exception($"items walk request count differs: {sends}");
    if (Query(stub.Requests[2])["offset"] != "3") throw new Exception("third request offset differs");
}

// Cursor walk: cursor=c2 on the second request; stops when the token is
// missing from the page.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"items":["x","y"],"next_page_token":"c2"}""");
    queue.Enqueue("""{"items":["z"]}""");
    var events = new List<string>();
    await foreach (var item in client.ListEventsItems(new ListEventsInput())) { events.Add(item); }
    if (!events.SequenceEqual(new[] { "x", "y", "z" })) throw new Exception("cursor item walk differs");
    if (sends != 2) throw new Exception($"cursor walk request count differs: {sends}");
    if (Query(stub.Requests[0]).ContainsKey("cursor")) throw new Exception("first request invented a cursor");
    if (Query(stub.Requests[1])["cursor"] != "c2") throw new Exception("second request cursor differs");
}

// An empty cursor token stops the walk.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"items":["x"],"next_page_token":""}""");
    var pages = 0;
    await foreach (var page in client.ListEventsPagesAsync(new ListEventsInput())) { pages++; }
    if (pages != 1 || sends != 1) throw new Exception($"empty token must stop the walk; pages={pages} sends={sends}");
}

// Early break after the first item issues no second request.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"items":["x","y"],"next_page_token":"c2"}""");
    queue.Enqueue("""{"items":["z"]}""");
    await foreach (var item in client.ListEventsItems(new ListEventsInput()))
    {
        break;
    }
    if (sends != 1) throw new Exception($"early break issued another request: {sends}");
}

// A repeated identical continuation value throws the typed pagination error
// instead of looping forever.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"items":["x"],"next_page_token":"again"}""");
    queue.Enqueue("""{"items":["x"],"next_page_token":"again"}""");
    var thrown = false;
    try
    {
        await foreach (var item in client.ListEventsItems(new ListEventsInput())) { _ = item; }
    }
    catch (PaginationException error)
    {
        thrown = true;
        if (!error.Message.Contains("identical continuation")) throw new Exception($"unexpected message: {error.Message}");
    }
    if (!thrown) throw new Exception("a repeated continuation did not throw PaginationException");
    if (sends != 2) throw new Exception($"the walk must stop at the repeated value; sends={sends}");
}

// Manual driving through NextPage: fetches the page described by the input and
// returns the rebuilt input for the following page, or null at the stop rule.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    queue.Enqueue("""{"data":["a","b"],"total":4}""");
    queue.Enqueue("""{"data":["c","d"],"total":4}""");
    queue.Enqueue("""{"data":[],"total":4}""");
    var input = new ListWidgetsInput { Limit = new JsonInteger("2"), Filter = "active" };
    var followed = 0;
    for (;;)
    {
        var next = await client.ListWidgetsNextPageAsync(input);
        if (next is null) break;
        input = next;
        followed++;
    }
    if (followed != 2) throw new Exception($"next-page driving differs: {followed}");
    if (input.Offset.Value.Token != "4") throw new Exception($"next input lost the advanced offset: {input.Offset.Value.Token}");
    if (input.Filter.Value != "active") throw new Exception("next input dropped the filter");
    if (sends != 3) throw new Exception($"next-page driving issued extra requests: {sends}");
}

// Cancellation before enumeration issues no request.
{
    sends = 0; stub.Requests.Clear(); queue.Clear();
    using var cancelled = new CancellationTokenSource();
    cancelled.Cancel();
    try
    {
        await foreach (var page in client.ListEventsPagesAsync(new ListEventsInput(), cancelled.Token)) { _ = page; }
        throw new Exception("a cancelled walk must not complete");
    }
    catch (Exception error) when (error is OperationCanceledException or SdkException) { }
    if (sends != 0) throw new Exception($"a cancelled walk reached the transport: {sends}");
}

Console.WriteLine("pagination behavior verified: " + sends + " requests in the last check");
return 0;

sealed class Stub(Func<string> respond) : HttpMessageHandler
{
    internal readonly List<string> Requests = new();
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        Requests.Add(request.RequestUri!.Query.Length == 0 ? "" : request.RequestUri.Query[1..]);
        var content = new StringContent(respond(), Encoding.UTF8, "application/json");
        return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = content });
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
fn native_walks_drive_a_stubbed_http_handler() {
    let Some(dotnet) = dotnet() else {
        eprintln!(
            "csharp_pagination: skipping — {}",
            suspect_codegen::toolchain::guidance("dotnet")
        );
        return;
    };
    eprintln!("csharp_pagination: {dotnet}");
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-pagination");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("pagination-")
        .tempdir_in(fs::canonicalize(&parent).unwrap())
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generate(contract(), &configured_options()), &root).unwrap();
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
    let package = root.join("feed/acme.pagination-sdk.0.1.0.nupkg");
    assert!(
        package.is_file(),
        "NuGet artifact missing: {}",
        package.display()
    );
    fs::create_dir_all(root.join("consumer")).unwrap();
    fs::write(
        root.join("consumer/Consumer.csproj"),
        project("acme.pagination-sdk", "net8.0"),
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
        assets["libraries"]["acme.pagination-sdk/0.1.0"]["type"], "package",
        "the consumer must use the installed package"
    );
}
