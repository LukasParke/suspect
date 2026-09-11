//! C# plan admission and actual .NET package/consumer/runtime gates.
//! Native gates retain their artifacts and command logs under target/sdk-csharp-*.

#![cfg(feature = "csharp-sdk")]

use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};
use suspect_codegen::csharp_sdk::{SdkConfig, models::CsDecl, models::CsType, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &std::path::Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn contract(schemas: Value, paths: Value) -> Arc<Contract> {
    raw_contract(json!({
        "openapi":"3.1.0", "info":{"title":"C# SDK","version":"1"},
        "servers":[{"url":"https://api.example.test"}], "security":[{"apiKey":[]}],
        "paths":paths, "components":{"schemas":schemas,"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}}
    }))
}

fn raw_contract(value: Value) -> Arc<Contract> {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-test-sources");
    fs::create_dir_all(&parent).unwrap();
    let directory = tempfile::tempdir_in(parent).unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    load(&path)
}

fn one_operation(schemas: Value) -> Arc<Contract> {
    contract(
        schemas,
        json!({
            "/things": {
                "get": {
                    "operationId": "listThings",
                    "responses": {
                        "200": {
                            "description": "ok",
                            "content": {
                                "application/json": {"schema": {"$ref": "#/components/schemas/Thing"}}
                            }
                        }
                    }
                }
            }
        }),
    )
}

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract.operations().map(|o| o.source().clone()).collect()
}

#[test]
fn plan_admits_object_records_with_presence_and_extras() {
    let contract = one_operation(json!({
        "Thing": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "score": {"type": "number"},
                "count": {"type": "integer"},
                "note": {"type": ["string", "null"]},
                "tags": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["name", "score"],
            "additionalProperties": {"type": "string"}
        }
    }));
    let plan = plan_sdk(contract.clone(), &selected(&contract), SdkConfig::default())
        .expect("object records are representable");
    assert_eq!(plan.operations().len(), 1);
    assert_eq!(plan.operations()[0].method_name, "ListThingsAsync");
    let record = plan
        .models()
        .declarations
        .values()
        .find_map(|decl| match decl {
            CsDecl::Record { fields, extras } => Some((fields, extras)),
            _ => None,
        })
        .expect("Thing lowers to a record");
    let (fields, extras) = record;
    assert_eq!(fields.len(), 5);
    let score = fields.iter().find(|f| f.wire == "score").unwrap();
    assert!(
        matches!(score.ty, CsType::Number),
        "wire numbers keep exact tokens"
    );
    let count = fields.iter().find(|f| f.wire == "count").unwrap();
    assert!(matches!(count.ty, CsType::Integer));
    let note = fields.iter().find(|f| f.wire == "note").unwrap();
    assert!(note.nullable, "type:[string,null] keeps the null state");
    assert!(
        !note.required,
        "optional null fields track absence distinctly"
    );
    let name = fields.iter().find(|f| f.wire == "name").unwrap();
    assert!(name.required && !name.nullable);
    assert!(
        extras.is_some(),
        "additionalProperties schema keeps typed extras"
    );
}

#[test]
fn one_of_ref_branches_plan_as_sealed_unions() {
    let contract = one_operation(json!({
        "A": {"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": false},
        "B": {"type": "object", "properties": {"y": {"type": "integer"}}, "additionalProperties": false},
        "Either": {"oneOf": [{"$ref": "#/components/schemas/A"}, {"$ref": "#/components/schemas/B"}]},
        "Thing": {"$ref": "#/components/schemas/Either"}
    }));
    let plan = plan_sdk(contract.clone(), &selected(&contract), SdkConfig::default())
        .expect("ref unions are representable");
    assert!(
        plan.models()
            .declarations
            .values()
            .any(|decl| matches!(decl, CsDecl::Union { branches } if branches.len() == 2))
    );
}

#[test]
fn unsupported_shapes_reject_with_located_findings() {
    let cases = [
        // allOf intersection with an inline member has no faithful shape.
        (
            "Nope",
            json!({"Nope": {"allOf": [
                {"$ref": "#/components/schemas/A"},
                {"type": "object", "properties": {"z": {"type": "string"}}}
            ]}, "A": {"type": "object", "properties": {}, "additionalProperties": false}}),
        ),
        // Mixed literal kinds have no single underlying C# type.
        ("Mixed", json!({"Mixed": {"enum": ["a", 1]}})),
        // Multiple non-null type alternatives are not invented unions.
        ("Join", json!({"Join": {"type": ["string", "integer"]}})),
    ];
    for (root, schemas) in cases {
        // The unsupported shape replaces the operation root so it is
        // actually reachable during planning.
        let paths = json!({
            "/things": {
                "get": {
                    "operationId": "listThings",
                    "responses": {
                        "200": {
                            "description": "ok",
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": format!("#/components/schemas/{root}")}
                                }
                            }
                        }
                    }
                }
            }
        });
        let contract = contract(schemas, paths);
        let error = plan_sdk(contract.clone(), &selected(&contract), SdkConfig::default())
            .expect_err("unsupported shapes must reject");
        assert!(!error.is_empty(), "findings must be explicit for {root}");
        assert!(
            error.iter().all(|d| !d.message.is_empty()),
            "every finding carries a reason: {error:?}"
        );
    }
}

#[test]
fn packaging_identity_is_validated() {
    let contract = one_operation(json!({"Thing": {"type": "object", "properties": {}}}));
    let error = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            version: ">=1.0.0".into(),
            ..SdkConfig::default()
        },
    )
    .expect_err("version ranges are not exact SemVer");
    assert!(error.iter().any(|d| d.code == "http-packaging-identity"));
}

#[test]
fn retained_native_descriptors_and_source_bound_onboarding_are_deterministic() {
    let contract = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    let config = SdkConfig {
        name: "Example.Widgets".into(),
        version: "1.2.3".into(),
        namespace: "Example.Widgets".into(),
    };
    let plan = plan_sdk(contract.clone(), &selected(&contract), config.clone()).unwrap();
    assert_eq!(plan.package(), &config);
    assert!(Arc::ptr_eq(plan.contract(), &contract));
    let create = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "createWidget")
        .unwrap();
    assert_eq!(
        (create.http_method.as_str(), create.path.as_str()),
        ("POST", "/widgets")
    );
    assert_eq!(create.body.as_ref().unwrap().native_type, "WidgetInput");
    assert!(create.body.as_ref().unwrap().required);
    assert_eq!(
        create
            .responses
            .iter()
            .find(|r| r.may_succeed())
            .unwrap()
            .type_name,
        "CreateWidgetResult"
    );
    assert_eq!(
        create
            .responses
            .iter()
            .find(|r| r.wire.status_key() == "401")
            .unwrap()
            .error_type_name,
        "CreateWidgetApiException.Status401"
    );
    plan.program().check().unwrap();
    let first = plan.render().unwrap();
    let second = plan.render().unwrap();
    assert_eq!(
        first
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>()
    );
    assert!(first.iter().all(|f| f.path.starts_with("csharp/")));
    let quickstart = &first
        .iter()
        .find(|f| f.path == "csharp/examples/Quickstart.cs")
        .unwrap()
        .content;
    assert!(quickstart.contains("new WidgetInput { Name = \"alpha\" }"));
    assert!(quickstart.contains("catch (CreateWidgetApiException.Status401 error)"));
    assert!(!quickstart.contains("Codecs.Decode"));
    let example_project = &first
        .iter()
        .find(|f| f.path == "csharp/examples/Examples.csproj")
        .unwrap()
        .content;
    assert!(
        example_project.contains("PackageReference")
            && !example_project.contains("ProjectReference")
    );
    assert!(
        first
            .iter()
            .find(|f| f.path == "csharp/docs/reference.json")
            .unwrap()
            .content
            .contains("M:Example.Widgets.Client.CreateWidgetAsync")
    );
}

#[test]
fn unsupported_protocols_and_dialects_have_source_located_admission_failures() {
    let original = one_operation(json!({"Thing":{"type":"object","properties":{}}}));
    let base = original.document(original.entry()).unwrap();
    let mut cases = Vec::new();
    let mut value = base.clone();
    value["paths"]["/things"]["get"]["parameters"] = json!([{"name":"x","in":"query","style":"deepObject","explode":false,"schema":{"type":"object"}}]);
    cases.push((value, "http-parameter-combination-undefined"));
    let mut value = base.clone();
    value["components"]["schemas"]["Thing"]["properties"] =
        json!({"private":{"type":"string","writeOnly":true}});
    cases.push((value, "csharp-directional-codec-unsupported"));
    for (value, code) in cases {
        let contract = raw_contract(value);
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap_err();
        let error = errors
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("expected {code}: {errors:?}"));
        assert!(
            contract.source(&error.source).is_some(),
            "original source address for {code}"
        );
        assert!(error.at.end > error.at.start, "original span for {code}");
    }
}

#[test]
fn unsupported_fields_never_receive_json_or_string_fallbacks() {
    let contract = one_operation(
        json!({"Thing":{"type":"object","properties":{"unsupported":{"enum":["x",1,{"a":true}]}}}}),
    );
    let errors = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "csharp-literal-representation-unsupported"
                && e.source.pointer().ends_with("/properties/unsupported"))
    );
    let mut value = contract.document(contract.entry()).unwrap().clone();
    value["components"]["schemas"]["Thing"]["properties"]["unsupported"] = json!({"type":"string"});
    value["components"]["schemas"]["Unused"] =
        json!({"allOf":[{"type":"string"},{"type":"object"}]});
    let selected_only = raw_contract(value);
    plan_sdk(
        selected_only.clone(),
        &selected(&selected_only),
        Default::default(),
    )
    .expect("an unselected schema does not expand the requested SDK closure");
}

#[test]
fn split_static_references_keep_native_identity_and_example_bindings() {
    let contract =
        load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/split.openapi.yaml"));
    let plan = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap();
    let operation = &plan.operations()[0];
    let body = operation.body.as_ref().unwrap();
    assert_eq!(body.native_type, "Node");
    assert!(
        body.media[0]
            .wire
            .source()
            .terminal()
            .source()
            .document()
            .as_str()
            .ends_with("split-resources.yaml")
    );
    assert_ne!(
        body.source.document(),
        body.media[0].wire.source().terminal().source().document()
    );
    assert!(plan.examples().operations()[0].entries.iter().any(|entry| {
        entry
            .declared_source
            .as_ref()
            .is_some_and(|id| id.document().as_str().ends_with("split-resources.yaml"))
    }));
    assert!(
        plan.render()
            .unwrap()
            .iter()
            .any(|file| file.path == "csharp/examples/Quickstart.cs"
                && file.content.contains("new Node { Label = \"seed\" }"))
    );
}

#[test]
#[ignore = "requires installed .NET SDKs 8.0.424 and 10.0.400; retains native evidence"]
fn native_package_builds_and_consumes_negative_types() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let original = fs::read(&path).unwrap();
    let contract = load(&path);
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            name: "Suspect.Csharp.M2".into(),
            version: "1.0.0".into(),
            namespace: "Suspect.Csharp.M2".into(),
        },
    )
    .unwrap();
    for (sdk, framework) in TOOLCHAINS {
        let native = Native::new("m2", sdk, framework, &plan.render().unwrap());
        native.write("source.openapi.yaml", &original);
        let package = native.package(plan.package());
        native.consumer(
            plan.package(),
            include_str!("../src/csharp_sdk/testdata/M2Consumer.cs"),
            &package,
        );
        native.examples();
        native.negative(plan.package(), &[
            ("body-required", "_ = new CreateWidgetInput();", "CS9035"),
            ("name-required", "_ = new WidgetInput();", "CS9035"),
            ("nonnull-body", "_ = new CreateWidgetInput { Body = null };", "CS8625"),
            ("nonnull-name", "_ = new WidgetInput { Name = null };", "CS8625"),
            ("exact-number", "_ = new WidgetInput { Name = \"x\", Amount = 1.5 };", "CS0029"),
            ("boolean-number", "JsonNumber value = true; _ = value;", "CS0029"),
            ("wrong-union-arm", "_ = new WidgetPayload.StandardPayload(new SecurePayload { Vault = \"x\" });", "CS1503"),
            ("union-closed", "_ = new WidgetPayload();", "CS0144"),
            ("union-inheritance", "_ = 0; sealed class Foreign : WidgetPayload { }", "CS1729"),
            ("nonnull-optional", "_ = new WidgetNode { Label = \"x\", Child = Optional<WidgetNode>.Present(null) };", "CS8625"),
            ("unknown-field", "_ = new WidgetInput { Name = \"x\", NoSuch = true };", "CS0117"),
        ]);
        native.finish("m2", &original);
    }
    assert_eq!(
        fs::read(path).unwrap(),
        original,
        "the M2 fixture is read-only"
    );
}

const TOOLCHAINS: &[(&str, &str)] = &[("8.0.424", "net8.0"), ("10.0.400", "net10.0")];

#[test]
#[ignore = "newly admitted real OpenRouter anonymous/undeclared-body operation; .NET 8/10 installed consumers"]
fn native_openrouter_expanded_anonymous_operation() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/github/openrouter-web"));
    let path = root.join("projects/docs/openapi/openapi.yaml");
    let before = fs::read(&path).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .filter(|op| {
            [
                "createCoinbaseCharge",
                "downloadContainerFileContent",
                "downloadFileContent",
            ]
            .contains(&op.operation_id().unwrap_or_default())
        })
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    let config = SdkConfig {
        name: "Suspect.Csharp.Coinbase".into(),
        version: "1.0.0".into(),
        namespace: "Suspect.Csharp.Coinbase".into(),
    };
    assert!(
        plan_sdk(contract.clone(), &selected, config.clone()).is_err(),
        "legacy binary string markers require explicit opt-in"
    );
    let plan = suspect_codegen::csharp_sdk::plan_sdk_with_options(
        contract,
        &selected,
        config,
        suspect_codegen::csharp_sdk::protocol::ProtocolOptions {
            compatibility_profiles: vec![
                suspect_codegen::http_protocol::CompatibilityProfile::LegacyBinaryStringV1,
            ],
            ..Default::default()
        },
    )
    .unwrap();
    for (sdk, framework) in TOOLCHAINS {
        let native = Native::new(
            "openrouter-expanded",
            sdk,
            framework,
            &plan.render().unwrap(),
        );
        let package = native.package(plan.package());
        native.consumer(plan.package(),r#"
using Suspect.Csharp.Coinbase;
using System.Net;
using System.Text;
var sends=0;
using var http=new HttpClient(new Handler(request=>{
    if(request.Method.Method=="GET"){
        sends++;if(request.Headers.Authorization?.ToString()!="Bearer explicit-key"||!(request.RequestUri!.OriginalString=="https://openrouter.ai/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA/content"||request.RequestUri.OriginalString=="https://openrouter.ai/api/v1/files/or_file_abc/content"))throw new Exception("Actual download request differs");
        var bytes=new HttpResponseMessage(HttpStatusCode.OK){Content=new ByteArrayContent(new byte[]{0,255,42})};bytes.Content.Headers.TryAddWithoutValidation("Content-Type","application/octet-stream");return bytes;
    }
    if(request.Method.Method!="POST"||request.RequestUri!.OriginalString!="https://openrouter.ai/api/v1/credits/coinbase"||request.Headers.Contains("Authorization")||request.Content is not null)throw new Exception("Actual source wire contract differs");
    var response=new HttpResponseMessage(sends++==0?HttpStatusCode.OK:HttpStatusCode.Gone){Content=new ByteArrayContent(sends==1?new byte[]{0,255,42}:Encoding.UTF8.GetBytes("{\"error\":{\"code\":410,\"message\":\"gone\"}}"))};
    if(sends!=1)response.Content.Headers.TryAddWithoutValidation("Content-Type","application/json");return response;
}));
using var client=new Client(new Credentials{ApiKey="explicit-key"},httpClient:http);
var response=await client.CreateCoinbaseChargeAsync();if(!response.Data.SequenceEqual(new byte[]{0,255,42}))throw new Exception("Unspecified response bytes changed");
try{await client.CreateCoinbaseChargeAsync();throw new Exception("Expected declared 410");}catch(CreateCoinbaseChargeApiException.Status410 error){if(error.Data.Error.Message!="gone"||error.Response?.Status!=410)throw new Exception("Typed source error differs");}
var container=await client.DownloadContainerFileContentAsync(new DownloadContainerFileContentInput{ContainerId="sess_abc123",FileId="a/b 雪"});
var file=await client.DownloadFileContentAsync(new DownloadFileContentInput{FileId="or_file_abc"});
if(!container.Data.SequenceEqual(new byte[]{0,255,42})||!file.Data.SequenceEqual(container.Data)||sends!=4)throw new Exception("Exact download bytes or send count differ");Console.WriteLine("ACTUAL OPENROUTER EXPANSION PASS: createCoinbaseCharge, downloadContainerFileContent, downloadFileContent; explicit legacy binary profile, auth separation, raw bytes, typed 410; "+System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription);
sealed class Handler(Func<HttpRequestMessage,HttpResponseMessage> respond):HttpMessageHandler{protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken token){token.ThrowIfCancellationRequested();return Task.FromResult(respond(request));}}
"#,&package);
        native.examples();
        native.finish("actual-openrouter-expanded-anonymous", &before);
    }
    assert_eq!(
        fs::read(path).unwrap(),
        before,
        "tracked OpenRouter source stays read-only"
    );
}

struct Native {
    root: PathBuf,
    framework: String,
}
impl Native {
    fn new(label: &str, sdk: &str, framework: &str, files: &[suspect_codegen::OutFile]) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-native");
        fs::create_dir_all(&parent).unwrap();
        let parent = fs::canonicalize(parent).unwrap();
        let root = tempfile::Builder::new()
            .prefix(&format!("{label}-{sdk}-"))
            .tempdir_in(parent)
            .unwrap()
            .keep();
        let this = Self {
            root,
            framework: framework.into(),
        };
        suspect_codegen::write_files(files, &this.root).unwrap();
        this.write(
            "global.json",
            format!("{{\"sdk\":{{\"version\":\"{sdk}\",\"rollForward\":\"disable\"}}}}").as_bytes(),
        );
        fs::create_dir_all(this.root.join("feed")).unwrap();
        this.write("NuGet.Config", b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n");
        let output = this.checked("toolchain", ".", &["--version"]);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), sdk);
        println!("native C# artifacts: {}", this.root.display());
        this
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn command(&self, directory: &str, args: &[&str]) -> Command {
        let dotnet = std::env::var_os("SUSPECT_DOTNET_BIN")
            .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into());
        let mut command = Command::new(dotnet);
        command
            .args(args)
            .current_dir(self.root.join(directory))
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE", "true")
            .env("MSBUILDDISABLENODEREUSE", "1")
            .env("DOTNET_CLI_HOME", self.root.join("dotnet-home"))
            .env("NUGET_PACKAGES", self.root.join("nuget-cache"));
        command
    }
    fn output(&self, label: &str, directory: &str, args: &[&str]) -> Output {
        let mut command = self.command(directory, args);
        let output = command
            .output()
            .unwrap_or_else(|error| panic!("{command:?}: {error}"));
        self.write(&format!("logs/{label}.stdout.log"), &output.stdout);
        self.write(&format!("logs/{label}.stderr.log"), &output.stderr);
        self.write(
            &format!("logs/{label}.command.json"),
            serde_json::to_vec_pretty(
                &json!({"command":format!("{command:?}"),"status":output.status.code()}),
            )
            .unwrap()
            .as_slice(),
        );
        output
    }
    fn checked(&self, label: &str, directory: &str, args: &[&str]) -> Output {
        let output = self.output(label, directory, args);
        assert!(
            output.status.success(),
            "{label}; artifacts {}\n{}{}",
            self.root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn package(&self, config: &SdkConfig) -> PathBuf {
        self.checked(
            "restore",
            "csharp",
            &[
                "restore",
                "Suspect.csproj",
                "--configfile",
                "../NuGet.Config",
            ],
        );
        self.checked(
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
        self.checked(
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
                "--include-source",
                "--include-symbols",
            ],
        );
        let package = self
            .root
            .join(format!("feed/{}.{}.nupkg", config.name, config.version));
        assert!(
            package.is_file(),
            "NuGet artifact missing: {}",
            package.display()
        );
        package
    }
    fn project(&self, config: &SdkConfig, negative: bool) -> String {
        format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors>{}</PropertyGroup><ItemGroup><PackageReference Include=\"{}\" Version=\"[{}]\" />{}</ItemGroup></Project>\n",
            self.framework,
            if negative {
                "<EnableDefaultCompileItems>false</EnableDefaultCompileItems><ProbeCase Condition=\"'$(ProbeCase)' == ''\">base</ProbeCase>"
            } else {
                ""
            },
            config.name,
            config.version,
            if negative {
                "<Compile Include=\"$(ProbeCase).cs\" />"
            } else {
                ""
            }
        )
    }
    fn consumer(&self, config: &SdkConfig, source: &str, package: &Path) {
        self.write(
            "consumer/Consumer.csproj",
            self.project(config, false).as_bytes(),
        );
        self.write("consumer/Program.cs", source.as_bytes());
        self.checked(
            "consumer-restore",
            "consumer",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        self.checked(
            "consumer-build",
            "consumer",
            &["build", "-c", "Release", "--no-restore", "-m:1"],
        );
        self.checked(
            "consumer-run",
            "consumer",
            &[
                "run",
                "-c",
                "Release",
                "--no-build",
                "--",
                package.to_str().unwrap(),
            ],
        );
        let assets: Value = serde_json::from_slice(
            &fs::read(self.root.join("consumer/obj/project.assets.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            assets["libraries"][format!("{}/{}", config.name, config.version)]["type"],
            "package",
            "consumer must use installed package rather than project/source inclusion"
        );
    }
    fn examples(&self) {
        self.checked(
            "examples-restore",
            "csharp/examples",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        self.checked(
            "examples-run",
            "csharp/examples",
            &["run", "-c", "Release", "--no-restore"],
        );
    }
    fn negative(&self, config: &SdkConfig, cases: &[(&str, &str, &str)]) {
        self.write(
            "negative/Negative.csproj",
            self.project(config, true).as_bytes(),
        );
        self.write(
            "negative/base.cs",
            format!("using {}; _ = typeof(Client);\n", config.namespace).as_bytes(),
        );
        self.checked(
            "negative-restore",
            "negative",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        self.checked(
            "negative-control",
            "negative",
            &["build", "-c", "Release", "--no-restore", "-m:1"],
        );
        for (name, source, code) in cases {
            self.write(
                &format!("negative/{name}.cs"),
                format!("using {}; {source}\n", config.namespace).as_bytes(),
            );
            let output = self.output(
                &format!("negative-{name}"),
                "negative",
                &[
                    "build",
                    "-c",
                    "Release",
                    "--no-restore",
                    "-m:1",
                    &format!("-p:ProbeCase={name}"),
                ],
            );
            assert!(
                !output.status.success() && String::from_utf8_lossy(&output.stdout).contains(code),
                "negative {name} must fail with {code}; artifacts {}\n{}{}",
                self.root.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    fn finish(&self, profile: &str, source: &[u8]) {
        use sha2::{Digest, Sha256};
        self.write("PASS.json", serde_json::to_vec_pretty(&json!({"profile":profile,"framework":self.framework,"sourceSha256":format!("{:x}",Sha256::digest(source)),"result":"passed","evidence":"logs/*.command.json, stdout.log and stderr.log; independent installed consumer"})).unwrap().as_slice());
    }
}

#[test]
#[ignore = "requires the read-only OpenRouter corpus and installed .NET SDKs"]
fn native_actual_openrouter_five_operations() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/github/openrouter-web"));
    let files = [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ];
    let snapshots = files.map(|path| fs::read(root.join(path)).unwrap());
    let contract = load(&root.join(files[0]));
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected: Vec<_> = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|o| o.source().clone())
        .collect();
    assert_eq!(selected.len(), 5);
    let plan = plan_sdk(
        contract,
        &selected,
        SdkConfig {
            name: "Suspect.Csharp.OpenRouter".into(),
            version: "1.0.0".into(),
            namespace: "Suspect.Csharp.OpenRouter".into(),
        },
    )
    .unwrap();
    let body = |name: &str| {
        plan.operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap()
            .body
            .as_ref()
            .unwrap()
    };
    let reset_type = |name: &str| {
        let mut key = (
            body(name).media[0].schema().unwrap().clone(),
            suspect_codegen::csharp_sdk::models::RepresentationRole::Model,
        );
        loop {
            match &plan.models().declarations()[&key] {
                CsDecl::Record { fields, .. } => {
                    break plan
                        .models()
                        .render_type(&fields.iter().find(|f| f.wire == "limit_reset").unwrap().ty);
                }
                CsDecl::Alias(CsType::Named(next)) => key = next.clone(),
                other => panic!("unexpected native request representation: {other:?}"),
            }
        }
    };
    let source = include_str!("../src/csharp_sdk/testdata/OpenRouterConsumer.cs")
        .replace("__CREATE__", &body("createKeys").native_type)
        .replace("__UPDATE__", &body("updateKeys").native_type)
        .replace("__CREATE_RESET__", &reset_type("createKeys"))
        .replace("__UPDATE_RESET__", &reset_type("updateKeys"));
    for (sdk, framework) in TOOLCHAINS {
        let native = Native::new("openrouter", sdk, framework, &plan.render().unwrap());
        native.write(
            "consumer/responses.json",
            include_bytes!("fixtures/openrouter-five-responses.json"),
        );
        let package = native.package(plan.package());
        native.consumer(plan.package(), &source, &package);
        native.examples();
        native.finish("actual-openrouter-five", &snapshots[0]);
    }
    for (index, path) in files.iter().enumerate() {
        assert_eq!(
            fs::read(root.join(path)).unwrap(),
            snapshots[index],
            "tracked source must remain unchanged: {path}"
        );
    }
}

#[test]
#[ignore = "requires installed .NET SDKs; executes the independent 17-case shared runtime contract"]
fn native_portable_validation_executes_shared_vectors_and_failure_controls() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let mut schemas = serde_json::Map::new();
    for case in vectors["cases"].as_array().unwrap() {
        schemas.insert(
            case["name"].as_str().unwrap().into(),
            case["schema"].clone(),
        );
    }
    schemas.insert(
        "Loop".into(),
        json!({"anyOf":[true,{"$ref":"#/components/schemas/Loop"}]}),
    );
    schemas.insert(
        "EscapedPath".into(),
        json!({"type":"object","properties":{"a/b~":{"type":"number"}}}),
    );
    let source = contract(Value::Object(schemas), json!({}));
    let compiled = suspect_schema::OwnedCompiler::new(suspect_schema::Config {
        max_depth: 128,
        ..Default::default()
    })
    .compile(source.clone(), source.schema_roots())
    .unwrap();
    let program = serde_json::to_vec_pretty(&compiled.program()).unwrap();
    let m2 = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    let plan = plan_sdk(
        m2.clone(),
        &selected(&m2),
        SdkConfig {
            namespace: "Suspect.Csharp.Vectors".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let assets = plan.render().unwrap();
    for (sdk, framework) in TOOLCHAINS {
        let native = Native::new("validation", sdk, framework, &[]);
        for name in [
            "JsonRuntime.cs",
            "ValidationRuntime.cs",
            "ValidationProgram.g.cs",
        ] {
            let asset = assets
                .iter()
                .find(|f| f.path == format!("csharp/src/{name}"))
                .unwrap();
            native.write(&format!("validation/{name}"), asset.content.as_bytes());
        }
        native.write("validation/validation-program.json", &program);
        native.write(
            "validation/runtime-contract-v1.json",
            include_bytes!("fixtures/runtime-contract-v1.json"),
        );
        native.write(
            "validation/Program.cs",
            include_bytes!("../src/csharp_sdk/testdata/ValidationConsumer.cs"),
        );
        native.write("validation/Validation.csproj", format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>{framework}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><EmbeddedResource Include=\"validation-program.json\" LogicalName=\"Suspect.ValidationProgram.json\" /></ItemGroup></Project>\n").as_bytes());
        native.checked(
            "validation-restore",
            "validation",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "validation-run",
            "validation",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish(
            "shared-17-runtime-contract",
            include_bytes!("fixtures/runtime-contract-v1.json"),
        );
    }
}

#[test]
#[ignore = "requires installed .NET SDKs; proves nullable types, selected and parent union validation, and native names"]
fn native_nullable_union_and_name_adversarial_contract() {
    let schemas = json!({
        "Thing":{"type":"object","required":["name","required_nullable","choice"],"additionalProperties":false,"properties":{
            "name":{"type":"string"},"required_nullable":{"type":["string","null"]},"optional":{"type":"string"},"nullable":{"type":["string","null"]},
            "choice":{"$ref":"#/components/schemas/Choice"},"never":false,
            "mixed":{"$ref":"#/components/schemas/Mixed"},"nullable_union":{"$ref":"#/components/schemas/NullableUnion"},"nullable_enum":{"$ref":"#/components/schemas/NullableEnum"},
            "number_set":{"$ref":"#/components/schemas/NumberSet"},"overlap":{"$ref":"#/components/schemas/Overlap"},"inclusive":{"$ref":"#/components/schemas/Inclusive"},
            "integer_map":{"$ref":"#/components/schemas/IntegerMap"},"names":{"$ref":"#/components/schemas/Names"},"reserved":{"$ref":"#/components/schemas/Console"}
        }},
        "Choice":{"oneOf":[{"type":"string","minLength":3},{"type":"string","maxLength":2}]},
        "Overlap":{"oneOf":[{"type":"string","minLength":1},{"type":"string","maxLength":3}]},
        "Inclusive":{"anyOf":[{"type":"string","minLength":1},{"type":"string","maxLength":3}]},
        "Record":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1}},"additionalProperties":false},
        "Mixed":{"oneOf":[{"type":"string","const":"auto"},{"$ref":"#/components/schemas/Record"}]},
        "NullableUnion":{"anyOf":[{"type":"null"},{"$ref":"#/components/schemas/Record"}]},
        "NullableEnum":{"type":["string","null"],"enum":["a","b",null]},
        "NumberSet":{"type":"number","enum":[1,2]},
        "IntegerMap":{"type":"object","additionalProperties":{"type":"integer"}},
        "Names":{"type":"object","description":"<script>alert(1)</script>\n/// </summary>\u{2028}readonly source prose","properties":{
            "a-b":{"type":"string"},"a_b":{"type":"string"},"getType":{"type":"string"},"extra":{"type":"string"},"😀":{"type":"string"},"a/b~":{"type":"string"}
        },"additionalProperties":false},
        "Console":{"type":"object","properties":{"httpStatusCode":{"$ref":"#/components/schemas/HttpStatusCode"}}},
        "HttpStatusCode":{"type":"string","enum":["ok","not-ok"]}
    });
    let paths = json!({
        "/things":{"get":{"operationId":"probeThing","responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}}}}},
        "/optional":{"post":{"operationId":"optionalBody","requestBody":{"required":false,"content":{"application/json":{"schema":{"type":["string","null"]}}}},"responses":{
            "200":{"description":"record","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}},
            "201":{"description":"null","content":{"application/json":{"schema":{"type":"null"}}}}
        }}}
    });
    let contract = contract(schemas, paths);
    let original = serde_json::to_vec_pretty(contract.document(contract.entry()).unwrap()).unwrap();
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            name: "Suspect.Csharp.Adversarial".into(),
            version: "1.0.0".into(),
            namespace: "Suspect.Csharp.Adversarial".into(),
        },
    )
    .unwrap();
    for (sdk, framework) in TOOLCHAINS {
        let native = Native::new("adversarial", sdk, framework, &plan.render().unwrap());
        native.write("source.openapi.json", &original);
        let package = native.package(plan.package());
        native.consumer(
            plan.package(),
            include_str!("../src/csharp_sdk/testdata/AdversarialConsumer.cs"),
            &package,
        );
        native.examples();
        native.negative(plan.package(), &[
            ("required-nullable", "_ = new Thing { Name = \"x\", Choice = new Choice.Variant2(\"x\") };", "CS9035"),
            ("optional-nonnull", "_ = new Thing { Name = \"x\", RequiredNullable = null, Choice = new Choice.Variant2(\"x\"), Optional = Optional<string>.Present(null) };", "CS8625"),
            ("nullable-type", "NullableEnum value = Codecs.DecodeNullableEnum(\"null\"); _ = value;", "CS0266"),
            ("typed-extras", "var map = new IntegerMap(); map.Extra[\"x\"] = true;", "CS0029"),
        ]);
        native.finish("nullable-unions-names", &original);
    }
}
