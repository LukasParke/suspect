#![cfg(feature = "csharp-sdk")]
//! Expanded C# protocol witnesses. Wire expectations come from normative fixtures
//! and literal independent bytes, never from the emitted serializer's answers.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility,
    csharp_sdk::{self, SdkConfig, SdkPlan},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> Value {
    let mut value = json!({"openapi":"3.2.0","info":{"title":"C# protocol witnesses","version":"1"},"servers":[{"url":"https://api.example.test/api"},{"url":"../{base}","variables":{"base":{"default":"v2","enum":["v2","v3"]}}}],
    "components":{"securitySchemes":{
        "bearer":{"type":"http","scheme":"BeArEr"},"basic":{"type":"http","scheme":"bAsIc"},
        "headerKey":{"type":"apiKey","in":"header","name":"X-Api-Key"},"queryKey":{"type":"apiKey","in":"query","name":"api_key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
        "oauth":{"type":"oauth2","flows":{"authorizationCode":{"authorizationUrl":"https://auth.example/authorize","tokenUrl":"https://auth.example/token","scopes":{"read:items":"Read items"}}}},
        "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.example/.well-known/openid-configuration"}
    },"schemas":{"Value":{"type":"object","required":["value"],"properties":{"value":{"type":"string"}},"additionalProperties":false},"Event":{"type":"object","required":["data"],"properties":{"data":{"type":"string"},"id":{"type":"string"},"event":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false}}},
    "paths":{
        "/anonymous":{"get":{"operationId":"anonymous","responses":{"200":{"content":{"application/json":{"schema":{"type":"string"},"example":"ok"}}}}}},
        "/auth":{"get":{"operationId":"authorize","security":[{"bearer":["reader"],"headerKey":[],"queryKey":[],"cookieKey":[]},{"basic":[]},{"oauth":["read:items"]},{"oidc":["openid"]},{}],"responses":{"200":{"content":{"application/json":{"schema":{"type":"string"},"example":"ok"}}}}}},
        "/response":{"get":{"operationId":"selectResponse","responses":{
            "200":{"headers":{"X-Limit":{"required":true,"schema":{"type":"integer"}},"X-Tags":{"schema":{"type":"array","items":{"type":"string"}}},"X-Meta":{"explode":true,"schema":{"type":"object","additionalProperties":{"type":"integer"}}}},"links":{"next":{"operationId":"anonymous","parameters":{"id":"$response.body#/value"},"requestBody":{"schema":1,"$ref":"literal"}}},"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value"}},"text/plain":{"schema":{"type":"string"}},"application/*":{},"*/*":{}}},
            "2XX":{},"default":{}
        }}},
        "/text":{"post":{"operationId":"postText","requestBody":{"required":true,"content":{"text/plain":{"schema":{"type":"integer"},"example":1}}},"responses":{"200":{"content":{"text/plain":{"schema":{"type":"integer"},"example":1}}}}}},
        "/bytes":{"put":{"operationId":"putBytes","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"maxLength":3}}}},"responses":{"200":{"content":{"application/octet-stream":{"schema":{"maxLength":3}}}}}}},
        "/choose":{"post":{"operationId":"choose","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value"}},"application/*":{}}},"responses":{"204":{}}}},
        "/head":{"head":{"operationId":"headValue","responses":{"200":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value"}}}}}}},
        "/form":{"post":{"operationId":"postForm","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"metadata":{"$ref":"#/components/schemas/Value"},"tags":{"type":"array","items":{"type":"string"}},"codes":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false},"encoding":{"codes":{"style":"form","explode":false}}}}},"responses":{"204":{}}}},
        "/form-output":{"get":{"operationId":"getForm","responses":{"200":{"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"tags":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false}}}}}}},
        "/events":{"get":{"operationId":"events","responses":{"200":{"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}}}},
        "/lines":{"get":{"operationId":"lines","responses":{"200":{"content":{"application/x-ndjson":{"itemSchema":{"type":"number"}}}}}}}
    }});
    let normative: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    value["paths"]["/response"]["get"]["responses"]["200"]["content"]["application/json;profile=rich"] =
        json!({"schema":{"$ref":"#/components/schemas/Value"}});
    value["paths"]["/default"] = json!({"get":{"operationId":"defaultResponse","responses":{"default":{"content":{"application/json":{"schema":{"type":"string"},"example":"default"}}}}}});
    value["paths"]["/free-json"] = json!({"post":{"operationId":"freeJson","requestBody":{"required":true,"content":{"application/json":{}}},"responses":{"200":{"content":{"application/json":{}}}}}});
    value["paths"]["/unspecified"] = json!({"get":{"operationId":"unspecified"}});
    value["paths"]["/unnamed"] = json!({"get":{"responses":{"204":{}}}});
    for (i, case) in normative["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(normative["querystringCases"].as_array().unwrap())
        .enumerate()
    {
        let mut parameter = case["parameter"].clone();
        if parameter["in"] == "cookie"
            && parameter.get("schema").is_some()
            && parameter.get("style").is_none()
            && case.get("version").is_none()
        {
            parameter["style"] = json!("form");
        }
        let path = if parameter["in"] == "path" {
            format!("/vector/{i}/{{{}}}", parameter["name"].as_str().unwrap())
        } else {
            format!("/vector/{i}")
        };
        value["paths"][path] = json!({"get":{"operationId":format!("vector{i}"),"parameters":[parameter],"responses":{"204":{}}}});
    }
    let multipart = normative["multipart"]["paths"]["/upload"]["post"]["requestBody"].clone();
    value["paths"]["/multipart"] =
        json!({"post":{"operationId":"upload","requestBody":multipart,"responses":{"204":{}}}});
    value["paths"]["/multipart"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"]
        ["properties"]["files"] = json!({"type":"array","minItems":1,"maxItems":2,"items":{}});
    value["paths"]["/multipart"]["post"]["requestBody"]["content"]["multipart/form-data"]["encoding"]
        ["files"] = json!({"contentType":"application/octet-stream"});
    value["paths"]["/multipart-output"] = json!({"get":{"operationId":"downloadParts","responses":{"200":{"content":value["paths"]["/multipart"]["post"]["requestBody"]["content"].clone()}}}});
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
    ] {
        value["paths"][format!("/method/{method}")] =
            json!({method:{"operationId":format!("method_{method}"),"responses":{"204":{}}}});
    }
    value["paths"]["/custom"] = json!({"additionalOperations":{"COPY":{"operationId":"copy","responses":{"204":{}}},"x-PING":{"operationId":"ping","responses":{"204":{}}}}});
    value
}
fn root(label: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-protocol");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
        .keep()
}
fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn plan(root: &Path) -> SdkPlan {
    let path = root.join("source.openapi.json");
    fs::write(&path, serde_json::to_vec_pretty(&fixture()).unwrap()).unwrap();
    let c = load(&path);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    csharp_sdk::plan_sdk(
        c,
        &selected,
        SdkConfig {
            name: "Protocol.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Protocol.Csharp".into(),
        },
    )
    .unwrap()
}

#[test]
fn protocol_plan_preserves_actual_codec_roots_and_native_payload_families() {
    let root = root("planning-");
    let plan = plan(&root);
    assert!(plan.protocol().is_admitted());
    assert!(plan.operations().iter().any(|o| o.http_method == "HEAD"));
    let upload = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "upload")
        .unwrap();
    let parts = upload.body.as_ref().unwrap().media[0]
        .parts
        .as_ref()
        .unwrap();
    assert_eq!(
        parts
            .fields
            .iter()
            .find(|p| p.property_name == "File")
            .unwrap()
            .value_type,
        "byte[]"
    );
    assert!(
        !plan.protocol().codec_roots().contains(
            parts
                .fields
                .iter()
                .find(|p| p.property_name == "File")
                .unwrap()
                .wire
                .schema()
                .id()
        )
    );
    assert!(
        plan.operations()
            .iter()
            .find(|o| o.operation_id == "headValue")
            .unwrap()
            .responses[0]
            .always_empty
    );
    assert!(
        plan.operations()
            .iter()
            .find(|o| o.operation_id == "events")
            .unwrap()
            .responses[0]
            .native_type
            .starts_with("HttpStream<")
    );
    let selected = plan
        .operations()
        .iter()
        .map(|o| o.source.clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Protocol.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Protocol.Csharp".into()),
    };
    let files = backend::generate(plan.contract().clone(), &selected, &target).unwrap();
    assert!(files.iter().all(|f| f.path.starts_with("csharp/")));
    let snapshot = compatibility::snapshot(plan.contract().clone(), &[], &[target]).unwrap();
    assert!(
        snapshot.native[0].findings.is_empty(),
        "{:?}",
        snapshot.native[0].findings
    );
}

#[test]
fn case_distinct_known_methods_are_source_refused_for_the_httpclient_profile() {
    let root = root("case-refusal-");
    let mut source = fixture();
    source["paths"]["/case"] =
        json!({"additionalOperations":{"head":{"operationId":"caseHead","responses":{"200":{}}}}});
    let path = root.join("source.openapi.json");
    fs::write(&path, source.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let errors = csharp_sdk::plan_sdk(contract, &selected, Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "csharp-method-case-unsupported"
                && e.source.pointer().contains("additionalOperations/head"))
    );
}

#[test]
fn rich_native_snapshot_records_parts_headers_media_and_stream_contracts() {
    let directory = root("compatibility-");
    let plan = plan(&directory);
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Protocol.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Protocol.Csharp".into()),
    };
    let before =
        compatibility::snapshot(plan.contract().clone(), &[], std::slice::from_ref(&target))
            .unwrap();
    let native = &before.native[0];
    for role in [
        "named-parts",
        "multipart-part",
        "part-headers",
        "response-headers",
        "request-media-union",
        "response-body-union",
    ] {
        assert!(
            native.models.iter().any(|m| m.role == role),
            "missing {role}"
        );
    }
    let event = native
        .operations
        .iter()
        .find(|o| o.operation_id == "events")
        .unwrap();
    assert_eq!(
        event.descriptor["responses"][0]["fields"][0]["type"]["name"],
        "Protocol.Csharp.HttpStream"
    );
    let mut changed = fixture();
    changed["paths"]["/multipart"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"]["required"].as_array_mut().unwrap().push(json!("files"));
    let path = directory.join("source.openapi.json");
    fs::write(&path, changed.to_string()).unwrap();
    let after = compatibility::snapshot(load(&path), &[], &[target]).unwrap();
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed"
                && c.subject.contains("UploadRequestMultipart")),
        "{:?}",
        report.native[0].changes
    );
    assert!(!report.wire.is_empty());
}

#[test]
#[ignore = "native .NET 8/10 protocol build/pack/installed-consumer witnesses"]
fn native_protocol_matrix() {
    let dotnet = std::env::var_os("SUSPECT_DOTNET_BIN")
        .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into());
    let root = root("native-");
    let plan = plan(&root);
    for (sdk, framework) in [("8.0.424", "net8.0"), ("10.0.400", "net10.0")] {
        let case = root.join(sdk);
        fs::create_dir_all(case.join("feed")).unwrap();
        fs::write(
            case.join("global.json"),
            json!({"sdk":{"version":sdk,"rollForward":"disable"}}).to_string(),
        )
        .unwrap();
        fs::write(case.join("NuGet.Config"),"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources></configuration>").unwrap();
        suspect_codegen::write_files(&plan.render().unwrap(), &case).unwrap();
        let run = |dir: &str, label: &str, args: &[&str]| {
            let mut command = Command::new(&dotnet);
            command
                .args(args)
                .current_dir(case.join(dir))
                .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
                .env("DOTNET_NOLOGO", "1")
                .env("DOTNET_CLI_HOME", case.join("dotnet-home"))
                .env("NUGET_PACKAGES", case.join("nuget-cache"));
            let output = command.output().unwrap();
            fs::write(
                case.join(format!("{label}.log")),
                [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
            )
            .unwrap();
            fs::write(
                case.join(format!("{label}.command.json")),
                json!({"command":format!("{command:?}"),"status":output.status.code(),"sdk":sdk})
                    .to_string(),
            )
            .unwrap();
            output
        };
        let checked = |dir: &str, label: &str, args: &[&str]| {
            let output = run(dir, label, args);
            assert!(
                output.status.success(),
                "{label}: {}\n{}{}",
                case.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        let inventory = checked(".", "sdk-version", &["--version"]);
        assert_eq!(
            String::from_utf8_lossy(&inventory.stdout).trim(),
            sdk,
            "global.json must select the exact SDK through SUSPECT_DOTNET_BIN"
        );
        checked(
            "csharp",
            "restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        checked(
            "csharp",
            "pack",
            &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
        );
        fs::create_dir(case.join("consumer")).unwrap();
        fs::write(case.join("consumer/Consumer.csproj"),format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{framework}</TargetFramework><OutputType>Exe</OutputType><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"Protocol.Csharp\" Version=\"[1.0.0]\" /></ItemGroup></Project>")).unwrap();
        fs::write(case.join("consumer/Program.cs"), consumer(&plan)).unwrap();
        checked(
            "consumer",
            "consumer-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        checked(
            "consumer",
            "consumer-run",
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../feed/Protocol.Csharp.1.0.0.nupkg",
            ],
        );
        fs::create_dir(case.join("negative")).unwrap();
        fs::write(case.join("negative/Negative.csproj"),format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{framework}</TargetFramework><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"Protocol.Csharp\" Version=\"[1.0.0]\" /></ItemGroup></Project>")).unwrap();
        let upload = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == "upload")
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .media[0]
            .parts
            .as_ref()
            .unwrap();
        let file = upload
            .fields
            .iter()
            .find(|f| f.property_name == "File")
            .unwrap();
        fs::write(case.join("negative/Negative.cs"),format!("using Protocol.Csharp; public static class Negative {{ public static PutBytesInput WrongBytes() => new PutBytesInput {{ Body = \"not bytes\" }}; public static ChooseBody ClosedUnion() => new ChooseBody(); public static {} MissingHeader() => new {} {{ Value = new byte[0] }}; }}",file.wrapper_type.as_ref().unwrap(),file.wrapper_type.as_ref().unwrap())).unwrap();
        checked(
            "negative",
            "negative-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        let negative = run(
            "negative",
            "negative",
            &["build", "-c", "Release", "--no-restore"],
        );
        let errors = String::from_utf8_lossy(&negative.stdout);
        assert!(
            !negative.status.success()
                && ["CS0029", "CS0144", "CS9035"]
                    .iter()
                    .all(|code| errors.contains(code)),
            "wrong compiler-negative outcome: {errors}"
        );
        checked(
            "csharp/examples",
            "examples-restore",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        checked(
            "csharp/examples",
            "examples-run",
            &["run", "-c", "Release", "--no-restore"],
        );
        fs::write(case.join("PASS.json"),json!({"sdk":sdk,"framework":framework,"gate":"expanded-csharp-http","result":"passed"}).to_string()).unwrap();
    }
    println!("C# protocol native artifacts: {}", root.display());
}
fn consumer(plan: &SdkPlan) -> String {
    let normative: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut calls = String::new();
    for (i, case) in normative["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(normative["querystringCases"].as_array().unwrap())
        .enumerate()
    {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == format!("vector{i}"))
            .unwrap();
        let p = &op.parameters[0];
        let json = serde_json::to_string(&case["value"].to_string()).unwrap();
        let expected = serde_json::to_string(case["wire"].as_str().unwrap()).unwrap();
        calls.push_str(&format!("await client.{}(new {} {{ {} = Codecs.Decode{}({json}) }}); CheckLast({i}, {}, {expected});\n",op.method_name,op.input_type,p.property_name,plan.models().codec_name(&p.schema),serde_json::to_string(case["parameter"]["in"].as_str().unwrap()).unwrap()));
    }
    let form = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "postForm")
        .unwrap()
        .body
        .as_ref()
        .unwrap();
    let form_input = format!(
        "new PostFormInput {{ Body = new {} {{ Name = \"a + b\", Metadata = new Value {{ Value2 = \"v\" }}, Tags = new List<string> {{ \"x\", \"y\" }}, Codes = new List<JsonInteger> {{ 1, 2 }} }} }}",
        form.native_type
    );
    let upload = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "upload")
        .unwrap()
        .body
        .as_ref()
        .unwrap();
    let parts = upload.media[0].parts.as_ref().unwrap();
    let file = parts
        .fields
        .iter()
        .find(|p| p.property_name == "File")
        .unwrap();
    let metadata = parts
        .fields
        .iter()
        .find(|p| p.property_name == "Metadata")
        .unwrap();
    let repeated = parts
        .fields
        .iter()
        .find(|p| p.property_name == "Files")
        .unwrap()
        .wrapper_type
        .as_ref()
        .unwrap();
    let upload_input = format!(
        "new UploadInput {{ Body = new {} {{ File = new {} {{ Value = new byte[] {{ 0, 255, 42 }}, FileName = \"x.bin\", ContentType = \"image/png\", Headers = new {} {{ XPartId = \"part-1\" }} }}, Files = new List<{repeated}> {{ new {repeated} {{ Value = new byte[] {{ 1, 255 }} }}, new {repeated} {{ Value = new byte[] {{ 2, 255 }} }} }}, Metadata = new {} {{ Value = new {} {{ Title = \"native\" }} }} }} }}",
        upload.native_type,
        file.wrapper_type.as_ref().unwrap(),
        file.header_type.as_ref().unwrap(),
        metadata.wrapper_type.as_ref().unwrap(),
        metadata.value_type
    );
    let response = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "selectResponse")
        .unwrap()
        .responses
        .iter()
        .find(|r| r.wire.status_key() == "200")
        .unwrap();
    let case = |media: &str| {
        format!(
            "{}.{}",
            response.native_type,
            response
                .media
                .iter()
                .find(|m| m.wire.media_type().declared() == media)
                .unwrap()
                .variant_name
        )
    };
    include_str!("../src/csharp_sdk/testdata/ProtocolConsumer.cs")
        .replace("__VECTORS__", &calls)
        .replace("__FORM_INPUT__", &form_input)
        .replace("__UPLOAD_INPUT__", &upload_input)
        .replace("__APP_BYTES_CASE__", &case("application/*"))
        .replace("__ANY_BYTES_CASE__", &case("*/*"))
        .replace(
            "__PROFILE_JSON_CASE__",
            &case("application/json;profile=rich"),
        )
}
