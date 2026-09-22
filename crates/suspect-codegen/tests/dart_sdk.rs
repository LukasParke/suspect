//! Dart native package admission, installed consumers and independent wire gates.
#![cfg(feature = "dart-sdk")]

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_codegen::dart_sdk::{DartConfig, DartShape, Plan, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn fixture(document: &Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
}

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}

fn envelope(paths: Value, schemas: Value) -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Independent Dart fixture","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":schemas},
        "paths":paths})
}

fn schema_api(schema: Value) -> Value {
    envelope(
        json!({"/value":{"get":{"operationId":"getValue","responses":{
            "200":{"description":"Value","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value"}}}}
        }}}}),
        json!({"Value":schema}),
    )
}

fn plan_document(document: &Value, config: DartConfig) -> Plan {
    let contract = fixture(document);
    plan_sdk(contract.clone(), &selected(&contract), config).unwrap()
}

fn m2() -> Arc<Contract> {
    load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"))
}

#[test]
fn retained_plans_expose_native_members_codecs_and_statuses() {
    let contract = m2();
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        DartConfig::default(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(plan.contract(), &contract));
    assert!(plan.program().check().is_ok());
    assert_eq!(plan.operations().len(), 4);
    let input = plan
        .models()
        .symbols()
        .iter()
        .find(|model| model.name == "WidgetInput")
        .unwrap();
    assert_eq!(input.codec_name, "widgetInputCodec");
    assert!(
        input
            .members()
            .iter()
            .any(|field| field.wire_name == "name" && field.name == "name" && field.required)
    );
    assert!(
        input
            .members()
            .iter()
            .any(|field| field.wire_name == "amount" && !field.required)
    );
    let payload = plan
        .models()
        .symbols()
        .iter()
        .find(|model| model.name == "WidgetPayload")
        .unwrap();
    assert!(matches!(
        payload.shape,
        DartShape::Union { direct: true, .. }
    ));
    let get = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "getWidget")
        .unwrap();
    assert_eq!(get.parameters[0].name, "widgetId");
    assert_eq!(get.parameters[0].wire_name, "widget_id");
    assert_eq!(get.parameters[0].native_type, "String");
    assert_eq!(
        get.statuses
            .iter()
            .map(|status| (
                status.wire.status_key().parse::<u16>().unwrap(),
                status
                    .success_name
                    .as_ref()
                    .or(status.error_name.as_ref())
                    .unwrap()
                    .as_str(),
                status.success_name.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            (200, "GetWidgetStatus200", true),
            (404, "GetWidgetStatus404", false)
        ]
    );
    assert_eq!(get.statuses[0].native_type, "Widget");
    assert_eq!(plan.render(), plan.render());
    let paths = plan
        .render()
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    for path in [
        "dart/pubspec.yaml",
        "dart/lib/generated_sdk.dart",
        "dart/lib/generated_sdk_io.dart",
        "dart/lib/src/transport.dart",
        "dart/lib/src/client.dart",
        "dart/example/source_examples.dart",
        "dart/dartdoc_options.yaml",
        "dart/sdk-manifest.json",
    ] {
        assert!(
            paths.iter().any(|actual| actual == path),
            "missing package artifact {path}"
        );
    }
}

#[test]
fn selected_source_controls_admission_without_extensions_or_business_inference() {
    let mut document = schema_api(json!({"type":"number"}));
    document["paths"]["/value"]["get"]["operationId"] = json!("streamRetryPaginateAuthenticate");
    let plan = plan_document(&document, DartConfig::default());
    let manifest: Value = serde_json::from_str(
        &plan
            .render()
            .into_iter()
            .find(|f| f.path.ends_with("sdk-manifest.json"))
            .unwrap()
            .content,
    )
    .unwrap();
    assert!(plan.operations().iter().all(|op| !op.stream));
    assert_eq!(manifest["automatic_retries"], false);
    assert_eq!(manifest["inferred_pagination"], false);
    for schema in [
        json!({"type":"string","readOnly":true}),
        json!({"type":"object","patternProperties":{"[":{}}}),
        json!({"contains":true,"minContains":1.5}),
    ] {
        let contract = fixture(&schema_api(schema));
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            DartConfig::default(),
        )
        .unwrap_err();
        assert!(
            errors.iter().any(
                |d| d.source.pointer().starts_with("/components/schemas/Value")
                    && d.at.end > d.at.start
            ),
            "{errors:#?}"
        );
    }
    for schema in [
        json!({"type":"object","patternProperties":{"^x":{}}}),
        json!({"if":{"type":"string"},"then":{"minLength":1}}),
    ] {
        let plan = plan_document(&schema_api(schema), DartConfig::default());
        assert_eq!(
            plan.program().version,
            suspect_schema::OwnedProgram::V2_VERSION
        );
    }
    let contract = fixture(&document);
    assert!(
        plan_sdk(contract, &[], DartConfig::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code == "http-no-operations")
    );
}

#[test]
fn unsupported_uri_spelling_is_located_before_emission() {
    for path in [
        "/x/{",
        "/x/}",
        "/x/%",
        "/x/%2e%2E",
        "/x/..",
        "/x?query",
        "/x/%5c",
    ] {
        let contract = fixture(&envelope(
            json!({path:{"get":{"operationId":"getX","responses":{
                "200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}
            }}}}),
            json!({}),
        ));
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            DartConfig::default(),
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|d| d.code.contains("path") && d.at.end > d.at.start),
            "{path}: {errors:#?}"
        );
    }
}

#[test]
fn protocol_profile_does_not_inherit_unverified_dialects_or_encodings() {
    {
        let mut document = schema_api(json!({"type":"string"}));
        document["openapi"] = json!("3.0.3");
        let contract = fixture(&document);
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            DartConfig::default(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|d| d.source.pointer() == "/openapi"));
    }
    for status in [101, 204, 205, 304] {
        let mut document = schema_api(json!({"type":"string"}));
        let response = document["paths"]["/value"]["get"]["responses"]["200"].clone();
        document["paths"]["/value"]["get"]["responses"] = json!({status.to_string():response});
        let contract = fixture(&document);
        let plan = plan_sdk(
            contract.clone(),
            &selected(&contract),
            DartConfig::default(),
        )
        .unwrap();
        assert_eq!(plan.operations()[0].statuses[0].native_type, "NoBody");
    }
    for (key, value, code) in [
        (
            "responses",
            json!({"200":{"description":"stream","content":{"text/event-stream":{"schema":{"type":"string"}}}}}),
            "http-stream-item-schema-required",
        ),
        (
            "requestBody",
            json!({"content":{"multipart/form-data":{"schema":{"type":"object"}}}}),
            "http-form-untyped-extras",
        ),
    ] {
        let mut document = schema_api(json!({"type":"string"}));
        document["paths"]["/value"]["get"][key] = value;
        let contract = fixture(&document);
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            DartConfig::default(),
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|d| d.code == code && d.at.end > d.at.start),
            "{errors:#?}"
        );
    }
}

#[test]
fn package_identity_and_resource_policies_are_admitted_explicitly() {
    let contract = fixture(&schema_api(json!({"type":"string"})));
    for name in [
        "",
        "Foo",
        "class",
        "with-dash",
        "x\nmalicious: yes",
        "dart.io",
    ] {
        let mut config = DartConfig::default();
        config.package.name = name.into();
        assert!(
            plan_sdk(contract.clone(), &selected(&contract), config)
                .unwrap_err()
                .iter()
                .any(|d| d.code == "dart-package-name")
        );
    }
    for version in ["1", "^1.0.0", "1.0.0\nfoo: bar"] {
        let mut config = DartConfig::default();
        config.package.version = version.into();
        assert!(
            plan_sdk(contract.clone(), &selected(&contract), config)
                .unwrap_err()
                .iter()
                .any(|d| d.code == "dart-package-version")
        );
    }
    for config in [
        DartConfig {
            max_request_bytes: 0,
            ..Default::default()
        },
        DartConfig {
            max_capture_bytes: 9 * 1024 * 1024,
            ..Default::default()
        },
        DartConfig {
            max_json_depth: 129,
            ..Default::default()
        },
    ] {
        assert!(
            plan_sdk(contract.clone(), &selected(&contract), config)
                .unwrap_err()
                .iter()
                .any(|d| d.code == "dart-resource-policy")
        );
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn dart() -> PathBuf {
    std::env::var_os("SUSPECT_DART_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-tools/dart-sdk/bin/dart"))
}

fn gate_root(name: &str) -> PathBuf {
    let parent = std::env::var_os("SUSPECT_DART_GATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-native"));
    std::fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(parent)
        .unwrap()
        .keep()
}

fn dart_command(root: &Path) -> Command {
    let mut command = Command::new(dart());
    let tools = repository().join("target/sdk-dart-tools");
    std::fs::create_dir_all(tools.join("home")).unwrap();
    command
        .current_dir(root)
        .env("HOME", tools.join("home"))
        .env("PUB_CACHE", root.join("pub-cache"))
        .env("DART_SUPPRESS_ANALYTICS", "true")
        .env("CI", "true");
    command
}

fn checked(command: &mut Command, root: &Path, name: &str) -> Output {
    let output = command
        .output()
        .expect("the required native Dart tool must be installed");
    std::fs::create_dir_all(root.join("logs")).unwrap();
    std::fs::write(
        root.join("logs").join(format!("{name}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "native fixture retained: {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
#[ignore = "required native Dart floor/current package, installed-consumer and wire gate"]
fn native_m2_installed_consumer() {
    let contract = m2();
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        DartConfig {
            max_capture_bytes: 64,
            ..Default::default()
        },
    )
    .unwrap();
    let root = gate_root("m2-");
    native_package(&plan, &root);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/support.dart"),
        include_str!("../src/dart_sdk/native_support.dart"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("../src/dart_sdk/native_portable.dart"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("bin/main.dart"),
        include_str!("../src/dart_sdk/native_m2.dart"),
    )
    .unwrap();
    let server = m2_server(&root);
    native_consumer(&root, &server, true);
    verify_m2_wire(&server.records.lock().unwrap());
    native_quickstart(&root, &server, "m2-key");
    {
        let records = server.records.lock().unwrap();
        let request = records.last().unwrap();
        assert_eq!(records.len(), 14);
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v1/widgets");
        assert_eq!(request.body, br#"{"name":"alpha"}"#);
    }
    native_negative_types(&root);
    native_docs(&plan, &root);
    println!("DART_M2_GATE_ROOT={}", root.display());
}

fn native_package(plan: &Plan, root: &Path) {
    suspect_codegen::write_files(&plan.render(), root).unwrap();
    checked(dart_command(root).arg("--version"), root, "version");
    let archive = root.join("generated-sdk.tar.gz");
    checked(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(root.join("dart"))
            .arg("."),
        root,
        "package-archive",
    );
    let archive_bytes = std::fs::read(&archive).unwrap();
    let hash = format!("{:x}", Sha256::digest(&archive_bytes));
    let package = plan.config().package.clone();
    let name = package.name.clone();
    let version = package.version.clone();
    let hosted = LocalServer::start(move |mut socket, request, base| {
        if request.target == format!("/api/packages/{name}") {
            let release = json!({"version":version,"pubspec":{"name":name,"version":version,
                "description":"Source-selected exact JSON SDK generated from OpenAPI.","environment":{"sdk":">=3.9.4 <4.0.0"}},
                "archive_url":format!("{base}/archives/{name}-{version}.tar.gz"),"archive_sha256":hash,
                "published":"2026-09-10T00:00:00Z"});
            respond(
                &mut socket,
                200,
                "application/json",
                json!({"name":name,"latest":release,"versions":[release]})
                    .to_string()
                    .as_bytes(),
                &[],
            );
        } else if request.target.starts_with("/archives/") {
            respond(
                &mut socket,
                200,
                "application/octet-stream",
                &archive_bytes,
                &[],
            );
        } else if request.target.ends_with("/advisories") {
            respond(
                &mut socket,
                200,
                "application/json",
                br#"{"advisories":[],"advisoriesUpdated":"2026-09-10T00:00:00Z"}"#,
                &[],
            );
        } else {
            respond(&mut socket, 404, "text/plain", b"not found", &[]);
        }
    });
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("bin")).unwrap();
    std::fs::write(consumer.join("pubspec.yaml"), format!("name: independent_consumer\nversion: 0.0.0\npublish_to: none\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\ndependencies:\n  {}:\n    hosted: {}\n    version: {}\n", package.name, hosted.base, package.version)).unwrap();
    std::fs::copy(
        root.join("dart/analysis_options.yaml"),
        consumer.join("analysis_options.yaml"),
    )
    .unwrap();
    checked(
        dart_command(root)
            .args(["pub", "get"])
            .current_dir(&consumer),
        root,
        "consumer-install-hosted-archive",
    );
    assert!(
        hosted
            .records
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.target.starts_with("/archives/")),
        "consumer did not install the package archive"
    );
    drop(hosted);
    checked(
        dart_command(root)
            .args(["pub", "get", "--offline"])
            .current_dir(&consumer),
        root,
        "consumer-offline-installed",
    );
    let config: Value = serde_json::from_slice(
        &std::fs::read(consumer.join(".dart_tool/package_config.json")).unwrap(),
    )
    .unwrap();
    let installed = config["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == package.name)
        .unwrap();
    assert!(
        installed["rootUri"]
            .as_str()
            .unwrap()
            .contains("pub-cache/hosted/"),
        "consumer must use installed bytes, not the generated source directory"
    );
    std::fs::write(
        root.join("installed-package.json"),
        serde_json::to_string_pretty(installed).unwrap(),
    )
    .unwrap();
    checked(
        dart_command(root)
            .args(["pub", "get", "--offline"])
            .current_dir(root.join("dart")),
        root,
        "sdk-pub-get",
    );
    checked(
        dart_command(root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(root.join("dart")),
        root,
        "sdk-analyze",
    );
    checked(
        dart_command(root)
            .args(["compile", "exe", "example/source_examples.dart", "-o"])
            .arg(root.join("source-examples"))
            .current_dir(root.join("dart")),
        root,
        "compile-source-examples",
    );
    checked(
        &mut Command::new(root.join("source-examples")),
        root,
        "run-compiled-source-examples",
    );
    checked(
        dart_command(root)
            .args(["compile", "exe", "example/quickstart.dart", "-o"])
            .arg(root.join("quickstart"))
            .current_dir(root.join("dart")),
        root,
        "compile-quickstart",
    );
    let readme = std::fs::read_to_string(root.join("dart/README.md")).unwrap();
    for (index, block) in readme.split("```dart\n").skip(1).enumerate() {
        let block = block.split("```").next().unwrap();
        let source = if block.contains("Future<void> main()") || block.contains("void main()") {
            block.to_owned()
        } else {
            format!(
                "import 'package:{}/{0}.dart';\nvoid main() {{\n{block}\n}}\n",
                package.name
            )
        };
        let file = consumer.join(format!("bin/readme_{index}.dart"));
        std::fs::write(&file, source).unwrap();
        checked(
            dart_command(root)
                .args(["compile", "exe"])
                .arg(&file)
                .arg("-o")
                .arg(root.join(format!("readme-{index}")))
                .current_dir(&consumer),
            root,
            &format!("compile-readme-{index}"),
        );
        if index > 0 {
            checked(
                &mut Command::new(root.join(format!("readme-{index}"))),
                root,
                &format!("run-readme-{index}"),
            );
        }
    }
}

fn native_consumer(root: &Path, server: &LocalServer, portable: bool) {
    let consumer = root.join("consumer");
    checked(
        dart_command(root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        root,
        "consumer-positive-analyze",
    );
    checked(
        dart_command(root)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root.join("native-consumer"))
            .current_dir(&consumer),
        root,
        "compile-installed-consumer",
    );
    checked(
        Command::new(root.join("native-consumer"))
            .env("SUSPECT_DART_HTTP_BASE", format!("{}/api/v1", server.base))
            .env("SUSPECT_DART_HTTP_MARKERS", root),
        root,
        "run-installed-native-consumer",
    );
    if portable {
        checked(
            dart_command(root)
                .args(["compile", "js", "bin/portable.dart", "-o"])
                .arg(root.join("portable.js"))
                .current_dir(&consumer),
            root,
            "compile-portable-consumer-js",
        );
        checked(
            Command::new("node")
                .arg("-e")
                .arg("globalThis.self = globalThis; require(process.argv[1]);")
                .arg(root.join("portable.js")),
            root,
            "run-portable-consumer-js",
        );
    }
}

fn native_quickstart(root: &Path, server: &LocalServer, token: &str) {
    // This binary is compiled from the exact README code block against the
    // installed archive, proving the customer-facing first-request recipe.
    checked(
        Command::new(root.join("readme-0"))
            .env("API_TOKEN", token)
            .env("API_SERVER", format!("{}/api/v1", server.base)),
        root,
        "run-readme-first-request",
    );
}

fn native_negative_types(root: &Path) {
    for (index, (source, code)) in [
        ("void main() { WidgetInput(); }", "missing_required_argument"),
        ("void main() { WidgetInput(name: null); }", "argument_type_not_assignable"),
        ("void main() { WidgetInput(name: 'x', amount: const Present(1.5)); }", "argument_type_not_assignable"),
        ("void main() { WidgetInput(name: 'x', amount: const Present(null)); }", "argument_type_not_assignable"),
        ("void main() { WidgetInput(name: 'x', amount: JsonNumber.parse('1')); }", "argument_type_not_assignable"),
        ("WidgetPayload invalid() => JsonString('not-a-branch');", "return_of_invalid_type"),
        ("Future<void> call(Client client) async { await client.createWidget(); }", "missing_required_argument"),
        ("void main() { StandardPayload(text: 'x').kind = 'spoof'; }", "assignment_to_final_no_setter"),
        ("class Forged implements JsonNumber { @override dynamic noSuchMethod(Invocation invocation) => null; }", "invalid_use_of_type_outside_library"),
        ("String branch(WidgetPayload value) => switch(value) { StandardPayload() => 'x' };", "non_exhaustive_switch_expression"),
    ].into_iter().enumerate() {
        let directory = root.join(format!("negative/{index}"));
        std::fs::create_dir_all(directory.join(".dart_tool")).unwrap();
        std::fs::copy(root.join("consumer/.dart_tool/package_config.json"), directory.join(".dart_tool/package_config.json")).unwrap();
        std::fs::copy(root.join("consumer/pubspec.yaml"), directory.join("pubspec.yaml")).unwrap();
        std::fs::copy(root.join("consumer/analysis_options.yaml"), directory.join("analysis_options.yaml")).unwrap();
        std::fs::write(directory.join("negative.dart"), format!("import 'package:generated_sdk/generated_sdk.dart';\n{source}\n")).unwrap();
        let mut command = dart_command(root);
        command.args(["analyze", "--fatal-infos", "negative.dart"]).current_dir(&directory);
        let output = command.output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        std::fs::write(root.join(format!("logs/negative-{index}.log")), format!("{command:?}\n{text}")).unwrap();
        assert!(!output.status.success() && text.contains(code) && !text.contains("uri_does_not_exist"), "negative type case {index} failed for the wrong reason: {text}");
    }
}

fn native_docs(plan: &Plan, root: &Path) {
    let output = checked(
        dart_command(root)
            .args(["doc", "--validate-links", "--output"])
            .arg(root.join("dartdoc"))
            .current_dir(root.join("dart")),
        root,
        "dartdoc",
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !text.contains("warning:") && !text.contains("error:"),
        "native dartdoc findings: {text}"
    );
    assert!(root.join("dartdoc/index.html").is_file());
    let index = std::fs::read_to_string(root.join("dartdoc/index.json")).unwrap();
    for op in plan.operations() {
        assert!(
            index.contains(&format!("\"name\":\"{}\"", op.method_name)),
            "native docs missing method {}",
            op.method_name
        );
        for status in &op.statuses {
            for name in status.success_name.iter().chain(&status.error_name) {
                assert!(index.contains(name));
            }
        }
    }
    for model in plan.models().symbols() {
        assert!(index.contains(&model.codec_name));
    }
}

#[derive(Clone, Debug)]
struct WireRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl WireRequest {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map_or("", |(_, value)| value)
    }
}

struct LocalServer {
    base: String,
    records: Arc<std::sync::Mutex<Vec<WireRequest>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl LocalServer {
    fn start(
        handler: impl Fn(std::net::TcpStream, WireRequest, &str) + Send + Sync + 'static,
    ) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (stopped, seen, url, handler) = (
            stop.clone(),
            records.clone(),
            base.clone(),
            Arc::new(handler),
        );
        let thread = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // BSD accepted sockets inherit the nonblocking listener flag.
                        stream.set_nonblocking(false).unwrap();
                        let (seen, url, handler) = (seen.clone(), url.clone(), handler.clone());
                        workers.push(std::thread::spawn(move || {
                            if let Some(request) = read_request(&mut stream) {
                                seen.lock().unwrap().push(request.clone());
                                handler(stream, request, &url);
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2))
                    }
                    Err(error) => panic!("independent fixture listener: {error}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            base,
            records,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<WireRequest> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let end = loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 64 * 1024 {
            return None;
        }
        if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().into(), value.trim().into()))
        .collect();
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map_or(Some(0), |(_, value)| value.parse::<usize>().ok())?;
    if length > 2 * 1024 * 1024 {
        return None;
    }
    while bytes.len() - end < length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Some(WireRequest {
        method,
        target,
        headers,
        body: bytes[end..end + length].to_vec(),
    })
}

fn respond(
    stream: &mut std::net::TcpStream,
    status: u16,
    media: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) {
    use std::io::Write;
    let mut head = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (key, value) in extra {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

const WIDGET: &[u8] = br#"{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}"#;
const WIDGET_LIST: &[u8] = br#"{"items":[{"id":"w2","amount":0.0000000000000000001,"payload":{"kind":"secure","vault":"vlt-1"}}]}"#;

fn m2_server(root: &Path) -> LocalServer {
    let root = root.to_owned();
    LocalServer::start(move |mut stream, request, base| {
        use std::io::{Read, Write};
        let tail = request.target.rsplit('/').next().unwrap();
        if matches!(
            tail,
            "body-cancel" | "body-timeout" | "body-close" | "header-cancel"
        ) {
            if tail != "header-cancel" {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\n{\"i\r\n").unwrap();
                stream.flush().unwrap();
            }
            std::fs::write(root.join(format!("{tail}-started")), b"sent").unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let closed = stream.read(&mut [0_u8; 1]);
            std::fs::write(
                root.join(format!("{tail}-read-result")),
                format!("{closed:?}"),
            )
            .unwrap();
            if matches!(closed, Ok(0)) {
                std::fs::write(root.join(format!("{tail}-closed")), b"peer EOF observed").unwrap();
            }
        } else if tail == "redirect" {
            respond(
                &mut stream,
                302,
                "text/plain",
                b"redirect",
                &[("Location", &format!("{base}/leak"))],
            );
        } else if tail == "declared-large" {
            respond(
                &mut stream,
                404,
                "application/json",
                format!(r#"{{"message":"{}"}}"#, "x".repeat(256)).as_bytes(),
                &[],
            );
        } else if tail == "unknown" {
            respond(&mut stream, 500, "text/plain", &[b'x'; 256], &[]);
        } else if tail == "chunked-large" {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n80\r\n");
            let _ = stream.write_all(&[b'x'; 128]);
            let _ = stream.write_all(b"\r\n0\r\n\r\n");
        } else {
            respond(
                &mut stream,
                200,
                "Application/JSON; charset=utf-8",
                if request.target.contains('?') {
                    WIDGET_LIST
                } else {
                    WIDGET
                },
                &[],
            );
        }
    })
}

fn verify_m2_wire(records: &[WireRequest]) {
    for request in records {
        assert_eq!(request.header("authorization"), "Bearer m2-key");
        assert_eq!(request.header("accept"), "application/json");
        assert_eq!(request.header("accept-encoding"), "identity");
        assert!(
            !request.target.contains("/leak"),
            "native adapter followed a redirect"
        );
    }
    for (index, method, target, body) in [
        (0, "POST", "/api/v1/widgets", r#"{"name":"alpha"}"#),
        (
            1,
            "GET",
            "/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2",
            "",
        ),
        (
            2,
            "GET",
            "/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A",
            "",
        ),
        (
            3,
            "PATCH",
            "/api/v1/widgets/w1",
            r#"{"amount":0.0000000000000000001}"#,
        ),
    ] {
        let request = &records[index];
        assert_eq!(request.method, method);
        assert_eq!(request.target, target);
        assert_eq!(request.body, body.as_bytes());
        assert_eq!(
            request.header("content-type"),
            if body.is_empty() {
                ""
            } else {
                "application/json"
            }
        );
    }
    for suffix in [
        "redirect",
        "declared-large",
        "unknown",
        "chunked-large",
        "body-cancel",
        "after-cancel",
        "body-timeout",
        "header-cancel",
        "body-close",
    ] {
        assert!(
            records.iter().any(|r| r.target.ends_with(suffix)),
            "native gate omitted {suffix}"
        );
    }
    assert_eq!(records.len(), 13, "unexpected native HTTP exchanges");
}

fn dart_quote(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap()
        .replace('$', "\\$")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[test]
#[ignore = "required Dart checked-program, shared-vector, reference and resource gate"]
fn native_runtime_vectors_refs_and_resource_classification() {
    let root = gate_root("runtime-");
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    assert_eq!(vectors["cases"].as_array().unwrap().len(), 17);
    let mut paths = serde_json::Map::new();
    let mut schemas = serde_json::Map::new();
    for (index, vector) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        schemas.insert(format!("Vector{index:02}"), vector["schema"].clone());
    }
    for (name, schema) in [
        (
            "AnyFailure",
            json!({"anyOf":[true,{"type":"array","items":{"type":"integer"}}]}),
        ),
        (
            "OneFailure",
            json!({"oneOf":[true,{"type":"array","items":{"type":"integer"}}]}),
        ),
        (
            "AllFailure",
            json!({"allOf":[false,{"type":"array","items":{"type":"integer"}}]}),
        ),
        (
            "NegatedFailure",
            json!({"not":{"type":"array","items":{"type":"integer"}}}),
        ),
        (
            "BranchPick",
            json!({"anyOf":vec![json!({"type":"array","items":{"type":"string"}});8]}),
        ),
        ("ConvertedArray", json!({"type":"array","items":{}})),
        ("UniqueBudget", json!({"type":"array","uniqueItems":true})),
        (
            "Presence",
            json!({"type":"object","required":["requiredValue","requiredNullable"],"properties":{
            "requiredValue":{"type":"string"},"requiredNullable":{"type":["string","null"]},
            "optionalValue":{"type":"string"},"optionalNullable":{"type":["string","null"]}},"additionalProperties":false}),
        ),
        (
            "TypedExtras",
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}},"additionalProperties":{"type":"integer"}}),
        ),
        (
            "External",
            json!({"$ref":"external.json#/components/schemas/ExternalNode"}),
        ),
        (
            "Narrow",
            json!({"$ref":"external.json#/components/schemas/ExternalNode","properties":{"label":{"minLength":2}}}),
        ),
        (
            "Escaped",
            json!({"type":"object","properties":{"a/b~c":{"type":"string"},"😀":{"type":"string"}},"additionalProperties":false}),
        ),
        (
            "MixedLiteral",
            json!({"enum":[[1,true,null],{"kind":false},"text"]}),
        ),
        (
            "Intersection",
            json!({"allOf":[{"type":"object","required":["left"],"properties":{"left":{"type":"string"}}},
            {"type":"object","required":["right"],"properties":{"right":{"type":"number"}}}]}),
        ),
    ] {
        schemas.insert(name.into(), schema);
    }
    for (index, name) in schemas.keys().enumerate() {
        paths.insert(format!("/value/{index}"), json!({"get":{"operationId":format!("get{name}"),"responses":{
            "200":{"description":"Exact source value","content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{name}")}}}}
        }}}));
    }
    let input = root.join("input");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("main.json"),
        envelope(Value::Object(paths), Value::Object(schemas)).to_string(),
    )
    .unwrap();
    std::fs::write(input.join("external.json"), envelope(json!({}), json!({"ExternalNode":{"type":"object","required":["label"],"properties":{
        "label":{"type":"string","minLength":1},"child":{"$ref":"#/components/schemas/ExternalNode"}}
    }})).to_string()).unwrap();
    let contract = load(&input.join("main.json"));
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        DartConfig {
            schema: suspect_schema::Config {
                max_depth: 128,
                max_evaluation_steps: 256,
                max_equality_steps: 32,
                max_number_bytes: 64,
                ..Default::default()
            },
            max_conversion_steps: 128,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(plan.program().check().is_ok());
    let mut tests = String::new();
    for (index, vector) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        let model = plan
            .models()
            .symbols()
            .iter()
            .find(|model| model.source.pointer() == format!("/components/schemas/Vector{index:02}"))
            .unwrap();
        let codec = &model.codec_name;
        let label = dart_quote(vector["name"].as_str().unwrap());
        let valid = vector["valid"].as_array().unwrap();
        if !valid.is_empty() {
            tests.push_str(&format!("for (final wire in <String>[{}]) {{\n  check({codec}.validate(parseJson(wire)).isValid, {label});\n  final native = {codec}.decode(wire);\n  check({codec}.validate({codec}.toJson(native)).isValid, {label});\n}}\n", valid.iter().map(|v| dart_quote(v.as_str().unwrap())).collect::<Vec<_>>().join(", ")));
        }
        for invalid in vector["invalid"].as_array().unwrap() {
            tests.push_str(&format!("check({codec}.validate(parseJson({})).status == ValidationStatus.invalid, {label});\ncheck(fails<CodecException>(() => {codec}.decode({})).kind == CodecFailureKind.invalid, {label});\n", dart_quote(invalid.as_str().unwrap()), dart_quote(invalid.as_str().unwrap())));
        }
    }
    // Presence is reserved for the public wrapper; all bindings come from descriptors.
    let presence = plan
        .models()
        .symbols()
        .iter()
        .find(|m| m.source.pointer() == "/components/schemas/Presence")
        .unwrap();
    let source = include_str!("../src/dart_sdk/native_runtime.dart")
        .replace("__VECTORS__", &tests)
        .replace("presenceCodec", &presence.codec_name);
    native_package(&plan, &root);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/support.dart"),
        include_str!("../src/dart_sdk/native_support.dart"),
    )
    .unwrap();
    std::fs::write(consumer.join("bin/main.dart"), &source).unwrap();
    std::fs::write(consumer.join("bin/portable.dart"), &source).unwrap();
    let unused = LocalServer::start(|mut stream, _, _| {
        respond(&mut stream, 500, "text/plain", b"no HTTP expected", &[])
    });
    native_consumer(&root, &unused, true);
    assert!(
        unused.records.lock().unwrap().is_empty(),
        "codec gate must be independent of networking"
    );
    println!("DART_RUNTIME_GATE_ROOT={}", root.display());
}

#[test]
#[ignore = "required actual OpenRouter source, Dart installed pub package, HTTP and dartdoc gate"]
fn native_five_actual_openrouter_operations() {
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    let before = Sha256::digest(std::fs::read(&path).expect("actual OpenRouter input is required"));
    let contract = load(&path);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let operations = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), wanted.len());
    let plan = plan_sdk(
        contract,
        &operations,
        DartConfig {
            max_capture_bytes: 64,
            ..Default::default()
        },
    )
    .unwrap();
    let body = |id: &str| {
        &plan
            .operations()
            .iter()
            .find(|op| op.operation_id == id)
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .native_type
    };
    let source = include_str!("../src/dart_sdk/native_openrouter.dart")
        .replace("__CREATE_BODY__", body("createKeys"))
        .replace("__UPDATE_BODY__", body("updateKeys"))
        .replace(
            "__UPDATE_CODEC__",
            plan.operations()
                .iter()
                .find(|op| op.operation_id == "updateKeys")
                .unwrap()
                .body
                .as_ref()
                .unwrap()
                .media[0]
                .payload
                .codec_name()
                .unwrap(),
        );
    let root = gate_root("openrouter-");
    std::fs::write(
        root.join("source-provenance.json"),
        serde_json::to_string_pretty(&json!({
            "source":path,"sha256":format!("{before:x}"),"operations":wanted,
            "wire_fixtures":"crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json"
        }))
        .unwrap(),
    )
    .unwrap();
    native_package(&plan, &root);
    std::fs::write(root.join("consumer/bin/main.dart"), source).unwrap();
    std::fs::write(
        root.join("consumer/bin/support.dart"),
        include_str!("../src/dart_sdk/native_support.dart"),
    )
    .unwrap();
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/openrouter-five-responses.json")).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let server = LocalServer::start(move |mut stream, request, _| {
        let (status, body) = if request.target == "/api/v1/credits" {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                (200, fixtures["credits"].as_str().unwrap())
            } else {
                (
                    401,
                    r#"{"error":{"code":401,"message":"Missing Authentication header"}}"#,
                )
            }
        } else if request.method == "POST" {
            (201, fixtures["create"].as_str().unwrap())
        } else if request.method == "PATCH" {
            (200, fixtures["update"].as_str().unwrap())
        } else if request.target == "/api/v1/containers/sess_abc123/files"
            || request.target.contains('?')
        {
            (200, fixtures["list"].as_str().unwrap())
        } else {
            (200, fixtures["file"].as_str().unwrap())
        };
        respond(
            &mut stream,
            status,
            "application/json; charset=UTF-8",
            body.as_bytes(),
            &[],
        );
    });
    native_consumer(&root, &server, false);
    let records = server.records.lock().unwrap();
    let expected = [
        ("GET", "/api/v1/credits", ""),
        ("GET", "/api/v1/credits", ""),
        (
            "POST",
            "/api/v1/keys",
            r#"{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}"#,
        ),
        (
            "PATCH",
            "/api/v1/keys/fixture-hash",
            r#"{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}"#,
        ),
        (
            "GET",
            "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA",
            "",
        ),
        (
            "GET",
            "/api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A",
            "",
        ),
        ("GET", "/api/v1/containers/sess_abc123/files", ""),
    ];
    assert_eq!(records.len(), expected.len());
    for (record, (method, target, body)) in records.iter().zip(expected) {
        assert_eq!(record.method, method);
        assert_eq!(record.target, target);
        assert_eq!(record.body, body.as_bytes());
        assert_eq!(
            record.header("authorization"),
            "Bearer test-management-token"
        );
        assert_eq!(record.header("accept"), "application/json");
        assert_eq!(
            record.header("content-type"),
            if body.is_empty() {
                ""
            } else {
                "application/json"
            }
        );
    }
    std::fs::write(root.join("wire-records.txt"), format!("{records:#?}")).unwrap();
    drop(records);
    native_quickstart(&root, &server, "test-management-token");
    {
        let records = server.records.lock().unwrap();
        let request = records.last().unwrap();
        assert_eq!(records.len(), 8);
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v1/keys");
        assert_eq!(request.body, br#"{"expires_at":"2027-12-31T23:59:59Z","include_byok_in_limit":true,"limit":50,"limit_reset":"monthly","name":"My New API Key"}"#);
    }
    native_docs(&plan, &root);
    assert_eq!(
        before,
        Sha256::digest(std::fs::read(&path).unwrap()),
        "native admission must not rewrite actual source"
    );
    println!("DART_OPENROUTER_GATE_ROOT={}", root.display());
}

#[test]
#[ignore = "required installed Dart naming, nullable-body, query-work and source-escaping gate"]
fn native_adversarial_names_and_optional_body() {
    let reserved = [
        "Client",
        "JsonNumber",
        "String",
        "List",
        "Deprecated",
        "HttpClient",
        "Presence",
        "Future",
        "Credentials",
        "Enum",
        "MapEntry",
        "Comparable",
        "Record",
    ];
    let fields = [
        "value",
        "c",
        "object",
        "extraFields",
        "class",
        "runtimeType",
        "foo-bar",
        "foo_bar",
        "雪",
        "a/b~c",
        "line\nfield",
        "$dollar",
    ];
    let mut schemas = serde_json::Map::new();
    for name in reserved {
        schemas.insert(name.into(), json!({"type":"object","required":["value"],
            "description":"Literal prose: */ [link](javascript:bad) `code` $interpolation\nsecond line\u{2028}third line",
            "properties":fields.iter().map(|name| ((*name).to_owned(), json!({"type":"string"}))).collect::<serde_json::Map<_,_>>() }));
    }
    schemas.insert("Body".into(), json!({"type":["object","null"],"required":["value"],"properties":{
        "value":{"type":"string"},"impossible":{"$ref":"#/components/schemas/Impossible"},"models":{"type":"object","properties":reserved.iter().map(|name| ((*name).to_owned(),json!({"$ref":format!("#/components/schemas/{name}")}))).collect::<serde_json::Map<_,_>>()}
    }}));
    schemas.insert("Impossible".into(), json!({"type":"object","required":["value","number","nothing"],"properties":{
        "value":{"type":"string","const":false},"number":{"type":"integer","const":true},"nothing":{"type":"null","const":"x"}
    }}));
    schemas.insert("CloseStatus200".into(), json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}},"additionalProperties":false}));
    schemas.insert("WireEnum".into(), json!({"enum":["values","name","index","wireValue","class","foo-bar","foo_bar","line\nbreak","$interpolation","雪"]}));
    let parameters = ["client", "body", "cancellation", "timeout", "path", "query", "response", "data", "class", "foo-bar", "foo_bar", "雪"]
        .iter().map(|name| json!({"name":name,"in":if *name == "client" {"path"} else {"query"},"required":true,"schema":{"type":"string","example":"x"}})).collect::<Vec<_>>();
    let response = json!({"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/CloseStatus200"}}}}});
    let long_key = "_".repeat(4096);
    let mut document = envelope(
        json!({
            "/names/{client}":{"post":{"operationId":"close","parameters":parameters,
                "requestBody":{"required":false,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Body"},"example":{"value":"body"}}}},"responses":response}},
            "/other":{"get":{"operationId":"close!","responses":response}},
        "/query":{"get":{"operationId":"query","parameters":[{"name":long_key,"in":"query","required":true,"schema":{"type":"array","items":{"type":"string"}}}],"responses":response}},
        "/scalars":{"get":{"operationId":"echoScalars","parameters":[
            {"name":"enabled","in":"query","required":true,"schema":{"type":"boolean"}},
            {"name":"amount","in":"query","required":true,"schema":{"type":"number"}},
            {"name":"flags","in":"query","schema":{"type":"array","items":{"type":"boolean"}}},
            {"name":"counts","in":"query","explode":false,"schema":{"type":"array","items":{"type":"integer"}}}
        ],"responses":response}},
            "/value":{"get":{"operationId":"value\nfrom source","responses":{"200":{"description":"literal","content":{"application/json":{"schema":{"$ref":"#/components/schemas/WireEnum"}}}}}}}
        }),
        Value::Object(schemas),
    );
    document["security"] = json!([{"class":[]}]);
    document["components"]["securitySchemes"] = json!({"class":{"type":"http","scheme":"bearer"}});
    let plan = plan_document(&document, DartConfig::default());
    let model = |name: &str| {
        plan.models()
            .symbols()
            .iter()
            .find(|m| m.source.pointer() == format!("/components/schemas/{name}"))
            .unwrap()
    };
    let mut identities = std::collections::BTreeSet::new();
    for symbol in plan.models().symbols() {
        assert!(identities.insert(symbol.name.clone()));
        assert!(identities.insert(symbol.codec_name.clone()));
    }
    let mut model_cases = String::new();
    assert!(
        model("Impossible")
            .members()
            .iter()
            .all(|field| field.fixed.is_none()),
        "an invalid const literal cannot project a native getter"
    );
    for original in reserved {
        let model = model(original);
        assert_ne!(model.name, original, "runtime/import identity was shadowed");
        let args = model
            .members()
            .iter()
            .map(|field| {
                format!(
                    "{}: {}",
                    field.name,
                    if field.required {
                        dart_quote("value")
                    } else {
                        format!("Present({})", dart_quote(&field.wire_name))
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let expected = Value::Object(
            fields
                .iter()
                .map(|field| {
                    (
                        (*field).into(),
                        json!(if *field == "value" { "value" } else { field }),
                    )
                })
                .collect(),
        );
        model_cases.push_str(&format!(
            "check({}.encode({}({args})) == {}, 'native member allocation for {original}');\n",
            model.codec_name,
            model.name,
            dart_quote(&expected.to_string().replace("\\n", "\\u000a"))
        ));
    }
    let operation = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "close")
        .unwrap();
    assert_eq!(operation.method_name, "close2");
    assert_ne!(
        operation.statuses[0].success_name.as_deref(),
        Some("CloseStatus200")
    );
    let arguments = operation
        .parameters
        .iter()
        .map(|p| format!("{}: 'x'", p.name))
        .collect::<Vec<_>>()
        .join(", ");
    let other = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "close!")
        .unwrap();
    let query = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "query")
        .unwrap();
    let source = include_str!("../src/dart_sdk/native_names.dart")
        .replace("__MODEL_CASES__", &model_cases)
        .replace("__METHOD__", &operation.method_name)
        .replace("__ARGUMENTS__", &arguments)
        .replace("__OTHER_METHOD__", &other.method_name)
        .replace("__BODY__", &model("Body").name)
        .replace("__QUERY_METHOD__", &query.method_name)
        .replace("__ARRAY_ARGUMENT__", &query.parameters[0].name)
        .replace("__CREDENTIAL__", &plan.credentials()[0].name)
        .replace("__URL__", "'https://example.test/api/v1/names/x?body=x&cancellation=x&timeout=x&path=x&query=x&response=x&data=x&class=x&foo-bar=x&foo_bar=x&%E9%9B%AA=x'");
    let root = gate_root("names-");
    native_package(&plan, &root);
    std::fs::write(root.join("consumer/bin/main.dart"), &source).unwrap();
    std::fs::write(root.join("consumer/bin/portable.dart"), &source).unwrap();
    std::fs::write(
        root.join("consumer/bin/support.dart"),
        include_str!("../src/dart_sdk/native_support.dart"),
    )
    .unwrap();
    let unused = LocalServer::start(|mut stream, _, _| {
        respond(&mut stream, 500, "text/plain", b"no socket expected", &[])
    });
    native_consumer(&root, &unused, true);
    native_docs(&plan, &root);
    println!("DART_NAMES_GATE_ROOT={}", root.display());
}
