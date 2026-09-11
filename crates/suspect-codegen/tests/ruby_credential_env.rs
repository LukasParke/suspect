//! Explicit environment defaults through the admitted Ruby plan and installed
//! gems. Native controls use injected transports and original source HTTPS URLs.
//! Current no-policy/configured/disabled checks are source-location independent.
//! The sealed pre-policy 56-file comparison is a historical receipt, not an input.
#![cfg(feature = "ruby-sdk")]

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
    ruby_sdk::{self, PackageConfig, RubyConfig, SdkPlan},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const GENERATOR_CANARY: &str = "RubyGeneratorCanaryMustNotBeEmitted0123456789";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ruby_sdk/tests/credential_env.openapi.json")
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
fn supplied(value: Value) -> Arc<Contract> {
    let uri = Uri::parse("https://physical.credential-env.ruby.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&value).unwrap(),
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
fn policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("apiKey", "RUBY_ENV_BEARER"),
            ("otherBearer", "RUBY_ENV_OTHER"),
            ("headerKey", "RUBY_ENV_HEADER"),
            ("queryKey", "RUBY_ENV_QUERY"),
            ("cookieKey", "RUBY_ENV_COOKIE"),
        ]
        .map(|(name, variable)| (name.to_owned(), variable.to_owned()))
        .into_iter()
        .collect(),
    )
}
fn package_identity() -> PackageConfig {
    PackageConfig {
        name: "openrouter".into(),
        version: "0.1.0".into(),
        require_name: "openrouter".into(),
        namespace: "OpenRouter".into(),
    }
}
fn plan(contract: Arc<Contract>, config: RubyConfig) -> SdkPlan {
    ruby_sdk::plan_sdk(contract.clone(), &selected(&contract), config)
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
}
fn emitted(plan: &SdkPlan) -> BTreeMap<String, String> {
    ruby_sdk::emit_sdk(plan, &package_identity())
        .unwrap()
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

fn assert_no_policy_artifacts(files: &BTreeMap<String, String>) {
    assert!(
        !files
            .keys()
            .any(|name| name.contains("credential_env") || name.contains("credential-env"))
    );
    assert!(!files["ruby/lib/openrouter.rb"].contains("credential_env"));
    assert!(!files["ruby/README.md"].contains("Runtime environment credentials"));
}

fn assert_current_policy_delta(
    plain: &BTreeMap<String, String>,
    configured: &BTreeMap<String, String>,
) {
    assert_no_policy_artifacts(plain);
    for (path, text) in plain {
        assert!(configured.contains_key(path), "policy removed {path}");
        if matches!(path.as_str(), "ruby/lib/openrouter.rb" | "ruby/README.md") {
            assert_ne!(configured[path], *text, "policy did not update {path}");
        } else {
            assert_eq!(
                configured[path], *text,
                "policy changed unrelated bytes: {path}"
            );
        }
    }
    assert_eq!(
        configured
            .keys()
            .filter(|path| !plain.contains_key(*path))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "ruby/lib/openrouter/credential-env.json",
            "ruby/lib/openrouter/credential_env.rb"
        ]
    );
    assert!(
        configured["ruby/lib/openrouter.rb"]
            .contains("require_relative \"openrouter/credential_env\"")
    );
    assert!(configured["ruby/README.md"].contains("Runtime environment credentials"));
}

fn artifact_hashes(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(path, text)| {
            (
                path.clone(),
                format!("{:x}", Sha256::digest(text.as_bytes())),
            )
        })
        .collect()
}

fn current_session_roundtrip(path: &Path) -> Value {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        generation_session::{Session, SessionConfig},
    };
    let mut config = SessionConfig {
        targets: vec![TargetConfig {
            backend: Backend::RubyHttp,
            package_name: "openrouter".into(),
            package_version: "0.1.0".into(),
            import_name: Some("OpenRouter".into()),
        }],
        ..Default::default()
    };
    let mut session = Session::new(path, config.clone()).unwrap();
    let initial = session.generate().unwrap();
    let files = |output: &suspect_codegen::generation_session::SessionOutput| {
        output
            .files
            .iter()
            .map(|file| (file.path.clone(), file.content.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let plain = files(&initial);
    assert_no_policy_artifacts(&plain);
    config.generation.credential_env = Some(policy());
    session.set_config(config.clone()).unwrap();
    let enabled = session.generate().unwrap();
    let configured = files(&enabled);
    assert_ne!(initial.revision, enabled.revision);
    assert_current_policy_delta(&plain, &configured);
    let document = Uri::from_path(path).unwrap().to_string();
    for artifacts in [&plain, &configured] {
        let source_map: Value = serde_json::from_str(&artifacts["ruby/source-map.json"]).unwrap();
        for operation in source_map["operations"].as_array().unwrap() {
            assert!(
                operation["source"]
                    .as_str()
                    .unwrap()
                    .starts_with(&(document.clone() + "#"))
            );
        }
    }
    let bound: Value =
        serde_json::from_str(&configured["ruby/lib/openrouter/credential-env.json"]).unwrap();
    for binding in bound["bindings"].as_array().unwrap() {
        let at = &binding["scheme"]["use_site"];
        assert_eq!(at["source"]["document"], document);
        assert!(
            at["source"]["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/components/securitySchemes/")
        );
        assert!(at["span"]["end"].as_u64().unwrap() > at["span"]["start"].as_u64().unwrap());
    }
    config.generation.credential_env = None;
    session.set_config(config).unwrap();
    let disabled = session.generate().unwrap();
    let reverted = files(&disabled);
    assert_no_policy_artifacts(&reverted);
    assert_eq!(
        reverted, plain,
        "disabling the policy did not restore the current output"
    );
    assert_eq!(disabled.revision, initial.revision);
    assert!(
        disabled
            .changed_paths
            .iter()
            .any(|path| path == "ruby/lib/openrouter/credential_env.rb")
    );
    assert!(
        disabled
            .changed_paths
            .iter()
            .any(|path| path == "ruby/lib/openrouter/credential-env.json")
    );
    if std::env::var_os("SUSPECT_RUBY_ENV_HOST_PROBE_OUTPUT").is_some() {
        let canary = std::env::var("RUBY_ENV_BEARER").unwrap();
        assert!(
            [
                "RubyIsolatedAmbientAlpha0123456789",
                "RubyIsolatedAmbientBeta9876543210"
            ]
            .contains(&canary.as_str())
        );
        assert!(
            [&plain, &configured, &reverted]
                .into_iter()
                .flat_map(BTreeMap::values)
                .all(|text| !text.contains(&canary)),
            "ambient credential entered an artifact"
        );
    }
    json!({"document":document,"plain":artifact_hashes(&plain),"configured":artifact_hashes(&configured),"disabled":artifact_hashes(&reverted),"plainRevision":initial.revision,"configuredRevision":enabled.revision,"disabledRevision":disabled.revision})
}

fn isolated_current_contract() {
    let base = root().join("target/sdk-ruby-credential-env-host");
    std::fs::create_dir_all(&base).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("runner-")
        .tempdir_in(base)
        .unwrap();
    let first = directory.path().join("original.json");
    let second = directory.path().join("relocated.json");
    let bytes = std::fs::read(fixture()).unwrap();
    std::fs::write(&first, &bytes).unwrap();
    std::fs::write(&second, &bytes).unwrap();
    let a = current_session_roundtrip(&first);
    let b = current_session_roundtrip(&second);
    assert_ne!(a["document"], b["document"]);
    assert_ne!(
        a["plain"]["ruby/source-map.json"], b["plain"]["ruby/source-map.json"],
        "physical provenance was erased"
    );
    let mut receipts = Vec::new();
    for (index, canary) in [
        "RubyIsolatedAmbientAlpha0123456789",
        "RubyIsolatedAmbientBeta9876543210",
    ]
    .into_iter()
    .enumerate()
    {
        let receipt = directory.path().join(format!("ambient-{index}.json"));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bound_policy_uses_admitted_source_names_and_keeps_no_policy_bytes",
                "--nocapture",
            ])
            .env("SUSPECT_RUBY_ENV_HOST_PROBE_INPUT", &first)
            .env("SUSPECT_RUBY_ENV_HOST_PROBE_OUTPUT", &receipt)
            .env_remove("SUSPECT_RUBY_GENERATOR_CANARY")
            .env("OPENROUTER_API_KEY", canary);
        for variable in policy().schemes.values() {
            command.env(variable, canary);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "host probe failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        receipts.push(serde_json::from_slice::<Value>(&std::fs::read(receipt).unwrap()).unwrap());
    }
    assert_eq!(
        receipts[0], receipts[1],
        "ambient credential values changed fresh generator output or revisions"
    );
    eprintln!(
        "Current Ruby env contract passed: two physical source locations; Session no-policy -> configured -> disabled; exact support-file delta; provenance retained; two fresh host processes with different ambient values. No historical baseline input."
    );
}

#[test]
fn bound_policy_uses_admitted_source_names_and_keeps_no_policy_bytes() {
    // Private host-only probe: separate processes exercise generation under two
    // real ambient values. It does not launch Ruby, a CLI or a native SDK suite.
    if let Some(output) = std::env::var_os("SUSPECT_RUBY_ENV_HOST_PROBE_OUTPUT") {
        let input = std::env::var_os("SUSPECT_RUBY_ENV_HOST_PROBE_INPUT").unwrap();
        std::fs::write(
            output,
            serde_json::to_vec(&current_session_roundtrip(Path::new(&input))).unwrap(),
        )
        .unwrap();
        return;
    }
    let contract = load(&fixture());
    let plain = plan(contract.clone(), RubyConfig::default());
    assert!(plain.credential_env().is_none());
    let old = emitted(&plain);
    assert_no_policy_artifacts(&old);
    let configured = plan(
        contract.clone(),
        RubyConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    );
    let bound = configured.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 5);
    let bearer = bound
        .bindings()
        .iter()
        .find(|binding| binding.name() == "apiKey")
        .unwrap();
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(bearer.variable(), "RUBY_ENV_BEARER");
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert_eq!(
        Some(bearer.scheme().use_site().span()),
        contract.source_span(bearer.scheme().use_site().source())
    );
    let descriptor = serde_json::to_string(&bound.semantic_descriptor()).unwrap();
    assert!(!descriptor.contains("file:") && !descriptor.contains("provenance"));
    assert!(descriptor.contains("RUBY_ENV_BEARER"));
    assert!(
        configured
            .operations()
            .iter()
            .any(|operation| operation.method_name == "snapshot")
    );
    assert!(
        configured
            .operations()
            .iter()
            .any(|operation| operation.method_name == "env_bindings")
    );
    let files = emitted(&configured);
    assert_current_policy_delta(&old, &files);
    assert!(
        files["ruby/lib/openrouter/credential_env.rb"]
            .contains("private_constant :BINDINGS, :OMITTED_AUTH")
    );
    assert!(files["ruby/lib/openrouter/credential-env.json"].contains("RUBY_ENV_BEARER"));
    assert!(files["ruby/README.md"].contains("Client.open"));
    if let Ok(canary) = std::env::var("SUSPECT_RUBY_GENERATOR_CANARY") {
        assert_eq!(canary, GENERATOR_CANARY);
        assert_eq!(std::env::var("RUBY_ENV_BEARER").unwrap(), canary);
    }
    assert!(files.values().all(|text| !text.contains(GENERATOR_CANARY)));
    isolated_current_contract();
}

#[test]
fn credential_env_refusals_are_shared_and_follow_protocol_admission() {
    let value: Value = serde_json::from_str(include_str!(
        "../src/ruby_sdk/tests/credential_env.openapi.json"
    ))
    .unwrap();
    for name in ["unknown", "basic", "oauth", "oidc"] {
        let contract = supplied(value.clone());
        let config = RubyConfig {
            credential_env: Some(CredentialEnv::v1(
                [(name.into(), "CONTROLLED_VALUE".into())]
                    .into_iter()
                    .collect(),
            )),
            ..Default::default()
        };
        let errors =
            ruby_sdk::plan_sdk(contract.clone(), &selected(&contract), config).unwrap_err();
        assert!(errors.iter().any(|error| error.code
            == if name == "unknown" {
                "sdk-credential-env-unbound"
            } else {
                "sdk-credential-env-kind"
            }));
        assert!(
            errors
                .iter()
                .all(|error| error.at == contract.source_span(&error.source).unwrap())
        );
    }
    let mut conflict = value;
    conflict["paths"]["/both"]["get"]["security"] = json!([{"apiKey":[],"otherBearer":[]}]);
    let contract = supplied(conflict);
    let config = RubyConfig {
        credential_env: Some(CredentialEnv::v1(
            [("unknown".into(), "BAD-VARIABLE".into())]
                .into_iter()
                .collect(),
        )),
        ..Default::default()
    };
    let errors = ruby_sdk::plan_sdk(contract.clone(), &selected(&contract), config).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-security-attachment-conflict")
    );
    assert!(
        errors
            .iter()
            .all(|error| !error.code.starts_with("sdk-credential-env"))
    );
}

#[test]
fn canonical_credential_env_capture_is_semantic_and_keeps_credential_surface() {
    use suspect_codegen::{
        backend::{Backend, GenerationOptions, TargetConfig},
        compatibility::{self, PlanStatus},
    };
    let contract = load(&fixture());
    let target = TargetConfig {
        backend: Backend::RubyHttp,
        package_name: "openrouter".into(),
        package_version: "0.1.0".into(),
        import_name: Some("OpenRouter".into()),
    };
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let snapshot = compatibility::snapshot_with_options(
        contract.clone(),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    let planned = plan(
        contract.clone(),
        RubyConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    );
    assert_eq!(
        native.credential_env.as_ref(),
        Some(&planned.credential_env().unwrap().semantic_descriptor())
    );
    assert!(
        native
            .operations
            .iter()
            .all(
                |operation| operation.source.document == contract.entry().as_str()
                    && operation.source.span.is_some()
            )
    );
    let plain =
        compatibility::snapshot(contract.clone(), &[], std::slice::from_ref(&target)).unwrap();
    assert!(plain.native[0].credential_env.is_none());
    assert!(
        serde_json::to_value(&plain.native[0])
            .unwrap()
            .get("credential_env")
            .is_none()
    );
    for (before, after) in plain.native[0].operations.iter().zip(&native.operations) {
        assert_eq!(before.operation_id, after.operation_id);
        assert_eq!(
            before.descriptor["credential"],
            after.descriptor["credential"]
        );
    }
    let relocated = supplied(
        serde_json::from_str(include_str!(
            "../src/ruby_sdk/tests/credential_env.openapi.json"
        ))
        .unwrap(),
    );
    let relocated = compatibility::snapshot_with_options(
        relocated,
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    let comparison = compatibility::compare_snapshots(&snapshot, &relocated);
    assert!(
        comparison.is_proven_compatible(),
        "{}",
        comparison.migration_notes()
    );
    let mut next = options;
    next.credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "RUBY_ENV_NEW_BEARER".into());
    let changed = compatibility::snapshot_with_options(contract, &[], &[target], &next).unwrap();
    let comparison = compatibility::compare_snapshots(&snapshot, &changed);
    assert!(
        comparison.wire.is_empty(),
        "{}",
        comparison.migration_notes()
    );
    assert!(
        comparison.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-credential-env-changed")
    );
}

fn ruby_home() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}
fn default_gems() -> PathBuf {
    std::fs::read_dir(ruby_home().join("lib/ruby/gems"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap()
}
fn tools() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_GEMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target/sdk-ruby-tools/gems"))
}
fn ruby() -> Command {
    let mut command = Command::new(ruby_home().join("bin/ruby"));
    command
        .env("GEM_HOME", tools())
        .env(
            "GEM_PATH",
            format!("{}:{}", tools().display(), default_gems().display()),
        )
        .env(
            "PATH",
            format!(
                "{}:{}",
                ruby_home().join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("OPENROUTER_API_KEY", "controlled-native-process-token")
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB");
    for key in [
        "RUBY_ENV_BEARER",
        "RUBY_ENV_OTHER",
        "RUBY_ENV_HEADER",
        "RUBY_ENV_QUERY",
        "RUBY_ENV_COOKIE",
    ] {
        command.env_remove(key);
    }
    command
}
fn checked(command: &mut Command, directory: &Path, label: &str) {
    let output = command.output().unwrap();
    let text = format!(
        "{command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(directory.join(format!("{label}.log")), &text).unwrap();
    assert!(output.status.success(), "{}\n{text}", directory.display());
    assert!(!text.contains("[error]"), "{text}");
}
fn native_package(plan: &SdkPlan, prefix: &str) -> PathBuf {
    let base = root().join("target/sdk-ruby-credential-env-native");
    std::fs::create_dir_all(&base).unwrap();
    let directory = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(base)
        .unwrap()
        .keep();
    let files = emitted(plan);
    for (name, content) in &files {
        assert!(
            !content.contains(GENERATOR_CANARY),
            "generator value appeared in {name}"
        );
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    checked(ruby().arg("-v"), &directory, "ruby-version");
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["build", "openrouter.gemspec"])
            .current_dir(directory.join("ruby")),
        &directory,
        "gem-build",
    );
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["install", "--local", "--no-document"])
            .arg(directory.join("ruby/openrouter-0.1.0.gem"))
            .env("GEM_HOME", directory.join("installed"))
            .env("GEM_PATH", default_gems()),
        &directory,
        "gem-install",
    );
    let installed = directory.join("installed/gems/openrouter-0.1.0");
    for (name, content) in files {
        if !name.ends_with(".gemspec") {
            assert_eq!(
                std::fs::read(installed.join(name.strip_prefix("ruby/").unwrap())).unwrap(),
                content.as_bytes()
            );
        }
    }
    std::fs::write(directory.join("production-assets.json"), serde_json::to_vec_pretty(&ruby_sdk::source_assets().iter().map(|(name, bytes)| json!({"path":name,"sha256":format!("{:x}", Sha256::digest(bytes))})).collect::<Vec<_>>()).unwrap()).unwrap();
    eprintln!(
        "Ruby credential-env native package: {}",
        directory.display()
    );
    directory
}
fn consumer(directory: &Path, text: &str) {
    std::fs::write(directory.join("consumer.rb"), text).unwrap();
    checked(
        ruby()
            .arg("consumer.rb")
            .current_dir(directory)
            .env("GEM_HOME", directory.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    directory.join("installed").display(),
                    default_gems().display()
                ),
            ),
        directory,
        "native",
    );
}

#[test]
#[ignore = "requires Ruby 3.3.12/4.0.6; isolated gem installation and bounded credential-env transport controls"]
fn installed_credential_env_omission_precedence_choices_and_snapshot() {
    let plan = plan(
        load(&fixture()),
        RubyConfig {
            credential_env: Some(policy()),
            ..Default::default()
        },
    );
    let directory = native_package(&plan, "controls-");
    consumer(
        &directory,
        include_str!("../src/ruby_sdk/tests/credential_env.rb"),
    );
}

fn openrouter_source() -> PathBuf {
    std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("../openrouter-web"))
        .join("projects/docs/openapi/openapi.yaml")
        .canonicalize()
        .unwrap()
}
fn openrouter_plan(contract: Arc<Contract>, env: bool) -> SdkPlan {
    let selection = contract
        .operations()
        .filter(|operation| {
            matches!(
                operation.operation_id(),
                Some("getCurrentKey" | "getCredits")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selection.len(), 2);
    let config = RubyConfig {
        credential_env: env.then(|| {
            CredentialEnv::v1(
                [("apiKey".into(), "OPENROUTER_API_KEY".into())]
                    .into_iter()
                    .collect(),
            )
        }),
        ..Default::default()
    };
    ruby_sdk::plan_sdk(contract, &selection, config).unwrap_or_else(|errors| panic!("{errors:#?}"))
}

#[test]
#[ignore = "requires actual OpenRouter source, native Ruby, installed gem, RBS/Steep/YARD; HTTPS captured by a controlled transport"]
fn installed_openrouter_env_uses_source_default_https_and_real_key_schemas() {
    let contract = load(&openrouter_source());
    let plain = openrouter_plan(contract.clone(), false);
    let plan = openrouter_plan(contract, true);
    assert_current_policy_delta(&emitted(&plain), &emitted(&plan));
    let binding = &plan.credential_env().unwrap().bindings()[0];
    assert_eq!(
        (binding.name(), binding.variable(), binding.kind()),
        ("apiKey", "OPENROUTER_API_KEY", CredentialEnvKind::Bearer)
    );
    let operation = plan
        .operations()
        .iter()
        .find(|operation| operation.operation_id == "getCurrentKey")
        .unwrap();
    assert_eq!(
        (
            operation.method.as_str(),
            operation.path.as_str(),
            operation.method_name.as_str()
        ),
        ("GET", "/key", "get_current_key")
    );
    assert_eq!(
        operation.wire.servers().candidates()[0].template(),
        "https://openrouter.ai/api/v1"
    );
    let directory = native_package(&plan, "openrouter-");
    std::fs::write(directory.join("source.json"), serde_json::to_vec_pretty(&json!({"path":openrouter_source(),"sha256":format!("{:x}",Sha256::digest(std::fs::read(openrouter_source()).unwrap())),"operations":["getCurrentKey","getCredits"]})).unwrap()).unwrap();
    consumer(
        &directory,
        include_str!("../src/ruby_sdk/tests/credential_env_openrouter.rb"),
    );
    let installed = directory.join("installed/gems/openrouter-0.1.0");
    checked(
        ruby()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed.join("sig"))
            .arg("validate"),
        &directory,
        "rbs",
    );
    checked(
        ruby()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(&installed),
        &directory,
        "yard",
    );
    let types = directory.join("types");
    std::fs::create_dir_all(&types).unwrap();
    std::fs::write(
        types.join("Steepfile"),
        "target :consumer do\n  library 'openrouter'\n  check 'consumer.rb'\nend\n",
    )
    .unwrap();
    std::fs::write(types.join("consumer.rb"), "require 'openrouter'\nclient = OpenRouter::Client.new\nkey = client.get_current_key\nputs key.data.data.label\nclient.close\nOpenRouter::Client.open { |open| puts open.get_current_key.status }\nOpenRouter::Client.new(auth: {})\nOpenRouter::Client.new(auth: {'apiKey' => 'explicit'})\n").unwrap();
    checked(
        ruby()
            .arg(tools().join("bin/steep"))
            .args(["check", "--jobs=1"])
            .current_dir(&types)
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}:{}",
                    tools().display(),
                    directory.join("installed").display(),
                    default_gems().display()
                ),
            ),
        &directory,
        "types",
    );
}
