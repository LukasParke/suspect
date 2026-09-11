#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
//! Runtime environment defaults over the admitted native protocol seam.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, PlanStatus},
    credential_env::CredentialEnv,
    generation_session::{Session, SessionConfig},
    kotlin_sdk::{self, Plan, SdkConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;
#[path = "kotlin_support/mod.rs"]
#[allow(dead_code)]
mod support;

fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-credential-env");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gate-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn controls_contract() -> Arc<Contract> {
    let uri =
        Uri::parse("https://fixtures.suspect.test/kotlin-credential-env/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            include_bytes!("kotlin_support/credential-env.openapi.json").to_vec(),
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
fn policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("apiKey".into(), "KOTLIN_ENV_A".into()),
            ("headerKey".into(), "KOTLIN_ENV_B".into()),
            ("queryKey".into(), "KOTLIN_ENV_QUERY".into()),
            ("cookieKey".into(), "KOTLIN_ENV_COOKIE".into()),
            ("aliasA".into(), "KOTLIN_ENV_A".into()),
            ("aliasB".into(), "KOTLIN_ENV_ALIAS_B".into()),
        ]
        .into(),
    )
}
fn controls_config(env: Option<CredentialEnv>) -> SdkConfig {
    SdkConfig {
        group_id: "test.suspect.kotlin".into(),
        artifact_id: "credential-env-controls".into(),
        version: "0.1.0".into(),
        package_name: "example.credentialenv".into(),
        credential_env: env,
    }
}
fn control_plan(env: Option<CredentialEnv>) -> Plan {
    let contract = controls_contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk(contract, &selected, controls_config(env)).unwrap()
}

#[test]
fn binds_declared_schemes_and_preserves_alias_credentials() {
    let plan = control_plan(Some(policy()));
    let env = plan.credential_env().unwrap();
    assert_eq!(env.bindings().len(), 6);
    let a = env
        .bindings()
        .iter()
        .find(|b| b.name() == "aliasA")
        .unwrap();
    let b = env
        .bindings()
        .iter()
        .find(|b| b.name() == "aliasB")
        .unwrap();
    assert_eq!(a.scheme().terminal(), b.scheme().terminal());
    assert_ne!(a.scheme().use_site(), b.scheme().use_site());
    let plain = control_plan(None);
    let fields = |plan: &Plan| {
        plan.credentials()
            .values()
            .map(|c| (c.source.clone(), c.name.clone(), c.kotlin_type.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        fields(&plan),
        fields(&plain),
        "the existing explicit credential constructor must retain its fields and aliases"
    );
    let semantic = serde_json::to_value(env.semantic_descriptor()).unwrap();
    assert!(
        semantic["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b.get("scheme").is_none())
    );
    assert!(
        plan.operations()
            .iter()
            .any(|op| op.operation_id == "fromEnv" && op.method_name == "fromEnv2")
    );
    assert!(
        plan.models()
            .symbols()
            .iter()
            .any(|s| s.name == "CredentialEnvironment2")
    );
    let files = plan.render().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("/CredentialEnvironment.kt"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.path == "kotlin/docs/credential-env.json")
    );
}

#[test]
fn no_policy_emits_no_environment_helpers_or_branches() {
    let plan = control_plan(None);
    assert!(plan.credential_env().is_none());
    let files = plan.render().unwrap();
    assert!(
        !files
            .iter()
            .any(|f| f.path.ends_with("/CredentialEnvironment.kt")
                || f.path == "kotlin/docs/credential-env.json")
    );
    let client = &files
        .iter()
        .find(|f| f.path.ends_with("/Client.kt"))
        .unwrap()
        .content;
    assert!(client.contains("private val credentials: Credentials = Credentials()"));
    assert!(!client.contains("EnvironmentCredentials"));
    assert!(
        !files
            .iter()
            .any(|f| f.content.contains("__CREDENTIAL_IDENTITY__"))
    );
}

#[test]
fn unsupported_environment_kinds_and_unbound_names_are_source_findings() {
    let contract = controls_contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    for (name, code) in [
        ("basic", "sdk-credential-env-kind"),
        ("oauth", "sdk-credential-env-kind"),
        ("unselected", "sdk-credential-env-unbound"),
    ] {
        let config = controls_config(Some(CredentialEnv::v1(
            [(name.into(), "KOTLIN_ENV_X".into())].into(),
        )));
        let errors = kotlin_sdk::plan_sdk(contract.clone(), &selected, config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == code && e.at.end > e.at.start),
            "{errors:?}"
        );
    }
}

#[test]
fn generator_environment_canary_is_not_emitted() {
    const CANARY: &str = "generation-only-value-must-never-enter-sdk-artifacts";
    if std::env::var_os("KOTLIN_ENV_CANARY_CHILD").is_some() {
        assert_eq!(std::env::var("KOTLIN_ENV_A").unwrap(), CANARY);
        let plan = control_plan(Some(policy()));
        assert!(
            plan.render()
                .unwrap()
                .iter()
                .all(|f| !f.content.contains(CANARY))
        );
        return;
    }
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "generator_environment_canary_is_not_emitted",
            "--nocapture",
        ])
        .env("KOTLIN_ENV_CANARY_CHILD", "1")
        .env("KOTLIN_ENV_A", CANARY)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::KotlinHttp,
        package_name: "test.suspect.kotlin:credential-env-controls".into(),
        package_version: "0.1.0".into(),
        import_name: Some("example.credentialenv".into()),
    }
}

#[test]
fn canonical_capture_uses_semantic_env_descriptor_and_actual_native_overloads() {
    let contract = controls_contract();
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let snapshot =
        compatibility::snapshot_with_options(contract.clone(), &[], &[target()], &options).unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert_eq!(
        native.credential_env,
        control_plan(Some(policy()))
            .credential_env()
            .map(|plan| plan.semantic_descriptor())
    );
    let semantic = serde_json::to_value(native.credential_env.as_ref().unwrap()).unwrap();
    assert_eq!(semantic["version"], "v1");
    assert!(
        semantic["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("scheme").is_none())
    );
    let operation = native
        .operations
        .iter()
        .find(|op| op.operation_id == "protectedCall")
        .unwrap();
    let client = &operation.descriptor["constructor"]["client"];
    assert_eq!(client["signature"]["parameters"][0]["hasDefault"], false);
    assert_eq!(client["environmentFactory"]["name"], "fromEnv");
    assert_eq!(client["environmentFactory"]["snapshot"], "client-creation");
    assert_eq!(
        client["environmentReader"]["type"],
        "example.credentialenv.CredentialEnvironment"
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        backend::generate_with_options(contract.clone(), &selected, &target(), &options).unwrap(),
        control_plan(Some(policy())).render().unwrap()
    );
    let comparison = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&[target()], &GenerationOptions::default()),
        (&[target()], &options),
    )
    .unwrap();
    assert!(
        comparison.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-credential-env-changed")
    );
    assert!(
        !comparison
            .wire
            .iter()
            .any(|c| c.code == "wire-interpretation-profile-changed")
    );
}

#[test]
fn policy_edit_and_revert_are_session_identity_without_secret_values() {
    let root = root();
    let file = root.join("api.json");
    std::fs::write(
        &file,
        include_str!("kotlin_support/credential-env.openapi.json"),
    )
    .unwrap();
    let mut config = SessionConfig {
        targets: vec![target()],
        ..Default::default()
    };
    let mut session = Session::new(&file, config.clone()).unwrap();
    let plain = session.generate().unwrap();
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    config.generation.credential_env = Some(policy());
    session.set_config(config.clone()).unwrap();
    let enabled = session.generate().unwrap();
    assert_ne!(enabled.revision, plain.revision);
    assert_eq!(enabled.delta.renders, 1);
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    config
        .generation
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "KOTLIN_ENV_EDITED".into());
    session.set_config(config.clone()).unwrap();
    let edited = session.generate().unwrap();
    assert_ne!(edited.revision, enabled.revision);
    config.generation.credential_env = None;
    session.set_config(config).unwrap();
    let restored = session.generate().unwrap();
    assert_eq!(restored.files, plain.files);
    let direct = control_plan(Some(policy()));
    let descriptors: Value =
        serde_json::to_value(direct.credential_env().unwrap().semantic_descriptor()).unwrap();
    assert_eq!(descriptors["bindings"].as_array().unwrap().len(), 6);
}

fn openrouter() -> Plan {
    use sha2::{Digest, Sha256};
    let base = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = base.join("projects/docs/openapi/openapi.yaml");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821"
    );
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
        .filter(|op| op.operation_id() == Some("getCurrentKey"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 1);
    kotlin_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            group_id: "ai.openrouter".into(),
            artifact_id: "openrouter-kotlin".into(),
            version: "0.1.0".into(),
            package_name: "ai.openrouter.kotlin".into(),
            credential_env: Some(CredentialEnv::v1(
                [("apiKey".into(), "OPENROUTER_API_KEY".into())].into(),
            )),
        },
    )
    .unwrap()
}
fn native_package(plan: &Plan, source: &str, modes: &[&str], negatives: &[&str]) -> PathBuf {
    let root = root();
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    let primary = plan
        .credential_env()
        .unwrap()
        .bindings()
        .iter()
        .find(|binding| binding.name() == "apiKey")
        .map(|binding| {
            plan.credentials()[binding.scheme().terminal().source()]
                .name
                .as_str()
        })
        .unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/NativeEnv.kt"),
        source.replace("__PRIMARY_MEMBER__", primary),
    )
    .unwrap();
    let readme = std::fs::read_to_string(root.join("kotlin/README.md")).unwrap();
    let quickstart = readme
        .split("```kotlin\n")
        .skip(1)
        .map(|p| p.split("\n```").next().unwrap())
        .find(|p| p.contains("public object Quickstart"))
        .unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/Quickstart.kt"),
        format!("package consumer\n{quickstart}"),
    )
    .unwrap();
    let cfg = plan.config();
    std::fs::write(consumer.join("pom.xml"),format!(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>credential-env-consumer</artifactId><version>0.1.0</version><properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon><envMode>positive</envMode></properties><dependencies><dependency><groupId>{}</groupId><artifactId>{}</artifactId><version>{}</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin><plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>consumer.NativeEnvKt</argument><argument>${{envMode}}</argument></arguments></configuration></plugin></plugins></build></project>"#,cfg.group_id,cfg.artifact_id,cfg.version,kotlin_sdk::KOTLIN_VERSION)).unwrap();
    for (version, home) in support::java_homes() {
        let output = support::maven(&home)
            .arg("install")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("KOTLIN_ENV_A")
            .current_dir(root.join("kotlin"))
            .output()
            .unwrap();
        check(&root, &format!("sdk-{version}"), output);
        support::native_docs(plan, &root, &version);
        for (index, mode) in modes.iter().enumerate() {
            let mut cmd = support::maven(&home);
            if index == 0 {
                cmd.arg("compile");
            }
            cmd.arg("exec:exec")
                .arg(format!("-DenvMode={mode}"))
                .env("KOTLIN_ENV_A", "system-token")
                .current_dir(&consumer);
            match *mode {
                "missing" => {
                    cmd.env_remove("OPENROUTER_API_KEY");
                }
                "empty" => {
                    cmd.env("OPENROUTER_API_KEY", "");
                }
                _ => {
                    cmd.env("OPENROUTER_API_KEY", "runtime-env-token");
                }
            }
            check(
                &root,
                &format!("consumer-{version}-{mode}"),
                cmd.output().unwrap(),
            );
        }
        for (index, case) in negatives.iter().enumerate() {
            let path = consumer.join("src/main/kotlin/Negative.kt");
            std::fs::write(
                &path,
                format!("package consumer\nimport {}.*\n{case}\n", cfg.package_name),
            )
            .unwrap();
            let output = support::maven(&home)
                .arg("compile")
                .current_dir(&consumer)
                .output()
                .unwrap();
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::write(root.join(format!("negative-{version}-{index}.log")), &log).unwrap();
            assert!(
                !output.status.success()
                    && log.contains("Negative.kt")
                    && !log.contains("Unresolved reference"),
                "uncontrolled negative: {log}"
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    println!("Kotlin credential-env native evidence: {}", root.display());
    root
}
fn check(root: &Path, label: &str, output: std::process::Output) {
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(root.join(format!("{label}.log")), &log).unwrap();
    assert!(output.status.success(), "{}\n{log}", root.display());
}

#[test]
#[ignore = "JDK21/25 installed env precedence/identity/OR/AND/anonymous/snapshot controls"]
fn native_environment_controls() {
    native_package(
        &control_plan(Some(policy())),
        include_str!("../src/kotlin_sdk/native_credential_env.kt"),
        &["controls"],
        &[
            "val bad = Client(credentials = null)",
            "val bad = Client.fromEnv(environment = \"not a reader\")",
        ],
    );
}

#[test]
#[ignore = "JDK21/25 actual source getCurrentKey, default HTTPS, env positive/missing/empty and compiled helper"]
fn native_openrouter_current_key_environment() {
    native_package(
        &openrouter(),
        include_str!("../src/kotlin_sdk/native_openrouter_env.kt"),
        &["positive", "missing", "empty"],
        &["val bad = Client(credentials = null)"],
    );
}
