//! SDK-owned API-error lifetimes and fail-closed missing response admission.
#![cfg(feature = "http-protocol")]
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::terraform::{
    MappingProfile, TargetConfig, emit_provider, parse_mapping, plan_provider,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn source(root: &Path, missing: bool) -> Arc<Contract> {
    std::fs::create_dir_all(root).unwrap();
    let mut spec: Value =
        serde_json::from_str(include_str!("fixtures/terraform-v1/openapi.json")).unwrap();
    spec["openapi"] = json!("3.2.0");
    let stream = json!({"description":"Deferred SDK error payload", "content":{"application/jsonl":{"itemSchema":{"type":"string"}}}});
    for (path, method) in [
        ("/records", "post"),
        ("/records/{record-id}", "get"),
        ("/records/{record-id}", "put"),
        ("/records/{record-id}", "delete"),
    ] {
        spec["paths"][path][method]["responses"]["409"] = stream.clone();
    }
    for method in ["get", "delete"] {
        spec["paths"]["/records/{record-id}"][method]["responses"]["503"] = stream.clone();
    }
    if missing {
        spec["paths"]["/records/{record-id}"]["get"]["responses"]["404"] = stream.clone();
        spec["paths"]["/records/{record-id}"]["delete"]["responses"]["404"] = stream;
    }
    std::fs::write(root.join("openapi.json"), spec.to_string()).unwrap();
    std::fs::write(
        root.join("schemas.json"),
        include_str!("fixtures/terraform-v1/schemas.json"),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::from_path(&root.join("openapi.json")).unwrap(),
        )
        .unwrap(),
    )
}
fn mapping() -> MappingProfile {
    parse_mapping(include_str!("fixtures/terraform-v1/mapping.json")).unwrap()
}
fn config() -> TargetConfig {
    serde_json::from_str(include_str!("fixtures/terraform-v1/target.json")).unwrap()
}

#[test]
fn missing_requires_completed_sdk_validation_before_any_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let errors = plan_provider(source(root.path(), true), mapping(), config()).unwrap_err();
    let mut pointers = errors
        .iter()
        .filter(|e| e.code == "terraform-missing-response-deferred")
        .map(|e| e.mapping_pointer.as_str())
        .collect::<Vec<_>>();
    pointers.sort();
    assert_eq!(
        pointers,
        [
            "/data_sources/record/read/missing/0",
            "/resources/record/delete/missing/0",
            "/resources/record/read/missing/0"
        ]
    );
    assert!(
        errors.iter().all(|e| e
            .source
            .pointer()
            .ends_with("/responses/404/content/application~1jsonl")
            && e.at.end > e.at.start),
        "{errors:?}"
    );
}

#[test]
#[ignore = "focused real SDK/Framework error ownership; no completed lifecycle matrix replay"]
fn native_sdk_error_lifetimes_and_state_distinctions() {
    let evidence = PathBuf::from(
        std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE").expect("fresh evidence directory required"),
    );
    std::fs::create_dir(&evidence).unwrap();
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-terraform-errors-")
        .tempdir_in("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    let plan = plan_provider(source(&root.join("sources"), false), mapping(), config()).unwrap();
    suspect_codegen::write_files_with_owner(
        &emit_provider(&plan),
        &root.join("generated"),
        "terraform-errors",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(
        manifest.join("tests/fixtures/terraform-v1/provider_test.go"),
        root.join("generated/terraform/provider/base_test.go"),
    )
    .unwrap();
    std::fs::copy(
        manifest.join("tests/fixtures/terraform-errors/error_ownership_test.go"),
        root.join("generated/terraform/provider/error_ownership_test.go"),
    )
    .unwrap();
    let output = std::process::Command::new("python3")
        .arg(manifest.join("../../tools/sdk-terraform-errors.py"))
        .arg("--root")
        .arg(&root)
        .arg("--evidence")
        .arg(&evidence)
        .output()
        .unwrap();
    std::fs::write(evidence.join("runner.stdout.log"), &output.stdout).unwrap();
    std::fs::write(evidence.join("runner.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        evidence.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
