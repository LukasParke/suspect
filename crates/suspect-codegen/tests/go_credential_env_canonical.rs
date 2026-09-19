//! Go-only canonical capture after the native environment policy gate is open.
use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use suspect_codegen::{
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, PlanStatus},
    credential_env::CredentialEnv,
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract(entry: &str) -> Arc<Contract> {
    let definition = "https://go-env.example.test/security.json";
    let documents = [
        (
            entry,
            json!({
                "openapi":"3.1.2", "info":{"title":"Go canonical environment","version":"1"},
                "servers":[{"url":"https://go-env.example.test/api/v1"}],
                "components":{"securitySchemes":{"apiKey":{"$ref":format!("{definition}#/Bearer")}}},
                "paths":{"/check":{"get":{"operationId":"check","security":[{"apiKey":[]}],
                    "responses":{"200":{"description":"Checked response","content":{"application/json":{
                        "schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}
                    }}}}
                }}}
            }),
        ),
        (
            definition,
            json!({"Bearer":{"type":"http","scheme":"bearer"}}),
        ),
    ];
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
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap())
}

#[test]
fn canonical_go_environment_capture_retains_bound_semantics_and_fingerprinted_factory() {
    let target = TargetConfig {
        backend: Backend::GoHttp,
        package_name: "example.com/go-env-canonical".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    };
    let options = GenerationOptions {
        credential_env: Some(CredentialEnv::v1(
            [("apiKey".into(), "GO_CANONICAL_TOKEN".into())]
                .into_iter()
                .collect(),
        )),
        ..Default::default()
    };
    let mut descriptors = Vec::new();
    for (case, entry) in [
        ("before", "https://before.go-env.example.test/openapi.json"),
        (
            "relocated",
            "https://after.go-env.example.test/openapi.json",
        ),
    ] {
        let contract = contract(entry);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let plan = go_http::plan_http(
            contract.clone(),
            &selected,
            go_http::HttpConfig {
                credential_env: options.credential_env.clone(),
                attribution: Some(suspect_codegen::attribution::AttributionDescriptor::plan(
                    env!("CARGO_PKG_VERSION"),
                    &target.package_name,
                    &target.package_version,
                    contract.openapi_version(),
                    Backend::GoHttp.language_tag(),
                )),
                ..Default::default()
            },
        )
        .unwrap();
        let bound = plan.credential_env().unwrap();
        let scheme = bound.bindings()[0].scheme();
        assert_eq!(scheme.use_site().source().document().as_str(), entry);
        assert_eq!(
            scheme.use_site().source().pointer(),
            "/components/securitySchemes/apiKey"
        );
        assert_eq!(scheme.terminal().source().pointer(), "/Bearer");
        assert!(!scheme.references().is_empty());
        assert!(!scheme.use_site().span().is_empty());
        assert_eq!(plan.credential_env_factory(), Some("NewClientFromEnv"));

        let files =
            backend::generate_with_options(contract.clone(), &selected, &target, &options).unwrap();
        assert_eq!(
            files,
            go_http::emit_http(
                &plan,
                &go_http::PackageConfig {
                    module_path: target.package_name.clone(),
                    package_name: "sdk".into(),
                    version: target.package_version.clone(),
                },
            )
            .unwrap()
        );
        let snapshot = compatibility::snapshot_with_options(
            contract.clone(),
            &[],
            std::slice::from_ref(&target),
            &options,
        )
        .unwrap();
        let native = &snapshot.native[0];
        assert_eq!(native.status, PlanStatus::Planned);
        assert!(native.findings.is_empty(), "{:?}", native.findings);
        assert_eq!(native.operations.len(), selected.len());
        assert_eq!(native.generation, options);
        assert_eq!(
            native.credential_env.as_ref(),
            Some(&bound.semantic_descriptor())
        );
        assert!(
            native
                .runtime
                .fingerprinted_assets
                .iter()
                .any(|asset| asset == "go_http/credential_env.rs")
        );
        let descriptor = serde_json::to_value(native.credential_env.as_ref().unwrap()).unwrap();
        assert_eq!(
            descriptor,
            json!({"version":"v1","bindings":[{
                "name":"apiKey","variable":"GO_CANONICAL_TOKEN","kind":"bearer"
            }]})
        );

        let ordinary =
            compatibility::snapshot(contract, &[], std::slice::from_ref(&target)).unwrap();
        assert_eq!(ordinary.native[0].status, PlanStatus::Planned);
        assert!(ordinary.native[0].credential_env.is_none());
        let mut without_environment_factory = native.operations.clone();
        for operation in &mut without_environment_factory {
            assert_eq!(
                operation.symbols.remove("env-factory").as_deref(),
                plan.credential_env_factory()
            );
            let client = operation.descriptor["constructor"]
                .as_object_mut()
                .unwrap()
                .remove("client")
                .unwrap();
            assert_eq!(client["environmentFactory"]["name"], "NewClientFromEnv");
        }
        assert_eq!(ordinary.native[0].operations, without_environment_factory);
        assert_eq!(ordinary.native[0].models, native.models);
        let report = compatibility::compare_snapshots(&ordinary, &snapshot);
        assert!(report.wire.is_empty(), "{:#?}", report.wire);
        assert_eq!(report.native[0].changes.len(), 3);
        for code in [
            "native-credential-env-changed",
            "native-operation-symbol-changed",
            "native-input-constructor-changed",
        ] {
            assert!(
                report.native[0]
                    .changes
                    .iter()
                    .any(|change| change.code == code)
            );
        }
        assert!(!report.is_proven_compatible());

        if let Some(root) = std::env::var_os("SUSPECT_GO_CANONICAL_ENV_EVIDENCE") {
            let root = PathBuf::from(root).join(case);
            std::fs::create_dir_all(root.parent().unwrap()).unwrap();
            std::fs::create_dir(&root)
                .expect("use fresh canonical evidence; preserve prior receipts");
            suspect_codegen::write_files(&files, &root).unwrap();
            std::fs::write(
                root.join("capture.json"),
                serde_json::to_vec_pretty(native).unwrap(),
            )
            .unwrap();
            std::fs::write(
                root.join("report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            eprintln!("Go canonical env capture evidence: {}", root.display());
        }
        descriptors.push(descriptor);
    }
    assert_eq!(descriptors[0], descriptors[1]);
}
