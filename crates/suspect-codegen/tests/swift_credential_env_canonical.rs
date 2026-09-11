//! Swift-only closure of canonical forwarding/capture/Session after native approval.
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, PlanStatus},
    credential_env::CredentialEnv,
    generation_session::{Session, SessionConfig, SessionError},
    swift_sdk::{PackageConfig, SwiftConfig, emit_sdk, plan_sdk},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> serde_json::Value {
    let response = json!({"description":"Key status","content":{"application/json":{"schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]},"example":{"ok":true}}}});
    json!({"openapi":"3.1.0","info":{"title":"Canonical Swift environment policy","version":"1"},
        "servers":[{"url":"https://canonical.swift.test/api/v1"}],
        "components":{"securitySchemes":{"apiKey":{"$ref":"#/components/securitySchemes/bearer"},"bearer":{"type":"http","scheme":"bearer"}}},
        "paths":{"/key":{"get":{"operationId":"getCurrentKey","security":[{"apiKey":[]}],"responses":{"200":response}}}}})
}

fn contract(uri: &str) -> Arc<Contract> {
    let entry = Uri::parse(uri).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            document().to_string().into_bytes(),
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

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::SwiftHttp,
        package_name: "SwiftCanonicalEnv".into(),
        package_version: "1.0.0".into(),
        import_name: Some("SwiftCanonicalEnv".into()),
    }
}

fn generation(variable: &str) -> GenerationOptions {
    GenerationOptions {
        credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
            "apiKey".into(),
            variable.into(),
        )]))),
        ..Default::default()
    }
}

#[test]
fn canonical_generation_and_capture_match_the_bound_swift_plan() {
    let target = target();
    let source = contract("https://canonical.swift.test/source.json");
    let options = generation("SWIFT_CANONICAL_KEY");
    let plan = plan_sdk(
        source.clone(),
        &selected(&source),
        SwiftConfig {
            credential_env: options.credential_env.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let package = PackageConfig {
        name: target.package_name.clone(),
        module_name: target.import_name.clone().unwrap(),
        version: target.package_version.clone(),
    };
    let expected = emit_sdk(&plan, &package)
        .unwrap()
        .into_iter()
        .map(|file| OutFile {
            path: format!("swift/{}", file.path),
            content: file.content,
        })
        .collect::<Vec<_>>();
    let actual =
        backend::generate_with_options(source.clone(), &selected(&source), &target, &options)
            .unwrap();
    assert_eq!(actual, expected);

    let snapshot = compatibility::snapshot_with_options(
        source.clone(),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(snapshot.native.len(), 1);
    assert_eq!(
        snapshot.native[0].status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.native[0].findings
    );
    let descriptor = plan.credential_env().unwrap().semantic_descriptor();
    assert_eq!(
        snapshot.native[0].credential_env.as_ref(),
        Some(&descriptor)
    );
    assert!(
        !serde_json::to_string(&descriptor)
            .unwrap()
            .contains("https://")
    );

    let relocated = compatibility::snapshot_with_options(
        contract("https://relocated.swift.test/source.json"),
        &[],
        std::slice::from_ref(&target),
        &options,
    )
    .unwrap();
    assert_eq!(relocated.native[0].status, PlanStatus::Planned);
    assert_eq!(
        relocated.native[0].credential_env,
        snapshot.native[0].credential_env
    );

    let ordinary =
        compatibility::snapshot(source.clone(), &[], std::slice::from_ref(&target)).unwrap();
    assert_eq!(ordinary.native[0].status, PlanStatus::Planned);
    assert!(ordinary.native[0].credential_env.is_none());
    assert!(
        serde_json::to_value(&ordinary.native[0])
            .unwrap()
            .get("credential_env")
            .is_none()
    );
    assert_eq!(
        backend::generate(source.clone(), &selected(&source), &target).unwrap(),
        backend::generate_with_options(
            source.clone(),
            &selected(&source),
            &target,
            &GenerationOptions::default()
        )
        .unwrap()
    );
}

#[test]
fn canonical_session_policy_edits_reverts_and_refusals_stay_isolated() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("api.json");
    std::fs::write(&input, document().to_string()).unwrap();
    let target = target();
    let ordinary = SessionConfig {
        targets: vec![target.clone()],
        ..Default::default()
    };
    let mut session = Session::new(&input, ordinary.clone()).unwrap();
    let before = session.generate().unwrap();
    let configured = SessionConfig {
        generation: generation("SWIFT_CANONICAL_KEY"),
        ..ordinary.clone()
    };
    session.set_config(configured.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.delta.compiles, 0);
    assert_eq!(first.delta.renders, 1);
    assert!(Arc::ptr_eq(&before.contract, &first.contract));
    assert_ne!(before.revision, first.revision);
    assert_eq!(
        first.files.as_ref(),
        &backend::generate_with_options(
            first.contract.clone(),
            &selected(&first.contract),
            &target,
            &configured.generation
        )
        .unwrap()
    );

    let changed = SessionConfig {
        generation: generation("SWIFT_CANONICAL_KEY_CHANGED"),
        ..configured.clone()
    };
    session.set_config(changed.clone()).unwrap();
    let second = session.generate().unwrap();
    assert_eq!(second.delta.compiles, 0);
    assert_eq!(second.delta.renders, 1);
    assert_ne!(second.revision, first.revision);
    assert_ne!(second.files, first.files);
    let report = compatibility::compare_with_options(
        first.contract.clone(),
        first.contract.clone(),
        &[],
        (std::slice::from_ref(&target), &configured.generation),
        (std::slice::from_ref(&target), &changed.generation),
    )
    .unwrap();
    assert!(report.wire.is_empty());
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-credential-env-changed")
    );
    assert!(
        !report.native[0]
            .changes
            .iter()
            .any(|change| change.code.contains("wire-interpretation"))
    );

    let invalid = SessionConfig {
        generation: GenerationOptions {
            credential_env: Some(CredentialEnv::v1(BTreeMap::from([(
                "unused".into(),
                "UNUSED_ENV".into(),
            )]))),
            ..Default::default()
        },
        ..configured.clone()
    };
    session.set_config(invalid).unwrap();
    let Err(SessionError::Backend(errors)) = session.generate() else {
        panic!("invalid binding must not reuse a valid cached SDK")
    };
    assert!(
        errors
            .iter()
            .any(|error| error.code == "sdk-credential-env-unbound")
    );

    session.set_config(configured).unwrap();
    let restored = session.generate().unwrap();
    assert_eq!(restored.revision, first.revision);
    assert_eq!(restored.delta.compiles, 0);
    assert_eq!(restored.delta.renders, 0);
    assert_eq!(restored.delta.cache_hits, 1);
    assert!(Arc::ptr_eq(&restored.files, &first.files));
    session.set_config(ordinary).unwrap();
    let restored = session.generate().unwrap();
    assert_eq!(restored.revision, before.revision);
    assert!(Arc::ptr_eq(&restored.files, &before.files));
}
