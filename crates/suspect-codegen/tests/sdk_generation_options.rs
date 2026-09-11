//! Canonical configuration must select the same semantics in generation,
//! cached sessions and typed compatibility capture.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use suspect_codegen::{
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, Impact, PlanStatus},
    generation_session::{Session, SessionConfig, SessionError},
    http_protocol::CompatibilityProfile,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn document() -> Value {
    json!({"openapi":"3.1.2","info":{"title":"Wire formats","version":"api-version"},
        "servers":[{"url":"https://example.test/v1"}],"security":[],"paths":{
        "/blob":{"get":{"operationId":"downloadBlob","responses":{"200":{"description":"Raw bytes",
            "content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}},
        "/json":{"get":{"operationId":"jsonValue","responses":{"200":{"description":"A JSON string",
            "content":{"application/json":{"schema":{"type":"string","format":"binary"}}}}}}},
        "/health":{"get":{"responses":{"204":{"description":"No payload"}}}}
    }})
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

fn options() -> GenerationOptions {
    GenerationOptions {
        compatibility_profiles: [CompatibilityProfile::LegacyBinaryStringV1]
            .into_iter()
            .collect(),
        ..Default::default()
    }
}

fn target(backend: Backend) -> TargetConfig {
    TargetConfig {
        backend,
        package_name: match backend.name() {
            "typescript-http" => "@fixture/profile-sdk",
            "go-http" => "example.test/profile-sdk",
            "swift-http" => "ProfileSdk",
            "csharp-http" => "Fixture.ProfileSdk",
            "java-http" => "test.fixture:profile-sdk",
            "kotlin-http" => "test.fixture:profile-sdk",
            "php-http" => "fixture/profile-sdk",
            "dart-http" => "profile_sdk",
            "cpp-http" => "profile_sdk",
            _ => "profile-sdk",
        }
        .into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

#[test]
fn canonical_profiles_require_explicit_legacy_interpretation_and_preserve_json_models() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    let bytes = serde_json::to_vec_pretty(&document()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let contract = load(&path);
    let source = |id: &str| {
        contract
            .operations()
            .find(|op| op.operation_id() == Some(id))
            .unwrap()
            .source()
            .clone()
    };
    for backend in Backend::ALL {
        let config = target(*backend);
        let errors =
            backend::generate(contract.clone(), &[source("downloadBlob")], &config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.source.as_ref().is_some_and(|source| source
                    .pointer()
                    .starts_with(
                        "/paths/~1blob/get/responses/200/content/application~1octet-stream"
                    ))),
            "{}: {errors:?}",
            backend.name()
        );
        let enabled = backend::generate_with_options(
            contract.clone(),
            &[source("downloadBlob")],
            &config,
            &options(),
        )
        .unwrap_or_else(|errors| panic!("{}: {errors:?}", backend.name()));
        assert!(enabled.iter().any(|file| file.path.ends_with("README.md")));
        let json = compatibility::snapshot_with_options(
            contract.clone(),
            &["jsonValue".into()],
            std::slice::from_ref(&config),
            &options(),
        )
        .unwrap();
        assert_eq!(
            json.native[0].status,
            PlanStatus::Planned,
            "{}: {:?}",
            backend.name(),
            json.native[0].findings
        );
        assert!(
            !json.native[0].models.is_empty(),
            "JSON must keep real models in {}",
            backend.name()
        );
        let native = compatibility::snapshot_with_options(
            contract.clone(),
            &["downloadBlob".into()],
            &[config],
            &options(),
        )
        .unwrap();
        assert_eq!(
            native.native[0].status,
            PlanStatus::Planned,
            "{}: {:?}",
            backend.name(),
            native.native[0].findings
        );
        assert_eq!(native.native[0].generation, options());
    }
    let native = compatibility::snapshot_with_options(
        contract,
        &["downloadBlob".into()],
        &[target(Backend::GoHttp)],
        &options(),
    )
    .unwrap();
    assert_eq!(
        native.native[0].operations[0].descriptor["responses"][0]["model"],
        "[]byte"
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn interpretation_changes_invalidate_target_caches_and_cached_reverts_keep_their_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let settings = SessionConfig {
        targets: vec![target(Backend::GoHttp)],
        operation_ids: vec!["jsonValue".into()],
        ..Default::default()
    };
    let mut session = Session::new(&path, settings.clone()).unwrap();
    let first = session.generate().unwrap();
    let mut enabled = settings.clone();
    enabled.generation = options();
    session.set_config(enabled.clone()).unwrap();
    let changed = session.generate().unwrap();
    assert!(Arc::ptr_eq(&first.contract, &changed.contract));
    assert_eq!(changed.delta.compiles, 0);
    assert_eq!(
        changed.delta.renders, 1,
        "interpretation is part of the target cache key"
    );
    assert_ne!(changed.revision, first.revision);
    assert_ne!(
        changed.files, first.files,
        "the emitted protocol must record the explicit choice"
    );
    assert_eq!(session.generate().unwrap().delta.renders, 0);
    session.set_config(settings).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.revision, first.revision);
    assert_eq!(reverted.delta.renders, 0);
    assert!(Arc::ptr_eq(&first.files, &reverted.files));
    enabled.operation_ids = vec!["downloadBlob".into()];
    session.set_config(enabled.clone()).unwrap();
    session.generate().unwrap();
    enabled.generation = GenerationOptions::default();
    session.set_config(enabled).unwrap();
    assert!(
        matches!(session.generate(), Err(SessionError::Backend(_))),
        "removing the opt-in cannot reuse an admitted byte package"
    );
}

#[test]
fn interpretation_changes_are_reported_even_when_native_json_types_stay_equal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let contract = load(&path);
    let targets = [target(Backend::GoHttp)];
    let before =
        compatibility::snapshot(contract.clone(), &["jsonValue".into()], &targets).unwrap();
    let after = compatibility::snapshot_with_options(
        contract.clone(),
        &["jsonValue".into()],
        &targets,
        &options(),
    )
    .unwrap();
    assert_eq!(before.native[0].operations, after.native[0].operations);
    let changed = compatibility::compare_snapshots(&before, &after);
    assert!(!changed.is_proven_compatible());
    assert!(changed.wire.iter().any(|finding| finding.code
        == "wire-interpretation-profile-changed"
        && finding.impact == Impact::Unknown));
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|finding| finding.code == "native-interpretation-profile-changed")
    );
    let stable = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &["jsonValue".into()],
        (&targets, &options()),
        (&targets, &options()),
    )
    .unwrap();
    assert!(stable.is_proven_compatible(), "{stable:?}");
}

#[test]
fn profile_names_are_closed_versioned_and_ordinary_configuration_is_empty() {
    let ordinary: GenerationOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(ordinary, GenerationOptions::default());
    assert_eq!(
        serde_json::from_value::<GenerationOptions>(
            json!({"compatibility_profiles":["legacy-binary-string-v1"]})
        )
        .unwrap(),
        options()
    );
    for invalid in [
        json!({"compatibility_profiles":["legacy-binary-string-v2"]}),
        json!({"compatibility_profiles":["binary"]}),
        json!({"implicit":true}),
    ] {
        assert!(serde_json::from_value::<GenerationOptions>(invalid).is_err());
    }
    for profile in CompatibilityProfile::ALL {
        assert_eq!(serde_json::to_value(profile).unwrap(), profile.name());
    }
}

#[test]
fn runtime_asset_fingerprints_use_the_same_recorded_bytes_for_empty_and_planned_selections() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let targets = Backend::ALL.iter().copied().map(target).collect::<Vec<_>>();
    let planned = compatibility::snapshot(load(&path), &["jsonValue".into()], &targets).unwrap();
    let mut empty = document();
    empty["paths"] = json!({});
    std::fs::write(&path, empty.to_string()).unwrap();
    let empty = compatibility::snapshot(load(&path), &[], &targets).unwrap();
    for (planned, empty) in planned.native.iter().zip(&empty.native) {
        assert_eq!(
            planned.status,
            PlanStatus::Planned,
            "{:?}",
            planned.findings
        );
        assert_eq!(empty.status, PlanStatus::EmptySelection);
        assert_eq!(planned.target.backend, empty.target.backend);
        assert_eq!(
            planned.runtime.fingerprinted_assets,
            empty.runtime.fingerprinted_assets,
            "asset identity must not depend on the selected operation count: {}",
            planned.target.backend.name()
        );
        assert_eq!(
            planned.runtime.plan_and_runtime_sha256,
            empty.runtime.plan_and_runtime_sha256,
            "empty selection must retain the same adapter runtime identity: {}",
            planned.target.backend.name()
        );
        let assets = &planned.runtime.fingerprinted_assets;
        assert!(!assets.is_empty());
        assert!(
            assets.windows(2).all(|pair| pair[0] < pair[1]),
            "asset names must be unique and ordered"
        );
        let mut hash = Sha256::new();
        for name in assets {
            let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name))
                .unwrap();
            hash.update((name.len() as u64).to_be_bytes());
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        assert_eq!(
            planned.runtime.plan_and_runtime_sha256,
            format!("{:x}", hash.finalize()),
            "the recorded paths must reproduce the compiled fingerprint: {}",
            planned.target.backend.name()
        );
    }
}

#[test]
fn unsupported_import_overrides_fail_generation_and_native_capture_instead_of_being_ignored() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .filter(|op| op.operation_id() == Some("jsonValue"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    for backend in Backend::ALL.iter().copied().filter(|backend| {
        matches!(
            backend.name(),
            "typescript-http" | "rust-http" | "go-http" | "dart-http"
        )
    }) {
        let mut config = target(backend);
        config.import_name = Some("silently_ignored_before".into());
        let errors = backend::generate(contract.clone(), &selected, &config).unwrap_err();
        assert!(
            errors.iter().any(|e| e.code == "sdk-package"
                && e.source.is_none()
                && e.message.contains("import_name")),
            "{}: {errors:?}",
            backend.name()
        );
        let snapshot =
            compatibility::snapshot(contract.clone(), &["jsonValue".into()], &[config]).unwrap();
        assert_eq!(snapshot.native[0].status, PlanStatus::Unavailable);
        assert!(snapshot.native[0].operations.is_empty());
        assert!(
            snapshot.native[0]
                .findings
                .iter()
                .any(|e| e.code == "sdk-package"
                    && e.source.is_none()
                    && e.message.contains("import_name"))
        );
    }
}

#[test]
fn canonical_rust_generation_session_and_capture_select_the_verified_scoped_profile() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scoped.json");
    let schema = json!({"type":"object","required":["name"],
        "properties":{"name":{"type":"string"},"kind":{"type":"string"},
            "values":{"type":"array","items":{"type":"integer"},
                "contains":{"type":"integer","minimum":1},"minContains":1,"unevaluatedItems":false}},
        "patternProperties":{"^x-":{"type":"integer"}},"additionalProperties":false,
        "propertyNames":{"minLength":1},"dependentRequired":{"kind":["name"]},
        "dependentSchemas":{"kind":{"properties":{"name":{"minLength":1}}}},
        "if":{"properties":{"kind":{"const":"full"}}},"then":{"required":["values"]},
        "else":true,"unevaluatedProperties":false});
    let example = json!({"name":"ready","kind":"full","values":[1],"x-extra":2});
    let value = json!({"openapi":"3.1.2","info":{"title":"Scoped native integration","version":"1"},
        "servers":[{"url":"https://example.test/v1"}],"security":[],
        "components":{"schemas":{"Scoped":schema}},
        "paths":{"/scoped":{"post":{"operationId":"scopedEcho",
            "requestBody":{"required":true,"content":{"application/json":{
                "schema":{"$ref":"#/components/schemas/Scoped"},"example":example}}},
            "responses":{"200":{"description":"Echo","content":{"application/json":{
                "schema":{"$ref":"#/components/schemas/Scoped"},"example":example}}}}}}}});
    std::fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert!(
        suspect_codegen::rust_http::plan_http(contract.clone(), &selected, Default::default())
            .is_err(),
        "the explicitly retained Rust v1 library API must keep its admission boundary"
    );
    let config = target(Backend::RustHttp);
    let direct =
        suspect_codegen::rust_http::plan_http_v2(contract.clone(), &selected, Default::default())
            .unwrap();
    let expected = suspect_codegen::rust_http::emit_http(
        &direct,
        &suspect_codegen::rust_http::PackageConfig {
            name: config.package_name.clone(),
            version: config.package_version.clone(),
        },
    )
    .unwrap();
    let resource_capable =
        suspect_codegen::rust_http::plan_http_v3(contract.clone(), &selected, Default::default())
            .unwrap();
    let resource_capable_files = suspect_codegen::rust_http::emit_http(
        &resource_capable,
        &suspect_codegen::rust_http::PackageConfig {
            name: config.package_name.clone(),
            version: config.package_version.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        resource_capable_files
            .iter()
            .filter(|file| !expected.contains(file))
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        Vec::<&str>::new(),
        "resource-capable canonical Rust planning must retain ordinary v2 HTTP artifacts"
    );
    assert_eq!(resource_capable_files.len(), expected.len());
    let generated = backend::generate(contract.clone(), &selected, &config).unwrap();
    assert_eq!(
        generated, expected,
        "canonical generation must retain the witnessed ordinary v2 native output"
    );
    let snapshot = compatibility::snapshot(
        contract.clone(),
        &["scopedEcho".into()],
        std::slice::from_ref(&config),
    )
    .unwrap();
    assert_eq!(
        snapshot.native[0].status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.native[0].findings
    );
    assert!(snapshot.native[0].findings.is_empty());
    let report = compatibility::compare(
        contract.clone(),
        contract,
        &["scopedEcho".into()],
        std::slice::from_ref(&config),
    )
    .unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    let mut session = Session::new(
        &path,
        SessionConfig {
            targets: vec![config],
            operation_ids: vec!["scopedEcho".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let first = session.generate().unwrap();
    let mut sorted = expected;
    sorted.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(first.files.as_ref(), &sorted);
    let warm = session.generate().unwrap();
    assert_eq!(warm.delta.compiles, 0);
    assert_eq!(warm.delta.renders, 0);
    assert!(Arc::ptr_eq(&first.files, &warm.files));
}

#[test]
fn verified_resource_adapters_share_canonical_generation_and_capture_admission() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("resources.json");
    let value = json!({"openapi":"3.2.0","info":{"title":"Canonical resources","version":"1"},
    "servers":[{"url":"https://example.test/v1"}],"security":[],
    "components":{"schemas":{
        "Outer":{"$id":"https://schema.test/outer","$defs":{"Value":{"$dynamicAnchor":"value","type":"integer"}},"$ref":"use"},
        "Use":{"$id":"https://schema.test/use","$dynamicRef":"fallback#value"},
        "Fallback":{"$id":"https://schema.test/fallback","$dynamicAnchor":"value","type":"string"}
    }},"paths":{"/value":{"post":{"operationId":"resourceEcho",
        "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"https://schema.test/outer"},"example":7}}},
        "responses":{"200":{"description":"Echo","content":{"application/json":{"schema":{"$ref":"https://schema.test/outer"},"example":7}}}}
    }}}});
    std::fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    // Every registered adapter has completed its independent V3/native SDK proof.
    let targets = Backend::ALL.iter().copied().map(target).collect::<Vec<_>>();
    let mut expected = Vec::new();
    for config in &targets {
        let files = backend::generate(contract.clone(), &selected, config)
            .unwrap_or_else(|errors| panic!("{}: {errors:?}", config.backend.name()));
        assert!(files.iter().any(|file| file.path.ends_with("README.md")));
        expected.extend(files);
    }
    expected.sort_by(|left, right| left.path.cmp(&right.path));
    let mut session = Session::new(
        &path,
        SessionConfig {
            targets: targets.clone(),
            operation_ids: vec!["resourceEcho".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.files.as_ref(), &expected);
    assert_eq!(first.delta.compiles, 1);
    assert_eq!(first.delta.renders, targets.len());
    let warm = session.generate().unwrap();
    assert_eq!(warm.delta.compiles, 0);
    assert_eq!(warm.delta.renders, 0);
    assert!(Arc::ptr_eq(&first.files, &warm.files));
    let snapshot =
        compatibility::snapshot(contract.clone(), &["resourceEcho".into()], &targets).unwrap();
    for native in &snapshot.native {
        assert_eq!(
            native.status,
            PlanStatus::Planned,
            "{}: {:?}",
            native.target.backend.name(),
            native.findings
        );
        assert!(
            native.findings.is_empty(),
            "{}: {:?}",
            native.target.backend.name(),
            native.findings
        );
        assert_eq!(native.operations.len(), 1);
        assert!(!native.models.is_empty());
    }
    let comparison = compatibility::compare(
        contract.clone(),
        contract,
        &["resourceEcho".into()],
        &targets,
    )
    .unwrap();
    assert!(
        comparison
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown),
        "native resource admission must not fabricate a dynamic wire equivalence proof"
    );
    assert!(
        comparison
            .native
            .iter()
            .all(|native| native.summary.is_proven_compatible()),
        "{comparison:#?}"
    );
}
