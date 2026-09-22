//! Bounded runtime environment-policy tests for the installed C++ SDK.
#![cfg(feature = "cpp-sdk")]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    cpp_sdk::{SdkConfig, SdkPlan, plan_sdk},
    credential_env::CredentialEnv,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn root(label: &str) -> PathBuf {
    let base = repo().join("target/sdk-cpp-credential-env-gates");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
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
fn openrouter() -> Arc<Contract> {
    let base = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    load(&base.join("projects/docs/openapi/openapi.yaml"))
}
fn config() -> SdkConfig {
    SdkConfig {
        name: "openrouter".into(),
        namespace: "openrouter".into(),
        ..Default::default()
    }
}
fn selected_plan(contract: Arc<Contract>, names: &[&str], config: SdkConfig) -> SdkPlan {
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| names.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), names.len());
    plan_sdk(contract, &selected, config).unwrap_or_else(|e| panic!("{e:#?}"))
}
fn policy(pairs: &[(&str, &str)]) -> CredentialEnv {
    CredentialEnv::v1(
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<BTreeMap<_, _>>(),
    )
}
fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    use std::io::Write;
    let output = command.output().unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        log,
        "{command:?}\n{}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn tool(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.into())
}

fn cpp_string(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::from("std::string(\"");
    for byte in value.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            32..=126 => out.push(byte as char),
            _ => write!(out, "\\{byte:03o}").unwrap(),
        }
    }
    write!(out, "\", {})", value.len()).unwrap();
    out
}
fn package(plan: &SdkPlan, root: &Path) {
    suspect_codegen::write_files(&plan.render().unwrap(), &root.join("generated")).unwrap();
    let cmake = tool("SUSPECT_CPP_CMAKE", "cmake");
    checked(
        Command::new(&cmake)
            .arg("-S")
            .arg(root.join("generated/cpp"))
            .arg("-B")
            .arg(root.join("build"))
            .arg(format!(
                "-DCMAKE_CXX_COMPILER={}",
                tool("SUSPECT_CPP_CXX", "clang++").display()
            ))
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                root.join("install").display()
            ))
            .arg("-DSUSPECT_SDK_BUILD_DOCS=ON")
            .arg(format!(
                "-DDOXYGEN_EXECUTABLE={}",
                tool("SUSPECT_CPP_DOXYGEN", "doxygen").display()
            )),
        root,
    );
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.join("build"))
            .args(["--parallel", "2"]),
        root,
    );
    checked(
        Command::new(cmake.with_file_name("ctest"))
            .arg("--test-dir")
            .arg(root.join("build"))
            .arg("--output-on-failure"),
        root,
    );
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.join("build"))
            .args(["--target", "sdk_docs"]),
        root,
    );
    checked(
        Command::new(&cmake)
            .arg("--install")
            .arg(root.join("build")),
        root,
    );
}
fn consumer(root: &Path, package: &str, text: &str) -> PathBuf {
    let dir = root.join("consumer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.cpp"), text).unwrap();
    std::fs::write(dir.join("CMakeLists.txt"),format!("cmake_minimum_required(VERSION 3.24)\nproject(CredentialEnvConsumer LANGUAGES CXX)\nfind_package({package} CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE {package}::{package})\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n")).unwrap();
    let cmake = tool("SUSPECT_CPP_CMAKE", "cmake");
    checked(
        Command::new(&cmake)
            .arg("-S")
            .arg(&dir)
            .arg("-B")
            .arg(root.join("build-consumer"))
            .arg(format!(
                "-DCMAKE_PREFIX_PATH={}",
                root.join("install").display()
            ))
            .arg(format!(
                "-DCMAKE_CXX_COMPILER={}",
                tool("SUSPECT_CPP_CXX", "clang++").display()
            )),
        root,
    );
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.join("build-consumer")),
        root,
    );
    root.join("build-consumer/consumer")
}

#[test]
#[ignore = "actual OpenRouter source, installed branded C++ env factory, controlled source-default HTTPS requests"]
fn native_openrouter_credential_env_snapshot_and_explicit_precedence() {
    let contract = openrouter();
    let config = SdkConfig {
        credential_env: Some(policy(&[("apiKey", "OPENROUTER_API_KEY")])),
        ..config()
    };
    let plan = selected_plan(contract.clone(), &["getCurrentKey", "getCredits"], config);
    let root = root("openrouter-");
    let environment = plan.credential_env().unwrap();
    assert_eq!(environment.bindings().len(), 1);
    assert_eq!(environment.bindings()[0].name(), "apiKey");
    assert_eq!(
        environment.bindings()[0].kind(),
        suspect_codegen::credential_env::CredentialEnvKind::Bearer
    );
    let files = plan.render().unwrap();
    let canary = std::env::var("OPENROUTER_API_KEY").ok();
    if let Some(canary) = canary.filter(|s| s.starts_with("cpp-generator-process-canary")) {
        assert!(files.iter().all(|f| !f.content.contains(&canary)));
    }
    package(&plan, &root);
    let document = contract.document(contract.entry()).unwrap();
    assert_eq!(
        document["servers"][0]["url"],
        "https://openrouter.ai/api/v1"
    );
    let key = &document["paths"]["/key"]["get"]["responses"]["200"]["content"]["application/json"]
        ["example"];
    let credits = &document["paths"]["/credits"]["get"]["responses"]["200"]["content"]["application/json"]
        ["example"];
    assert!(key.is_object() && credits.is_object());
    let source = include_str!("../src/cpp_sdk/tests/native_credential_env_openrouter.cpp")
        .replace("__KEY_RESPONSE__", &cpp_string(&key.to_string()))
        .replace("__CREDITS_RESPONSE__", &cpp_string(&credits.to_string()));
    let executable = consumer(&root, "openrouter", &source);
    checked(&mut Command::new(executable), &root);
    println!(
        "C++ configured branded OpenRouter env/default-HTTPS native receipt: {}",
        root.display()
    );
}

fn security_document() -> Value {
    let success = json!({"204":{"description":"No content"}});
    json!({"openapi":"3.2.0","info":{"title":"C++ environment controls","version":"1"},"servers":[{"url":"https://env-fixture.invalid/api"}],"components":{"securitySchemes":{
        "bearer":{"type":"http","scheme":"bearer"},"headerKey":{"type":"apiKey","in":"header","name":"X-Key"},"queryKey":{"type":"apiKey","in":"query","name":"api_key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
        "aliasOne":{"$ref":"#/components/securitySchemes/bearer"},"aliasTwo":{"$ref":"#/components/securitySchemes/bearer"}
    }},"paths":{
        "/public":{"get":{"operationId":"publicCall","security":[],"responses":success}},
        "/protected":{"get":{"operationId":"protectedCall","security":[{"bearer":[]}],"responses":success}},
        "/either":{"get":{"operationId":"either","security":[{"bearer":[]},{"headerKey":[]}],"responses":success}},
        "/together":{"get":{"operationId":"together","security":[{"bearer":[],"headerKey":[],"queryKey":[],"cookieKey":[]}],"responses":success}},
        "/optional":{"get":{"operationId":"optional","security":[{},{"bearer":[]}],"responses":success}},
        "/aliases":{"get":{"operationId":"aliases","security":[{"aliasOne":[]},{"aliasTwo":[]}],"responses":success}},
        "/factory-name":{"get":{"operationId":"fromEnv","security":[],"responses":success}},
        "/transport-factory-name":{"get":{"operationId":"fromEnvWithTransport","security":[],"responses":success}}
    }})
}
fn synthetic_plan(root: &Path, configured: bool) -> SdkPlan {
    let path = root.join("source.json");
    std::fs::write(&path, security_document().to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(
        contract,
        &selected,
        SdkConfig {
            name: "env_sdk".into(),
            namespace: "env_sdk".into(),
            credential_env: configured.then(|| {
                policy(&[
                    ("bearer", "CPP_ENV_BEARER"),
                    ("headerKey", "CPP_ENV_HEADER"),
                    ("queryKey", "CPP_ENV_QUERY"),
                    ("cookieKey", "CPP_ENV_COOKIE"),
                    ("aliasOne", "CPP_ENV_ALIAS_ONE"),
                    ("aliasTwo", "CPP_ENV_ALIAS_TWO"),
                ])
            }),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn credential_env_binds_native_declarations_and_reserves_only_configured_helpers() {
    let root = root("bindings-");
    let plain = synthetic_plan(&root, false);
    let configured = synthetic_plan(&root, true);
    assert!(plain.credential_env().is_none());
    assert!(
        plain
            .operations()
            .iter()
            .any(|o| o.method_name == "from_env")
    );
    assert!(
        plain
            .operations()
            .iter()
            .any(|o| o.method_name == "from_env_with_transport")
    );
    let bindings = configured.credential_env().unwrap();
    assert_eq!(bindings.bindings().len(), 6);
    assert!(!configured.operations().iter().any(|o| matches!(
        o.method_name.as_str(),
        "from_env" | "from_env_with_transport"
    )));
    let aliases = bindings
        .bindings()
        .iter()
        .filter(|b| b.name().starts_with("alias"))
        .collect::<Vec<_>>();
    assert_ne!(
        aliases[0].scheme().use_site().source(),
        aliases[1].scheme().use_site().source()
    );
    assert_eq!(
        aliases[0].scheme().terminal().source(),
        aliases[1].scheme().terminal().source()
    );
    for binding in bindings.bindings() {
        assert!(
            configured
                .credentials()
                .contains_key(binding.scheme().use_site().source())
        );
    }
    let plain_files = plain.render().unwrap();
    assert!(
        !plain_files.iter().any(|f| f.content.contains("std::getenv")
            || f.content.contains("CPP_ENV_BEARER")
            || f.path.ends_with("credential-env.json"))
    );
    let files = configured.render().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f.path == "cpp/docs/credential-env.json")
    );
    assert!(
        files
            .iter()
            .any(|f| f.path == "cpp/src/client.cpp" && f.content.contains("std::getenv"))
    );
    let descriptor = serde_json::to_value(bindings.semantic_descriptor()).unwrap();
    assert_eq!(descriptor["version"], "v1");
    assert!(!descriptor.to_string().contains("document"));
    assert!(!descriptor.to_string().contains("pointer"));
}

#[test]
#[ignore = "actual OpenRouter source generation/no-policy/canary receipt; no native or account calls"]
fn actual_openrouter_credential_env_generation_contract() {
    let contract = openrouter();
    let names = ["getCurrentKey", "getCredits"];
    let plain = selected_plan(contract.clone(), &names, config());
    assert!(plain.credential_env().is_none());
    let plain_files = plain.render().unwrap();
    let configured = selected_plan(
        contract.clone(),
        &names,
        SdkConfig {
            credential_env: Some(policy(&[("apiKey", "OPENROUTER_API_KEY")])),
            ..config()
        },
    );
    let files = configured.render().unwrap();
    let reverted = selected_plan(
        contract,
        &names,
        SdkConfig {
            credential_env: None,
            ..config()
        },
    );
    assert_eq!(plain_files, reverted.render().unwrap());
    assert!(
        plain_files
            .iter()
            .all(|f| !f.content.contains("std::getenv")
                && !f.content.contains("OPENROUTER_API_KEY")
                && !f.path.ends_with("credential-env.json"))
    );
    if let Ok(canary) = std::env::var("OPENROUTER_API_KEY")
        && canary.starts_with("cpp-generator-process-canary")
    {
        assert!(
            files
                .iter()
                .chain(&plain_files)
                .all(|f| !f.content.contains(&canary))
        );
    }
    let root = root("generation-");
    std::fs::write(
        root.join("no-policy-after.json"),
        serde_json::to_vec_pretty(
            &plain_files
                .iter()
                .map(|f| json!({"path":f.path,"content":f.content}))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "cpp/sdk-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        manifest["credential_env"]["bindings"][0]["variable"],
        "OPENROUTER_API_KEY"
    );
    assert!(!manifest["credential_env"].to_string().contains("document"));
    std::fs::write(
        root.join("policy-descriptor.json"),
        serde_json::to_vec_pretty(&configured.credential_env().unwrap().semantic_descriptor())
            .unwrap(),
    )
    .unwrap();
    println!(
        "C++ actual OpenRouter no-policy generation receipt: {}",
        root.display()
    );
}

#[test]
#[ignore = "C++ env OR/AND/anonymous/API-key/alias and core-only construction controls"]
fn native_credential_env_security_and_portable_controls() {
    let root = root("security-");
    let plan = synthetic_plan(&root, true);
    package(&plan, &root);
    let executable = consumer(
        &root,
        "env_sdk",
        include_str!("../src/cpp_sdk/tests/native_credential_env_security.cpp"),
    );
    checked(&mut Command::new(executable), &root);
    let cmake = tool("SUSPECT_CPP_CMAKE", "cmake");
    let core = root.join("core");
    std::fs::create_dir(&core).unwrap();
    checked(
        Command::new(&cmake)
            .arg("-S")
            .arg(root.join("generated/cpp"))
            .arg("-B")
            .arg(core.join("build"))
            .arg("-DSUSPECT_SDK_WITH_CURL=OFF")
            .arg("-DSUSPECT_SDK_BUILD_DOCS=OFF")
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(format!(
                "-DCMAKE_CXX_COMPILER={}",
                tool("SUSPECT_CPP_CXX", "clang++").display()
            ))
            .arg(format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                core.join("install").display()
            )),
        &root,
    );
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(core.join("build"))
            .args(["--parallel", "2"]),
        &root,
    );
    checked(
        Command::new(&cmake)
            .arg("--install")
            .arg(core.join("build")),
        &root,
    );
    let executable = consumer(
        &core,
        "env_sdk",
        include_str!("../src/cpp_sdk/tests/native_credential_env_security.cpp"),
    );
    checked(&mut Command::new(executable), &root);
    println!("C++ env security and core-only receipt: {}", root.display());
}
