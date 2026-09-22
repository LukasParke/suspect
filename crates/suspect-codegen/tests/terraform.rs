//! Lifecycle mapping admission, real native bindings and ownership boundaries.
#![cfg(feature = "http-protocol")]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::terraform::*;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terraform-v1")
}
fn contract() -> Arc<Contract> {
    let root = fixture();
    load(&root)
}

fn load(root: &Path) -> Arc<Contract> {
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::from_path(&root.join("openapi.json")).unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn every_missing_or_ambiguous_operation_use_keeps_its_mapping_pointer() {
    let mut p = profile();
    p.resources.get_mut("record").unwrap().read.operation_id = "missing".into();
    p.data_sources.get_mut("record").unwrap().read.operation_id = "missing".into();
    let expected = [
        "/data_sources/record/read/operation_id",
        "/resources/record/read/operation_id",
    ];
    let errors = plan_provider(contract(), p, config()).unwrap_err();
    assert_eq!(errors.len(), 2);
    let mut pointers = errors
        .iter()
        .map(|e| e.mapping_pointer.as_str())
        .collect::<Vec<_>>();
    pointers.sort();
    assert_eq!(pointers, expected);
    for error in &errors {
        assert_eq!(error.code, "terraform-operation");
        assert!(error.message.contains("found 0"));
        assert_eq!(error.source.pointer(), "");
        assert!(error.at.end > error.at.start);
    }

    let directory = tempfile::tempdir().unwrap();
    let mut document: Value =
        serde_json::from_str(include_str!("fixtures/terraform-v1/openapi.json")).unwrap();
    // A second real outgoing operation claims the same ID. Do not pick either.
    document["paths"]["/duplicate/{record-id}"] = document["paths"]["/records/{record-id}"].clone();
    std::fs::write(directory.path().join("openapi.json"), document.to_string()).unwrap();
    std::fs::write(
        directory.path().join("schemas.json"),
        include_str!("fixtures/terraform-v1/schemas.json"),
    )
    .unwrap();
    let errors = plan_provider(load(directory.path()), profile(), config()).unwrap_err();
    let shared = errors
        .iter()
        .filter(|e| e.mapping_pointer.ends_with("/read/operation_id"))
        .collect::<Vec<_>>();
    assert_eq!(shared.len(), 2);
    assert!(
        shared
            .iter()
            .all(|e| e.message.contains("found 2") && e.code == "terraform-operation")
    );
}
fn profile() -> MappingProfile {
    parse_mapping(include_str!("fixtures/terraform-v1/mapping.json")).unwrap()
}
fn config() -> TargetConfig {
    serde_json::from_str(include_str!("fixtures/terraform-v1/target.json")).unwrap()
}
fn plan() -> ProviderPlan {
    plan_provider(contract(), profile(), config()).unwrap()
}

#[test]
fn exact_native_sdk_bindings_drive_a_separate_artifact_target() {
    let plan = plan();
    assert_eq!(plan.sdk_plan().operations().len(), 4);
    let files = emit_provider(&plan);
    let by_path: BTreeMap<_, _> = files
        .iter()
        .map(|f| (f.path.as_str(), f.content.as_str()))
        .collect();
    assert_eq!(
        by_path["go/go.mod"],
        "module example.com/lifecycle-sdk\n\ngo 1.23\n"
    );
    assert!(by_path["terraform/go.mod"].contains("example.com/lifecycle-sdk v0.4.2"));
    assert!(!by_path["terraform/go.mod"].contains("replace"));
    let bindings: Value = serde_json::from_str(by_path["terraform/source-bindings.json"]).unwrap();
    for op in plan.sdk_plan().operations() {
        assert!(
            bindings["native_bindings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["native_method"] == op.method_name
                    && b["operation_id"] == op.operation_id)
        );
        assert!(
            by_path["terraform/provider/resource_record.go"]
                .contains(&format!("client.{}(ctx, input)", op.method_name))
        );
    }
    // Two operation IDs normalize together; native field names also collide.
    let names = plan
        .sdk_plan()
        .operations()
        .iter()
        .filter(|o| o.operation_id.starts_with("read"))
        .map(|o| o.method_name.clone())
        .collect::<Vec<_>>();
    assert_ne!(names[0], names[1]);
    assert!(by_path["terraform/provider/resource_record.go"].contains("body.SetExtra2"));
    assert!(
        bindings["native_bindings"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|b| b["inputs"].as_array().unwrap())
            .any(|i| i["source"]["document"]
                .as_str()
                .unwrap()
                .ends_with("schemas.json"))
    );
    for f in files
        .iter()
        .filter(|f| f.path.starts_with("terraform/") && f.path.ends_with(".go"))
    {
        for forbidden in [
            "net/http",
            "encoding/json",
            "http.NewRequest",
            "json.Marshal",
            "json.Unmarshal",
            "context.Background()",
        ] {
            // The executable server root owns process context; API glue cannot detach it.
            if f.path != "terraform/main.go" {
                assert!(!f.content.contains(forbidden), "{}: {forbidden}", f.path);
            }
        }
    }
}

#[test]
fn profile_is_closed_and_no_operation_name_infers_a_lifecycle() {
    let mut raw: Value =
        serde_json::from_str(include_str!("fixtures/terraform-v1/mapping.json")).unwrap();
    raw["resources"]["record"]["guess_crud"] = json!(true);
    assert!(parse_mapping(&raw.to_string()).is_err());
    raw["resources"]["record"]
        .as_object_mut()
        .unwrap()
        .remove("guess_crud");
    raw["resources"]["record"]["retry"] = json!("automatic");
    assert!(parse_mapping(&raw.to_string()).is_err());
    let mut p = profile();
    p.resources.get_mut("record").unwrap().create.operation_id = "createRecord".into();
    let errors = plan_provider(contract(), p, config()).unwrap_err();
    assert!(errors.iter().any(|e| e.code == "terraform-operation"));
    let mut p = profile();
    p.format = "suspect.terraform.lifecycle.v2".into();
    assert!(
        plan_provider(contract(), p, config())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "terraform-profile")
    );
}

#[test]
fn unsupported_mappings_fail_with_source_and_mapping_locations_before_artifacts() {
    type Mutation = fn(&mut MappingProfile);
    let mutations: &[(&str, Mutation)] = &[
        ("terraform-native-input", |p| {
            p.resources.get_mut("record").unwrap().create.inputs[0].target = InputTarget::Body {
                path: vec!["missing".into()],
            }
        }),
        ("terraform-native-input", |p| {
            p.resources.get_mut("record").unwrap().create.inputs[0].target = InputTarget::Body {
                path: vec!["SetExtra".into(), "nested".into()],
            }
        }),
        ("terraform-input-coverage", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .create
                .inputs
                .remove(0);
        }),
        ("terraform-input-shape", |p| {
            p.resources.get_mut("record").unwrap().create.inputs[0].null = NullInput::SendNull
        }),
        ("terraform-input-shape", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .attributes
                .get_mut("name")
                .unwrap()
                .r#type = AttributeType::Bool
        }),
        ("terraform-response-status", |p| {
            p.resources.get_mut("record").unwrap().read.missing = vec![410]
        }),
        ("terraform-state-coverage", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .read
                .success
                .state
                .remove("name");
        }),
        ("terraform-state-shape", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .read
                .success
                .state
                .insert("secret".into(), vec!["credential-fingerprint".into()]);
        }),
        ("terraform-write-only-trigger", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .update
                .inputs
                .last_mut()
                .unwrap()
                .when = InputWhen::Always
        }),
        ("terraform-identity", |p| {
            p.resources.get_mut("record").unwrap().identity.attribute = "description".into()
        }),
        ("terraform-lifecycle-coverage", |p| {
            p.resources
                .get_mut("record")
                .unwrap()
                .update
                .inputs
                .remove(3);
        }),
    ];
    for (code, mutate) in mutations {
        let mut p = profile();
        mutate(&mut p);
        let raw_mapping = serde_json::to_value(&p).unwrap();
        let errors = plan_provider(contract(), p, config()).unwrap_err();
        assert!(errors.iter().any(|e| e.code == *code), "{code}: {errors:?}");
        assert!(
            errors.iter().all(|e| !e.mapping_pointer.is_empty()
                && e.source.document().as_str().starts_with("file:")),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.at.end > e.at.start),
            "source spans lost: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|e| raw_mapping.pointer(&e.mapping_pointer).is_some()),
            "not an actual mapping JSON Pointer: {errors:?}"
        );
    }
}

#[test]
fn package_and_sdk_pins_are_exact_and_injection_safe() {
    for mutate in [
        (|c: &mut TargetConfig| c.version = "latest".into()) as fn(&mut TargetConfig),
        |c| c.sdk.version = "0.4".into(),
        |c| c.sdk.version = "2.0.0".into(),
        |c| c.module_path = "../escape".into(),
        |c| c.provider_name = "provider\"\n".into(),
        |c| c.framework_version = "1.18.0".into(),
        |c| c.go_toolchain = "auto".into(),
        |c| c.terraform_version = "1.10.0".into(),
        |c| c.sdk.module_path = "github.com/hashicorp/terraform-plugin-framework".into(),
    ] {
        let mut c = config();
        mutate(&mut c);
        assert!(plan_provider(contract(), profile(), c).is_err());
    }
}

#[test]
fn anonymous_admission_uses_canonical_empty_security_and_rejects_unmapped_alternatives() {
    let directory = tempfile::tempdir().unwrap();
    let mut document: Value =
        serde_json::from_str(include_str!("fixtures/terraform-v1/openapi.json")).unwrap();
    std::fs::write(
        directory.path().join("schemas.json"),
        include_str!("fixtures/terraform-v1/schemas.json"),
    )
    .unwrap();
    for security in [json!([]), json!([{}])] {
        document["security"] = security;
        std::fs::write(directory.path().join("openapi.json"), document.to_string()).unwrap();
        let mut p = profile();
        p.authentication = Authentication::None;
        let planned = plan_provider(load(directory.path()), p, config()).unwrap();
        let files = emit_provider(&planned);
        let provider = files
            .iter()
            .find(|f| f.path == "terraform/provider/provider.go")
            .unwrap();
        assert!(!provider.content.contains("Token types.String"));
        assert!(provider.content.contains("sdk.Credentials{}"));
    }
    document["security"] = json!([{}, {"NewClient":[]}]);
    std::fs::write(directory.path().join("openapi.json"), document.to_string()).unwrap();
    let mut p = profile();
    p.authentication = Authentication::None;
    assert!(plan_provider(load(directory.path()), p, config()).unwrap_err().iter().all(|e| e.code == "terraform-authentication" && e.mapping_pointer == "/authentication"));
}

#[test]
fn identity_is_an_explicit_binding_not_a_hardcoded_attribute_name() {
    let mut p = profile();
    let r = p.resources.get_mut("record").unwrap();
    let id = r.attributes.remove("id").unwrap();
    r.attributes.insert("remote_key".into(), id);
    r.identity.attribute = "remote_key".into();
    for inputs in [
        &mut r.create.inputs,
        &mut r.read.inputs,
        &mut r.update.inputs,
        &mut r.delete.inputs,
    ] {
        for input in inputs {
            if input.attribute == "id" {
                input.attribute = "remote_key".into();
            }
        }
    }
    for response in std::iter::once(&mut r.create.success)
        .chain(&mut r.create.partial)
        .chain(std::iter::once(&mut r.read.success))
        .chain(std::iter::once(&mut r.update.success))
        .chain(&mut r.update.partial)
    {
        let path = response.state.remove("id").unwrap();
        response.state.insert("remote_key".into(), path);
    }
    let planned = plan_provider(contract(), p, config()).unwrap();
    let files = emit_provider(&planned);
    let resource = files
        .iter()
        .find(|f| f.path == "terraform/provider/resource_record.go")
        .unwrap();
    assert!(resource.content.contains("path.Root(\"remote_key\")"));
    assert!(!resource.content.contains("path.Root(\"id\")"));
}

#[test]
fn one_semantic_pipeline_preserves_canonical_sdk_bytes() {
    let plan = plan();
    let sdk = suspect_codegen::go_http::emit_http(
        plan.sdk_plan(),
        &suspect_codegen::go_http::PackageConfig {
            module_path: config().sdk.module_path,
            package_name: "sdk".into(),
            version: config().sdk.version,
        },
    )
    .unwrap();
    let actual = emit_provider(&plan)
        .into_iter()
        .filter(|f| f.path.starts_with("go/"))
        .collect::<Vec<_>>();
    assert_eq!(sdk, actual);
    assert_eq!(emit_provider(&plan), emit_provider(&plan));
    // Optional immutable output retention for focused post-review byte checks.
    // The semantic assertions above always execute in the ordinary host suite.
    if let Some(root) = std::env::var_os("SUSPECT_TERRAFORM_ARTIFACT_SNAPSHOT") {
        let root = PathBuf::from(root);
        std::fs::create_dir(&root).expect("artifact snapshot must be new");
        suspect_codegen::write_files_with_owner(
            &emit_provider(&plan),
            &root.join("generated"),
            "terraform-review-snapshot",
            suspect_codegen::Adoption::Refuse,
        )
        .unwrap();
        std::fs::copy(
            std::env::current_exe().unwrap(),
            root.join("generator-test.bin"),
        )
        .unwrap();
    }
}

#[test]
fn desired_artifacts_preserve_ownership_and_unchanged_metadata() {
    let root = tempfile::tempdir().unwrap();
    let files = emit_provider(&plan());
    let owner = "terraform-test-fixture";
    suspect_codegen::write_files_with_owner(
        &files,
        root.path(),
        owner,
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    let path = root.path().join("terraform/provider/resource_record.go");
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    suspect_codegen::write_files_with_owner(
        &files,
        root.path(),
        owner,
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    assert_eq!(
        before,
        std::fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert!(
        suspect_codegen::check_files_with_owner(&files, root.path(), owner)
            .unwrap()
            .is_current()
    );
    std::fs::write(&path, "user work\n").unwrap();
    assert!(
        suspect_codegen::write_files_with_owner(
            &files,
            root.path(),
            owner,
            suspect_codegen::Adoption::Refuse
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "user work\n");
}

#[test]
#[ignore = "requires real Go/Terraform; retains every native attempt under an explicit evidence root"]
fn real_terraform_lifecycle_through_pinned_generated_sdk() {
    let evidence = std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE")
        .expect("set a fresh SUSPECT_TERRAFORM_EVIDENCE directory");
    let evidence = PathBuf::from(evidence);
    std::fs::create_dir(&evidence).expect("native evidence must be new");
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-terraform-native-")
        .tempdir_in("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    let files = emit_provider(&plan());
    suspect_codegen::write_files_with_owner(
        &files,
        &root.join("generated"),
        "terraform-native-fixture",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/sdk-terraform-acceptance.py");
    let output = std::process::Command::new("python3")
        .arg(&script)
        .arg("--root")
        .arg(&root)
        .arg("--evidence")
        .arg(&evidence)
        .arg("--fixtures")
        .arg(fixture())
        .output()
        .unwrap();
    std::fs::write(evidence.join("runner.stdout.log"), &output.stdout).unwrap();
    std::fs::write(evidence.join("runner.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "retained {} and {}\n{}\n{}",
        root.display(),
        evidence.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "supplemental native profile/SDK-upgrade controls; requires a fresh evidence path"]
fn native_optional_computed_anonymous_and_sdk_upgrade_controls() {
    let evidence = PathBuf::from(
        std::env::var_os("SUSPECT_TERRAFORM_EVIDENCE").expect("set a fresh evidence directory"),
    );
    std::fs::create_dir(&evidence).unwrap();
    std::fs::copy(
        std::env::current_exe().unwrap(),
        evidence.join("generator-test.bin"),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-terraform-variants-")
        .tempdir_in("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
        .unwrap()
        .keep();
    std::fs::write(
        evidence.join("native-root.txt"),
        format!("{}\n", root.display()),
    )
    .unwrap();
    let mut p = profile();
    p.resources
        .get_mut("record")
        .unwrap()
        .attributes
        .get_mut("memo")
        .unwrap()
        .mode = AttributeMode::OptionalComputed;
    let optional = plan_provider(contract(), p, config()).unwrap();
    suspect_codegen::write_files_with_owner(
        &emit_provider(&optional),
        &root.join("optional-computed/generated"),
        "terraform-variant-optional",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();

    let sources = root.join("anonymous/sources");
    std::fs::create_dir_all(&sources).unwrap();
    let mut document: Value =
        serde_json::from_str(include_str!("fixtures/terraform-v1/openapi.json")).unwrap();
    document["security"] = json!([]);
    std::fs::write(sources.join("openapi.json"), document.to_string()).unwrap();
    std::fs::write(
        sources.join("schemas.json"),
        include_str!("fixtures/terraform-v1/schemas.json"),
    )
    .unwrap();
    let mut p = profile();
    p.resources.clear();
    p.authentication = Authentication::None;
    let mut c = config();
    c.sdk.version = "0.4.3".into();
    let anonymous = plan_provider(load(&sources), p, c).unwrap();
    assert_eq!(anonymous.sdk_plan().operations().len(), 1);
    assert_ne!(
        anonymous.sdk_plan().operations()[0].method_name,
        plan()
            .sdk_plan()
            .operations()
            .iter()
            .find(|o| o.operation_id == "read_item")
            .unwrap()
            .method_name
    );
    suspect_codegen::write_files_with_owner(
        &emit_provider(&anonymous),
        &root.join("anonymous/generated"),
        "terraform-variant-anonymous",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    // A current-source base artifact set supports byte comparison with the
    // completed full native receipt without replaying those lifecycle gates.
    suspect_codegen::write_files_with_owner(
        &emit_provider(&plan()),
        &root.join("base/generated"),
        "terraform-variant-base",
        suspect_codegen::Adoption::Refuse,
    )
    .unwrap();
    let output = std::process::Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/sdk-terraform-variants.py"))
        .arg("--root")
        .arg(&root)
        .arg("--evidence")
        .arg(&evidence)
        .arg("--fixtures")
        .arg(fixture())
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
