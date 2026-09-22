//! Explicit environment policy binds through the public, admitted protocol plan.
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::{credential_env, http_protocol, rust_http};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.env.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&json!({
                "openapi":"3.1.2", "info":{"title":"Explicit credentials","version":"1"},
                "servers":[{"url":"https://api.env.test/v1"}],
                "security":[{"apiKey":[]}],
                "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
                "paths":{
                    "/first":{"get":{"operationId":"first","responses":{"204":{"description":"Done"}}}},
                    "/second":{"get":{"operationId":"second","responses":{"204":{"description":"Done"}}}}
                }
            }))
            .unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

#[test]
fn explicit_bearer_mapping_binds_one_source_scheme_and_only_variable_names() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(&contract, &selected, rust_http::native_capabilities_v3())
        .into_result()
        .unwrap();
    let config = serde_json::from_value(json!({
        "version":"v1", "schemes":{"apiKey":"OPENROUTER_API_KEY"}
    }))
    .unwrap();
    let planned = credential_env::plan(&contract, &protocol, Some(&config))
        .unwrap()
        .unwrap();
    assert_eq!(planned.bindings().len(), 1);
    let binding = &planned.bindings()[0];
    assert_eq!(binding.name(), "apiKey");
    assert_eq!(binding.variable(), "OPENROUTER_API_KEY");
    assert_eq!(binding.kind(), credential_env::CredentialEnvKind::Bearer);
    assert_eq!(
        binding.scheme().use_site().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert_eq!(
        serde_json::to_value(planned.semantic_descriptor()).unwrap(),
        json!({"version":"v1","bindings":[{
            "name":"apiKey","variable":"OPENROUTER_API_KEY","kind":"bearer"
        }]})
    );
    assert!(
        credential_env::plan(&contract, &protocol, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn policy_syntax_is_closed_bounded_and_duplicate_keys_are_not_silently_replaced() {
    for raw in [
        r#"{"version":"v1","schemes":{}}"#.to_owned(),
        r#"{"version":"v1","schemes":{"apiKey":""}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"KEY-NAME"}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"9KEY"}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"KEY=VALUE"}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"KEY\u0000"}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"ENV_A","apiKey":"ENV_B"}}"#.into(),
        r#"{"version":"v2","schemes":{"apiKey":"ENV"}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":17}}"#.into(),
        r#"{"version":"v1","schemes":{"apiKey":"ENV"},"dotenv":true}"#.into(),
        r#"{"schemes":{"apiKey":"ENV"}}"#.into(),
        json!({"version":"v1","schemes":{"apiKey":"A".repeat(129)}}).to_string(),
    ] {
        assert!(
            serde_json::from_str::<credential_env::CredentialEnv>(&raw).is_err(),
            "invalid policy was admitted: {raw}"
        );
    }
    for variable in ["OPENROUTER_API_KEY", "_KEY_2", "lowercase_key"] {
        serde_json::from_value::<credential_env::CredentialEnv>(json!({
            "version":"v1","schemes":{"apiKey":variable}
        }))
        .unwrap();
    }
    let contract = contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(&contract, &selected, rust_http::native_capabilities_v3());
    let manual = credential_env::CredentialEnv::v1(
        [("apiKey".into(), "invalid-name".into())]
            .into_iter()
            .collect(),
    );
    let errors = credential_env::plan(&contract, &protocol, Some(&manual)).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "sdk-credential-env-config");
    assert_eq!(errors[0].source.pointer(), "");
    assert!(errors[0].at.end > errors[0].at.start);
}

#[test]
fn environment_defaults_are_captured_as_client_policy_not_wire_interpretation() {
    use suspect_codegen::{
        backend::{Backend, GenerationOptions, TargetConfig},
        compatibility,
    };
    let ordinary = GenerationOptions::default();
    assert_eq!(
        serde_json::to_value(&ordinary).unwrap(),
        json!({"compatibility_profiles":[]})
    );
    let configured: GenerationOptions = serde_json::from_value(json!({
        "credential_env":{"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}}
    }))
    .unwrap();
    assert!(
        serde_json::to_value(&configured)
            .unwrap()
            .get("credential_env")
            .is_some()
    );
    let contract = contract();
    let targets = [TargetConfig {
        backend: Backend::GoHttp,
        package_name: "example.com/env-sdk".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }];
    let report = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&targets, &ordinary),
        (&targets, &configured),
    )
    .unwrap();
    assert!(
        report.wire.is_empty(),
        "runtime defaults are not OpenAPI wire edits: {report:#?}"
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-credential-env-changed")
    );
    assert!(!report.is_proven_compatible());
    assert_eq!(report.after.generation, configured);
}

fn supplied(entry: &str, documents: Vec<(&str, serde_json::Value)>) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(uri, value)| {
            let uri = Uri::parse(uri).unwrap();
            ProvidedDocument::new(uri.clone(), uri, serde_json::to_vec(&value).unwrap()).unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap());
    assert!(workspace.failed_document_uris().is_empty());
    contract
}

fn auth_document(schemes: serde_json::Value, security: serde_json::Value) -> serde_json::Value {
    json!({"openapi":"3.1.2","info":{"title":"Explicit environment binding","version":"1"},
        "servers":[{"url":"https://service.env.test/v1"}],
        "components":{"securitySchemes":schemes},
        "paths":{"/value":{"get":{"operationId":"value","security":security,
            "responses":{"204":{"description":"Done"}}}}}})
}

fn bind(
    contract: &Contract,
    name: &str,
) -> Result<Option<credential_env::CredentialEnvPlan>, Vec<credential_env::HttpDiagnostic>> {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(contract, &selected, rust_http::native_capabilities_v3())
        .into_result()
        .unwrap();
    let config = credential_env::CredentialEnv::v1(
        [(name.into(), "RUNTIME_KEY".into())].into_iter().collect(),
    );
    credential_env::plan(contract, &protocol, Some(&config))
}

#[test]
fn unknown_unused_and_non_string_hooks_are_located_refusals() {
    let uri = "https://source.env.test/security.json";
    let bearer = json!({"apiKey":{"type":"http","scheme":"bearer"},
        "unused":{"type":"http","scheme":"bearer"}});
    let contract = supplied(
        uri,
        vec![(uri, auth_document(bearer, json!([{"apiKey":[]}])))],
    );
    for name in ["unknown", "unused"] {
        let errors = bind(&contract, name).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "sdk-credential-env-unbound");
        assert_eq!(errors[0].source.document().as_str(), uri);
        assert_eq!(errors[0].source.pointer(), "");
        assert!(errors[0].at.end > errors[0].at.start);
    }
    for scheme in [
        json!({"type":"http","scheme":"basic"}),
        json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.env.test/token","scopes":{}}}}),
        json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.env.test/discovery"}),
    ] {
        let contract = supplied(
            uri,
            vec![(
                uri,
                auth_document(json!({"complex":scheme}), json!([{"complex":[]}])),
            )],
        );
        let errors = bind(&contract, "complex").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "sdk-credential-env-kind");
        assert_eq!(errors[0].source.document().as_str(), uri);
        assert_eq!(
            errors[0].source.pointer(),
            "/components/securitySchemes/complex"
        );
        assert!(errors[0].at.end > errors[0].at.start);
    }
    for location in ["header", "query", "cookie"] {
        let contract = supplied(
            uri,
            vec![(
                uri,
                auth_document(
                    json!({"bearer":{"type":"apiKey","in":location,"name":"X-Source-Key"}}),
                    json!([{"bearer":[]}]),
                ),
            )],
        );
        let plan = bind(&contract, "bearer").unwrap().unwrap();
        assert_eq!(
            plan.bindings()[0].kind(),
            credential_env::CredentialEnvKind::ApiKey,
            "source type, not the configured name, determines the hook"
        );
    }
}

#[test]
fn empty_operation_capture_cannot_skip_configured_native_admission() {
    use suspect_codegen::{
        backend::{self, Backend, GenerationOptions, TargetConfig},
        compatibility::{self, PlanStatus},
    };
    let uri = "https://source.env.test/empty.json";
    let mut document = auth_document(
        json!({"apiKey":{"type":"http","scheme":"bearer"}}),
        json!([{"apiKey":[]}]),
    );
    document["paths"] = json!({});
    let contract = supplied(uri, vec![(uri, document)]);
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "env-client".into(),
        package_version: "0.1.0".into(),
        import_name: Some("env_client".into()),
    };
    let options = GenerationOptions {
        credential_env: Some(credential_env::CredentialEnv::v1(
            [("apiKey".into(), "RUNTIME_KEY".into())]
                .into_iter()
                .collect(),
        )),
        ..Default::default()
    };
    let errors =
        backend::generate_with_options(contract.clone(), &[], &target, &options).unwrap_err();
    assert!(!errors.is_empty());
    let snapshot = compatibility::snapshot_with_options(
        contract.clone(),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(
        snapshot.native[0].status,
        PlanStatus::Unavailable,
        "configured capture must retain the actual native refusal: {errors:?}"
    );
    for error in errors {
        assert!(
            snapshot.native[0]
                .findings
                .iter()
                .any(|finding| finding.code == error.code)
        );
    }
    assert!(snapshot.native[0].credential_env.is_none());
    let ordinary = compatibility::snapshot(contract, &[], &[target]).unwrap();
    assert_eq!(ordinary.native[0].status, PlanStatus::EmptySelection);
}

#[test]
fn referenced_schemes_keep_physical_provenance_without_relocation_in_semantic_equality() {
    let mut descriptors = Vec::new();
    let mut declarations = Vec::new();
    for entry in [
        "https://before.env.test/openapi.json",
        "https://after.env.test/openapi.json",
    ] {
        let definition = "https://definitions.env.test/credentials.json";
        let document = auth_document(
            json!({"apiKey":{"$ref":format!("{definition}#/Bearer")}}),
            json!([{"apiKey":[]}]),
        );
        let contract = supplied(
            entry,
            vec![
                (entry, document),
                (
                    definition,
                    json!({"Bearer":{"type":"http","scheme":"bearer"}}),
                ),
            ],
        );
        let plan = bind(&contract, "apiKey").unwrap().unwrap();
        let scheme = plan.bindings()[0].scheme();
        assert_eq!(scheme.use_site().source().document().as_str(), entry);
        assert_eq!(
            scheme.use_site().source().pointer(),
            "/components/securitySchemes/apiKey"
        );
        assert_eq!(scheme.terminal().source().document().as_str(), definition);
        assert_eq!(scheme.terminal().source().pointer(), "/Bearer");
        assert!(!scheme.references().is_empty());
        assert!(!scheme.terminal().span().is_empty());
        descriptors.push(plan.semantic_descriptor());
        declarations.push(scheme.use_site().source().clone());
    }
    assert_ne!(declarations[0], declarations[1]);
    assert_eq!(descriptors[0], descriptors[1]);
}

#[test]
fn same_name_in_distinct_requirement_documents_is_ambiguous_even_with_one_terminal() {
    let entry = "https://source.env.test/openapi.json";
    let other = "https://source.env.test/other.json";
    let common = "https://source.env.test/common.json";
    let scheme = json!({"apiKey":{"$ref":"common.json#/Bearer"}});
    let mut first = auth_document(scheme.clone(), json!([{"apiKey":[]}]));
    first["paths"]["/remote"] = json!({"$ref":"other.json#/paths/~1remote"});
    let second = json!({"openapi":"3.1.2","info":{"title":"Other declarations","version":"1"},
        "servers":[{"url":"https://service.env.test/v1"}],
        "components":{"securitySchemes":scheme},
        "paths":{"/remote":{"get":{"operationId":"remote","security":[{"apiKey":[]}],
            "responses":{"204":{"description":"Done"}}}}}});
    let contract = supplied(
        entry,
        vec![
            (entry, first),
            (other, second),
            (common, json!({"Bearer":{"type":"http","scheme":"bearer"}})),
        ],
    );
    let errors = bind(&contract, "apiKey").unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "sdk-credential-env-ambiguous");
    assert_eq!(
        errors[0].source.pointer(),
        "/components/securitySchemes/apiKey"
    );
    assert!(errors[0].at.end > errors[0].at.start);
}

#[test]
fn policy_edits_cannot_reuse_ordinary_target_artifacts_and_reverts_reuse_the_original_arc() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        generation_session::{Session, SessionConfig, SessionError},
    };
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("api.json");
    std::fs::write(
        &entry,
        auth_document(
            json!({"apiKey":{"type":"http","scheme":"bearer"}}),
            json!([{"apiKey":[]}]),
        )
        .to_string(),
    )
    .unwrap();
    let config = SessionConfig {
        targets: vec![TargetConfig {
            backend: Backend::GoHttp,
            package_name: "example.com/env-cache".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        }],
        ..Default::default()
    };
    let mut session = Session::new(&entry, config.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.delta.compiles, 1);
    assert_eq!(first.delta.renders, 1);
    let mut changed = config.clone();
    changed.generation.credential_env = Some(credential_env::CredentialEnv::v1(
        [("not-a-source-scheme".into(), "EXPLICIT_VARIABLE".into())]
            .into_iter()
            .collect(),
    ));
    session.set_config(changed).unwrap();
    let errors = match session.generate() {
        Err(SessionError::Backend(errors)) => errors,
        other => {
            panic!("changed environment policy reused or emitted an invalid package: {other:?}")
        }
    };
    assert!(
        errors
            .iter()
            .any(|error| error.code.starts_with("sdk-credential-env-"))
    );
    session.set_config(config).unwrap();
    let restored = session.generate().unwrap();
    assert_eq!(restored.revision, first.revision);
    assert_eq!(restored.delta.compiles, 0);
    assert_eq!(restored.delta.renders, 0);
    assert_eq!(restored.delta.cache_hits, 1);
    assert!(Arc::ptr_eq(&first.files, &restored.files));
    assert!(Arc::ptr_eq(&first.contract, &restored.contract));
}

#[test]
fn verified_native_defaults_share_generation_capture_and_policy_edit_cache_identity() {
    use suspect_codegen::{
        backend::{self, Backend, TargetConfig},
        compatibility::{self, PlanStatus},
        generation_session::{Session, SessionConfig},
    };
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("env.json");
    let mut document = auth_document(
        json!({"apiKey":{"type":"http","scheme":"bearer"}}),
        json!([{"apiKey":[]}]),
    );
    document["paths"]["/value"]["get"]["responses"] = json!({"200":{
        "description":"Decoded value","content":{"application/json":{
            "schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}},
            "example":{"ok":true}
        }}
    }});
    std::fs::write(&entry, document.to_string()).unwrap();
    let specs = [
        (Backend::TypescriptHttp, "env-client", ""),
        (Backend::RustHttp, "env-client", ""),
        (Backend::PythonHttp, "env-client", "env_client"),
        (Backend::GoHttp, "example.com/env-client", ""),
        (Backend::SwiftHttp, "EnvClient", "EnvClient"),
        #[cfg(feature = "java-sdk")]
        (Backend::JavaHttp, "test.env:env-client", "test.env.sdk"),
        #[cfg(feature = "php-sdk")]
        (Backend::PhpHttp, "test/env-client", "EnvClient"),
        #[cfg(feature = "dart-sdk")]
        (Backend::DartHttp, "env_client", ""),
        #[cfg(feature = "ruby-sdk")]
        (Backend::RubyHttp, "env_client", "EnvClient"),
        #[cfg(feature = "csharp-sdk")]
        (Backend::CsharpHttp, "Env.Client", "EnvClient"),
        #[cfg(feature = "cpp-sdk")]
        (Backend::CppHttp, "env_client", "env_client"),
        #[cfg(feature = "kotlin-sdk")]
        (
            Backend::KotlinHttp,
            "test.env:env-client",
            "test.env.kotlin",
        ),
    ];
    assert_eq!(specs.len(), Backend::ALL.len());
    let targets = specs
        .into_iter()
        .map(|(backend, package, import)| TargetConfig {
            backend,
            package_name: package.into(),
            package_version: "0.1.0".into(),
            import_name: (!import.is_empty()).then(|| import.into()),
        })
        .collect::<Vec<_>>();
    let ordinary = SessionConfig {
        targets: targets.clone(),
        ..Default::default()
    };
    let mut session = Session::new(&entry, ordinary.clone()).unwrap();
    let before = session.generate().unwrap();
    let mut configured = ordinary.clone();
    configured.generation.credential_env = Some(credential_env::CredentialEnv::v1(
        [("apiKey".into(), "ENV_CLIENT_A".into())]
            .into_iter()
            .collect(),
    ));
    session.set_config(configured.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.delta.compiles, 0);
    assert_eq!(first.delta.renders, targets.len());
    assert_ne!(before.revision, first.revision);
    assert!(Arc::ptr_eq(&before.contract, &first.contract));
    let selected = first
        .contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let mut direct = Vec::new();
    for target in &targets {
        direct.extend(
            backend::generate_with_options(
                first.contract.clone(),
                &selected,
                target,
                &configured.generation,
            )
            .unwrap(),
        );
    }
    direct.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(first.files.as_ref(), &direct);
    let snapshot = compatibility::snapshot_with_options(
        first.contract.clone(),
        &[],
        &targets,
        &configured.generation,
    )
    .unwrap();
    assert_eq!(snapshot.native.len(), targets.len());
    for native in &snapshot.native {
        assert_eq!(
            native.status,
            PlanStatus::Planned,
            "{}: {:?}",
            native.target.backend.name(),
            native.findings
        );
        let bound = native
            .credential_env
            .as_ref()
            .expect("actual native plan retains environment policy");
        assert_eq!(bound.bindings.len(), 1);
        assert_eq!(bound.bindings[0].name, "apiKey");
        assert_eq!(bound.bindings[0].variable, "ENV_CLIENT_A");
        assert_eq!(
            bound.bindings[0].kind,
            credential_env::CredentialEnvKind::Bearer
        );
    }
    let mut changed = configured.clone();
    changed
        .generation
        .credential_env
        .as_mut()
        .unwrap()
        .schemes
        .insert("apiKey".into(), "ENV_CLIENT_B".into());
    session.set_config(changed.clone()).unwrap();
    let second = session.generate().unwrap();
    assert_eq!(second.delta.compiles, 0);
    assert_eq!(second.delta.renders, targets.len());
    assert_ne!(second.revision, first.revision);
    assert_ne!(second.files, first.files);
    let report = compatibility::compare_with_options(
        first.contract.clone(),
        first.contract.clone(),
        &[],
        (&targets, &configured.generation),
        (&targets, &changed.generation),
    )
    .unwrap();
    assert!(report.wire.is_empty());
    assert!(report.native.iter().all(|native| {
        native
            .changes
            .iter()
            .any(|change| change.code == "native-credential-env-changed")
    }));
    session.set_config(configured).unwrap();
    let restored = session.generate().unwrap();
    assert_eq!(restored.delta.compiles, 0);
    assert_eq!(restored.delta.renders, 0);
    assert_eq!(restored.delta.cache_hits, 1);
    assert!(Arc::ptr_eq(&restored.files, &first.files));
    session.set_config(ordinary).unwrap();
    assert!(Arc::ptr_eq(
        &session.generate().unwrap().files,
        &before.files
    ));
}
