//! Source-selected Swift SDK admission plus native SPM/codec/wire/DocC gates.

use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};
use suspect_codegen::swift_sdk::{SwiftConfig, emit_sdk, plan_sdk};
use suspect_ir::contract::Contract;
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

fn fixture(document: serde_json::Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
}

/// Full OpenAPI 3.1 envelope with one static HTTPS server and bearer auth.
fn enveloped(paths: serde_json::Value, schemas: serde_json::Value) -> serde_json::Value {
    json!({
        "openapi":"3.1.0", "info":{"title":"Swift SDK","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}], "security":[{"apiKey":[]}],
        "components":{
            "securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
            "schemas":schemas
        },
        "paths":paths
    })
}

fn credit_api() -> serde_json::Value {
    enveloped(
        json!({"/credits/{account}":{"get":{
            "operationId":"getCredits", "description":"Read exact credits.",
            "parameters":[
                {"name":"account","in":"path","required":true,"schema":{"type":"string","minLength":1}},
                {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}}
            ],
            "responses":{
                "200":{"description":"Credit","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Credit"}}}},
                "401":{"description":"Denied","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Failure"}}}}
            }
        }}}),
        json!({
            "Credit":{"type":"object","properties":{"total":{"type":"string"},"usage":{"type":"number"}}},
            "Failure":{"type":"object","properties":{"message":{"type":"string"}}}
        }),
    )
}

fn plan(value: serde_json::Value) -> suspect_codegen::swift_sdk::SdkPlan {
    let contract = fixture(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, SwiftConfig::default()).unwrap()
}

#[test]
fn plans_selected_operations_with_native_names() {
    let plan = plan(credit_api());
    assert_eq!(plan.operations().len(), 1);
    let op = &plan.operations()[0];
    assert_eq!(op.operation_id, "getCredits");
    assert_eq!(op.method_name, "getCredits");
    assert_eq!(op.input_type, "GetCreditsInput");
    assert_eq!(op.success_type, "GetCreditsResult");
    assert_eq!(op.error_type, "GetCreditsAPIError");
    assert_eq!(op.parameters.len(), 2);
    assert_eq!(op.parameters[0].field_name, "account");
    assert_eq!(op.parameters[1].field_name, "limit");
}

#[test]
fn empty_selection_is_rejected() {
    let contract = fixture(credit_api());
    let error = plan_sdk(contract.clone(), &[], SwiftConfig::default()).unwrap_err();
    assert!(error.iter().any(|d| d.code == "http-no-operations"));
}

#[test]
fn directional_annotations_are_rejected() {
    let contract = fixture(enveloped(
        json!({"/x":{"get":{"operationId":"getX","responses":{
            "200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}}
        }}}}),
        json!({"Thing":{"type":"object","properties":{"a":{"type":"string","readOnly":true}}}}),
    ));
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let error = plan_sdk(contract.clone(), &selected, SwiftConfig::default()).unwrap_err();
    assert!(
        error
            .iter()
            .any(|d| d.code == "http-directional-codec-unsupported")
    );
}

#[test]
fn zero_byte_ceilings_are_rejected() {
    let contract = fixture(credit_api());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let error = plan_sdk(
        contract,
        &selected,
        SwiftConfig {
            max_response_bytes: 0,
            ..SwiftConfig::default()
        },
    )
    .unwrap_err();
    assert!(error.iter().any(|d| d.code == "http-resource-policy"));
}

#[test]
fn renders_installable_package_layout() {
    let plan = plan(credit_api());
    let files = emit_sdk(&plan, &Default::default()).unwrap();
    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"Package.swift"));
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("Sources/GeneratedSDK/") && p.ends_with("ExactJson.swift"))
    );
    let manifest = &files
        .iter()
        .find(|f| f.path == "Package.swift")
        .unwrap()
        .content;
    assert!(manifest.contains("swift-tools-version: 6.0"));
    assert!(manifest.contains("swiftLanguageMode(.v6)"));
    let exact = &files
        .iter()
        .find(|f| f.path.ends_with("ExactJson.swift"))
        .unwrap()
        .content;
    // Exact numbers: raw token preservation, never NSNumber/binary floats.
    assert!(exact.contains("struct JsonNumber"));
    assert!(exact.contains("public let raw: String"));
    let client = &files
        .iter()
        .find(|f| f.path.ends_with("Client.swift"))
        .unwrap()
        .content;
    assert!(client.contains("func send"));
    assert!(client.contains("async throws"));
    assert!(files.iter().any(|f| f.path.ends_with("Models.swift")));
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("ValidationData.swift"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with(".docc/GettingStarted.md"))
    );
    let operations = &files
        .iter()
        .find(|f| f.path.ends_with("Operations.swift"))
        .unwrap()
        .content;
    assert!(operations.contains("public func getCredits"));
    assert!(operations.contains("throw GetCreditsAPIError.status401"));
    assert!(plan.program().check().is_ok());
    let presence = &files
        .iter()
        .find(|f| f.path.ends_with("Presence.swift"))
        .unwrap()
        .content;
    assert!(presence.contains("case missing"));
    assert!(presence.contains("case null"));
    assert!(presence.contains("case value"));
}

#[test]
fn package_identity_admission() {
    let plan = plan(credit_api());
    let bad = emit_sdk(
        &plan,
        &suspect_codegen::swift_sdk::PackageConfig {
            name: "class".into(),
            version: "1.0".into(),
            module_name: "Generated SDK".into(),
        },
    )
    .unwrap_err();
    assert_eq!(
        bad.len(),
        3,
        "reserved package name, module and version are rejected"
    );
}

fn selected(contract: &Arc<Contract>) -> Vec<suspect_ir::contract::SourceId> {
    contract.operations().map(|o| o.source().clone()).collect()
}

#[test]
fn original_m2_contract_is_admitted_without_schema_or_operation_replacement() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = load(&path);
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap();
    assert_eq!(plan.operations().len(), 4);
    for name in [
        "Widget",
        "WidgetInput",
        "WidgetPayload",
        "WidgetNode",
        "SecurePayload",
        "StandardPayload",
    ] {
        assert!(
            plan.symbols().values().any(|n| n == name),
            "missing native model {name}"
        );
    }
    let files = plan.render();
    assert_eq!(
        files,
        plan.render(),
        "repeat emission must be byte-identical"
    );
    assert!(files.iter().any(|f| f.path == "example-coverage.json"));
}

#[test]
fn unsupported_reachable_fields_fail_at_original_sources_before_emission() {
    for schema in [
        json!({"type":["string","number"]}),
        json!({"type":"array","prefixItems":[{"type":"string"}]}),
        // Ordinary pattern properties are in the verified scoped profile. A
        // nonportable lookahead still fails at the original pattern source.
        json!({"type":"object","patternProperties":{"(?=a)":{"type":"string"}}}),
        json!({"allOf":[{"type":"object","properties":{"a":{"type":"string"}}},{"type":"object","properties":{"b":{"type":"number"}}}]}),
    ] {
        let contract = fixture(enveloped(
            json!({"/x":{"get":{"operationId":"getX","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}}}}}}),
            json!({"Thing":{"type":"object","properties":{"unsupported":schema}}}),
        ));
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            SwiftConfig::default(),
        )
        .expect_err("unsupported reachable field must fail planning");
        assert!(
            errors.iter().any(|e| e
                .source
                .pointer()
                .starts_with("/components/schemas/Thing/properties/unsupported")
                && e.at.end > e.at.start),
            "{errors:#?}"
        );
    }
}

#[test]
fn source_linked_url_profile_rejections_precede_artifacts() {
    for path in [
        "/x/{",
        "/x/}",
        "/x/%",
        "/x/%2e%2E",
        "/x/..",
        "/x?query",
        "/雪",
    ] {
        let contract = fixture(enveloped(
            json!({path:{"get":{"operationId":"getX","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}}}}}),
            json!({}),
        ));
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            SwiftConfig::default(),
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|d| d.code == "swift-path-unsupported"
                && d.source.pointer().starts_with("/paths/")
                && d.at.end > d.at.start),
            "{path}: {errors:#?}"
        );
    }
}

#[test]
fn runtime_names_and_import_modules_cannot_be_shadowed() {
    let document = enveloped(
        json!({"/x":{"get":{"operationId":"decodeResponse","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/JSONParser"}}}}}}}}),
        json!({"JSONParser":{"type":"object","properties":{"object":{"type":"string"},"context":{"type":"string"},"path":{"type":"string"},"value":{"type":"string"}}}}),
    );
    let plan = plan(document);
    assert_eq!(plan.operations()[0].method_name, "decodeResponse2");
    assert!(plan.symbols().values().any(|name| name == "JSONParser2"));
    for module in ["Swift", "Foundation", "FoundationNetworking", "XCTest"] {
        assert!(
            emit_sdk(
                &plan,
                &suspect_codegen::swift_sdk::PackageConfig {
                    module_name: module.into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn docs_only_updates_leave_executable_and_package_artifacts_identical() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let mut document = credit_api();
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let first = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap()
    .render();
    document["paths"]["/credits/{account}"]["get"]["description"] =
        json!("A documentation-only edit with `source` prose.");
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let second = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap()
    .render();
    let changed: Vec<_> = first
        .iter()
        .zip(&second)
        .filter(|(a, b)| a.content != b.content)
        .map(|(a, b)| {
            assert_eq!(a.path, b.path);
            a.path.as_str()
        })
        .collect();
    assert_eq!(
        changed,
        [
            "Sources/GeneratedSDK/GeneratedSDK.docc/OperationReference.md",
            "Sources/GeneratedSDK/Operations.swift"
        ]
    );
    let implementation = |files: &Vec<suspect_codegen::OutFile>| {
        files
            .iter()
            .find(|f| f.path.ends_with("Operations.swift"))
            .unwrap()
            .content
            .lines()
            .filter(|line| !line.trim_start().starts_with("///"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(implementation(&first), implementation(&second));
}

fn checked(command: &mut Command, retained: &Path) -> Output {
    let result = command
        .output()
        .expect("required native tool is unavailable");
    assert!(
        result.status.success(),
        "fixture retained at {}\ncommand {command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

fn native_root(name: &str) -> PathBuf {
    let root = std::env::var_os("SUSPECT_SWIFT_GATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-sdk-gates"));
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(root)
        .unwrap()
        .keep()
}

fn swift() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/bin/swift".into())
}

fn swift_compiler() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFTC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| swift().with_file_name("swiftc"))
}

fn swift_command(action: &str) -> Command {
    let mut command = Command::new(swift());
    command.arg(action).env("SWIFT_EXEC", swift_compiler());
    if action == "test" {
        // Every generated/consumer case in this gate is XCTest. Explicitly
        // selecting it also avoids Swift 6.0.x launching an unused Swift
        // Testing helper against a newer Xcode's private test frameworks.
        command.arg("--disable-swift-testing");
    }
    if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("--sdk").arg(&sdk).env("SDKROOT", sdk);
    }
    command
}

fn swift_compiler_command() -> Command {
    let mut command = Command::new(swift_compiler());
    if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("-sdk").arg(&sdk).env("SDKROOT", sdk);
    }
    command
}

fn review_root(name: &str) -> PathBuf {
    if let Some(root) = std::env::var_os("SUSPECT_SWIFT_REVIEW_ROOT") {
        let root = PathBuf::from(root).join(name);
        std::fs::create_dir_all(&root).unwrap();
        root
    } else {
        native_root(&format!("review-{name}-"))
    }
}

fn review_plan(root: &Path, document: serde_json::Value) -> suspect_codegen::swift_sdk::SdkPlan {
    let path = root.join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap()
}

fn review_package(plan: &suspect_codegen::swift_sdk::SdkPlan, root: &Path, source: &str) {
    suspect_codegen::write_files(&plan.render(), &root.join("sdk")).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/ReviewConsumer")).unwrap();
    std::fs::write(consumer.join("Package.swift"), "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"ReviewConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"ReviewConsumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(
        consumer.join("Tests/ReviewConsumer/ReviewTests.swift"),
        source,
    )
    .unwrap();
    let output = checked(
        swift_command("test")
            .arg("--package-path")
            .arg(&consumer)
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        root,
    );
    for line in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains("SWIFT_REVIEW_"))
    {
        println!("{line}");
    }
}

#[test]
#[ignore = "native package regression: source parameter names must not shadow example locals"]
fn native_review_example_local_names() {
    let root = review_root("example-names");
    let parameters = [
        ("client", "path", "account"),
        ("body", "query", "query-body"),
        ("input", "query", "query-input"),
        ("_parameter0", "query", "argument-zero"),
        ("request_body", "query", "body-label"),
    ].into_iter().map(|(name, location, example)| json!({"name":name,"in":location,"required":true,"schema":{"type":"string","example":example}})).collect::<Vec<_>>();
    let plan = review_plan(
        &root,
        enveloped(
            json!({"/echo/{client}":{"post":{
                "operationId":"echoNames","parameters":parameters,
                "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/EchoBody"},"example":{"text":"payload"}}}},
                "responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}}}
            }}}),
            json!({
                "EchoBody":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false},
                "Reply":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}
            }),
        ),
    );
    let labels = plan.operations()[0]
        .parameters
        .iter()
        .map(|p| p.field_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["client", "querybody", "input", "parameter0", "requestBody"]
    );
    review_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/native_review_examples.swift"),
    );
    println!("Swift example-name regression passed: {}", root.display());
}

#[test]
#[ignore = "native public-client resource regression with Darwin peak-allocation evidence"]
fn native_review_query_expansion_budget() {
    let root = review_root("query-budget");
    let plan = review_plan(
        &root,
        enveloped(
            json!({"/expand/{id}":{"get":{
                "operationId":"expand",
                "parameters":[
                    {"name":"lead","in":"query","schema":{"type":"string"}},
                    {"name":"_".repeat(4096),"in":"query","explode":true,"schema":{"type":"array","items":{"type":"string"}}},
                    {"name":"labels","in":"query","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
                    {"name":"amount","in":"query","schema":{"type":"number"}},
                    {"name":"enabled","in":"query","schema":{"type":"boolean"}},
                    {"name":"id","in":"path","required":true,"schema":{"type":"string","example":"x"}}
                ],
                "responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}}}}}
            }}}),
            json!({}),
        ),
    );
    assert_eq!(plan.operations()[0].parameters[1].field_name, "value");
    review_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/native_review_query.swift"),
    );
    println!("Swift bounded-query regression passed: {}", root.display());
}

fn native_package(
    plan: &suspect_codegen::swift_sdk::SdkPlan,
    root: &Path,
    consumer_source: &str,
    openrouter: bool,
) {
    let sdk = root.join("sdk");
    suspect_codegen::write_files(&plan.render(), &sdk).unwrap();
    checked(
        swift_command("test")
            .arg("--package-path")
            .arg(&sdk)
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        root,
    );
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/NativeConsumer")).unwrap();
    std::fs::write(consumer.join("Package.swift"), "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"NativeConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"NativeConsumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(
        consumer.join("Tests/NativeConsumer/NativeTests.swift"),
        consumer_source,
    )
    .unwrap();
    let server = NativeHTTPServer::start(root, openrouter);
    checked(
        swift_command("test")
            .arg("--package-path")
            .arg(&consumer)
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"])
            .env("SUSPECT_SWIFT_HTTP_BASE", &server.base)
            .env("SUSPECT_SWIFT_HTTP_MARKERS", root),
        root,
    );
    server.verify(openrouter);
}

#[test]
#[ignore = "requires Swift 6 and DocC; generates and consumes the unchanged original M2 fixture"]
fn native_m2_swift_spm_codecs_http_and_docs() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = load(&path);
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig {
            max_stream_capture_bytes: 64,
            ..Default::default()
        },
    )
    .unwrap();
    let root = native_root("m2-");
    native_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/native_m2.swift"),
        false,
    );
    native_docs(&root);
    native_negative_types(&root);
    println!(
        "Swift M2 package, independent consumer and DocC passed: {}",
        root.display()
    );
}

fn native_docs(root: &Path) {
    let sdk = root.join("sdk");
    checked(
        swift_command("package")
            .arg("--package-path")
            .arg(&sdk)
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["dump-symbol-graph", "--minimum-access-level", "public"]),
        root,
    );
    let graph = find_directory(&root.join("build/sdk"), "symbolgraph")
        .expect("SwiftPM public symbol graphs missing");
    let mut docc = if let Some(binary) = std::env::var_os("SUSPECT_SWIFT_DOCC_BIN") {
        Command::new(binary)
    } else {
        let mut command = Command::new("xcrun");
        command.arg("docc");
        command
    };
    checked(
        docc.arg("convert")
            .arg(sdk.join("Sources/GeneratedSDK/GeneratedSDK.docc"))
            .arg("--additional-symbol-graph-dir")
            .arg(&graph)
            .arg("--output-path")
            .arg(root.join("GeneratedSDK.doccarchive"))
            .arg("--warnings-as-errors"),
        root,
    );
    assert!(
        root.join("GeneratedSDK.doccarchive/index.html").is_file(),
        "rendered DocC output missing"
    );
}

fn find_directory(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if path.file_name()?.to_str()? == name {
                return Some(path);
            }
            if let Some(found) = find_directory(&path, name) {
                return Some(found);
            }
        }
    }
    None
}

fn native_negative_types(root: &Path) {
    let build =
        find_directory(&root.join("build/sdk"), "Modules").expect("built Swift module missing");
    let positive = root.join("positive.swift");
    std::fs::write(&positive, "import GeneratedSDK\nlet _ = WidgetInput(name: \"valid\")\nlet _ = WidgetPayload.standardPayload(StandardPayload(text: \"valid\"))\n").unwrap();
    checked(
        swift_compiler_command()
            .args(["-typecheck", "-swift-version", "6", "-module-cache-path"])
            .arg(root.join("frontend-cache"))
            .arg("-I")
            .arg(&build)
            .arg(&positive),
        root,
    );
    for (index, source) in [
        "let _ = WidgetInput()",
        "let _ = WidgetInput(name: nil)",
        "let _ = WidgetInput(name: \"x\", amount: .null)",
        "let _ = WidgetPayload.standardPayload(SecurePayload(vault: \"v\"))",
        "let _ = CreateWidgetInput()",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("negative-{index}.swift"));
        std::fs::write(&path, format!("import GeneratedSDK\n{source}\n")).unwrap();
        let result = swift_compiler_command()
            .args(["-typecheck", "-swift-version", "6", "-module-cache-path"])
            .arg(root.join("frontend-cache"))
            .arg("-I")
            .arg(&build)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "negative type example unexpectedly compiled: {source}"
        );
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            !error.contains("no such module") && !error.contains("unable to load standard library"),
            "negative check failed due to missing import: {error}"
        );
    }
}

#[test]
#[ignore = "requires the actual OpenRouter checkout, Swift 6 and DocC"]
fn native_five_actual_openrouter_operations() {
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    assert!(
        path.is_file(),
        "actual OpenRouter corpus is required: {}",
        path.display()
    );
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
        .filter(|o| o.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), wanted.len());
    let plan = plan_sdk(contract, &operations, SwiftConfig::default()).unwrap();
    let body_symbol = |operation: &str| {
        let op = plan
            .operations()
            .iter()
            .find(|op| op.operation_id == operation)
            .unwrap();
        let schema = op
            .source
            .child("requestBody")
            .child("content")
            .child("application/json")
            .child("schema");
        plan.symbols()[&schema].clone()
    };
    let source = include_str!("../src/swift_sdk/native_openrouter.swift")
        .replace("__CREATE_BODY__", &body_symbol("createKeys"))
        .replace("__UPDATE_BODY__", &body_symbol("updateKeys"));
    let root = native_root("openrouter-");
    native_package(&plan, &root, &source, true);
    native_docs(&root);
    println!(
        "Swift five-operation OpenRouter package, consumer and DocC passed: {}",
        root.display()
    );
}

#[derive(Debug)]
struct WireRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}
impl WireRequest {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
}
struct NativeHTTPServer {
    base: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    records: Arc<std::sync::Mutex<Vec<WireRequest>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl NativeHTTPServer {
    fn start(root: &Path, openrouter: bool) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (stopped, recorded) = (stop.clone(), records.clone());
        let root = root.to_owned();
        let redirect_base = base.clone();
        let thread = std::thread::spawn(move || {
            let credits = Arc::new(AtomicUsize::new(0));
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (records, credits, root, base) = (
                            recorded.clone(),
                            credits.clone(),
                            root.clone(),
                            redirect_base.clone(),
                        );
                        workers.push(std::thread::spawn(move||{
                            if let Some(request)=read_request(&stream){
                                let target=request.target.clone();
                                records.lock().unwrap().push(request);
                                let (status,body)=if openrouter {
                                    if target=="/api/v1/credits" {
                                        if credits.fetch_add(1,Ordering::SeqCst)==0 {(200,r#"{"data":{"total_credits":100.50000000000000001,"total_usage":25.75}}"#.into())}
                                        else{(401,r#"{"error":{"code":401,"message":"Missing Authentication header"}}"#.into())}
                                    }else if target=="/api/v1/keys"{(201,swift_fixture("createJSON",true))}
                                    else if target.starts_with("/api/v1/keys/"){(200,swift_fixture("updateJSON",true))}
                                    else if target.starts_with("/api/v1/containers/sess_abc123/files?"){(200,r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#.into())}
                                    else{(200,swift_fixture("fileJSON",true))}
                                }else{
                                    match target.as_str(){
                                        "/api/v1/widgets/redirect"=>{write_redirect(stream,&base);return;},
                                        "/api/v1/widgets/oversized"=>{write_chunked(stream);return;},
                                        "/api/v1/widgets/cancelled"=>{std::fs::write(root.join("cancel-received"),b"received").unwrap();std::thread::sleep(std::time::Duration::from_secs(2));(200,swift_fixture("widgetJSON",false))},
                                        "/api/v1/widgets/delay"=>{std::thread::sleep(std::time::Duration::from_secs(2));(200,swift_fixture("widgetJSON",false))},
                                        "/api/v1/widgets/declared-large"=>(404,format!(r#"{{"message":"{}"}}"#,"x".repeat(256))),
                                        "/api/v1/widgets/unknown"=>(500,"x".repeat(256)),
                                        _=>(200,swift_fixture("widgetJSON",false)),
                                    }
                                };
                                write_response(stream,status,&body);
                            }
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(e) => panic!("fixture listener failed: {e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            records,
            thread: Some(thread),
        }
    }
    fn verify(&self, openrouter: bool) {
        let records = self.records.lock().unwrap();
        assert!(
            !records.is_empty(),
            "native URLSession sent no real HTTP requests"
        );
        assert!(
            !records.iter().any(|r| r.target.contains("/leak")),
            "URLSession followed a redirect"
        );
        let token = if openrouter {
            "test-management-token"
        } else {
            "m2-key"
        };
        for request in records.iter() {
            assert_eq!(request.header("authorization"), format!("Bearer {token}"));
            assert_eq!(request.header("accept"), "application/json");
        }
        if openrouter {
            assert_eq!(
                records.len(),
                6,
                "six actual URLSession exchanges including the declared API failure"
            );
            assert_eq!(records[0].target, "/api/v1/credits");
            assert_eq!(records[0].method, "GET");
            assert_eq!(records[2].method, "POST");
            assert_eq!(records[2].target, "/api/v1/keys");
            assert_eq!(
                records[2].body,
                r#"{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}"#
            );
            assert_eq!(records[2].header("content-type"), "application/json");
            assert_eq!(records[3].method, "PATCH");
            assert_eq!(
                records[3].body,
                r#"{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}"#
            );
            assert_eq!(
                records[4].target,
                "/api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A"
            );
            assert_eq!(
                records[5].target,
                "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA"
            );
        } else {
            for target in [
                "wire-check",
                "redirect",
                "oversized",
                "cancelled",
                "delay",
                "declared-large",
                "unknown",
            ] {
                assert!(
                    records
                        .iter()
                        .any(|r| r.target == format!("/api/v1/widgets/{target}")),
                    "missing native URLSession case {target}"
                );
            }
        }
    }
}
impl Drop for NativeHTTPServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn swift_fixture(name: &str, openrouter: bool) -> String {
    let source = if openrouter {
        include_str!("../src/swift_sdk/native_openrouter.swift")
    } else {
        include_str!("../src/swift_sdk/native_m2.swift")
    };
    source
        .split(&format!("private let {name} = #\""))
        .nth(1)
        .unwrap()
        .split("\"#")
        .next()
        .unwrap()
        .to_owned()
}
fn read_request(mut stream: &std::net::TcpStream) -> Option<WireRequest> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
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
    let text = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let mut lines = text.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.into();
    let target = first.next()?.into();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().into(), v.trim().into()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    while bytes.len() - end < length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 64 * 1024 {
            return None;
        }
    }
    Some(WireRequest {
        method,
        target,
        headers,
        body: String::from_utf8(bytes[end..end + length].to_vec()).ok()?,
    })
}
fn write_response(mut stream: std::net::TcpStream, status: u16, body: &str) {
    use std::io::Write;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}
fn write_redirect(mut stream: std::net::TcpStream, base: &str) {
    use std::io::Write;
    let _ = write!(
        stream,
        "HTTP/1.1 302 Fixture\r\nLocation: {base}/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}
fn write_chunked(mut stream: std::net::TcpStream) {
    use std::io::Write;
    let body = "x".repeat(512);
    let _ = write!(
        stream,
        "HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n200\r\n{body}\r\n0\r\n\r\n"
    );
}
