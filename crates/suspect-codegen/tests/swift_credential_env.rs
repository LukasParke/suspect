//! Explicit environment names -> bound Swift plan -> installed native clients.
//! Source fixtures and expected requests are literal; account calls are captured.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    credential_env::{CredentialEnv, CredentialEnvKind},
    swift_sdk::{PackageConfig, SdkPlan, SwiftConfig, emit_sdk, plan_sdk},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

#[allow(dead_code)]
#[path = "../src/swift_sdk/validation_v2_support.rs"]
mod support;

const ENTRY: &str = "https://swift-credential.test/openapi.json";

fn document() -> Value {
    let ok = json!({"200":{"description":"OK","content":{"application/json":{"schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}}}}});
    json!({"openapi":"3.1.0","info":{"title":"Swift environment credentials","version":"1"},"servers":[{"url":"https://default.swift.test/api/v1"}],"security":[{"apiKey":[]}],
        "paths":{
            "/key":{"get":{"operationId":"getCurrentKey","responses":ok}},
            "/either":{"get":{"operationId":"eitherKey","security":[{"apiKey":[]},{"headerKey":[]}],"responses":ok}},
            "/both":{"get":{"operationId":"bothKeys","security":[{"apiKey":[],"headerKey":[]}],"responses":ok}},
            "/query":{"get":{"operationId":"queryCredential","security":[{"queryKey":[]}],"responses":ok}},
            "/cookie":{"get":{"operationId":"cookieCredential","security":[{"cookieKey":[]}],"responses":ok}},
            "/anonymous":{"get":{"operationId":"anonymous","security":[],"responses":ok}},
            "/choice":{"get":{"operationId":"anonymousChoice","security":[{"apiKey":[]},{}],"responses":ok}},
            "/model":{"get":{"operationId":"fromEnvironment","security":[],"responses":{"200":{"description":"A source name matching a Foundation type","content":{"application/json":{"schema":{"$ref":"#/components/schemas/ProcessInfo"}}}}}}}
        },
        "components":{"securitySchemes":{
            "baseBearer":{"type":"http","scheme":"Bearer"},"apiKey":{"$ref":"#/components/securitySchemes/baseBearer"},
            "headerKey":{"type":"apiKey","in":"header","name":"X-API-Key"},
            "queryKey":{"type":"apiKey","in":"query","name":"access_key"},
            "cookieKey":{"type":"apiKey","in":"cookie","name":"session"}
        },"schemas":{"ProcessInfo":{"type":"object","properties":{"environment":{"type":"string"}},"required":["environment"],"additionalProperties":false}}}})
}

fn contract(value: &Value) -> Arc<Contract> {
    let uri = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            value.to_string().into_bytes(),
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

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn package() -> PackageConfig {
    PackageConfig {
        name: "SwiftEnvSDK".into(),
        module_name: "SwiftEnvSDK".into(),
        version: "1.0.0".into(),
    }
}

fn hashes(files: &[suspect_codegen::OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                format!("{:x}", Sha256::digest(file.content.as_bytes())),
            )
        })
        .collect()
}

fn policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("apiKey", "SUSPECT_SWIFT_ENV_BEARER"),
            ("headerKey", "SUSPECT_SWIFT_ENV_HEADER"),
            ("queryKey", "SUSPECT_SWIFT_ENV_QUERY"),
            ("cookieKey", "SUSPECT_SWIFT_ENV_COOKIE"),
        ]
        .into_iter()
        .map(|(name, variable)| (name.into(), variable.into()))
        .collect(),
    )
}

fn configured(contract: Arc<Contract>, policy: CredentialEnv) -> SdkPlan {
    plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig {
            credential_env: Some(policy),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn no_policy_artifacts_are_frozen() {
    let contract = contract(&document());
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap();
    let files = emit_sdk(&plan, &package()).unwrap();
    let expected: BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/swift-credential-env-no-policy.json")).unwrap();
    assert_eq!(hashes(&files), expected);
    assert!(plan.credential_env().is_none());
}

#[test]
fn bound_policy_retains_source_identity_and_reserves_only_configured_helpers() {
    let contract = contract(&document());
    let plan = configured(contract.clone(), policy());
    let bound = plan.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 4);
    let bearer = bound
        .bindings()
        .iter()
        .find(|binding| binding.name() == "apiKey")
        .unwrap();
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(bearer.variable(), "SUSPECT_SWIFT_ENV_BEARER");
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert_eq!(
        bearer.scheme().terminal().source().pointer(),
        "/components/securitySchemes/baseBearer"
    );
    assert!(bearer.scheme().use_site().span().end > bearer.scheme().use_site().span().start);
    let descriptor = serde_json::to_string(&bound.semantic_descriptor()).unwrap();
    assert!(descriptor.contains("SUSPECT_SWIFT_ENV_BEARER"));
    assert!(
        !descriptor.contains(ENTRY)
            && !descriptor.contains("Provenance")
            && !descriptor.contains("span")
    );
    let renamed = plan
        .operations()
        .iter()
        .find(|operation| operation.operation_id == "fromEnvironment")
        .unwrap();
    assert_eq!(renamed.method_name, "fromEnvironment2");
    let ordinary = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap();
    assert_eq!(
        ordinary
            .operations()
            .iter()
            .find(|operation| operation.operation_id == "fromEnvironment")
            .unwrap()
            .method_name,
        "fromEnvironment"
    );
    let files = emit_sdk(&plan, &package()).unwrap();
    assert!(
        files
            .iter()
            .find(|file| file.path.ends_with("/Client.swift"))
            .unwrap()
            .content
            .contains("Foundation.ProcessInfo.processInfo.environment")
    );
    assert!(
        files
            .iter()
            .find(|file| file.path.ends_with("GettingStarted.md"))
            .unwrap()
            .content
            .contains("8192 UTF-8 bytes")
    );
}

#[test]
fn shared_binding_failures_precede_artifacts() {
    let contract = contract(&document());
    for policy in [
        CredentialEnv::v1(BTreeMap::new()),
        CredentialEnv::v1(BTreeMap::from([("unused".into(), "UNUSED_KEY".into())])),
        CredentialEnv::v1(BTreeMap::from([("apiKey".into(), "bad-name".into())])),
    ] {
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            SwiftConfig {
                credential_env: Some(policy),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code.starts_with("sdk-credential-env"))
        );
    }
    for kind in [
        json!({"type":"http","scheme":"basic"}),
        json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.test/token","scopes":{}}}}),
        json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.test/discovery"}),
    ] {
        let mut value = document();
        value["components"]["securitySchemes"]["baseBearer"] = kind;
        let contract = self::contract(&value);
        let errors = plan_sdk(
            contract.clone(),
            &selected(&contract),
            SwiftConfig {
                credential_env: Some(policy()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == "sdk-credential-env-kind"
                    && error.source.pointer() == "/components/securitySchemes/baseBearer"
                    && error.at.end > error.at.start),
            "{errors:?}"
        );
    }
}

#[test]
fn generation_never_reads_credential_values() {
    const CANARY: &str = "GENERATOR_ONLY_SWIFT_CREDENTIAL_CANARY_7A35";
    if std::env::var_os("SUSPECT_SWIFT_CREDENTIAL_CANARY_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generation_never_reads_credential_values",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SUSPECT_SWIFT_CREDENTIAL_CANARY_CHILD", "1")
            .env("SUSPECT_SWIFT_ENV_BEARER", CANARY)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "generator canary child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let plan = configured(contract(&document()), policy());
    for file in emit_sdk(&plan, &package()).unwrap() {
        assert!(
            !file.content.contains(CANARY),
            "generation exposed a credential value in {}",
            file.path
        );
    }
    assert!(
        !serde_json::to_string(plan.credential_env().unwrap())
            .unwrap()
            .contains(CANARY)
    );
}

fn root(label: &str) -> PathBuf {
    let base = std::env::var_os("SUSPECT_SWIFT_CREDENTIAL_ENV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-credential-env"));
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

fn installed(root: &Path, plan: &SdkPlan, package: &PackageConfig, native: &str) {
    let files = emit_sdk(plan, package).unwrap();
    std::fs::write(
        root.join("generated-hashes.json"),
        serde_json::to_vec_pretty(&hashes(&files)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("bound-credential-env.json"),
        serde_json::to_vec_pretty(plan.credential_env().unwrap()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("credential-env-interface.json"),
        serde_json::to_vec_pretty(&plan.credential_env().unwrap().semantic_descriptor()).unwrap(),
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root.join("sdk")).unwrap();
    support::checked(
        support::swift("test")
            .args(["--jobs", "4"])
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        root,
    );
    std::fs::create_dir_all(root.join("consumer/Tests/Consumer")).unwrap();
    std::fs::write(root.join("consumer/Package.swift"), format!("// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"Consumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"Consumer\", dependencies: [.product(name: \"{}\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n", package.name)).unwrap();
    std::fs::write(
        root.join("consumer/Tests/Consumer/CredentialEnv.swift"),
        native,
    )
    .unwrap();
    let command = &mut support::swift("test");
    command
        .args(["--jobs", "4"])
        .arg("--package-path")
        .arg(root.join("consumer"))
        .arg("--scratch-path")
        .arg(root.join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"]);
    for variable in [
        "SUSPECT_SWIFT_ENV_BEARER",
        "SUSPECT_SWIFT_ENV_HEADER",
        "SUSPECT_SWIFT_ENV_QUERY",
        "SUSPECT_SWIFT_ENV_COOKIE",
    ] {
        command.env_remove(variable);
    }
    command.env("OPENROUTER_API_KEY", "swift-controlled-native-key");
    support::checked(command, root);
    support::docs(root, &package.module_name);
}

fn typing(root: &Path, module: &str, positive: &str, negatives: &[(&str, &str)]) {
    let modules = support::directory(&root.join("build/sdk"), "Modules").unwrap();
    let positive_path = root.join("positive.swift");
    std::fs::write(&positive_path, format!("import {module}\n{positive}\n")).unwrap();
    support::checked(
        support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&positive_path),
        root,
    );
    for (index, (source, expected)) in negatives.iter().enumerate() {
        let path = root.join(format!("negative-{index}.swift"));
        std::fs::write(&path, format!("import {module}\n{source}\n")).unwrap();
        let output = support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&path)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        std::fs::write(root.join(format!("negative-{index}.log")), error.as_bytes()).unwrap();
        assert!(
            !output.status.success() && error.contains(expected),
            "negative typing reason: {expected}\n{error}"
        );
        assert!(!error.contains("no such module"));
    }
}

#[test]
#[ignore = "installed Swift environment defaults/precedence/security/snapshot/byte limits and DocC on the selected tier"]
fn native_credential_env_precedence_snapshot_and_docs() {
    let root = root("synthetic-");
    let plan = configured(contract(&document()), policy());
    installed(
        &root,
        &plan,
        &package(),
        include_str!("../src/swift_sdk/credential_env_native.swift"),
    );
    typing(
        &root,
        "SwiftEnvSDK",
        "let _ = Client()\nlet _ = Client.fromEnvironment()\nlet _ = Client(credentials: Credentials())\nlet _ = Client(credentials: Credentials(apiKey: nil))\nlet _: (Credentials, any HTTPTransport, ClientOptions) -> Client = Client.init(credentials:transport:options:)\nlet _: (any HTTPTransport, ClientOptions) -> Client = Client.fromEnvironment(transport:options:)",
        &[
            ("let _ = Client(credentials: nil)", "not compatible"),
            ("let _ = Client(credentials: [:])", "cannot convert"),
            (
                "let _ = Client.fromEnvironment(credentials: Credentials())",
                "extra argument",
            ),
        ],
    );
    println!(
        "Swift credential environment native precedence/snapshot/DocC passed at {}",
        root.display()
    );
}

#[test]
#[ignore = "actual OpenRouter getCurrentKey/getCredits schemas, controlled HTTPS transport capture, environment API and DocC"]
fn native_openrouter_current_key_environment_defaults() {
    let root = root("openrouter-");
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let input = checkout.join("projects/docs/openapi/openapi.yaml");
    let bytes = std::fs::read(&input).expect("actual OpenRouter source checkout required");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(input.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap());
    let operations = contract
        .operations()
        .filter(|operation| {
            matches!(
                operation.operation_id(),
                Some("getCurrentKey" | "getCredits")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), 2);
    let plan = plan_sdk(
        contract.clone(),
        &operations,
        SwiftConfig {
            credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
                "apiKey".into(),
                "OPENROUTER_API_KEY".into(),
            )]))),
            ..Default::default()
        },
    )
    .unwrap();
    let source_hash = format!("{:x}", Sha256::digest(&bytes));
    std::fs::write(root.join("source-receipt.json"), serde_json::to_vec_pretty(&json!({"source":input,"sha256":source_hash,"operations":["getCurrentKey","getCredits"],"mode":"controlled-native-transport"})).unwrap()).unwrap();
    let package = PackageConfig {
        name: "OpenRouter".into(),
        module_name: "OpenRouter".into(),
        version: "1.0.0".into(),
    };
    installed(
        &root,
        &plan,
        &package,
        include_str!("../src/swift_sdk/credential_env_openrouter.swift"),
    );
    typing(
        &root,
        "OpenRouter",
        "let _ = Client()\nlet _ = Client.fromEnvironment(options: ClientOptions(timeout: 15))\nlet _ = Client(credentials: Credentials(apiKey: nil))",
        &[("let _ = Client(credentials: nil)", "not compatible")],
    );
    assert_eq!(std::fs::read(input).unwrap(), bytes);
    println!(
        "Swift actual OpenRouter GET /key environment default/HTTPS source/DocC passed at {}",
        root.display()
    );
}
