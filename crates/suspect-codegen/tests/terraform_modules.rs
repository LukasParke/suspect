//! Native Go import-path collisions at the separate Terraform package boundary.
#![cfg(feature = "http-protocol")]
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::{go_http, terraform::*};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn inputs() -> (Arc<Contract>, MappingProfile, TargetConfig) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terraform-v1");
    let ws = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    (
        Arc::new(
            Contract::from_workspace(&ws, &Uri::from_path(&root.join("openapi.json")).unwrap())
                .unwrap(),
        ),
        parse_mapping(include_str!("fixtures/terraform-v1/mapping.json")).unwrap(),
        serde_json::from_str(include_str!("fixtures/terraform-v1/target.json")).unwrap(),
    )
}

#[test]
fn sdk_modules_cannot_shadow_generated_provider_packages() {
    let (contract, mapping, base) = inputs();
    for sdk in [
        base.module_path.clone(),
        format!("{}/provider", base.module_path),
    ] {
        let mut config = base.clone();
        config.sdk.module_path = sdk;
        let errors = plan_provider(contract.clone(), mapping.clone(), config).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == "terraform-package"
                && error.mapping_pointer.starts_with("/config/sdk")),
            "{errors:?}"
        );
    }
    let mut config = base.clone();
    config.module_path = format!("{}/examples/validated", config.sdk.module_path);
    let errors = plan_provider(contract.clone(), mapping.clone(), config).unwrap_err();
    assert!(
        errors.iter().any(|error| error.code == "terraform-package"
            && error.mapping_pointer == "/config/sdk/module_path"),
        "{errors:?}"
    );
    for sdk in [
        "example.com/separate-sdk",
        "example.com/team/sdk",
        "example.com/terraform-provider-fixture/client-sdk",
    ] {
        let mut config = base.clone();
        config.sdk.module_path = sdk.into();
        assert!(
            plan_provider(contract.clone(), mapping.clone(), config).is_ok(),
            "{sdk}"
        );
    }
}

#[test]
#[ignore = "native pinned Go package lookup; no SDK replacement or lifecycle replay"]
fn native_pinned_module_import_boundaries() {
    let evidence =
        PathBuf::from(std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE").expect("fresh evidence root"));
    std::fs::create_dir(&evidence).unwrap();
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-terraform-modules-")
        .tempdir_in("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    let (contract, mapping, base) = inputs();
    for (label, sdk) in [
        ("separate", "example.com/separate-sdk"),
        ("sibling", "example.com/team/sdk"),
        (
            "nested",
            "example.com/terraform-provider-fixture/client-sdk",
        ),
    ] {
        let mut config = base.clone();
        if label == "sibling" {
            config.module_path = "example.com/team/provider".into()
        };
        config.sdk.module_path = sdk.into();
        let plan = plan_provider(contract.clone(), mapping.clone(), config).unwrap();
        suspect_codegen::write_files_with_owner(
            &emit_provider(&plan),
            &root.join(label).join("generated"),
            label,
            suspect_codegen::Adoption::Refuse,
        )
        .unwrap();
    }
    // Independent native import oracle: a minimal local provider package and
    // the actual canonical SDK module share the same import path. No emitted
    // package is rewritten or repaired to manufacture this negative control.
    let collision = format!("{}/provider", base.module_path);
    let ordinary = plan_provider(contract, mapping, base.clone()).unwrap();
    let sdk = go_http::emit_http(
        ordinary.sdk_plan(),
        &go_http::PackageConfig {
            module_path: collision.clone(),
            package_name: "sdk".into(),
            version: base.sdk.version.clone(),
        },
    )
    .unwrap();
    suspect_codegen::write_files_with_owner(
        &sdk,
        &root.join("collision/generated"),
        "collision-oracle-sdk",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    let package = root.join("collision/generated/terraform");
    std::fs::create_dir_all(package.join("provider")).unwrap();
    std::fs::write(
        package.join("go.mod"),
        format!(
            "module {}\n\ngo 1.23.0\n\nrequire {collision} v{}\n",
            base.module_path, base.sdk.version
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("main.go"),
        format!("package main\nimport _ {collision:?}\nfunc main() {{}}\n"),
    )
    .unwrap();
    std::fs::write(package.join("provider/provider.go"), "package provider\n").unwrap();
    std::fs::write(
        root.join("collision/config.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"sdk":{"module_path":collision,"version":base.sdk.version}}),
        )
        .unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/sdk-terraform-modules.py"))
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
