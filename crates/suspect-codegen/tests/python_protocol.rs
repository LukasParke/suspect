//! Independent wire/byte/security/framing witnesses through an installed SDK.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    python_http::{self, HttpConfig, HttpPlan, PackageConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const IMPORT: &str = "protocol_python_sdk";
fn root(label: &str) -> PathBuf {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&target).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("sdk-python-protocol-{label}-"))
        .tempdir_in(target)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
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
fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-native-python-tools/bin/python")
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
        "{command:?}\n{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn readme_snippets(files: &[OutFile], root: &Path) -> PathBuf {
    let directory = root.join("readme_snippets");
    fs::create_dir(&directory).unwrap();
    let readme = &files
        .iter()
        .find(|file| file.path == "python/README.md")
        .unwrap()
        .content;
    for (index, block) in readme.split("```python\n").skip(1).enumerate() {
        let snippet = block.split_once("\n```").unwrap().0.to_owned() + "\n";
        assert!(
            files
                .iter()
                .any(|file| file.path.starts_with("python/examples/") && file.content == snippet),
            "README snippet differs from executable source"
        );
        fs::write(directory.join(format!("snippet_{index}.py")), snippet).unwrap();
    }
    directory
}
fn fixture() -> Value {
    let normative: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let json_ok = json!({"200":{"description":"JSON","content":{"application/json":{"schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}},"example":{"ok":true}}}}});
    let mut document = json!({"openapi":"3.2.0","info":{"title":"Native Python protocol","version":"1"},"servers":[{"url":"https://protocol.example/v1"}],"paths":{},"components":{"schemas":{
        "Payload":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1},"amount":{"type":"number"}},"additionalProperties":false},
        "Event":{"type":"object","required":["data"],"properties":{"data":{"type":"string","contentMediaType":"application/json","contentSchema":{"type":"object"}},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false},
        "Row":{"type":"object","required":["value"],"properties":{"value":{"type":"number"}},"additionalProperties":false}
    },"securitySchemes":{
        "bearer":{"type":"http","scheme":"BeArEr"},"basic":{"type":"http","scheme":"BaSiC"},
        "headerKey":{"type":"apiKey","in":"header","name":"X-Api-Key"},"queryKey":{"type":"apiKey","in":"query","name":"key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
        "oauth":{"type":"oauth2","oauth2MetadataUrl":"https://auth.example/metadata","flows":{"authorizationCode":{"authorizationUrl":"https://auth.example/authorize","tokenUrl":"https://auth.example/token","scopes":{"read":"Read data"}}}},
        "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.example/.well-known/openid-configuration"}
    }}});
    for (index, case) in normative["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let path = if case["parameter"]["in"] == "path" {
            format!("/parameter/{index}/{{color}}")
        } else {
            format!("/parameter/{index}")
        };
        let mut parameter = case["parameter"].clone();
        parameter["example"] = case["value"].clone();
        document["paths"][&path] = json!({"get":{"operationId":format!("parameter{index}"),"parameters":[parameter],"responses":json_ok.clone()}});
    }
    for (index, case) in normative["querystringCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut parameter = case["parameter"].clone();
        parameter["example"] = case["value"].clone();
        document["paths"][format!("/whole/{index}")] = json!({"get":{"operationId":format!("whole{index}"),"parameters":[parameter],"responses":json_ok.clone()}});
    }
    document["paths"]["/json"] = json!({"post":{"operationId":"writeJson","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"},"example":{"name":"native"}}}},"responses":json_ok.clone()}});
    document["paths"]["/text"] = json!({"post":{"operationId":"writeText","requestBody":{"required":true,"content":{"text/plain; charset=UTF-8":{"schema":{"type":"string"},"example":"héllo\nworld"}}},"responses":{"200":{"description":"text","content":{"text/plain":{"schema":{"type":"string"},"example":"héllo"}}}}}});
    document["paths"]["/bytes"] = json!({"put":{"operationId":"writeBytes","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"maxLength":64}}}},"responses":{"200":{"description":"bytes","content":{"application/octet-stream":{"schema":{"maxLength":64}}}}}}});
    document["paths"]["/free"] = json!({"post":{"operationId":"freeJson","requestBody":{"required":true,"content":{"application/merge-patch+json":{}}},"responses":{"200":{"description":"schema-free JSON","content":{"application/problem+json":{}}}}}});
    document["paths"]["/numeric-text"] = json!({"post":{"operationId":"numericText","requestBody":{"required":true,"content":{"text/plain":{"schema":{"type":"integer"},"example":1000}}},"responses":{"200":{"description":"exact textual number","content":{"text/plain":{"schema":{"type":"number"},"example":1}}}}}});
    document["paths"]["/choice"] = json!({"post":{"operationId":"writeChoice","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"},"example":{"name":"native"}},"application/*":{},"*/*":{}}},"responses":json_ok.clone()}});
    document["paths"]["/selection"] = json!({"get":{"operationId":"selection","responses":{
        "200":{"description":"exact","content":{"application/json":{"schema":{"type":"string"}},"application/json; profile=two":{"schema":{"type":"integer"}},"text/plain":{"schema":{"type":"string"}}}},
        "2XX":{"description":"range","content":{"application/*":{},"*/*":{}}},"default":{"description":"unspecified bytes"}
    }}});
    document["paths"]["/default"] = json!({"get":{"operationId":"defaultOnly","responses":{"default":{"description":"actual status decides success","content":{"application/json":{"schema":{"type":"integer"},"example":42}}}}}});
    document["paths"]["/undeclared"] = json!({"get":{"operationId":"undeclared"}});
    document["paths"]["/head"] = json!({"head":{"operationId":"headerOnly","responses":{"200":{"description":"header only","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"content":{"application/json":{"schema":{"type":"object","required":["unused"],"properties":{"unused":{"type":"string"}}}}}}}}});
    document["paths"]["/empty"] = json!({"get":{"operationId":"emptyBody","responses":{"204":{"description":"empty","content":{"application/json":{"schema":{"type":"object"}}}},"205":{"description":"reset"}}}});
    let response_headers = json!({
        "X-Count":{"required":true,"schema":{"type":"integer"}},"X-Values":{"schema":{"type":"array","items":{"type":"integer"}}},"X-Flags":{"explode":true,"schema":{"type":"object","properties":{"enabled":{"type":"boolean"},"count":{"type":"integer"}},"additionalProperties":false}},
        "X-Json":{"content":{"application/json":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}}}}
    });
    document["paths"]["/headers"] = json!({"get":{"operationId":"getHeaders","responses":{"200":{"description":"typed headers","headers":response_headers,"links":{"read":{"operationId":"writeJson","parameters":{"literal":{"source":"literal","schema":true}},"requestBody":{"$ref":"literal-data"}}}}}}});
    for (name, security) in [
        ("anonymous", json!([])),
        ("either", json!([{"bearer":[]},{"headerKey":[]}])),
        ("anonymousFirst", json!([{}, {"bearer":[]}])),
        (
            "together",
            json!([{"headerKey":["reader"],"queryKey":[],"cookieKey":[]}]),
        ),
        ("basicAuth", json!([{"basic":[]}])),
        ("oauthAuth", json!([{"oauth":["read"]}])),
        ("oidcAuth", json!([{"oidc":["openid","profile"]}])),
    ] {
        document["paths"][format!("/auth/{name}")] =
            json!({"get":{"operationId":name,"security":security,"responses":json_ok.clone()}});
    }
    document["paths"]["/servers"] = json!({"get":{"operationId":"serverChoice","servers":[{"url":"https://{tenant}.example:{port}/{base}","name":"tenant","variables":{"tenant":{"default":"demo"},"port":{"default":"443","enum":["443","8443"]},"base":{"default":"v2"}}},{"url":"../api","name":"relative"}],"responses":json_ok.clone()}});
    document["paths"]["/methods"] =
        normative["oas32Methods"]["document"]["paths"]["/methods"].clone();
    let form = json!({"type":"object","required":["name","enabled"],"properties":{"name":{"type":"string"},"enabled":{"type":"boolean"},"labels":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":3},"metadata":{"type":"object","required":["title"],"properties":{"title":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false});
    document["paths"]["/form"] = json!({"post":{"operationId":"writeForm","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":form.clone()}}},"responses":{"200":{"description":"form","content":{"application/x-www-form-urlencoded":{"schema":form}}}}}});
    let multipart = json!({"schema":{"type":"object","required":["file","metadata"],"minProperties":2,"maxProperties":4,"properties":{"file":{},"metadata":{"$ref":"#/components/schemas/Payload"},"files":{"type":"array","items":{},"minItems":1,"maxItems":2}},"additionalProperties":{"type":"string"}},"encoding":{"file":{"contentType":"image/png, image/jpeg","headers":{"X-Checksum":{"required":true,"schema":{"type":"string","minLength":1}}}},"files":{"contentType":"application/octet-stream"}}});
    document["paths"]["/multipart"] = json!({"post":{"operationId":"uploadParts","requestBody":{"required":true,"content":{"multipart/form-data":multipart.clone()}},"responses":{"200":{"description":"parts","content":{"multipart/form-data":multipart}}}}});
    document["paths"]["/events"] = json!({"get":{"operationId":"events","responses":{"200":{"description":"events","content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}}}});
    document["paths"]["/rows"] = json!({"get":{"operationId":"rows","responses":{"200":{"description":"rows","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Row"}}}}}}});
    document["paths"]["/send-rows"] = json!({"post":{"operationId":"sendRows","requestBody":{"required":true,"content":{"application/jsonl":{"itemSchema":{"$ref":"#/components/schemas/Row"}}}},"responses":json_ok.clone()}});
    document["paths"]["/send-events"] = json!({"post":{"operationId":"sendEvents","requestBody":{"required":true,"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}},"responses":json_ok}});
    document
}
fn emit(root: &Path) -> (HttpPlan, Vec<OutFile>) {
    let source = root.join("api.json");
    fs::write(&source, fixture().to_string()).unwrap();
    let contract = load(&source);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = HttpConfig {
        max_stream_item_bytes: 256,
        max_part_bytes: 128,
        max_parts: 64,
        ..Default::default()
    };
    let plan = python_http::plan_http(contract, &selected, config).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "protocol-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, root).unwrap();
    (plan, files)
}

#[test]
fn protocol_plan_retains_real_roots_and_public_native_bindings() {
    let root = root("plan");
    let (plan, files) = emit(&root);
    assert!(plan.operations().len() > 40);
    assert!(!plan.protocol().codec_roots().iter().any(|id| id.pointer()
        == "/paths/~1bytes/put/requestBody/content/application~1octet-stream/schema"));
    assert!(plan.protocol().codec_roots().iter().any(|id| id.pointer()
        == "/paths/~1events/get/responses/200/content/text~1event-stream/itemSchema"));
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == format!("python/src/{IMPORT}/http-manifest.json"))
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["operationsModule"], format!("{IMPORT}.operations"));
    assert!(
        manifest["publicExports"]["root"]
            .as_array()
            .unwrap()
            .contains(&json!("BasicAuth"))
    );
    assert!(
        manifest["publicExports"]["root"]
            .as_array()
            .unwrap()
            .contains(&json!("AsyncStream"))
    );
    let snapshot = suspect_codegen::compatibility::snapshot(
        plan.contract().clone(),
        &[],
        &[suspect_codegen::backend::TargetConfig {
            backend: suspect_codegen::backend::Backend::PythonHttp,
            package_name: "protocol-python-sdk".into(),
            package_version: "1.0.0".into(),
            import_name: Some(IMPORT.into()),
        }],
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    let stream = native
        .operations
        .iter()
        .find(|operation| operation.operation_id == "events")
        .unwrap();
    assert_eq!(stream.symbols["module"], format!("{IMPORT}.operations"));
    assert_eq!(
        stream.descriptor["responses"][0]["__module__"],
        format!("{IMPORT}.operations")
    );
    assert_ne!(stream.symbols["success"], stream.symbols["async-success"]);
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .contains(&"python_http/streams.py".into())
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .contains(&"http_protocol/model.rs".into())
    );
    let headers = native
        .operations
        .iter()
        .find(|operation| operation.operation_id == "getHeaders")
        .unwrap();
    assert_eq!(
        headers.descriptor["protocol"]["responses"][0]["links"][0]["parameters"]["literal"],
        json!({"source":"literal","schema":true})
    );
    fs::write(
        root.join("normative.json"),
        include_str!("fixtures/http-protocol-v1.json"),
    )
    .unwrap();
    println!("Protocol plan candidate: {}", root.display());
}

#[test]
#[ignore = "requires native Python/httpx/build/mypy/Sphinx tools and cached wheel dependencies"]
fn installed_protocol_vectors_and_stream_lifetimes() {
    let root = root("native");
    let (_, files) = emit(&root);
    let snippets = readme_snippets(&files, &root);
    fs::write(
        root.join("normative.json"),
        include_str!("fixtures/http-protocol-v1.json"),
    )
    .unwrap();
    fs::write(root.join("consumer.py"), CONSUMER).unwrap();
    fs::write(root.join("bad_types.py"),"from protocol_python_sdk import Client, operations\ndef wrong(client: Client) -> None:\n    client.write_bytes(body='text is not bytes')\n    client.write_json()\n    operations.UploadPartsRequestMultipartFilePart(value='not bytes', headers=operations.UploadPartsRequestMultipartFileHeaders(x_checksum='x'))\n").unwrap();
    let package = root.join("python");
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(&package),
        &root,
        "build",
    );
    let wheel = fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let environment = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&environment),
            &root,
            &format!("venv-{version}"),
        );
        let python = environment.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel),
            &root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("consumer.py"))
                .arg(package.join("examples"))
                .arg(&snippets),
            &root,
            &format!("mypy-{version}"),
        );
        checked(
            Command::new(&python)
                .arg(root.join("consumer.py"))
                .current_dir(&root),
            &root,
            &format!("native-{version}"),
        );
        let negative = Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                version,
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join(format!("negative-mypy-{version}")))
            .arg(root.join("bad_types.py"))
            .output()
            .unwrap();
        fs::write(
            root.join(format!("negative-types-{version}.log")),
            &negative.stdout,
        )
        .unwrap();
        assert!(!negative.status.success());
        let diagnostics = String::from_utf8_lossy(&negative.stdout);
        assert!(
            diagnostics.contains("[arg-type]") && diagnostics.contains("[call-arg]"),
            "{diagnostics}"
        );
        checked(
            Command::new(&python).arg(package.join("examples/validated.py")),
            &root,
            &format!("examples-{version}"),
        );
        let output = Command::new(&python)
            .args(["-c", "import site; print(site.getsitepackages()[0])"])
            .output()
            .unwrap();
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "sphinx",
                    "-W",
                    "--keep-going",
                    "-E",
                    "-b",
                    "html",
                    "docs",
                ])
                .arg(format!("docs/_build/{version}"))
                .env(
                    "PYTHONPATH",
                    String::from_utf8(output.stdout).unwrap().trim(),
                )
                .current_dir(&package),
            &root,
            &format!("sphinx-{version}"),
        );
    }
    checked(
        Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--cache-dir",
            ])
            .arg(root.join("package-mypy"))
            .arg(package.join(format!("src/{IMPORT}"))),
        &root,
        "package-mypy",
    );
    assert!(files.iter().any(|file| file.path == "python/README.md"));
    println!("Protocol native evidence: {}", root.display());
}

#[test]
fn undefined_profiles_and_conflicting_attachments_are_located_refusals() {
    let cases = [
        (
            json!({"security":[{"a":[],"b":[]}],"components":{"securitySchemes":{"a":{"type":"http","scheme":"bearer"},"b":{"type":"http","scheme":"basic"}}},"paths":{"/x":{"get":{"operationId":"conflict","responses":{"200":{"description":"bytes"}}}}}}),
            "http-security-attachment-conflict",
        ),
        (
            json!({"paths":{"/x":{"post":{"operationId":"unknownParts","requestBody":{"content":{"multipart/form-data":{"schema":{"type":"object","properties":{"file":{}},"additionalProperties":true}}}},"responses":{"204":{"description":"ok"}}}}}}),
            "http-form-untyped-extras",
        ),
        (
            json!({"paths":{"/x":{"get":{"operationId":"legacyStream","responses":{"200":{"description":"stream","content":{"text/event-stream":{"schema":{"type":"object"},"x-speakeasy-sse-sentinel":"[DONE]"}}}}}}}}),
            "http-stream-item-schema-required",
        ),
    ];
    for (index, (overlay, code)) in cases.into_iter().enumerate() {
        let root = root(&format!("decline-{index}"));
        let path = root.join("api.json");
        let mut document = json!({"openapi":"3.1.2","info":{"title":"Refusal witness","version":"1"},"servers":[{"url":"https://example.test"}]});
        document
            .as_object_mut()
            .unwrap()
            .extend(overlay.as_object().unwrap().clone());
        fs::write(&path, document.to_string()).unwrap();
        let contract = load(&path);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let errors = python_http::plan_http(contract, &selected, Default::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == code && !error.at.is_empty()),
            "{errors:?}"
        );
    }
}

#[test]
fn canonical_generation_options_match_python_compatibility_capture() {
    use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig};
    use suspect_codegen::http_protocol::CompatibilityProfile;
    let root = root("canonical-options");
    let path = root.join("api.json");
    fs::write(&path,json!({"openapi":"3.1.2","info":{"title":"Explicit byte profile","version":"1"},"servers":[{"url":"https://example.test/v1"}],"paths":{"/content":{"get":{"operationId":"download","responses":{"200":{"description":"raw bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}}}).to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "protocol-python-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    };
    let ids = vec!["download".to_owned()];
    let ordinary = GenerationOptions::default();
    let refused = suspect_codegen::compatibility::snapshot_with_options(
        contract.clone(),
        &ids,
        std::slice::from_ref(&target),
        &ordinary,
    )
    .unwrap();
    assert!(
        refused.native[0]
            .findings
            .iter()
            .any(|finding| finding.code == "http-binary-legacy-marker")
    );
    assert!(
        suspect_codegen::backend::generate_with_options(
            contract.clone(),
            &selected,
            &target,
            &ordinary
        )
        .is_err()
    );
    let options = GenerationOptions {
        compatibility_profiles: std::collections::BTreeSet::from([
            CompatibilityProfile::LegacyBinaryStringV1,
        ]),
        ..Default::default()
    };
    let captured = suspect_codegen::compatibility::snapshot_with_options(
        contract.clone(),
        &ids,
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert!(
        captured.native[0].findings.is_empty(),
        "{:?}",
        captured.native[0].findings
    );
    assert_eq!(captured.native[0].generation, options);
    assert_eq!(
        captured.native[0].operations[0].descriptor["responses"][0]["dataType"],
        "bytes"
    );
    let generated =
        suspect_codegen::backend::generate_with_options(contract, &selected, &target, &options)
            .unwrap();
    let manifest: Value = serde_json::from_str(
        &generated
            .iter()
            .find(|file| file.path == format!("python/src/{IMPORT}/http-manifest.json"))
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        manifest["protocol"]["capabilities"]["profiles"],
        json!(["legacy-binary-string-v1"])
    );
    assert_eq!(
        manifest["operations"][0]["responses"][0]["dataType"],
        captured.native[0].operations[0].descriptor["responses"][0]["dataType"]
    );
    fs::write(
        root.join("capture.json"),
        serde_json::to_string_pretty(&captured.native[0]).unwrap(),
    )
    .unwrap();
    println!("Canonical Python options evidence: {}", root.display());
}

#[test]
#[ignore = "requires tracked OpenRouter source and native wheel/type/docs tools"]
fn installed_real_binary_and_extra_json_operations() {
    let root = root("openrouter");
    let source = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = load(&source);
    let wanted = [
        "downloadContainerFileContent",
        "downloadFileContent",
        "listOauthJwks",
        "listModelsCount",
    ];
    let selected = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let errors =
        python_http::plan_http(contract.clone(), &selected, Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-binary-legacy-marker")
    );
    let mut config = HttpConfig::default();
    config.capabilities = config
        .capabilities
        .with_profile(suspect_codegen::http_protocol::CompatibilityProfile::LegacyBinaryStringV1);
    let plan = python_http::plan_http(contract, &selected, config).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "protocol-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    fs::write(root.join("consumer.py"), REAL_CONSUMER).unwrap();
    let package = root.join("python");
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(&package),
        &root,
        "build",
    );
    let wheel = fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let environment = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&environment),
            &root,
            &format!("venv-{version}"),
        );
        let python = environment.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel),
            &root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("consumer.py"))
                .arg(package.join("examples")),
            &root,
            &format!("mypy-{version}"),
        );
        checked(
            Command::new(&python).arg(root.join("consumer.py")),
            &root,
            &format!("native-{version}"),
        );
        checked(
            Command::new(&python).arg(package.join("examples/validated.py")),
            &root,
            &format!("examples-{version}"),
        );
        let output = Command::new(&python)
            .args(["-c", "import site; print(site.getsitepackages()[0])"])
            .output()
            .unwrap();
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "sphinx",
                    "-W",
                    "--keep-going",
                    "-E",
                    "-b",
                    "html",
                    "docs",
                ])
                .arg(format!("docs/_build/{version}"))
                .env(
                    "PYTHONPATH",
                    String::from_utf8(output.stdout).unwrap().trim(),
                )
                .current_dir(&package),
            &root,
            &format!("sphinx-{version}"),
        );
    }
    checked(
        Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--cache-dir",
            ])
            .arg(root.join("package-mypy"))
            .arg(package.join(format!("src/{IMPORT}"))),
        &root,
        "package-mypy",
    );
    println!("Real protocol evidence: {}", root.display());
}

const REAL_CONSUMER: &str = r#"import asyncio
import httpx
from protocol_python_sdk import Client, AsyncClient

seen:list[httpx.Request]=[]
def handle(request:httpx.Request)->httpx.Response:
    seen.append(request)
    assert request.headers['Authorization']=='Bearer fixture-token'
    if request.url.path.endswith('/content'):
        return httpx.Response(200,headers={'Content-Type':'application/octet-stream'},content=b'\x00\xff\r\nraw\x00')
    if request.url.path.endswith('/count'):
        return httpx.Response(200,headers={'Content-Type':'application/json'},content=b'{"data":{"count":150}}')
    return httpx.Response(200,headers={'Content-Type':'application/json'},content=b'{"keys":[{"kty":"EC","crv":"P-256","kid":"key1","x":"AA","y":"BB","alg":"ES256","use":"sig"}]}')

with httpx.MockTransport(handle) as transport:
    with Client(auth={'apiKey':'fixture-token'},transport=transport) as client:
        assert client.download_container_file_content(container_id='sess_a/b',file_id='cfile_name').data==b'\x00\xff\r\nraw\x00'
        assert seen[-1].url.raw_path==b'/api/v1/containers/sess_a%2Fb/files/cfile_name/content'
        assert client.download_file_content(file_id='or_file_demo',provider='openai').data==b'\x00\xff\r\nraw\x00'
        assert seen[-1].url.raw_path==b'/api/v1/files/or_file_demo/content?provider=openai'
        count=client.list_models_count(output_modalities='text,image')
        assert count.data.data.count==150
        assert seen[-1].url.query==b'output_modalities=text%2Cimage'
        assert client.list_oauth_jwks().data.keys[0].kid=='key1'

async def main()->None:
    async with httpx.MockTransport(handle) as transport:
        async with AsyncClient(auth={'apiKey':'fixture-token'},transport=transport) as client:
            assert (await client.download_file_content(file_id='or_file_demo')).data==b'\x00\xff\r\nraw\x00'
            assert (await client.list_oauth_jwks()).data.keys[0].kid=='key1'
asyncio.run(main())
assert len(seen)==6
print('real-protocol-requests',len(seen))
"#;

const CONSUMER: &str = r#"from __future__ import annotations
import asyncio
from collections.abc import AsyncIterator, Iterator
from pathlib import Path
from typing import Any
import json
import sys
import httpx
from protocol_python_sdk import Client, AsyncClient, BasicAuth, Authorization, CredentialRequest, Part, JsonNumber, SdkError, ApiError, CodecError, SyncStream, AsyncStream, models, operations

ROOT=Path(__file__).parent
sys.path.insert(0,str(ROOT/'python/examples'))
import forms as form_recipe
import request_parts as part_recipe
import streaming as stream_recipe
import transport as transport_recipe
for snippet in (ROOT/'readme_snippets').glob('*.py'):
    exec(compile(snippet.read_text(),str(snippet),'exec'),{'__name__':'readme_consumer'})
NORMATIVE=json.loads((ROOT/'normative.json').read_text())
MANIFEST=json.loads((ROOT/'python/src/protocol_python_sdk/http-manifest.json').read_text())
seen: list[httpx.Request]=[]
response_status=200
response_headers: list[tuple[str,str]]=[('Content-Type','application/json')]
response_body=b'{"ok":true}'

def handler(request:httpx.Request)->httpx.Response:
    seen.append(request)
    return httpx.Response(response_status,headers=response_headers,content=response_body)

def setup(status:int=200,content_type:str|None='application/json',body:bytes=b'{"ok":true}',headers:list[tuple[str,str]]|None=None)->None:
    global response_status,response_headers,response_body
    response_status=status
    response_headers=[] if content_type is None else [('Content-Type',content_type)]
    response_headers.extend(headers or [])
    response_body=body

def operation(name:str)->dict[str,Any]:
    return next(op for op in MANIFEST['operations'] if op['operationId']==name)

def parameter_value(op:dict[str,Any],value:object)->object:
    from protocol_python_sdk import codecs
    descriptor=op['parameters'][0]
    return getattr(codecs,descriptor['model']+'Codec').decode(json.dumps(value))

with httpx.MockTransport(handler) as transport:
    with Client(transport=transport) as client:
        for index,case in enumerate(NORMATIVE['parameterCases']):
            op=operation('parameter'+str(index))
            getattr(client,op['method'])(**{op['parameters'][0]['member']:parameter_value(op,case['value'])})
            request=seen[-1]
            where=case['parameter']['in']
            if where=='path':
                assert request.url.raw_path==('/v1/parameter/'+str(index)+'/'+case['wire']).encode()
            elif where=='query':
                assert request.url.query==case['wire'].encode()
            elif where=='header':
                assert request.headers[case['parameter']['name']]==case['wire']
            else:
                assert request.headers['Cookie']==case['wire']
        for index,case in enumerate(NORMATIVE['querystringCases']):
            op=operation('whole'+str(index))
            getattr(client,op['method'])(**{op['parameters'][0]['member']:parameter_value(op,case['value'])})
            assert seen[-1].url.query==case['wire'].encode(),seen[-1].url
        client.write_json(body=models.Payload(name='native',amount=JsonNumber('1.2500')))
        assert seen[-1].content==b'{"amount":1.2500,"name":"native"}'
        before=len(seen)
        try:
            client.write_json(body=models.Payload(name=''))
        except CodecError:
            pass
        else:
            raise AssertionError('invalid mutable input sent')
        assert len(seen)==before

        setup(content_type='text/plain; charset=UTF-8',body='héllo'.encode())
        assert client.write_text(body='héllo\nworld').data=='héllo'
        assert seen[-1].content=='héllo\nworld'.encode()
        setup(content_type='text/plain',body=b'1e-400')
        assert client.numeric_text(body=1000).data.token=='1e-400'
        assert seen[-1].content==b'1000'
        setup(content_type='text/plain; charset=iso-8859-1',body=b'capture')
        try:
            client.write_text(body='text')
        except SdkError as error:
            assert error.kind=='unexpected-response' and error.capture==b'capture'
        else:
            raise AssertionError('unimplemented text charset accepted')
        setup(content_type='application/problem+json',body=b'{"value":1e-400,"null":null}')
        free=client.free_json(body={'number':JsonNumber('1.2500'),'null':None})
        assert isinstance(free.data,dict) and isinstance(free.data['value'],JsonNumber)
        assert free.data['value'].token=='1e-400' and free.data['null'] is None
        assert seen[-1].content==b'{"number":1.2500,"null":null}'
        setup(content_type='application/octet-stream',body=b'\x00\xff\r\n')
        assert client.write_bytes(body=b'\x00\xff').data==b'\x00\xff\r\n'
        assert seen[-1].content==b'\x00\xff'
        setup()
        client.write_choice(body=b'\x00\xff',content_type='image/png')
        assert seen[-1].content==b'\x00\xff' and seen[-1].headers['Content-Type']=='image/png'
        before=len(seen)
        try:
            client.write_choice(body=b'{}',content_type='application/json')
        except CodecError:
            pass
        else:
            raise AssertionError('wildcard bypassed exact JSON codec')
        assert len(seen)==before
        setup(content_type='application/json; profile=two',body=b'42')
        assert client.selection().data==42
        setup(207,'application/pdf',b'\x00\xff')
        selected=client.selection()
        assert selected.status==207 and selected.data==b'\x00\xff'
        setup(201,None,b'untagged')
        try:
            client.selection()
        except SdkError:
            pass
        else:
            raise AssertionError('range media ignored missing Content-Type')
        setup(302,None,b'raw redirect')
        try:
            client.selection()
        except ApiError as error:
            assert error.status==302 and error.data==b'raw redirect'
        else:
            raise AssertionError('default non-success returned')
        setup(200,'image/png',b'no fallback')
        try:
            client.selection()
        except SdkError as error:
            assert error.kind=='unexpected-response' and error.capture==b'no fallback'
        else:
            raise AssertionError('exact status fell through to range/default media')
        setup(201,'application/json',b'42')
        default_result=client.default_only()
        assert default_result.status==201 and default_result.data==42
        setup(503,'application/json',b'42')
        try:
            client.default_only()
        except operations.DefaultOnlyStatusDefaultApiError as error:
            assert error.status==503 and error.data==42
        else:
            raise AssertionError('default classified by declaration instead of actual status')
        setup(200,None,b'not declared')
        try:
            client.undeclared()
        except SdkError as error:
            assert error.kind=='unexpected-response' and error.capture==b'not declared'
        else:
            raise AssertionError('missing responses invented a success')
        setup(200,None,b'ignored HEAD bytes',headers=[('X-Count','1e2')])
        result=client.header_only()
        assert result.data is None and result.typed_headers.x_count==100
        for status in [204,205]:
            setup(status,None,b'ignored')
            assert client.empty_body().data is None
        setup(200,None,b'opaque',headers=[('X-Count','42'),('X-Values','1,2,3'),('X-Flags','count=2,enabled=true'),('X-Json','{"name":"header"}')])
        result_headers=client.get_headers()
        assert result_headers.data==b'opaque'
        assert result_headers.typed_headers.x_count==42
        assert result_headers.typed_headers.x_values==[1,2,3]
        assert result_headers.links[0].parameters['literal']=={'source':'literal','schema':True}
        setup(200,None,b'capture')
        try:
            client.get_headers()
        except SdkError as error:
            assert error.kind=='response-decoding' and error.capture==b'capture'
        else:
            raise AssertionError('required response header absent')
        setup()
    with Client(auth={'bearer':'secret-token','headerKey':'header-secret'},transport=transport) as client:
        client.either()
        assert seen[-1].headers['Authorization']=='Bearer secret-token'
        assert 'X-Api-Key' not in seen[-1].headers
        client.anonymous_first()
        assert 'Authorization' not in seen[-1].headers
        client.anonymous()
        assert 'Authorization' not in seen[-1].headers
    with Client(auth={'bearer':'secret-token','headerKey':'header-secret'},auth_alternative=1,transport=transport) as client:
        client.either()
        assert seen[-1].headers['X-Api-Key']=='header-secret' and 'Authorization' not in seen[-1].headers
    before=len(seen)
    with Client(transport=transport) as client:
        try:
            client.either()
        except SdkError as error:
            assert error.kind=='request-validation'
        else:
            raise AssertionError('missing credentials reached transport')
    assert len(seen)==before
    with Client(auth={'headerKey':'h','queryKey':'q +&','cookieKey':'session-token'},transport=transport) as client:
        client.together()
        assert seen[-1].headers['X-Api-Key']=='h'
        assert seen[-1].url.query==b'key=q%20%2B%26'
        assert seen[-1].headers['Cookie']=='session=session-token'
    with Client(auth={'basic':BasicAuth(username='Aladdin',password='open sesame')},transport=transport) as client:
        client.basic_auth()
        assert seen[-1].headers['Authorization']=='Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=='
    contexts:list[CredentialRequest]=[]
    def provide(request:CredentialRequest)->Authorization:
        contexts.append(request)
        return Authorization('Custom supplied-token')
    with Client(auth={'oauth':provide,'oidc':provide},transport=transport) as client:
        client.oauth_auth()
        client.oidc_auth()
    assert contexts[0].permissions==('read',) and contexts[0].permissions_kind=='scopes'
    assert contexts[0].flows[0].token_url=='https://auth.example/token'
    assert contexts[1].discovery_url=='https://auth.example/.well-known/openid-configuration'
    assert seen[-1].headers['Authorization']=='Custom supplied-token'
    with Client(server='tenant',server_variables={'tenant':'customer','port':'8443','base':'v3'},transport=transport) as client:
        client.server_choice()
        assert str(seen[-1].url)=='https://customer.example:8443/v3/servers'
    with Client(server='relative',document_url='https://docs.example/spec/openapi.json',transport=transport) as client:
        client.server_choice()
        assert str(seen[-1].url)=='https://docs.example/api/servers'
    setup(200,None,b'method')
    with Client(transport=transport) as client:
        client.copy()
        assert seen[-1].method=='COPY'
        client.mixed_get()
        assert seen[-1].method=='GeT'
        client.lower_get()
        assert seen[-1].method=='get'
        client.extension_named_method()
        assert seen[-1].method=='x-PING'

def echo(request:httpx.Request)->httpx.Response:
    seen.append(request)
    return httpx.Response(200,headers={'Content-Type':request.headers['Content-Type']},content=request.content)

with httpx.MockTransport(echo) as transport:
    with Client(transport=transport) as client:
        form=operations.WriteFormRequestForm(name='a + b',enabled=True,labels=['one','two'],metadata=models.WriteFormRequestFormMetadata(title='a b'))
        echoed=client.write_form(body=form)
        assert seen[-1].content==b'enabled=true&labels=one&labels=two&metadata=%7B%22title%22%3A%22a+b%22%7D&name=a+%2B+b'
        assert echoed.data.name=='a + b' and echoed.data.enabled is True
        body=operations.UploadPartsRequestMultipart(
            file=operations.UploadPartsRequestMultipartFilePart(value=b'\x00\xffimage',content_type='image/png',filename='résumé.png',headers=operations.UploadPartsRequestMultipartFileHeaders(x_checksum='checked')),
            metadata=models.Payload(name='metadata'),files=[b'\x00one',b'\xfftwo'])
        body.set_extra('note','plain')
        uploaded=client.upload_parts(body=body)
        assert uploaded.data.file.value==b'\x00\xffimage'
        assert uploaded.data.file.filename=='résumé.png'
        assert uploaded.data.file.headers.x_checksum=='checked'
        from email.parser import BytesParser
        from email.policy import default
        message=BytesParser(policy=default).parsebytes(b'Content-Type: '+seen[-1].headers['Content-Type'].encode()+b'\r\n\r\n'+seen[-1].content)
        payloads=[part.get_payload(decode=True) for part in message.iter_parts()]
        assert b'\x00\xffimage' in payloads and b'\x00one' in payloads and b'\xfftwo' in payloads
        assert b'{"name":"metadata"}' in payloads and b'plain' in payloads
        before=len(seen)
        body.file=operations.UploadPartsRequestMultipartFilePart(value=b'x'*129,content_type='image/png',headers=operations.UploadPartsRequestMultipartFileHeaders(x_checksum='checked'))
        try:
            client.upload_parts(body=body)
        except SdkError as error:
            assert error.kind=='resource-limit'
        else:
            raise AssertionError('part byte limit ignored')
        assert len(seen)==before

        form_recipe.send_parts(client)
        part_recipe.send_parts(client,input_0_bytes=b'\x00\xff',input_1_bytes=[b'one'])

closed:list[str]=[]
reads:list[int]=[]
class BodyStream(httpx.SyncByteStream):
    def __init__(self,data:bytes,*,fail:bool=False,close_fail:bool=False)->None:
        self.data,self.fail,self.close_fail=data,fail,close_fail
    def __iter__(self)->Iterator[bytes]:
        for index,byte in enumerate(self.data):
            reads.append(index)
            yield bytes([byte])
        if self.fail:
            raise RuntimeError('secret-read-failure')
    def close(self)->None:
        closed.append('body')
        if self.close_fail:
            raise RuntimeError('secondary-close-failure')

payload=b'\xef\xbb\xbf:comment\r\nid:one\rdata: {not JSON}\r\ndata: \xe9\x9b\xaa\nretry:0010\nignored:field\n\ndata:[DONE]\n\ndata:\n\ndata:unfinished'
def stream_handler(request:httpx.Request)->httpx.Response:
    return httpx.Response(200,headers={'Content-Type':'text/event-stream'},stream=BodyStream(payload))
with httpx.MockTransport(stream_handler) as transport:
    with Client(transport=transport) as client:
        response=client.events()
        assert reads==[] and closed==[]
        with response.data as events:
            first=next(events)
            assert first.data=='{not JSON}\n雪' and first.id=='one' and first.retry==10
            assert len(reads)<len(payload),'stream eagerly consumed the body'
            second=next(events)
            assert second.data=='[DONE]'
            third=next(events)
            assert third.data==''
            assert list(events)==[],'unfinished SSE event must be discarded at EOF'
        assert closed==['body']
        response=client.events()
        with response.data as events:
            next(events)
        assert closed==['body','body']
        response=client.events()
    assert closed==['body','body','body'],'client exit did not close outstanding response'
    assert list(response.data)==[]

def rows_handler(request:httpx.Request)->httpx.Response:
    return httpx.Response(200,headers={'Content-Type':'application/x-ndjson'},stream=BodyStream(b'{"value":1.2500}\n{"value":1e-400}\r\n'))
with httpx.MockTransport(rows_handler) as transport:
    with Client(transport=transport) as client:
        response_rows=client.rows()
        with response_rows.data as rows:
            assert [row.value.token for row in rows]==['1.2500','1e-400']

for invalid,kind in [(b'data:'+b'x'*300+b'\n\n','resource-limit'),(b'data:x\nretry:'+b'9'*300+b'\n\n','resource-limit')]:
    def too_large(request:httpx.Request)->httpx.Response:
        return httpx.Response(200,headers={'Content-Type':'text/event-stream'},stream=BodyStream(invalid,close_fail=True))
    with httpx.MockTransport(too_large) as transport:
        with Client(transport=transport,max_capture_bytes=7) as client:
            try:
                list(client.events().data)
            except SdkError as error:
                assert error.kind==kind and error.status==200 and len(error.capture)<=7
                assert 'secondary' not in str(error)
            else:
                raise AssertionError('stream item budget ignored')

def broken(request:httpx.Request)->httpx.Response:
    return httpx.Response(200,headers={'Content-Type':'application/x-ndjson'},stream=BodyStream(b'{',fail=True,close_fail=True))
with httpx.MockTransport(broken) as transport:
    with Client(transport=transport) as client:
        try:
            list(client.rows().data)
        except SdkError as error:
            assert error.kind=='transport' and error.capture==b'{' and isinstance(error.cause,RuntimeError)
            assert str(error.cause)=='secret-read-failure' and 'secret' not in str(error)
        else:
            raise AssertionError('stream transport failure disappeared')

async def async_cases()->None:
    task=asyncio.current_task()
    started=asyncio.Event()
    owners:list[object]=[]
    async_closed:list[bool]=[]
    class AsyncBody(httpx.AsyncByteStream):
        async def __aiter__(self)->AsyncIterator[bytes]:
            owners.append(asyncio.current_task())
            yield b'{'
            started.set()
            await asyncio.Event().wait()
        async def aclose(self)->None:
            async_closed.append(True)
    def handle(request:httpx.Request)->httpx.Response:
        return httpx.Response(200,headers={'Content-Type':'application/x-ndjson'},stream=AsyncBody())
    async with httpx.MockTransport(handle) as transport:
        async with AsyncClient(transport=transport) as client:
            result=await client.rows()
            pending=asyncio.create_task(anext(result.data))
            await asyncio.wait_for(started.wait(),2)
            pending.cancel()
            try:
                await pending
            except asyncio.CancelledError:
                pass
            else:
                raise AssertionError('stream cancellation swallowed')
            assert owners==[pending] and async_closed==[True]
    async def provide(request:CredentialRequest)->Authorization:
        assert asyncio.current_task() is task
        await asyncio.sleep(0)
        return Authorization('Bearer explicit-async-token')
    setup()
    async with httpx.MockTransport(handler) as transport:
        async with AsyncClient(auth={'oauth':provide},transport=transport) as client:
            await client.oauth_auth()
            assert seen[-1].headers['Authorization']=='Bearer explicit-async-token'
            async def values()->AsyncIterator[models.Row]:
                yield models.Row(value=JsonNumber('1.2500'))
                yield models.Row(value=JsonNumber('1e-400'))
            await client.send_rows(body=values())
            assert seen[-1].content==b'{"value":1.2500}\n{"value":1e-400}\n'
            await client.send_events(body=[models.Event(data='[DONE]')])
            assert seen[-1].content==b'data: [DONE]\n\n'
            await client.send_events(body=[models.Event(data=' leading')])
            assert seen[-1].content==b'data:  leading\n\n'
    def complete_events(request:httpx.Request)->httpx.Response:
        return httpx.Response(200,headers={'Content-Type':'text/event-stream'},content=b'data:one\n\ndata:two\n\n')
    async with httpx.MockTransport(complete_events) as transport:
        async with AsyncClient(transport=transport) as client:
            assert await stream_recipe.count_items_async(client)==2
            partially_read=await client.events()
            assert (await anext(partially_read.data)).data=='one'
        assert [item async for item in partially_read.data]==[], 'closed client leaked buffered async items'
asyncio.run(async_cases())

def complete_events(request:httpx.Request)->httpx.Response:
    return httpx.Response(200,headers={'Content-Type':'text/event-stream'},content=b'data:one\n\ndata:two\n\n')
with httpx.MockTransport(complete_events) as transport:
    with Client(transport=transport) as client:
        assert stream_recipe.count_items(client)==2
        partially_read=client.events()
        assert next(partially_read.data).data=='one'
    assert list(partially_read.data)==[], 'closed client leaked buffered sync items'
transport_recipe.mock_request()

print('protocol-vectors',len(seen),'stream-closes',len(closed))
"#;
