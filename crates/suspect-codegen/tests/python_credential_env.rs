//! Source-bound, creation-time Python environment defaults over the real SDK.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::credential_env::{CredentialEnv, CredentialEnvKind};
use suspect_codegen::{
    OutFile,
    python_http::{self, HttpConfig, HttpPlan, PackageConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const IMPORT: &str = "credential_python_sdk";
const NO_POLICY_SHA256: &str = "5de4b56a07485f3549c79bb40c98002063a0a3661a62bc7996653e1758ed0426";

fn root(label: &str) -> PathBuf {
    tempfile::Builder::new()
        .prefix(&format!("sdk-python-credential-env-{label}-"))
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

fn fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.example.test/python-credential-env.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            include_bytes!("python_credential_env/api.json").to_vec(),
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

fn package(contract: Arc<Contract>, config: HttpConfig, import: &str) -> (HttpPlan, Vec<OutFile>) {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract, &selected, config).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: import.replace('_', "-"),
            version: "1.0.0".into(),
            import_name: import.into(),
        },
    )
    .unwrap();
    (plan, files)
}

fn fingerprint(files: &[OutFile]) -> String {
    let mut hash = Sha256::new();
    for file in files {
        hash.update((file.path.len() as u64).to_le_bytes());
        hash.update(file.path.as_bytes());
        hash.update((file.content.len() as u64).to_le_bytes());
        hash.update(file.content.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn no_policy_python_output_retains_pre_feature_bytes() {
    let root = root("no-policy");
    let (_, files) = package(fixture(), Default::default(), IMPORT);
    suspect_codegen::write_files(&files, &root).unwrap();
    let hash = fingerprint(&files);
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&json!({"artifacts":files.len(),"sha256":hash,"files":files.iter().map(|file|json!({"path":file.path,"sha256":format!("{:x}",Sha256::digest(file.content.as_bytes()))})).collect::<Vec<Value>>()})).unwrap()).unwrap();
    assert_eq!(hash, NO_POLICY_SHA256, "unconfigured output bytes changed");
    println!("Python no-policy bytes: {hash}; {}", root.display());
}

fn configured() -> HttpConfig {
    HttpConfig {
        credential_env: Some(CredentialEnv::v1(BTreeMap::from([
            ("apiKey".into(), "SUSPECT_PY_BEARER_ENV".into()),
            ("headerKey".into(), "SUSPECT_PY_HEADER_ENV".into()),
            ("queryKey".into(), "SUSPECT_PY_QUERY_ENV".into()),
            ("cookieKey".into(), "SUSPECT_PY_COOKIE_ENV".into()),
        ]))),
        ..Default::default()
    }
}

#[test]
fn python_credential_env_binds_source_schemes_and_emits_only_configured_defaults() {
    let (plan, files) = package(fixture(), configured(), IMPORT);
    let policy = plan.credential_env().unwrap();
    assert_eq!(policy.bindings().len(), 4);
    let bearer = policy
        .bindings()
        .iter()
        .find(|binding| binding.name() == "apiKey")
        .unwrap();
    assert_eq!(bearer.variable(), "SUSPECT_PY_BEARER_ENV");
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(
        bearer.scheme().terminal().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    let semantic = serde_json::to_value(policy.semantic_descriptor()).unwrap();
    assert!(
        semantic["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|binding| binding.get("scheme").is_none())
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == format!("python/src/{IMPORT}/_credential_env.py"))
    );
    assert!(
        plan.operations()
            .iter()
            .any(|operation| operation.snake_name == "credential_env")
    );
    assert!(
        plan.operations()
            .iter()
            .all(|operation| operation.snake_name != "__init__")
    );
    let (_, ordinary) = package(fixture(), Default::default(), IMPORT);
    assert_eq!(fingerprint(&ordinary), NO_POLICY_SHA256);
    assert!(
        !ordinary
            .iter()
            .any(|file| file.path.ends_with("/_credential_env.py"))
    );
    for source in ["basic", "oauth", "oidc", "unselected"] {
        let contract = fixture();
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let config = HttpConfig {
            credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
                source.into(),
                "SUSPECT_PY_TEST_ENV".into(),
            )]))),
            ..Default::default()
        };
        let errors = python_http::plan_http(contract, &selected, config).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code
                == if source == "unselected" {
                    "sdk-credential-env-unbound"
                } else {
                    "sdk-credential-env-kind"
                }),
            "{errors:?}"
        );
        if source != "unselected" {
            assert!(errors.iter().all(|error| !error.at.is_empty()));
        }
    }
}

#[test]
fn python_credential_env_canonical_capture_retains_bound_semantics() {
    use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig};
    let root = root("canonical-capture");
    let contract = fixture();
    let config = configured();
    let options = GenerationOptions {
        credential_env: config.credential_env.clone(),
        ..Default::default()
    };
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract.clone(), &selected, config).unwrap();
    let expected = plan.credential_env().unwrap().semantic_descriptor();
    let snapshot = suspect_codegen::compatibility::snapshot_with_options(
        contract,
        &[],
        &[TargetConfig {
            backend: Backend::PythonHttp,
            package_name: "credential-python-sdk".into(),
            package_version: "1.0.0".into(),
            import_name: Some(IMPORT.into()),
        }],
        &options,
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    assert_eq!(native.operations.len(), selected.len());
    assert_eq!(native.generation.credential_env, options.credential_env);
    assert_eq!(native.credential_env.as_ref(), Some(&expected));
    let descriptor = serde_json::to_value(native.credential_env.as_ref().unwrap()).unwrap();
    assert_eq!(descriptor["version"], "v1");
    assert!(
        descriptor["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|binding| {
                let keys = binding
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                keys == ["kind", "name", "variable"]
            })
    );
    for path in [
        "python_http/credential_env.rs",
        "python_http/credential_env.py",
    ] {
        assert!(
            native
                .runtime
                .fingerprinted_assets
                .iter()
                .any(|asset| asset == path)
        );
    }
    fs::write(
        root.join("capture.json"),
        serde_json::to_vec_pretty(native).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("descriptor.json"),
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();
    println!(
        "Python credential env canonical capture: {}",
        root.display()
    );
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

fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-native-python-tools/bin/python")
}

fn verify_package(root: &Path, files: &[OutFile], consumer: &str, negative: &str) {
    suspect_codegen::write_files(files, &root.join("generated")).unwrap();
    fs::write(root.join("consumer.py"), consumer).unwrap();
    fs::write(root.join("negative.py"), negative).unwrap();
    let package = root.join("generated/python");
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(&package),
        root,
        "build",
    );
    let wheel = fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let venv = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&venv),
            root,
            &format!("venv-{version}"),
        );
        let python = venv.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel)
                .arg("Sphinx==8.2.3"),
            root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(&python).arg("consumer.py").current_dir(root),
            root,
            &format!("native-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("consumer.py"))
                .arg(package.join("examples"))
                .arg(package.join("src")),
            root,
            &format!("mypy-{version}"),
        );
        let bad = Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--python-version",
                version,
                "--python-executable",
            ])
            .arg(&python)
            .arg("--cache-dir")
            .arg(root.join(format!("negative-cache-{version}")))
            .arg(root.join("negative.py"))
            .output()
            .unwrap();
        fs::write(root.join(format!("negative-{version}.log")), &bad.stdout).unwrap();
        assert!(!bad.status.success());
        let errors = String::from_utf8_lossy(&bad.stdout);
        for (index, line) in negative
            .lines()
            .enumerate()
            .filter(|(_, line)| line.ends_with("# rejected"))
        {
            assert!(
                errors.contains(&format!("negative.py:{}: error:", index + 1)),
                "{line}: {errors}"
            );
        }
        assert!(errors.contains("[arg-type]"), "{errors}");
        checked(
            Command::new(&python)
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
                .arg(root.join(format!("sphinx-{version}")))
                .current_dir(&package),
            root,
            &format!("sphinx-{version}"),
        );
    }
}

#[test]
#[ignore = "requires installed Python 3.11/3.14 and cached wheel/mypy/Sphinx tools"]
fn installed_python_credential_env_snapshots_and_explicit_auth_precedence() {
    let root = root("native");
    let (_, files) = package(fixture(), configured(), IMPORT);
    for file in &files {
        assert!(
            !file.content.contains("SUSPECT_GENERATION_CANARY_VALUE"),
            "generation leaked a credential value"
        );
    }
    verify_package(
        &root,
        &files,
        include_str!("python_credential_env/consumer.py"),
        include_str!("python_credential_env/negative.py"),
    );
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","interpreters":["3.11","3.14"],"artifacts":files.len(),"generatedSha256":fingerprint(&files),"noPolicySha256":NO_POLICY_SHA256})).unwrap()).unwrap();
    println!("Python credential env native evidence: {}", root.display());
}

#[test]
#[ignore = "requires actual OpenRouter source and installed Python 3.11/3.14; controlled transport only"]
fn installed_openrouter_python_credential_env_current_key_and_optional_credits() {
    let root = root("openrouter");
    let source = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../openrouter-web")
        })
        .join("projects/docs/openapi/openapi.yaml")
        .canonicalize()
        .unwrap();
    let bytes = fs::read(&source).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let names = ["getCurrentKey", "getCredits"];
    let selected = names
        .iter()
        .map(|name| {
            contract
                .operations()
                .find(|operation| operation.operation_id() == Some(*name))
                .unwrap()
                .source()
                .clone()
        })
        .collect::<Vec<_>>();
    let config = HttpConfig {
        credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
            "apiKey".into(),
            "OPENROUTER_API_KEY".into(),
        )]))),
        ..Default::default()
    };
    let plan = python_http::plan_http(contract, &selected, config).unwrap();
    let policy = plan.credential_env().unwrap();
    assert_eq!(policy.bindings().len(), 1);
    assert_eq!(policy.bindings()[0].kind(), CredentialEnvKind::Bearer);
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "openrouter-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: "openrouter_sdk".into(),
        },
    )
    .unwrap();
    for file in &files {
        assert!(
            !file.content.contains("SUSPECT_GENERATION_CANARY_VALUE"),
            "generation read a runtime secret"
        );
    }
    let responses = plan
        .examples()
        .operations()
        .iter()
        .map(|operation| {
            let entry = operation
                .entries
                .iter()
                .find(|entry| {
                    matches!(
                        entry.role,
                        suspect_codegen::examples::ExampleRole::Response { status: 200 }
                    )
                })
                .unwrap();
            (operation.operation_id.clone(), entry.value.to_string())
        })
        .collect::<BTreeMap<_, _>>();
    fs::write(
        root.join("responses.json"),
        serde_json::to_vec_pretty(&responses).unwrap(),
    )
    .unwrap();
    fs::write(root.join("source.json"),serde_json::to_vec_pretty(&json!({"path":source,"sha256":format!("{:x}",Sha256::digest(&bytes)),"operations":names,"credentialEnv":policy,"descriptor":policy.semantic_descriptor()})).unwrap()).unwrap();
    verify_package(
        &root,
        &files,
        include_str!("python_credential_env/openrouter.py"),
        include_str!("python_credential_env/openrouter_negative.py"),
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        bytes,
        "source description was changed"
    );
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","interpreters":["3.11","3.14"],"distribution":"openrouter-python-sdk","import":"openrouter_sdk","operations":names,"artifacts":files.len(),"generatedSha256":fingerprint(&files),"sourceSha256":format!("{:x}",Sha256::digest(&bytes)),"descriptor":policy.semantic_descriptor()})).unwrap()).unwrap();
    println!(
        "Python OpenRouter env credential evidence: {}",
        root.display()
    );
}
