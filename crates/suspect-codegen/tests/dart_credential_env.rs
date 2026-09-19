//! Focused credential-env policy, installed runtime and OpenRouter source gates.
#![cfg(feature = "dart-sdk")]
use codegen::credential_env::{CredentialEnv, CredentialEnvKind};
use codegen::dart_sdk::{self, DartConfig, PackageConfig};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen as codegen;
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;
mod dart_support;
use dart_support as support;

fn document() -> Value {
    let response = json!({"204":{"description":"credential control"}});
    let op = |name: &str, security: Value| json!({"get":{"operationId":name,"security":security,"responses":response}});
    json!({"openapi":"3.2.0","info":{"title":"Environment credential controls","version":"1"},"servers":[{"url":"https://source.example/api"}],
    "components":{"securitySchemes":{
        "apiKey":{"type":"http","scheme":"bearer"},
        "headerKey":{"type":"apiKey","in":"header","name":"X-Key"},
        "queryKey":{"type":"apiKey","in":"query","name":"token"},
        "cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
        "runtimeType":{"type":"apiKey","in":"header","name":"X-Extra"}
    }},"paths":{
        "/key":op("getCurrentKey",json!([{"apiKey":[]}])),
        "/either":op("either",json!([{"apiKey":[]},{"headerKey":[]}])),
        "/together":op("together",json!([{"apiKey":[],"headerKey":[]}])),
        "/anonymous":op("anonymous",json!([])),
        "/optional":op("optional",json!([{"apiKey":[]},{}])),
        "/query":op("queryKey",json!([{"queryKey":[]}])),
        "/cookie":op("cookieKey",json!([{"cookieKey":[]}])),
        "/allocated":op("allocated",json!([{"runtimeType":[]}]))
    }})
}
fn contract(document: &Value, uri: &str) -> Arc<Contract> {
    let uri = Uri::parse(uri).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(document).unwrap(),
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
fn config() -> DartConfig {
    DartConfig {
        package: PackageConfig {
            name: "openrouter".into(),
            version: "0.1.0".into(),
        },
        ..Default::default()
    }
}
fn policy() -> CredentialEnv {
    CredentialEnv::v1(BTreeMap::from([
        ("apiKey".into(), "OPENROUTER_API_KEY".into()),
        ("headerKey".into(), "DART_ENV_HEADER".into()),
        ("queryKey".into(), "DART_ENV_QUERY".into()),
        ("cookieKey".into(), "DART_ENV_COOKIE".into()),
        ("runtimeType".into(), "DART_ENV_EXTRA".into()),
    ]))
}
fn configured() -> DartConfig {
    DartConfig {
        credential_env: Some(policy()),
        attribution: Some(backend_attribution()),
        ..config()
    }
}
/// backend.rs compiles the same descriptor into every Backend::DartHttp package;
/// the direct native config must carry it for byte-equality with backend output.
fn backend_attribution() -> codegen::attribution::AttributionDescriptor {
    codegen::attribution::AttributionDescriptor::plan(
        env!("CARGO_PKG_VERSION"),
        "openrouter",
        "0.1.0",
        "3.2.0",
        "dart",
    )
}
fn plan(c: Arc<Contract>, config: DartConfig) -> dart_sdk::Plan {
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    dart_sdk::plan_sdk(c, &selected, config).unwrap()
}
fn hashes(files: &[codegen::OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|f| {
            (
                f.path.clone(),
                format!("{:x}", Sha256::digest(f.content.as_bytes())),
            )
        })
        .collect()
}

#[test]
fn no_policy_output_bytes_are_preserved() {
    let plan = plan(
        contract(&document(), "https://credential.example/openapi.json"),
        config(),
    );
    let hashes = hashes(&plan.render());
    let mut expected: BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/dart-credential-env-no-policy.json")).unwrap();
    // The separately witnessed Host/TLS transport repairs change only this IO asset.
    // Preserve the original 25-file env baseline as historical evidence.
    assert_eq!(
        expected.insert(
            "dart/lib/openrouter_io.dart".into(),
            include_str!("fixtures/dart-transport-tls-io.sha256")
                .trim()
                .into()
        ),
        // Updated with the ua/v1 constructor fix to the shared transport runtime
        // (final-field initialization), which this IO asset also contains.
        Some("04bd75a2e2f31d1e40cc25ca4a5401076b9899a755509acdc5fdb91fd161c58f".into())
    );
    assert_eq!(hashes, expected);
    if let Some(path) = std::env::var_os("SUSPECT_DART_ENV_RECORD_NO_POLICY") {
        let path = PathBuf::from(path);
        assert!(!path.exists());
        std::fs::write(path, serde_json::to_vec_pretty(&hashes).unwrap()).unwrap();
    }
    assert!(
        plan.render()
            .iter()
            .all(|f| !f.path.contains("environment"))
    );
}

#[test]
fn policy_binds_actual_source_names_and_keeps_semantics_relocation_independent() {
    let first = plan(
        contract(&document(), "https://credential.example/first.json"),
        configured(),
    );
    let second = plan(
        contract(&document(), "https://relocated.example/second.json"),
        configured(),
    );
    let bound = first.credential_env().unwrap();
    assert_eq!(bound.bindings().len(), 5);
    assert_eq!(
        bound
            .bindings()
            .iter()
            .find(|b| b.name() == "apiKey")
            .unwrap()
            .kind(),
        CredentialEnvKind::Bearer
    );
    assert_eq!(
        bound.semantic_descriptor(),
        second.credential_env().unwrap().semantic_descriptor()
    );
    assert_ne!(
        bound.bindings()[0].scheme(),
        second.credential_env().unwrap().bindings()[0].scheme()
    );
    assert_eq!(
        first
            .credentials()
            .iter()
            .find(|c| c.wire_name == "runtimeType")
            .unwrap()
            .name,
        "runtimeType2"
    );
    let text = first
        .render()
        .into_iter()
        .map(|f| f.content)
        .collect::<String>();
    assert!(!text.contains("DART_GENERATOR_ONLY_CANARY_9b701"));
    assert!(text.contains("OPENROUTER_API_KEY"));
    assert!(text.contains("runtimeType2: _environmentCredential"));
    for (kind, scheme) in [
        ("basic", json!({"type":"http","scheme":"basic"})),
        (
            "oauth",
            json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.example/token","scopes":{}}}}),
        ),
        (
            "oidc",
            json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.example/discovery"}),
        ),
    ] {
        let mut doc = document();
        doc["components"]["securitySchemes"]["apiKey"] = scheme;
        let c = contract(&doc, "https://credential.example/invalid.json");
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let errors = dart_sdk::plan_sdk(c, &selected, configured()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == "sdk-credential-env-kind" && !e.at.is_empty()),
            "{kind}: {errors:?}"
        );
    }
}

#[test]
fn canonical_policy_capture_and_session_identity_are_source_bound() {
    use codegen::{
        backend::{self, Backend, GenerationOptions, TargetConfig},
        compatibility,
        generation_session::{Session, SessionConfig},
    };
    let c = contract(&document(), "https://credential.example/openapi.json");
    let target = TargetConfig {
        backend: Backend::DartHttp,
        package_name: "openrouter".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    };
    let options = GenerationOptions {
        credential_env: Some(policy()),
        ..Default::default()
    };
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let direct = plan(c.clone(), configured());
    assert_eq!(
        backend::generate_with_options(c.clone(), &selected, &target, &options).unwrap(),
        direct.render()
    );
    let before = compatibility::snapshot_with_options(
        c.clone(),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(
        before.native[0].credential_env,
        Some(direct.credential_env().unwrap().semantic_descriptor())
    );
    let client = before.native[0]
        .models
        .iter()
        .find(|m| m.name == "Client" && m.role == "client")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    let credentials = client["constructor"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "credentials")
        .unwrap();
    assert_eq!(
        credentials["type"],
        json!({"kind":"named","name":"Credentials"})
    );
    assert_eq!(
        credentials["initialization"],
        json!({"kind":"omitted","runtimeDefault":"environment-snapshot"})
    );
    let mut changed = options.clone();
    changed
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "OPENROUTER_SECOND_KEY".into());
    let after =
        compatibility::snapshot_with_options(c, &[], std::slice::from_ref(&target), &changed)
            .unwrap();
    let comparison = compatibility::compare_snapshots(&before, &after);
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
    let root = support::root("credential-env-canonical-");
    let path = root.join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let original = SessionConfig {
        targets: vec![target],
        generation: options,
        ..Default::default()
    };
    let mut session = Session::new(&path, original.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(session.generate().unwrap().delta.renders, 0);
    let mut config = original.clone();
    config.generation = changed;
    session.set_config(config).unwrap();
    let next = session.generate().unwrap();
    assert_ne!(first.revision, next.revision);
    assert_eq!((next.delta.compiles, next.delta.renders), (0, 1));
    session.set_config(original).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.revision, first.revision);
    assert_eq!(reverted.delta.renders, 0);
    std::fs::write(
        root.join("capture.json"),
        serde_json::to_vec_pretty(&before.native).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("comparison.json"),
        serde_json::to_vec_pretty(&comparison).unwrap(),
    )
    .unwrap();
}

const GENERATOR_CANARY: &str = "DART_GENERATOR_ONLY_CANARY_9b701";
const VARIABLES: [&str; 5] = [
    "OPENROUTER_API_KEY",
    "DART_ENV_HEADER",
    "DART_ENV_QUERY",
    "DART_ENV_COOKIE",
    "DART_ENV_EXTRA",
];
fn clean_env(command: &mut std::process::Command) -> &mut std::process::Command {
    for key in VARIABLES {
        command.env_remove(key);
    }
    command
}
fn installed(root: &Path, p: &dart_sdk::Plan) {
    let files = p.render();
    assert!(files.iter().all(|f| !f.content.contains(GENERATOR_CANARY)));
    support::install_package(root, &files, "openrouter", "0.1.0");
}

#[test]
#[ignore = "actual SDK HTTPS, trust/hostname rejection and cancellation; local fixture credentials only"]
fn native_transport_uses_verified_https() {
    let root = support::root("transport-tls-");
    let c = contract(&document(), "https://credential.example/tls-control.json");
    let selected = c
        .operations()
        .filter(|o| o.operation_id() == Some("getCurrentKey"))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let p = dart_sdk::plan_sdk(c, &selected, config()).unwrap();
    installed(&root, &p);
    std::fs::write(root.join("certificate.cnf"),"[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=localhost\n[ext]\nsubjectAltName=DNS:localhost\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\n").unwrap();
    support::check(
        std::process::Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-config",
            ])
            .arg(root.join("certificate.cnf"))
            .arg("-keyout")
            .arg(root.join("fixture.key"))
            .arg("-out")
            .arg(root.join("fixture.crt")),
        &root,
        "fixture-certificate",
    );
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/tls.dart"),
        include_str!("../src/dart_sdk/native_tls.dart"),
    )
    .unwrap();
    support::check(
        support::dart(&root)
            .args(["compile", "exe", "bin/tls.dart", "-o"])
            .arg(root.join("tls-vm"))
            .current_dir(&consumer),
        &root,
        "tls-compile",
    );
    for mode in ["trusted", "untrusted"] {
        support::check(
            std::process::Command::new(root.join("tls-vm"))
                .arg(root.join("fixture.crt"))
                .arg(root.join("fixture.key"))
                .arg(mode),
            &root,
            &format!("tls-{mode}"),
        );
    }
}

#[test]
#[ignore = "focused real SDK HttpClient authority-header regression; loopback only"]
fn native_transport_preserves_required_host_header() {
    let root = support::root("transport-host-");
    let server = support::Server::start(|stream, request, url| {
        let authority = url.strip_prefix("http://").unwrap();
        if request
            .headers
            .iter()
            .any(|(name, value)| name == "host" && value == authority)
        {
            support::reply(stream, 204, "application/json", b"");
        } else {
            support::reply(stream, 400, "application/json", b"{}");
        }
    });
    let mut doc = document();
    doc["servers"] = json!([{"url":format!("{}/api",server.url)}]);
    let c = contract(&doc, "https://credential.example/host-control.json");
    let selected = c
        .operations()
        .filter(|o| o.operation_id() == Some("getCurrentKey"))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let p = dart_sdk::plan_sdk(c, &selected, config()).unwrap();
    installed(&root, &p);
    let consumer = root.join("consumer");
    std::fs::write(consumer.join("bin/host.dart"),"import 'package:openrouter/openrouter_io.dart';\nFuture<void> main() async {final client=Client(transport:IoTransport(),credentials:const Credentials(apiKey:'loopback-fixture'));try{final response=await client.getCurrentKey();if(response.status!=204)throw StateError('status');print('DART_HOST_HEADER_OK');}finally{await client.close();}}\n").unwrap();
    support::check(
        support::dart(&root)
            .args(["compile", "exe", "bin/host.dart", "-o"])
            .arg(root.join("host-vm"))
            .current_dir(&consumer),
        &root,
        "host-compile",
    );
    support::check(
        &mut std::process::Command::new(root.join("host-vm")),
        &root,
        "host-run",
    );
    let records = server.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "GET");
    assert_eq!(records[0].path, "/api/key");
    assert!(records[0].headers.iter().any(
        |(name, value)| name == "host" && value == server.url.strip_prefix("http://").unwrap()
    ));
    std::fs::write(root.join("wire-summary.json"),serde_json::to_vec_pretty(&json!({"method":records[0].method,"target":records[0].path,"headerNames":records[0].headers.iter().map(|(name,_)|name).collect::<Vec<_>>(),"hostMatchesOrigin":true})).unwrap()).unwrap();
}

#[test]
#[ignore = "configured installed VM/JS/browser credential-env controls on Dart floor/current"]
fn native_credential_env_controls() {
    let root = support::root("credential-env-controls-");
    let p = plan(
        contract(&document(), "https://credential.example/openapi.json"),
        configured(),
    );
    installed(&root, &p);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("../src/dart_sdk/native_credential_env.dart"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("bin/main.dart"),
        include_str!("../src/dart_sdk/native_credential_env_io.dart"),
    )
    .unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root.join("controls-vm"))
            .current_dir(&consumer),
        &root,
        "compile-vm",
    );
    for mode in ["controls", "missing", "empty", "positive"] {
        let mut command = std::process::Command::new(root.join("controls-vm"));
        command.arg(mode);
        clean_env(&mut command);
        if mode == "empty" {
            for key in VARIABLES {
                command.env(key, "");
            }
        }
        if mode == "positive" {
            for (key, value) in VARIABLES.into_iter().zip([
                "native-env",
                "native-header",
                "native/query",
                "native/cookie",
                "native-extra",
            ]) {
                command.env(key, value);
            }
        }
        support::check(&mut command, &root, &format!("run-vm-{mode}"));
    }
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/portable.dart", "-o"])
            .arg(root.join("controls.js"))
            .current_dir(&consumer),
        &root,
        "compile-js",
    );
    support::check(
        clean_env(
            std::process::Command::new("node")
                .args(["-e", "globalThis.self=globalThis;require(process.argv[1]);"])
                .arg(root.join("controls.js")),
        )
        .env("OPENROUTER_API_KEY", "node-env-must-not-be-read"),
        &root,
        "run-js",
    );
    for (n, (source, code)) in [
        (
            "void main(){Client(transport: null);}",
            "argument_type_not_assignable",
        ),
        (
            "void f(HttpTransport t){Client(transport:t,credentials:'not credentials');}",
            "argument_type_not_assignable",
        ),
        (
            "void f(HttpTransport t){Client(transport:t,credentials:null);}",
            "argument_type_not_assignable",
        ),
        (
            "void f(HttpTransport t){Client(transport:t,environment:(name)=>1);}",
            "return_of_invalid_type_from_closure",
        ),
    ]
    .iter()
    .enumerate()
    {
        let path = consumer.join(format!("negative-{n}.dart"));
        std::fs::write(
            &path,
            format!("import 'package:openrouter/openrouter.dart';\n{source}\n"),
        )
        .unwrap();
        let result = support::output(
            support::dart(&root)
                .args(["analyze", "--fatal-infos"])
                .arg(&path)
                .current_dir(&consumer),
            &root,
            &format!("negative-{n}"),
        );
        let text = String::from_utf8_lossy(&result.stdout);
        assert!(
            !result.status.success() && text.contains(code) && !text.contains("uri_does_not_exist"),
            "{text}"
        );
    }
    println!("DART_CREDENTIAL_ENV_ROOT={}", root.display());
}

#[test]
#[ignore = "targeted installed credential constructor null/type compatibility on VM/JS; separate browser receipt"]
fn native_credential_env_constructor_compatibility() {
    let root = support::root("credential-env-constructor-");
    let p = plan(
        contract(&document(), "https://credential.example/openapi.json"),
        configured(),
    );
    installed(&root, &p);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/controls.dart"),
        include_str!("../src/dart_sdk/native_credential_env.dart"),
    )
    .unwrap();
    std::fs::write(consumer.join("bin/constructor.dart"),"import 'controls.dart' as control;\nFuture<void> main() async {await control.constructorControls();await control.constructorDefaultEnvironment(null);print('DART_ENV_CONSTRUCTOR_PORTABLE_OK');}\n").unwrap();
    std::fs::write(consumer.join("bin/constructor_io.dart"),"import 'package:openrouter/openrouter_io.dart';\nimport 'controls.dart' as control;\nFuture<void> main() async {await control.constructorControls();await control.constructorDefaultEnvironment('native-constructor');var reads=0;final client=Client(transport:IoTransport(),credentials:const Credentials(),environment:(name){reads++;return 'not-read';});await client.close();control.check(reads==0,'explicit IO credentials skip environment');print('DART_ENV_CONSTRUCTOR_VM_OK');}\n").unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "constructor-analyze",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "exe", "bin/constructor_io.dart", "-o"])
            .arg(root.join("constructor-vm"))
            .current_dir(&consumer),
        &root,
        "constructor-compile-vm",
    );
    support::check(
        clean_env(&mut std::process::Command::new(root.join("constructor-vm")))
            .env("OPENROUTER_API_KEY", "native-constructor"),
        &root,
        "constructor-run-vm",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/constructor.dart", "-o"])
            .arg(root.join("constructor.js"))
            .current_dir(&consumer),
        &root,
        "constructor-compile-js",
    );
    support::check(
        clean_env(
            std::process::Command::new("node")
                .args(["-e", "globalThis.self=globalThis;require(process.argv[1]);"])
                .arg(root.join("constructor.js")),
        )
        .env("OPENROUTER_API_KEY", "node-env-must-not-be-read"),
        &root,
        "constructor-run-js",
    );
    let literal = consumer.join("literal_null.dart");
    std::fs::write(&literal,"import 'package:openrouter/openrouter.dart';\nvoid invalid(HttpTransport transport){Client(transport:transport,credentials:null);}\nvoid main(){}\n").unwrap();
    let analyzed = support::output(
        support::dart(&root)
            .args(["analyze", "--format", "machine"])
            .arg(&literal)
            .current_dir(&consumer),
        &root,
        "literal-null-analyze",
    );
    let findings = format!(
        "{}{}",
        String::from_utf8_lossy(&analyzed.stdout),
        String::from_utf8_lossy(&analyzed.stderr)
    );
    assert_eq!(analyzed.status.code(), Some(3), "{findings}");
    assert!(
        findings.contains("ARGUMENT_TYPE_NOT_ASSIGNABLE")
            && !findings.contains("URI_DOES_NOT_EXIST"),
        "{findings}"
    );
    for (format, output) in [("exe", "invalid-vm"), ("js", "invalid.js")] {
        let result = support::output(
            support::dart(&root)
                .args(["compile", format])
                .arg(&literal)
                .arg("-o")
                .arg(root.join(output))
                .current_dir(&consumer),
            &root,
            &format!("literal-null-compile-{format}"),
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            !result.status.success() && text.contains("Null") && text.contains("Credentials"),
            "{text}"
        );
    }
    println!("DART_ENV_CONSTRUCTOR_GATE_ROOT={}", root.display());
}

#[test]
#[ignore = "actual OpenRouter getCurrentKey/getCredits source with installed runtime env and source-default HTTPS capture"]
fn native_openrouter_credential_env() {
    let root = support::root("credential-env-openrouter-");
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    let source_hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
    let c = support::load(&path);
    let selected = c
        .operations()
        .filter(|o| matches!(o.operation_id(), Some("getCurrentKey" | "getCredits")))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let cfg = DartConfig {
        credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
            "apiKey".into(),
            "OPENROUTER_API_KEY".into(),
        )]))),
        ..config()
    };
    let p = dart_sdk::plan_sdk(c.clone(), &selected, cfg).unwrap();
    assert_eq!(
        p.credential_env().unwrap().bindings()[0].kind(),
        CredentialEnvKind::Bearer
    );
    let key = p
        .operations()
        .iter()
        .find(|o| o.operation_id == "getCurrentKey")
        .unwrap();
    assert_eq!(key.method_name, "getCurrentKey");
    assert_eq!(key.wire.path(), "/key");
    assert_eq!(
        key.wire.servers().candidates()[0]
            .resolve_document_url(&BTreeMap::new())
            .unwrap(),
        "https://openrouter.ai/api/v1"
    );
    std::fs::write(root.join("source-input.json"),serde_json::to_vec_pretty(&json!({"path":path,"sha256":source_hash,"operations":["getCurrentKey","getCredits"],"wire":"controlled in-memory transport; source-default HTTPS URLs, no account requests"})).unwrap()).unwrap();
    installed(&root, &p);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("../src/dart_sdk/native_credential_env_openrouter.dart"),
    )
    .unwrap();
    std::fs::write(consumer.join("bin/main.dart"),"import 'package:openrouter/openrouter_io.dart';\nimport 'portable.dart' as test;\nFuture<void> main(List<String> args) async { final unused=Client(transport:IoTransport());await unused.close();if(args.single=='positive'){await test.realEnvironment();}else{await test.unavailable();} }\n").unwrap();
    std::fs::copy(
        root.join("dart/example/source_examples.dart"),
        consumer.join("bin/examples.dart"),
    )
    .unwrap();
    support::check(
        support::dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    for input in ["main", "examples"] {
        support::check(
            support::dart(&root)
                .args(["compile", "exe"])
                .arg(format!("bin/{input}.dart"))
                .arg("-o")
                .arg(root.join(format!("{input}-vm")))
                .current_dir(&consumer),
            &root,
            &format!("compile-{input}-vm"),
        );
    }
    for mode in ["positive", "missing", "empty"] {
        let mut command = std::process::Command::new(root.join("main-vm"));
        command.arg(mode);
        clean_env(&mut command);
        if mode == "positive" {
            command.env("OPENROUTER_API_KEY", "native-openrouter");
        } else if mode == "empty" {
            command.env("OPENROUTER_API_KEY", "");
        }
        support::check(&mut command, &root, &format!("run-vm-{mode}"));
    }
    support::check(
        &mut std::process::Command::new(root.join("examples-vm")),
        &root,
        "examples-run-vm",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/portable.dart", "-o"])
            .arg(root.join("openrouter.js"))
            .current_dir(&consumer),
        &root,
        "compile-js",
    );
    support::check(
        clean_env(
            std::process::Command::new("node")
                .args(["-e", "globalThis.self=globalThis;require(process.argv[1]);"])
                .arg(root.join("openrouter.js")),
        )
        .env("OPENROUTER_API_KEY", "node-env-must-not-be-read"),
        &root,
        "run-js",
    );
    support::check(
        support::dart(&root)
            .args(["compile", "js", "bin/examples.dart", "-o"])
            .arg(root.join("examples.js"))
            .current_dir(&consumer),
        &root,
        "compile-examples-js",
    );
    support::node(&root, "examples.js", "run-examples-js");
    support::check(
        support::dart(&root)
            .args(["doc", "--validate-links", "--output"])
            .arg(root.join("dartdoc"))
            .current_dir(root.join("dart")),
        &root,
        "dartdoc",
    );
    let docs = std::fs::read_to_string(root.join("logs/dartdoc.log")).unwrap();
    assert!(docs.contains("Found 0 warnings and 0 errors."), "{docs}");
    let constructor =
        std::fs::read_to_string(root.join("dartdoc/openrouter/Client/Client.html")).unwrap();
    assert!(constructor.contains("environment") && constructor.contains("credentials"));
    assert_eq!(
        source_hash,
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()))
    );
    println!("DART_OPENROUTER_CREDENTIAL_ENV_ROOT={}", root.display());
}
