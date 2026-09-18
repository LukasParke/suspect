//! Package attribution and application-selected SDK defaults participate in
//! native compatibility capture without changing OpenAPI wire semantics.
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::{
    attribution,
    backend::{Backend, GenerationOptions, TargetConfig},
    compatibility::{self, Impact, PlanStatus},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.attrib.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&json!({
                "openapi":"3.1.2","info":{"title":"Attribution capture","version":"1"},
                "servers":[{"url":"https://api.attrib.test/v1"}],
                "paths":{
                    "/value":{"get":{"operationId":"value","responses":{"204":{"description":"Done"}}}}
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

fn targets(base: &str) -> Vec<TargetConfig> {
    vec![
        TargetConfig {
            backend: Backend::TypescriptHttp,
            package_name: format!("{base}-client"),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        TargetConfig {
            backend: Backend::RustHttp,
            package_name: format!("{base}-client"),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        TargetConfig {
            backend: Backend::PythonHttp,
            package_name: format!("{base}-client"),
            package_version: "0.1.0".into(),
            import_name: Some("attrib_client".into()),
        },
        TargetConfig {
            backend: Backend::GoHttp,
            package_name: format!("example.com/{base}-client"),
            package_version: "0.1.0".into(),
            import_name: None,
        },
    ]
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1","env_prefix":"OPENROUTER",
                "pagination":{"mode":"auto","page_size":25}
            }))
            .unwrap(),
        ),
        ..GenerationOptions::default()
    }
}

fn pagination(policy: serde_json::Value) -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({"version":"v1","pagination":policy})).unwrap(),
        ),
        ..GenerationOptions::default()
    }
}

fn configured_sdk_defaults_change(report: &compatibility::CompatibilityReport) {
    assert!(
        report.wire.is_empty(),
        "client defaults are not OpenAPI wire edits: {report:#?}"
    );
    assert!(
        !report.native.is_empty(),
        "the reference backends must be captured: {report:#?}"
    );
    for native in &report.native {
        let change = native
            .changes
            .iter()
            .find(|change| change.code == "native-sdk-defaults-changed")
            .unwrap_or_else(|| {
                panic!(
                    "{}: the configured client defaults must be flagged: {:?}",
                    native.backend.name(),
                    native.changes
                )
            });
        assert_eq!(change.impact, Impact::PotentiallyBreaking);
        assert_eq!(change.subject, native.backend.name());
        assert!(
            !native
                .changes
                .iter()
                .any(|change| change.code == "native-attribution-changed"),
            "{}: identical package identity cannot flag attribution: {:?}",
            native.backend.name(),
            native.changes
        );
    }
    assert!(!report.is_proven_compatible());
}

#[test]
fn configured_defaults_are_captured_and_flagged_without_wire_edits() {
    let contract = contract();
    let targets = targets("attrib");
    let configured = configured();
    let ordinary = GenerationOptions::default();
    let snapshot =
        compatibility::snapshot_with_options(contract.clone(), &[], &targets, &configured).unwrap();
    assert_eq!(snapshot.native.len(), targets.len());
    for native in &snapshot.native {
        assert_eq!(
            native.status,
            PlanStatus::Planned,
            "{}: {:?}",
            native.target.backend.name(),
            native.findings
        );
        assert!(native.findings.is_empty(), "{:?}", native.findings);
        let descriptor = native
            .attribution
            .as_ref()
            .expect("the actual native plan carries attribution");
        assert_eq!(
            descriptor.template_version,
            attribution::AttributionTemplateVersion::V1
        );
        assert_eq!(descriptor.language, native.target.backend.language_tag());
        assert_eq!(descriptor.sdk_version, "0.1.0");
        assert_eq!(descriptor.spec_version, "3.1.2");
        assert_eq!(
            descriptor.sdk_name,
            if native.target.backend == Backend::GoHttp {
                "example.com-attrib-client"
            } else {
                "attrib-client"
            }
        );
        assert_eq!(
            serde_json::to_value(native.sdk_defaults.as_ref().unwrap()).unwrap(),
            json!({
                "version":"v1","env_prefix":"OPENROUTER",
                "pagination":{"mode":"auto","page_size":25}
            })
        );
    }
    let stable = compatibility::compare_with_options(
        contract.clone(),
        contract.clone(),
        &[],
        (&targets, &configured),
        (&targets, &configured),
    )
    .unwrap();
    assert!(
        stable.is_proven_compatible(),
        "identical options must stay proven compatible: {stable:#?}"
    );
    assert!(
        stable.native.iter().all(|native| native.changes.is_empty()),
        "{stable:#?}"
    );
    let report = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&targets, &ordinary),
        (&targets, &configured),
    )
    .unwrap();
    configured_sdk_defaults_change(&report);
    assert_eq!(report.after.generation, configured);
}

#[test]
fn pagination_policy_changes_are_flagged_without_wire_edits() {
    let contract = contract();
    let targets = targets("attrib");
    let report = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&targets, &pagination(json!("off"))),
        (&targets, &pagination(json!({"mode":"auto","page_size":50}))),
    )
    .unwrap();
    configured_sdk_defaults_change(&report);
    for native in &report.native {
        let change = native
            .changes
            .iter()
            .find(|change| change.code == "native-sdk-defaults-changed")
            .unwrap();
        assert_eq!(
            change.before.as_ref().unwrap()["binding"]["pagination"]["mode"],
            json!("off")
        );
        assert_eq!(
            change.after.as_ref().unwrap()["binding"]["pagination"]["mode"],
            json!("auto")
        );
    }
}

#[test]
fn package_identity_renames_flag_attribution_and_version_bumps_do_not() {
    let contract = contract();
    let configured = configured();
    let before = targets("attrib");
    let after = targets("renamed");
    let report = compatibility::compare_with_options(
        contract.clone(),
        contract.clone(),
        &[],
        (&before, &configured),
        (&after, &configured),
    )
    .unwrap();
    assert!(report.wire.is_empty());
    for native in &report.native {
        let change = native
            .changes
            .iter()
            .find(|change| change.code == "native-attribution-changed")
            .unwrap_or_else(|| {
                panic!(
                    "{}: the package identity rename must flag attribution: {:?}",
                    native.backend.name(),
                    native.changes
                )
            });
        assert_eq!(change.impact, Impact::Unknown);
        assert_eq!(change.subject, native.backend.name());
        let before = change.before.as_ref().unwrap();
        let after = change.after.as_ref().unwrap();
        // The semantic projection excludes generator and package versions.
        assert_eq!(before["suspect_version"], json!(""));
        assert_eq!(before["sdk_version"], json!(""));
        assert_eq!(after["suspect_version"], json!(""));
        assert_eq!(after["sdk_version"], json!(""));
        assert_eq!(before["template_version"], after["template_version"]);
        assert_eq!(before["language"], after["language"]);
        assert_eq!(before["spec_version"], after["spec_version"]);
        assert_ne!(before["sdk_name"], after["sdk_name"]);
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-package-name-changed"
                    && change.impact == Impact::Breaking),
            "{:?}",
            native.changes
        );
    }
    let mut bumped = targets("attrib");
    for target in &mut bumped {
        target.package_version = "0.2.0".into();
    }
    let report = compatibility::compare_with_options(
        contract.clone(),
        contract,
        &[],
        (&before, &configured),
        (&bumped, &configured),
    )
    .unwrap();
    assert!(
        report.native.iter().all(|native| native
            .changes
            .iter()
            .all(|change| change.code != "native-attribution-changed")),
        "a package version increment alone is not an attribution change: {report:#?}"
    );
    assert!(report.is_proven_compatible(), "{report:#?}");
}

#[test]
fn generator_version_alone_cannot_flag_attribution() {
    let contract = contract();
    let targets = targets("attrib");
    let configured = configured();
    let before =
        compatibility::snapshot_with_options(contract.clone(), &[], &targets, &configured).unwrap();
    let mut after =
        compatibility::snapshot_with_options(contract, &[], &targets, &configured).unwrap();
    after.native[0]
        .attribution
        .as_mut()
        .unwrap()
        .suspect_version = "future-generator".into();
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.is_proven_compatible(),
        "the generator version is provenance, not attribution semantics: {report:#?}"
    );
    for native in &report.native {
        assert!(native.changes.is_empty(), "{:?}", native.changes);
    }
    // Positive control: the identity token itself is semantic.
    after.native[0].attribution.as_mut().unwrap().sdk_name = "renamed-sdk".into();
    let report = compatibility::compare_snapshots(&before, &after);
    let change = report.native[0]
        .changes
        .iter()
        .find(|change| change.code == "native-attribution-changed")
        .unwrap_or_else(|| panic!("identity changes must be flagged: {report:#?}"));
    assert_eq!(change.impact, Impact::Unknown);
    assert_eq!(change.before.as_ref().unwrap()["sdk_name"], "attrib-client");
    assert_eq!(change.after.as_ref().unwrap()["sdk_name"], "renamed-sdk");
}

#[test]
fn snapshots_recorded_before_the_new_keys_existed_still_deserialize() {
    let contract = contract();
    let targets = targets("attrib");
    let snapshot =
        compatibility::snapshot_with_options(contract, &[], &targets, &configured()).unwrap();
    let encoded = serde_json::to_vec(&snapshot.native[0]).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let object = value.as_object_mut().unwrap();
    assert!(object.remove("attribution").is_some());
    assert!(object.remove("sdk_defaults").is_some());
    let decoded: compatibility::NativeSnapshot =
        serde_json::from_slice(serde_json::to_vec(&value).unwrap().as_slice()).unwrap();
    assert!(decoded.attribution.is_none());
    assert!(decoded.sdk_defaults.is_none());
    assert_eq!(decoded.credential_env, None);
    assert_eq!(decoded.operations, snapshot.native[0].operations);
    assert_eq!(decoded.models, snapshot.native[0].models);
}
