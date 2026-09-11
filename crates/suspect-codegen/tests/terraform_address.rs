//! Native provider source-address admission, independent of API lifecycle gates.
#![cfg(feature = "http-protocol")]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_codegen::terraform::{
    MappingProfile, TargetConfig, emit_provider, parse_mapping, plan_provider,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn inputs() -> (Arc<Contract>, MappingProfile, TargetConfig) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terraform-v1");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    (
        Arc::new(
            Contract::from_workspace(
                &workspace,
                &Uri::from_path(&root.join("openapi.json")).unwrap(),
            )
            .unwrap(),
        ),
        parse_mapping(include_str!("fixtures/terraform-v1/mapping.json")).unwrap(),
        serde_json::from_str(include_str!("fixtures/terraform-v1/target.json")).unwrap(),
    )
}

fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/terraform-addresses.json")).unwrap()
}

#[test]
fn provider_source_components_are_admitted_before_any_artifacts() {
    let (contract, mapping, config) = inputs();
    for case in cases() {
        let mut config = config.clone();
        config.provider_address = case["address"].as_str().unwrap().into();
        config.provider_name = case["provider_name"].as_str().unwrap().into();
        let result = plan_provider(contract.clone(), mapping.clone(), config);
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["label"]
        );
        if let Err(errors) = result {
            assert!(
                errors.iter().any(|e| e.code == "terraform-package"
                    && e.mapping_pointer == "/config/provider_address"),
                "{errors:?}"
            );
            assert!(
                errors
                    .iter()
                    .all(|e| e.source.document() == contract.entry()
                        && e.source.pointer().is_empty()
                        && e.at.end > e.at.start),
                "{errors:?}"
            );
        }
    }
}

#[test]
#[ignore = "real Terraform address parsing only; fresh isolated evidence, no API or full lifecycle replay"]
fn native_provider_source_address_boundary() {
    let evidence = PathBuf::from(
        std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE").expect("set a fresh evidence directory"),
    );
    std::fs::create_dir(&evidence).unwrap();
    std::fs::create_dir(evidence.join("commands")).unwrap();
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-terraform-address-")
        .tempdir_in("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    let terraform = std::env::var_os("SUSPECT_TERRAFORM_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/Users/luke/.local/share/mise/installs/terraform/latest/terraform")
        });
    let cli_config = root.join("terraform.rc");
    std::fs::write(&cli_config, "disable_checkpoint = true\n").unwrap();
    let (contract, mapping, base_config) = inputs();
    let mut reports = Vec::new();
    for case in cases() {
        let label = case["label"].as_str().unwrap();
        let mut config = base_config.clone();
        config.provider_address = case["address"].as_str().unwrap().into();
        config.provider_name = case["provider_name"].as_str().unwrap().into();
        let plan = plan_provider(contract.clone(), mapping.clone(), config);
        let directory = root.join(label);
        std::fs::create_dir(&directory).unwrap();
        let (hcl, diagnostics, artifacts) = match plan {
            Ok(plan) => {
                let files = emit_provider(&plan);
                let hcl = files
                    .iter()
                    .find(|f| f.path == "terraform/examples/resources/record/main.tf")
                    .unwrap()
                    .content
                    .clone();
                if label == "ordinary" {
                    suspect_codegen::write_files_with_owner(
                        &files,
                        &root.join("ordinary-generated"),
                        "terraform-address-parity",
                        suspect_codegen::Adoption::Refuse,
                    )
                    .unwrap();
                    let hashes = files
                        .iter()
                        .map(|f| {
                            (
                                f.path.clone(),
                                json!(format!("{:x}", Sha256::digest(f.content.as_bytes()))),
                            )
                        })
                        .collect::<serde_json::Map<_, _>>();
                    std::fs::write(
                        evidence.join("ordinary-artifact-hashes.json"),
                        serde_json::to_vec_pretty(&hashes).unwrap(),
                    )
                    .unwrap();
                }
                (hcl, vec![], files.len())
            }
            Err(errors) => {
                let diagnostics = errors.into_iter().map(|e| json!({"code":e.code,"pointer":e.mapping_pointer,"message":e.message,"document":e.source.document().as_str(),"source_pointer":e.source.pointer(),"span":[e.at.start,e.at.end]})).collect::<Vec<_>>();
                // Independent native-oracle input, not emitted provider artifacts.
                let hcl = format!(
                    "terraform {{\n required_providers {{\n  fixture = {{\n   source = {}\n  }}\n }}\n}}\n",
                    case["address"]
                );
                (hcl, diagnostics, 0)
            }
        };
        std::fs::write(directory.join("main.tf"), hcl).unwrap();
        let output = Command::new(&terraform)
            .args(["providers", "-no-color"])
            .current_dir(&directory)
            .env("TF_CLI_CONFIG_FILE", &cli_config)
            .env("TF_IN_AUTOMATION", "1")
            .env("CHECKPOINT_DISABLE", "1")
            .env_remove("TF_CLI_ARGS")
            .env_remove("TF_CLI_ARGS_providers")
            .output()
            .unwrap();
        std::fs::write(
            evidence.join(format!("commands/{label}.stdout")),
            &output.stdout,
        )
        .unwrap();
        std::fs::write(
            evidence.join(format!("commands/{label}.stderr")),
            &output.stderr,
        )
        .unwrap();
        reports.push(json!({"case":case,"command":[terraform.to_string_lossy(),"providers","-no-color"],"cwd":directory,"exit_code":output.status.code(),"diagnostics":diagnostics,"emitted_artifacts":artifacts}));
        std::fs::write(
            evidence.join("cases.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
        assert_eq!(
            output.status.success(),
            case["valid"].as_bool().unwrap(),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            artifacts > 0,
            output.status.success(),
            "{label}: generator/native address admission disagreement"
        );
    }
    let output = Command::new(&terraform)
        .args(["version", "-json"])
        .env("CHECKPOINT_DISABLE", "1")
        .output()
        .unwrap();
    std::fs::write(evidence.join("terraform-version.json"), &output.stdout).unwrap();
    let version: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success());
    assert_eq!(version["terraform_version"], base_config.terraform_version);
    let report = json!({"format":"suspect.terraform.address-admission.v1","passed":true,"cases":reports.len(),"terraform":terraform,"terraform_sha256":format!("{:x}",Sha256::digest(std::fs::read(&terraform).unwrap())),"native_root":root,"version":version,"scope":"native address parser + emitted valid HCL; no API calls, dependency install, or lifecycle matrix replay"});
    std::fs::write(
        evidence.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
