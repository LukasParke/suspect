#![cfg(feature = "java-sdk")]
//! Focused credential-environment policy checks; previous native matrices are reused.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::credential_env::{CredentialEnv, CredentialEnvKind};
use suspect_codegen::java_sdk::{self, MavenConfig, PackageConfig, ProtocolConfig, SdkPlan};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const FIXTURE_URI: &str = "https://fixtures.example/java/credential-env-v1.json";
fn root() -> PathBuf {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-java-credential-env/java");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep()
}
fn fixture() -> Value {
    let mut paths = serde_json::Map::new();
    for (path, name, security) in [
        ("/bearer", "bearerValue", json!([{"bearer":[]} ])),
        ("/alias", "aliasValue", json!([{"alias":[]} ])),
        (
            "/keys",
            "keys",
            json!([{"header":[],"query":[],"cookie":[]} ]),
        ),
        ("/or", "either", json!([{"bearer":[]},{"header":[]} ])),
        ("/and", "both", json!([{"bearer":[],"header":[]} ])),
        ("/optional", "optionalAuth", json!([{"bearer":[]},{} ])),
        ("/public", "publicValue", json!([])),
        ("/from-env", "fromEnv", json!([])),
        ("/basic", "basicValue", json!([{"basic":[]} ])),
        ("/oauth", "oauthValue", json!([{"oauth":["read"]} ])),
    ] {
        paths.insert(path.into(),json!({"get":{"operationId":name,"security":security,"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"boolean"},"example":true}}}}}}));
    }
    json!({"openapi":"3.2.0","info":{"title":"Source-bound Java environment policy","version":"1"},"servers":[{"url":"https://sdk-env.example.test/v1"}],"paths":paths,"components":{"securitySchemes":{
        "bearer":{"type":"http","scheme":"bearer"},"alias":{"type":"http","scheme":"bearer"},
        "header":{"type":"apiKey","in":"header","name":"X-Key"},"query":{"type":"apiKey","in":"query","name":"key"},"cookie":{"type":"apiKey","in":"cookie","name":"session"},
        "basic":{"type":"http","scheme":"basic"},"oauth":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.example.test/token","scopes":{"read":"Read"}}}}
    }}})
}
fn contract_at(value: &Value, uri: &str) -> Arc<Contract> {
    let uri = Uri::parse(uri).unwrap();
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
    contract.operations().map(|o| o.source().clone()).collect()
}
fn package() -> PackageConfig {
    PackageConfig {
        package: "example.credentialenv".into(),
        version: "1.0.0".into(),
        api_name: "Client".into(),
    }
}
fn maven() -> MavenConfig {
    MavenConfig {
        group_id: Some("example.credentialenv".into()),
        artifact_id: "java-credential-env".into(),
        ..Default::default()
    }
}
fn plan(config: ProtocolConfig) -> SdkPlan {
    let contract = contract_at(&fixture(), FIXTURE_URI);
    java_sdk::plan_sdk_with_protocol_v3(
        contract.clone(),
        &selected(&contract),
        package(),
        &[],
        maven(),
        config,
    )
    .unwrap()
}
fn policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("bearer", "CRED_ENV_BEARER"),
            ("alias", "CRED_ENV_BEARER"),
            ("header", "CRED_ENV_HEADER"),
            ("query", "CRED_ENV_QUERY"),
            ("cookie", "CRED_ENV_COOKIE"),
        ]
        .into_iter()
        .map(|(name, variable)| (name.into(), variable.into()))
        .collect(),
    )
}
fn configured() -> SdkPlan {
    plan(ProtocolConfig {
        credential_env: Some(policy()),
        ..Default::default()
    })
}

#[test]
fn no_policy_emission_is_byte_stable() {
    let plan = plan(ProtocolConfig::default());
    let files = plan.render().unwrap();
    assert!(plan.credential_env().is_none());
    let hashes = files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                format!("{:x}", Sha256::digest(f.content.as_bytes())),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&hashes).unwrap()));
    assert_eq!(
        hash,
        "e5e06b4f0e74a5a743c93ee74b4b3ed85a6699395c4cb4d47c233a818dec3eeb"
    );
    assert!(
        !files
            .iter()
            .any(|f| f.content.contains("public static Client fromEnv("))
    );
    let path = root();
    std::fs::write(
        path.join("no-policy-hashes.json"),
        serde_json::to_vec_pretty(&hashes).unwrap(),
    )
    .unwrap();
    println!(
        "JAVA_CREDENTIAL_ENV_BASELINE {} {} {}",
        files.len(),
        hash,
        path.display()
    );
}

#[test]
fn bound_variable_names_and_helpers_retain_source_identity_without_values() {
    let plan = configured();
    let env = plan.credential_env().unwrap();
    assert_eq!(env.bindings().len(), 5);
    let bearer = env
        .bindings()
        .iter()
        .find(|b| b.name() == "bearer")
        .unwrap();
    assert_eq!(bearer.variable(), "CRED_ENV_BEARER");
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/bearer"
    );
    assert!(
        !serde_json::to_string(&env.semantic_descriptor())
            .unwrap()
            .contains(FIXTURE_URI)
    );
    assert!(serde_json::to_string(env).unwrap().contains(FIXTURE_URI));
    assert_eq!(
        plan.operations()
            .iter()
            .find(|o| o.operation_id == "fromEnv")
            .unwrap()
            .method_name,
        "fromEnv2"
    );
    let files = plan.render().unwrap();
    let client = &files
        .iter()
        .find(|f| f.path.ends_with("/Client.java"))
        .unwrap()
        .content;
    assert!(
        client.contains("public static Client fromEnv()")
            && client.contains("public Client(HttpRuntime.Options options)")
    );
    assert!(client.contains("_readCredentialEnv(environment,snapshot,\"CRED_ENV_BEARER\")"));
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("/credential-env.json"))
    );
    for canary in [
        "GENERATOR-ENV-VALUE-MUST-NOT-APPEAR",
        "env-bearer-secret",
        "env-header-secret",
    ] {
        assert!(
            files.iter().all(|f| !f.content.contains(canary)),
            "generator credential canary entered artifacts"
        );
    }
    let mut source = fixture();
    source["components"]["securitySchemes"]["definition"] =
        source["components"]["securitySchemes"]["bearer"].clone();
    source["components"]["securitySchemes"]["bearer"] =
        json!({"$ref":"#/components/securitySchemes/definition"});
    let contract = contract_at(&source, FIXTURE_URI);
    let bound = java_sdk::plan_sdk_with_protocol_v3(
        contract.clone(),
        &selected(&contract),
        package(),
        &[],
        maven(),
        ProtocolConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    )
    .unwrap();
    let bearer = bound
        .credential_env()
        .unwrap()
        .bindings()
        .iter()
        .find(|b| b.name() == "bearer")
        .unwrap();
    assert_eq!(
        bearer.scheme().terminal().source().pointer(),
        "/components/securitySchemes/definition"
    );
    assert_eq!(
        bound.credential_env().unwrap().semantic_descriptor(),
        env.semantic_descriptor()
    );
}

#[test]
fn native_policy_binding_keeps_shared_refusals_after_protocol_admission() {
    let contract = contract_at(&fixture(), FIXTURE_URI);
    for name in ["basic", "oauth", "unused"] {
        let policy = CredentialEnv::v1([(name.into(), "SDK_ENV".into())].into_iter().collect());
        let errors = java_sdk::plan_sdk_with_protocol_v3(
            contract.clone(),
            &selected(&contract),
            package(),
            &[],
            maven(),
            ProtocolConfig {
                credential_env: Some(policy),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(errors.iter().any(|e| e.code
            == if name == "unused" {
                "sdk-credential-env-unbound"
            } else {
                "sdk-credential-env-kind"
            }));
    }
    let mut value = fixture();
    value["paths"]["/blocked"] = json!({"additionalOperations":{"CONNECT":{"operationId":"tunnel","responses":{"200":{"description":"tunnel"}}}}});
    let contract = contract_at(&value, FIXTURE_URI);
    let errors = java_sdk::plan_sdk_with_protocol_v3(
        contract.clone(),
        &selected(&contract),
        package(),
        &[],
        maven(),
        ProtocolConfig {
            credential_env: Some(CredentialEnv::v1(
                [("unused".into(), "SDK_ENV".into())].into_iter().collect(),
            )),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "http-connect-tunnel-unsupported"
                || e.code == "java-connect-tunnel-unsupported")
    );
    assert!(
        errors
            .iter()
            .all(|e| !e.code.starts_with("sdk-credential-env-")),
        "environment binder ran before protocol admission"
    );
}

#[test]
fn canonical_capture_retains_semantic_policy_and_actual_factory_signatures() {
    use suspect_codegen::{
        backend::{Backend, GenerationOptions, TargetConfig},
        compatibility,
    };
    let contract = contract_at(&fixture(), FIXTURE_URI);
    let targets = [TargetConfig {
        backend: Backend::JavaHttp,
        package_name: "example.credentialenv:java-credential-env".into(),
        package_version: "1.0.0".into(),
        import_name: Some("example.credentialenv".into()),
    }];
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let before =
        compatibility::snapshot_with_options(contract.clone(), &[], &targets, &options).unwrap();
    let native = &before.native[0];
    assert_eq!(
        native.status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        native.findings
    );
    assert_eq!(
        native.credential_env.as_ref(),
        configured()
            .credential_env()
            .map(|v| v.semantic_descriptor())
            .as_ref()
    );
    let operation = native
        .operations
        .iter()
        .find(|o| o.operation_id == "bearerValue")
        .unwrap();
    assert_eq!(
        operation.symbols["env-factory"],
        "example.credentialenv.Client.fromEnv"
    );
    let factories = operation.descriptor["constructor"]["client"]["environmentFactories"]
        .as_array()
        .unwrap();
    assert_eq!(factories.len(), 3);
    assert_eq!(factories[0]["parameters"], json!([]));
    assert_eq!(
        factories[1]["parameters"][0]["type"]["name"],
        "java.net.http.HttpClient"
    );
    assert_eq!(factories[2]["parameters"][1]["type"]["kind"], "nullable");
    assert_eq!(
        factories[2]["parameters"][1]["type"]["type"]["name"],
        "java.util.function.Function"
    );
    let relocated = compatibility::snapshot_with_options(
        contract_at(&fixture(), "https://relocated.example/credential-env.json"),
        &[],
        &targets,
        &options,
    )
    .unwrap();
    assert_eq!(
        relocated.native[0].status,
        compatibility::PlanStatus::Planned
    );
    assert_eq!(native.credential_env, relocated.native[0].credential_env);
    let mut changed = options.clone();
    changed
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("bearer".into(), "CRED_ENV_BEARER_NEXT".into());
    let comparison = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&targets, &options),
        (&targets, &changed),
    )
    .unwrap();
    assert!(
        comparison.wire.is_empty(),
        "credential defaults changed wire declarations"
    );
    assert!(
        comparison.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-credential-env-changed")
    );
    assert!(
        !comparison.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-interpretation-profile-changed"
                || c.code == "native-plan-unknown")
    );
    let root = root();
    std::fs::write(
        root.join("capture.json"),
        serde_json::to_vec_pretty(native).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("policy-change.json"),
        serde_json::to_vec_pretty(&comparison).unwrap(),
    )
    .unwrap();
    println!("JAVA_CREDENTIAL_ENV_CAPTURE {}", root.display());
}

fn java() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .expect("select JDK21/25 with JAVA_HOME")
}
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-java-maven-cache/java/repository")
}
fn checked(command: &mut Command, root: &Path) {
    let result = command.output().unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let n = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{n:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn install(plan: &SdkPlan, root: &Path) -> PathBuf {
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!("-Dmaven.repo.local={}", repository().display()))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        root,
    );
    let artifact = &plan.maven().artifact_id;
    let version = &plan.package().version;
    let jar = repository()
        .join(plan.maven_group_id().replace('.', "/"))
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    assert_eq!(
        std::fs::read(&jar).unwrap(),
        std::fs::read(root.join(format!("java/target/{artifact}-{version}.jar"))).unwrap()
    );
    jar
}
fn runtime(command: &mut Command) {
    for name in [
        "CRED_ENV_BEARER",
        "CRED_ENV_HEADER",
        "CRED_ENV_QUERY",
        "CRED_ENV_COOKIE",
        "OPENROUTER_API_KEY",
    ] {
        command.env_remove(name);
    }
}

#[test]
#[ignore = "fresh credential-env controls on JDK21/25; installed package and source-default HTTPS interception"]
fn native_credential_env_controls() {
    let root = root();
    let plan = configured();
    let jar = install(&plan, &root);
    std::fs::write(
        root.join("NativeSupport.java"),
        include_str!("../src/java_sdk/NativeSupport.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("NativeCredentialEnv.java"),
        include_str!("../src/java_sdk/NativeCredentialEnv.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .args(["NativeSupport.java", "NativeCredentialEnv.java"])
            .current_dir(&root),
        &root,
    );
    let filter = std::env::var("SUSPECT_JAVA_CREDENTIAL_ENV_MODE").ok();
    assert!(
        filter.as_ref().is_none_or(|value| matches!(
            value.as_str(),
            "positive" | "missing" | "empty" | "invalid" | "alternative" | "snapshot" | "accessor"
        )),
        "unknown credential-env native mode"
    );
    for mode in [
        "positive",
        "missing",
        "empty",
        "invalid",
        "alternative",
        "snapshot",
        "accessor",
    ] {
        if filter.as_ref().is_some_and(|filter| filter != mode) {
            continue;
        }
        let mut command = Command::new(java().join("bin/java"));
        command
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .args(["NativeCredentialEnv", mode]);
        runtime(&mut command);
        match mode {
            "positive" | "accessor" => {
                command
                    .env("CRED_ENV_BEARER", "env-bearer-secret")
                    .env("CRED_ENV_HEADER", "env-header-secret")
                    .env("CRED_ENV_QUERY", "query +/雪")
                    .env("CRED_ENV_COOKIE", "cookie-secret");
            }
            "empty" => {
                for name in [
                    "CRED_ENV_BEARER",
                    "CRED_ENV_HEADER",
                    "CRED_ENV_QUERY",
                    "CRED_ENV_COOKIE",
                ] {
                    command.env(name, "");
                }
            }
            "invalid" => {
                command
                    .env("CRED_ENV_BEARER", "Bearer invalid")
                    .env("CRED_ENV_HEADER", "bad\nheader")
                    .env("CRED_ENV_COOKIE", "bad;cookie");
            }
            "alternative" => {
                command.env("CRED_ENV_HEADER", "env-header-secret");
            }
            _ => {}
        }
        checked(&mut command, &root);
    }
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("example.credentialenv.SdkExamples"),
        &root,
    );
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg("-d")
            .arg(&root)
            .arg(root.join("java/examples/GettingStarted.java")),
        &root,
    );
    println!("JAVA_CREDENTIAL_ENV_CONTROLS {}", root.display());
}

#[test]
#[ignore = "actual getCurrentKey/getCredits source schemas and branded fromEnv helper on JDK21/25"]
fn native_credential_env_openrouter() {
    let root = root();
    let source = std::env::var_os("SUSPECT_OPENROUTER_OPENAPI")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/github/openrouter-web/projects/docs/openapi/openapi.yaml".into()
        });
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let operations = contract
        .operations()
        .filter(|o| {
            o.operation_id()
                .is_some_and(|name| matches!(name, "getCurrentKey" | "getCredits"))
        })
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), 2);
    let policy = CredentialEnv::v1(
        [("apiKey".into(), "OPENROUTER_API_KEY".into())]
            .into_iter()
            .collect(),
    );
    let plan = java_sdk::plan_sdk_with_protocol_v3(
        contract,
        &operations,
        PackageConfig {
            package: "ai.openrouter.sdk".into(),
            version: "0.1.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            group_id: Some("ai.openrouter".into()),
            artifact_id: "openrouter-sdk".into(),
            ..Default::default()
        },
        ProtocolConfig {
            credential_env: Some(policy),
            ..Default::default()
        },
    )
    .unwrap();
    let files = plan.render().unwrap();
    assert!(files.iter().all(
        |f| !f.content.contains("GENERATOR-ENV-VALUE-MUST-NOT-APPEAR")
            && !f.content.contains("controlled-openrouter-token")
    ));
    std::fs::write(
        root.join("source-sha256.txt"),
        format!(
            "{:x}  {}\n",
            Sha256::digest(std::fs::read(&source).unwrap()),
            source.display()
        ),
    )
    .unwrap();
    let jar = install(&plan, &root);
    std::fs::write(
        root.join("NativeSupport.java"),
        include_str!("../src/java_sdk/NativeSupport.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("NativeCredentialEnvOpenRouter.java"),
        include_str!("../src/java_sdk/NativeCredentialEnvOpenRouter.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("GetCurrentKeyFromEnv.java"),
        include_str!("../src/java_sdk/NativeCredentialEnvExample.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .args([
                "NativeSupport.java",
                "NativeCredentialEnvOpenRouter.java",
                "GetCurrentKeyFromEnv.java",
            ])
            .current_dir(&root),
        &root,
    );
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg("-d")
            .arg(&root)
            .arg(root.join("java/examples/GettingStarted.java")),
        &root,
    );
    let mut command = Command::new(java().join("bin/java"));
    command
        .args(["-ea", "-cp"])
        .arg(format!("{}:{}", jar.display(), root.display()))
        .arg("NativeCredentialEnvOpenRouter");
    runtime(&mut command);
    command.env("OPENROUTER_API_KEY", "controlled-openrouter-token");
    checked(&mut command, &root);
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("ai.openrouter.sdk.SdkExamples"),
        &root,
    );
    std::fs::write(
        root.join("installed-jar-sha256.txt"),
        format!(
            "{:x}  {}\n",
            Sha256::digest(std::fs::read(&jar).unwrap()),
            jar.display()
        ),
    )
    .unwrap();
    println!("JAVA_CREDENTIAL_ENV_OPENROUTER {}", root.display());
}
