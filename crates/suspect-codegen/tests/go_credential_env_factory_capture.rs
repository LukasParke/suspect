//! An additive source model can displace an existing package-level factory.
use std::{path::PathBuf, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, CompatibilitySnapshot, Impact, PlanStatus},
    credential_env::CredentialEnv,
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document(collision: bool) -> Value {
    let mut document = json!({
        "openapi":"3.1.2", "info":{"title":"Factory compatibility","version":"1"},
        "servers":[{"url":"https://source.example.test/v1"}],
        "security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
        "paths":{"/stable":{"get":{"operationId":"stable","responses":{"204":{"description":"Done"}}}}}
    });
    if collision {
        document["components"]["schemas"]["NewClientFromEnv"] = json!({"type":"string"});
        document["paths"]["/added"] = json!({"get":{"operationId":"added","responses":{
            "200":{"description":"Added","content":{"application/json":{
                "schema":{"$ref":"#/components/schemas/NewClientFromEnv"}
            }}}
        }}});
    }
    document
}

fn capture(
    collision: bool,
    environment: bool,
    operation_ids: &[String],
    evidence_label: Option<&str>,
) -> (CompatibilitySnapshot, Option<String>) {
    let uri = Uri::parse(if collision {
        "https://after.go-factory.example.test/openapi.json"
    } else {
        "https://before.go-factory.example.test/openapi.json"
    })
    .unwrap();
    let input = document(collision);
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&input).unwrap(),
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
    let contract = Arc::new(Contract::from_workspace(&workspace, &uri).unwrap());
    let selected = contract
        .operations()
        .filter(|operation| {
            operation_ids.is_empty()
                || operation_ids
                    .iter()
                    .any(|id| operation.operation_id() == Some(id.as_str()))
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let options = GenerationOptions {
        credential_env: environment.then(|| {
            CredentialEnv::v1(
                [("apiKey".into(), "REVIEW_CREDENTIAL".into())]
                    .into_iter()
                    .collect(),
            )
        }),
        ..Default::default()
    };
    let target = TargetConfig {
        backend: Backend::GoHttp,
        package_name: "example.com/env-review".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    };
    let plan = go_http::plan_http(
        contract.clone(),
        &selected,
        go_http::HttpConfig {
            credential_env: options.credential_env.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let factory = plan.credential_env_factory().map(str::to_owned);
    let snapshot = compatibility::snapshot_with_options(
        contract.clone(),
        operation_ids,
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(snapshot.native[0].status, PlanStatus::Planned);
    assert!(snapshot.native[0].findings.is_empty());
    if let (Some(base), Some(label)) = (
        std::env::var_os("SUSPECT_GO_FACTORY_CAPTURE_EVIDENCE"),
        evidence_label,
    ) {
        let base = PathBuf::from(base);
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join(label);
        std::fs::create_dir(&path).expect("use fresh evidence; preserve prior captures");
        let files = backend::generate_with_options(contract, &selected, &target, &options).unwrap();
        suspect_codegen::write_files(&files, &path).unwrap();
        std::fs::write(
            path.join("input.json"),
            serde_json::to_vec_pretty(&input).unwrap(),
        )
        .unwrap();
        std::fs::write(
            path.join("native.json"),
            serde_json::to_vec_pretty(&snapshot.native[0]).unwrap(),
        )
        .unwrap();
    }
    (snapshot, factory)
}

#[test]
fn additive_model_collision_reports_source_corresponding_environment_factory_break() {
    let (before, old_factory) = capture(false, true, &[], Some("before-env"));
    let (after, new_factory) = capture(true, true, &[], Some("after-env"));
    assert_eq!(old_factory.as_deref(), Some("NewClientFromEnv"));
    assert_eq!(new_factory.as_deref(), Some("NewClientFromEnv2"));
    assert_eq!(
        before.native[0].credential_env,
        after.native[0].credential_env
    );

    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report
            .wire
            .iter()
            .all(|change| change.impact == Impact::Compatible)
    );
    let displaced = report.native[0].changes.iter().find(|change| {
        change.code == "native-operation-symbol-changed"
            && change.subject == "env-factory"
            && change.impact == Impact::Breaking
    }).unwrap_or_else(|| panic!("the existing factory was displaced, but capture omitted its native break: {report:#?}"));
    assert_eq!(displaced.before, Some(json!(old_factory)));
    assert_eq!(displaced.after, Some(json!(new_factory)));
    assert_eq!(displaced.operation_id_before.as_deref(), Some("stable"));
    assert_eq!(displaced.operation_id_after.as_deref(), Some("stable"));
    for (source, document) in [
        (
            displaced.source_before.as_ref().unwrap(),
            "https://before.go-factory.example.test/openapi.json",
        ),
        (
            displaced.source_after.as_ref().unwrap(),
            "https://after.go-factory.example.test/openapi.json",
        ),
    ] {
        assert_eq!(source.document, document);
        assert_eq!(source.pointer, "/paths/~1stable/get");
        let span = source.span.as_ref().unwrap();
        assert!(span.end > span.start);
    }
    assert!(!report.is_proven_compatible());
    for (snapshot, name) in [(&before, "NewClientFromEnv"), (&after, "NewClientFromEnv2")] {
        for operation in &snapshot.native[0].operations {
            assert_eq!(operation.symbols["env-factory"], name);
            assert_eq!(
                operation.descriptor["constructor"]["client"]["environmentFactory"],
                json!({
                    "kind":"function", "name":name,
                    "parameters":[{"name":"options","type":"ClientOptions","variadic":true}],
                    "returns":["*Client","error"], "acceptedOptions":{"minimum":0,"maximum":1}
                })
            );
        }
    }
}

#[test]
fn unselected_collision_and_absent_policy_keep_the_existing_surface_compatible() {
    for (environment, selected, prefix) in [
        (false, Vec::new(), "none"),
        (true, vec!["stable".into()], "unselected"),
    ] {
        let (before, old_factory) = capture(
            false,
            environment,
            &selected,
            Some(&format!("before-{prefix}")),
        );
        let (after, new_factory) = capture(
            true,
            environment,
            &selected,
            Some(&format!("after-{prefix}")),
        );
        assert_eq!(old_factory, new_factory);
        let report = compatibility::compare_snapshots(&before, &after);
        assert!(report.is_proven_compatible(), "{report:#?}");
        if !environment {
            for snapshot in [&before, &after] {
                assert!(snapshot.native[0].credential_env.is_none());
                for operation in &snapshot.native[0].operations {
                    assert!(!operation.symbols.contains_key("env-factory"));
                    assert!(operation.descriptor["constructor"].get("client").is_none());
                }
            }
        }
    }
}

#[test]
fn environment_factory_signature_is_part_of_native_constructor_comparison() {
    let (before, _) = capture(false, true, &[], None);
    let (mut changed, _) = capture(false, true, &[], None);
    let signature = &mut changed.native[0].operations[0].descriptor["constructor"]["client"]["environmentFactory"];
    assert_eq!(signature["parameters"][0]["variadic"], true);
    signature["parameters"][0]["variadic"] = json!(false);
    let report = compatibility::compare_snapshots(&before, &changed);
    assert!(report.wire.is_empty());
    assert!(report.native[0].changes.iter().any(|change| {
        change.code == "native-input-constructor-changed"
            && change.impact == Impact::PotentiallyBreaking
            && change.source_before.is_some()
            && change.source_after.is_some()
    }));
    assert!(!report.is_proven_compatible());
}
