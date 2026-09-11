//! Bounded source-policy, native environment snapshot and installed OpenRouter checks.
use super::validation_tests::{Native, write};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const SOURCE: &str = include_str!("testdata/credential-env.openapi.json");
fn directory(prefix: &str) -> PathBuf {
    let parent =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-credential-env");
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(parent).unwrap())
        .unwrap()
        .keep()
}
fn contract() -> Arc<Contract> {
    contract_at(SOURCE, "https://source.csharp.test/credential-env.json")
}
fn contract_at(text: &str, uri: &str) -> Arc<Contract> {
    let uri = Uri::parse(uri).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            text.as_bytes().to_vec(),
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
fn config() -> super::SdkConfig {
    super::SdkConfig {
        name: "OpenRouter.SDK".into(),
        version: "1.0.0".into(),
        namespace: "OpenRouter".into(),
    }
}
fn policy() -> crate::credential_env::CredentialEnv {
    crate::credential_env::CredentialEnv::v1(
        [
            ("apiKey", "CSHARP_ENV_BEARER"),
            ("headerKey", "CSHARP_ENV_HEADER"),
            ("queryKey", "CSHARP_ENV_QUERY"),
            ("cookieKey", "CSHARP_ENV_COOKIE"),
            ("aliasKey", "CSHARP_ENV_ALIAS"),
        ]
        .into_iter()
        .map(|(scheme, variable)| (scheme.into(), variable.into()))
        .collect(),
    )
}
fn plan(policy: Option<crate::credential_env::CredentialEnv>) -> super::SdkPlan {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk_with_options(
        contract,
        &selected,
        config(),
        super::protocol::ProtocolOptions {
            credential_env: policy,
            ..Default::default()
        },
    )
    .unwrap()
}
fn package(native: &Native, plan: &super::SdkPlan, consumer: &str) {
    crate::write_files(&plan.render().unwrap(), &native.root).unwrap();
    native.checked(
        "csharp",
        "package-restore",
        &["restore", "--configfile", "../NuGet.Config"],
    );
    native.checked(
        "csharp",
        "package-pack",
        &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
    );
    native.project("consumer", Some("OpenRouter.SDK"), true);
    write(&native.root.join("consumer/Program.cs"), consumer);
    native.checked(
        "consumer",
        "consumer-restore",
        &["restore", "--configfile", "../NuGet.Config"],
    );
    native.checked(
        "consumer",
        "consumer-run",
        &["run", "-c", "Release", "--no-restore"],
    );
}

#[test]
#[ignore = "bounded installed .NET8/10 environment snapshot, explicit credentials and auth alternatives"]
fn native_environment_credentials() {
    let root = directory("native-");
    let plan = plan(Some(policy()));
    let mut consumer = include_str!("testdata/CredentialEnvironmentConsumer.cs").to_owned();
    for binding in plan.credential_bindings() {
        consumer = consumer.replace(
            &format!("__{}__", binding.requirement.name()),
            &binding.property_name,
        );
    }
    write(&root.join("source.openapi.json"), SOURCE);
    write(
        &root.join("bound-policy.json"),
        serde_json::to_vec_pretty(plan.credential_env().unwrap()).unwrap(),
    );
    for (sdk, framework) in [("8.0.424", "net8.0"), ("10.0.400", "net10.0")] {
        let native = Native::new(&root, sdk, framework);
        package(&native, &plan, &consumer);
        native.finish("configured-environment-credentials");
    }
    println!("C# environment native evidence: {}", root.display());
}

#[test]
#[ignore = "actual OpenRouter getCurrentKey/getCredits schemas and source-default HTTPS through controlled .NET8/10 handlers"]
fn native_openrouter_environment_client() {
    let root = directory("openrouter-");
    let corpus = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/luke/github/openrouter-web"));
    let path = corpus.join("projects/docs/openapi/openapi.yaml");
    let original = fs::read(&path).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .filter(|op| matches!(op.operation_id(), Some("getCurrentKey" | "getCredits")))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let policy = crate::credential_env::CredentialEnv::v1(
        [("apiKey".into(), "OPENROUTER_API_KEY".into())]
            .into_iter()
            .collect(),
    );
    let plan = super::plan_sdk_with_options(
        contract.clone(),
        &selected,
        config(),
        super::protocol::ProtocolOptions {
            credential_env: Some(policy),
            ..Default::default()
        },
    )
    .unwrap();
    let mut responses = json!({});
    for operation in plan.operations() {
        for status in ["200", "401"] {
            let at = operation
                .source
                .child("responses")
                .child(status)
                .child("content")
                .child("application/json")
                .child("example");
            responses[&operation.operation_id][status] = contract
                .source(&at)
                .expect("actual source-declared response example")
                .clone();
        }
    }
    write(&root.join("source.openapi.yaml"), &original);
    write(
        &root.join("responses.json"),
        serde_json::to_vec_pretty(&responses).unwrap(),
    );
    for (sdk, framework) in [("8.0.424", "net8.0"), ("10.0.400", "net10.0")] {
        let native = Native::new(&root, sdk, framework);
        write(
            &native.root.join("consumer/responses.json"),
            serde_json::to_vec_pretty(&responses).unwrap(),
        );
        package(
            &native,
            &plan,
            include_str!("testdata/OpenRouterEnvironmentConsumer.cs"),
        );
        native.finish("actual-openrouter-environment-factory");
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "read-only actual source"
    );
    println!(
        "C# actual OpenRouter environment evidence: {}",
        root.display()
    );
}

#[test]
fn absent_policy_preserves_package_surface() {
    let root = directory("no-policy-");
    let ordinary = plan(None);
    let contract = contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let original = super::plan_sdk(contract, &selected, config()).unwrap();
    assert!(ordinary.credential_env().is_none());
    let before = original.render().unwrap();
    let after = ordinary.render().unwrap();
    assert_eq!(before.len(), after.len());
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old.path, new.path);
        assert_eq!(
            old.content, new.content,
            "no-policy entrypoint parity: {}",
            old.path
        );
        assert!(!new.path.ends_with("CredentialEnvironment.cs"));
        assert!(
            !new.content.contains("FromEnvironment"),
            "env helper leaked into unconfigured artifact {}",
            new.path
        );
    }
    assert_eq!(
        after
            .iter()
            .find(|f| f.path == "csharp/src/HttpRuntime.cs")
            .unwrap()
            .content,
        include_str!("HttpRuntime.cs").replace("__NAMESPACE__", "OpenRouter")
    );
    crate::write_files(&after, &root).unwrap();
    write(
        &root.join("PASS.json"),
        json!({"result":"passed","gate":"no-policy-package-surface","files":after.len()})
            .to_string(),
    );
    println!("C# no-policy evidence: {}", root.display());
}

#[test]
fn configured_environment_binds_declarations_without_terminal_alias_leakage() {
    let root = directory("binding-");
    let plan = plan(Some(policy()));
    let environment = plan.credential_env().unwrap();
    assert_eq!(environment.bindings().len(), 5);
    let api = environment
        .bindings()
        .iter()
        .find(|b| b.name() == "apiKey")
        .unwrap();
    let alias = environment
        .bindings()
        .iter()
        .find(|b| b.name() == "aliasKey")
        .unwrap();
    assert_eq!(
        api.scheme().terminal().source(),
        alias.scheme().terminal().source()
    );
    assert_ne!(
        api.scheme().use_site().source(),
        alias.scheme().use_site().source()
    );
    assert_ne!(
        plan.credentials()[&super::emit::source(api.scheme().use_site().source())],
        plan.credentials()[&super::emit::source(alias.scheme().use_site().source())]
    );
    assert_eq!(api.kind(), crate::credential_env::CredentialEnvKind::Bearer);
    assert!(
        environment
            .bindings()
            .iter()
            .filter(|b| b.name().ends_with("Key") && b.name() != "apiKey" && b.name() != "aliasKey")
            .all(|b| b.kind() == crate::credential_env::CredentialEnvKind::ApiKey)
    );
    let descriptor = serde_json::to_value(environment.semantic_descriptor()).unwrap();
    assert!(!descriptor.to_string().contains("source.csharp.test"));
    assert_eq!(descriptor["version"], "v1");
    let files = plan.render().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f.path == "csharp/src/CredentialEnvironment.cs")
    );
    let client = &files
        .iter()
        .find(|f| f.path == "csharp/src/Client.g.cs")
        .unwrap()
        .content;
    assert_eq!(
        client
            .matches("public static Client FromEnvironment(")
            .count(),
        1
    );
    assert!(client.contains("public Client(Credentials credentials, ClientOptions? options = null, HttpClient? httpClient = null)"));
    assert!(client.contains("public Client() : this(new Credentials())"));
    let reference: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "csharp/docs/reference.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(reference["credentialEnv"], descriptor);
    crate::write_files(&files, &root).unwrap();
    write(&root.join("PASS.json"),json!({"result":"passed","gate":"source-bound-env-plan","bindings":5,"descriptor":descriptor}).to_string());
    println!("C# environment binding evidence: {}", root.display());
}

#[test]
fn generator_environment_values_are_absent_from_artifacts() {
    const CANARY: &str = "SYNTHETIC_CSHARP_GENERATOR_SECRET_20260911";
    if std::env::var_os("SUSPECT_CSHARP_CREDENTIAL_CANARY_CHILD").is_none() {
        let root = directory("generator-canary-");
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","csharp_sdk::credential_env_tests::generator_environment_values_are_absent_from_artifacts","--nocapture","--test-threads=1"])
            .env("SUSPECT_CSHARP_CREDENTIAL_CANARY_CHILD","1")
            .env("CSHARP_ENV_BEARER",CANARY)
            .output().unwrap();
        write(
            &root.join("child.log"),
            [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
        );
        assert!(
            output.status.success(),
            "isolated generator canary: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        write(&root.join("PASS.json"),json!({"result":"passed","gate":"generator-secret-value-not-emitted","status":output.status.code()}).to_string());
        println!("C# generator canary evidence: {}", root.display());
        return;
    }
    assert_eq!(std::env::var("CSHARP_ENV_BEARER").unwrap(), CANARY);
    let plan = plan(Some(policy()));
    assert!(
        !serde_json::to_string(plan.credential_env().unwrap())
            .unwrap()
            .contains(CANARY)
    );
    for file in plan.render().unwrap() {
        assert!(
            !file.content.contains(CANARY),
            "generator secret entered {}",
            file.path
        );
    }
}

#[test]
fn canonical_environment_capture_keeps_typed_semantics() {
    use crate::{
        backend::{self, Backend, GenerationOptions, TargetConfig},
        compatibility,
    };
    let root = directory("capture-");
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "OpenRouter.SDK".into(),
        package_version: "1.0.0".into(),
        import_name: Some("OpenRouter".into()),
    };
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let before = compatibility::snapshot_with_options(
        contract(),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(
        before.native[0].status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        before.native[0].findings
    );
    assert_eq!(
        before.native[0].credential_env,
        Some(
            plan(Some(policy()))
                .credential_env()
                .unwrap()
                .semantic_descriptor()
        )
    );
    let descriptor =
        serde_json::to_string(before.native[0].credential_env.as_ref().unwrap()).unwrap();
    assert!(
        !descriptor.contains("source.csharp.test")
            && !descriptor.contains("terminal")
            && !descriptor.contains("references")
    );
    let source = contract();
    let selected = source
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let generated = backend::generate_with_options(source, &selected, &target, &options).unwrap();
    let direct = plan(Some(policy())).render().unwrap();
    assert_eq!(generated.len(), direct.len());
    for (a, b) in generated.iter().zip(&direct) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.content, b.content, "canonical env forwarding: {}", a.path);
    }
    let relocated = compatibility::snapshot_with_options(
        contract_at(SOURCE, "https://relocated.csharp.test/credential-env.json"),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(
        relocated.native[0].credential_env,
        before.native[0].credential_env
    );
    let mut changed = options.clone();
    changed
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "ANOTHER_CSHARP_ENV_BEARER".into());
    let after = compatibility::snapshot_with_options(contract(), &[], &[target], &changed).unwrap();
    let comparison = compatibility::compare_snapshots(&before, &after);
    assert!(
        comparison.wire.is_empty(),
        "environment policy is not an OpenAPI wire change"
    );
    assert!(
        comparison.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-credential-env-changed")
    );
    write(
        &root.join("before.json"),
        serde_json::to_vec_pretty(&before.native[0]).unwrap(),
    );
    write(
        &root.join("after.json"),
        serde_json::to_vec_pretty(&after.native[0]).unwrap(),
    );
    write(
        &root.join("comparison.json"),
        serde_json::to_vec_pretty(&comparison).unwrap(),
    );
    write(&root.join("PASS.json"),json!({"result":"passed","gate":"canonical-env-capture","typedDescriptor":true,"physicalProvenanceExcluded":true}).to_string());
    println!("C# canonical credential-env evidence: {}", root.display());
}

#[test]
fn environment_helper_names_and_unsupported_hooks_are_source_checked() {
    let mut source: Value = serde_json::from_str(SOURCE).unwrap();
    source["components"]["schemas"]["CredentialEnvironment"] =
        json!({"type":"object","properties":{"value":{"type":"string"}}});
    source["paths"]["/collision"] = json!({"get":{"operationId":"fromEnvironment","security":[{"apiKey":[]}],"responses":{"200":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/CredentialEnvironment"}}}}}}});
    let contract = contract_at(
        &source.to_string(),
        "https://source.csharp.test/collision.json",
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let configured = super::plan_sdk_with_options(
        contract.clone(),
        &selected,
        config(),
        super::protocol::ProtocolOptions {
            credential_env: Some(policy()),
            ..Default::default()
        },
    )
    .unwrap();
    let ordinary = super::plan_sdk(contract, &selected, config()).unwrap();
    let name = |p: &super::SdkPlan| {
        p.models()
            .names()
            .iter()
            .find(|(key, _)| key.0.pointer() == "/components/schemas/CredentialEnvironment")
            .unwrap()
            .1
            .clone()
    };
    assert_eq!(name(&ordinary), "CredentialEnvironment");
    assert_ne!(name(&configured), "CredentialEnvironment");
    assert!(
        configured
            .operations()
            .iter()
            .any(|op| op.operation_id == "fromEnvironment"
                && op.method_name == "FromEnvironmentAsync")
    );
    for scheme in [
        json!({"type":"http","scheme":"basic"}),
        json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.example/token","scopes":{}}}}),
        json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.example/discovery"}),
    ] {
        let mut source: Value = serde_json::from_str(SOURCE).unwrap();
        source["components"]["securitySchemes"]["apiKey"] = scheme;
        let contract = contract_at(
            &source.to_string(),
            "https://source.csharp.test/unsupported.json",
        );
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let errors = super::plan_sdk_with_options(
            contract.clone(),
            &selected,
            config(),
            super::protocol::ProtocolOptions {
                credential_env: Some(crate::credential_env::CredentialEnv::v1(
                    [("apiKey".into(), "OPENROUTER_API_KEY".into())]
                        .into_iter()
                        .collect(),
                )),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|e| e.code == "sdk-credential-env-kind"
                && e.source.pointer() == "/components/securitySchemes/apiKey"
                && e.at.end > e.at.start
                && contract.source_span(&e.source) == Some(e.at.clone())),
            "{errors:?}"
        );
    }
}
