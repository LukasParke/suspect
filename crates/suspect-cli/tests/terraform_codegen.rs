//! Real CLI routing for the separate Terraform artifact and pinned Go dependency.
use std::{collections::BTreeSet, path::Path, process::Command};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_source::Uri;

fn fixture(root: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suspect-codegen/tests/fixtures/terraform-v1");
    for name in [
        "openapi.json",
        "schemas.json",
        "mapping.json",
        "target.json",
    ] {
        std::fs::copy(source.join(name), root.join(name)).unwrap();
    }
}

fn run(root: &Path, args: &[&str], expected: i32) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn generate(root: &Path, extra: &[&str], expected: i32) -> Value {
    let mut args = vec![
        "codegen-terraform",
        "openapi.json",
        "--mapping",
        "mapping.json",
        "--target-config",
        "target.json",
        "--out",
        "generated",
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    run(root, &args, expected)
}

#[test]
fn explicit_lifecycle_cli_preserves_sdk_dependency_identity_and_owned_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root);
    let profiles = run(root, &["codegen-profiles", "--format", "json"], 0);
    assert_eq!(profiles["profiles"].as_array().unwrap().len(), 12);
    assert!(
        profiles["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|profile| !profile["profile"].as_str().unwrap().contains("terraform"))
    );
    let source = std::fs::read(root.join("openapi.json")).unwrap();
    let absent = generate(root, &["--check"], 1);
    assert_eq!(absent["status"], "drift");
    assert!(!root.join("generated").exists());
    let generated = generate(root, &[], 0);
    assert_eq!(generated["format"], "suspect.terraform.generation.v1");
    assert_eq!(generated["profile"], "suspect.terraform.lifecycle.v1");
    assert_eq!(generated["status"], "generated");
    assert_eq!(
        generated["sdkDependency"],
        json!({"module_path":"example.com/lifecycle-sdk","version":"0.4.2"})
    );
    let operations = generated["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["operationId"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        operations,
        BTreeSet::from(["read-item", "read_item", "erase", "observe"])
    );
    let provider = root.join("generated/terraform/go.mod");
    let sdk = root.join("generated/go/go.mod");
    let provider_bytes = std::fs::read(&provider).unwrap();
    let sdk_bytes = std::fs::read(&sdk).unwrap();
    let text = std::str::from_utf8(&provider_bytes).unwrap();
    assert!(text.contains("example.com/lifecycle-sdk v0.4.2"));
    assert!(!text.contains("replace "));
    let timestamp = std::fs::metadata(&provider).unwrap().modified().unwrap();
    assert_eq!(generate(root, &["--check"], 0)["status"], "current");
    assert_eq!(generate(root, &[], 0)["status"], "current");
    assert_eq!(
        std::fs::metadata(&provider).unwrap().modified().unwrap(),
        timestamp
    );
    assert_eq!(std::fs::read(&provider).unwrap(), provider_bytes);
    assert_eq!(std::fs::read(root.join("openapi.json")).unwrap(), source);

    let mut edited = provider_bytes;
    edited.extend_from_slice(b"\n// retained user edit\n");
    std::fs::write(&provider, &edited).unwrap();
    let conflict = generate(root, &[], 1);
    assert_eq!(conflict["status"], "conflict");
    assert!(
        conflict["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "terraform-artifact-conflict")
    );
    assert_eq!(std::fs::read(&provider).unwrap(), edited);
    assert_eq!(std::fs::read(&sdk).unwrap(), sdk_bytes);
}

#[test]
fn invalid_mapping_and_unknown_target_fields_remain_located_before_writes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root);
    let mut mapping: Value =
        serde_json::from_slice(&std::fs::read(root.join("mapping.json")).unwrap()).unwrap();
    mapping["resources"]["record"]["create"]["operation_id"] = json!("not-a-declared-operation");
    std::fs::write(root.join("mapping.json"), mapping.to_string()).unwrap();
    let report = generate(root, &[], 1);
    assert_eq!(report["status"], "failed");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["mappingPointer"]
                .as_str()
                .is_some_and(|pointer| pointer.contains("/resources/record/create"))
                && finding["file"]
                    .as_str()
                    .is_some_and(|file| file.ends_with("openapi.json")))
    );
    assert!(!root.join("generated").exists());

    fixture(root);
    let mut target: Value =
        serde_json::from_slice(&std::fs::read(root.join("target.json")).unwrap()).unwrap();
    target["guess_lifecycle"] = json!(true);
    std::fs::write(
        root.join("target.json"),
        serde_json::to_vec_pretty(&target).unwrap(),
    )
    .unwrap();
    let report = generate(root, &[], 1);
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "terraform-target-input"
                && finding["line"].as_u64().unwrap() > 0)
    );
    assert!(!root.join("generated").exists());
}

#[test]
fn cache_only_provider_generation_retains_physical_sources_after_entry_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fixture(root);
    let entry = Uri::from_path(&root.join("openapi.json"))
        .unwrap()
        .to_string();
    let resources = ["openapi.json", "schemas.json"].iter().map(|name| {
        let uri = Uri::from_path(&root.join(name)).unwrap().to_string();
        let bytes = std::fs::read(root.join(name)).unwrap();
        json!({"requested_uri":uri,"effective_uri":uri,"digest":format!("sha256-{:x}",Sha256::digest(&bytes)),
            "media_type":"application/json","via":"local","redirects":[],"retrieved_at":"2026-09-10T00:00:00Z","attempts":0})
    }).collect::<Vec<_>>();
    std::fs::write(
        root.join("pins.json"),
        json!({"manifest_version":1,"entry":entry,"resources":resources}).to_string(),
    )
    .unwrap();
    let acquisition = run(
        root,
        &[
            "acquire",
            "pins.json",
            "--cache-dir",
            "cache",
            "--format",
            "json",
        ],
        0,
    );
    assert_eq!(acquisition["networkRequests"], 0);
    std::fs::remove_file(root.join("openapi.json")).unwrap();
    std::fs::remove_file(root.join("schemas.json")).unwrap();
    let args = [
        "codegen-terraform",
        "--pins",
        "pins.json",
        "--cache-dir",
        "cache",
        "--mapping",
        "mapping.json",
        "--target-config",
        "target.json",
        "--out",
        "generated",
        "--format",
        "json",
    ];
    let report = run(root, &args, 0);
    assert_eq!(report["source"], entry);
    assert!(
        report["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|operation| operation["document"] == entry)
    );
    let provider = root.join("generated/terraform/go.mod");
    let bytes = std::fs::read(&provider).unwrap();
    let cached = acquisition["documents"][0]["cachePath"].as_str().unwrap();
    let cached = root.join(cached);
    // Acquired blobs are correctly read-only. Replace only this private test
    // blob to verify digest refusal without changing production cache policy.
    std::fs::remove_file(&cached).unwrap();
    std::fs::write(cached, b"corrupted cache fixture").unwrap();
    let rejected = run(root, &args, 1);
    assert_eq!(rejected["status"], "failed");
    assert!(
        rejected["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "pin-digest-mismatch"),
        "{rejected:#?}"
    );
    assert_eq!(std::fs::read(provider).unwrap(), bytes);
}
