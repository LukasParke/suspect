//! Exact JSON integration through the canonical Rust `ModelPlan` package.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use suspect_codegen::rust_models::plan_models;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Contract {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap()
}

fn synthetic(schemas: Value) -> Contract {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("openapi.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "openapi":"3.1.0", "info":{"title":"Rust JSON", "version":"1"},
            "paths":{}, "components":{"schemas":schemas}
        }))
        .unwrap(),
    )
    .unwrap();
    // Contract owns its normalized source data independently of the temp input.
    contract(&path)
}

fn root(contract: &Contract, name: &str) -> SchemaId {
    contract
        .schemas()
        .find(|schema| schema.id().pointer() == format!("/components/schemas/{name}"))
        .unwrap()
        .id()
        .clone()
}

#[test]
fn model_plan_emits_and_documents_public_exact_json_api() {
    let contract = synthetic(json!({"Payload":{"type":"object"}}));
    let plan = plan_models(&contract, &[root(&contract, "Payload")]);
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "model-codec-unimplemented")
    );
    let files = plan.render().unwrap();
    let json = files
        .iter()
        .find(|file| file.path == "rust/src/json.rs")
        .unwrap();
    assert!(json.content.contains("pub fn parse_json("));
    let lib = &files
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap()
        .content;
    for public in [
        "JsonError",
        "JsonErrorKind",
        "JsonLimits",
        "parse_json",
        "parse_json_bytes",
        "stringify_json",
    ] {
        assert!(lib.contains(public), "missing public JSON export {public}");
    }
    let readme = &files
        .iter()
        .find(|file| file.path == "rust/README.md")
        .unwrap()
        .content;
    assert!(readme.contains("do not validate schemas"));
    assert!(readme.contains("Unicode scalar values"));
}

#[test]
#[ignore = "requires native Cargo"]
fn qualified_source_models_coexist_with_root_json_support_exports() {
    let contract = synthetic(json!({
        "JsonValue":{"type":"string"}, "JsonError":{"type":"boolean"}
    }));
    let roots = [root(&contract, "JsonValue"), root(&contract, "JsonError")];
    let files = plan_models(&contract, &roots).render().unwrap();
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let consumer = directory.path().join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(consumer.join("Cargo.toml"), format!(
        "[package]\nname='qualified-json-names'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\ngenerated_models={{package='generated-models',path={:?}}}\n",
        directory.path().join("rust")
    )).unwrap();
    std::fs::write(consumer.join("src/lib.rs"), r#"
pub fn model_value(value: generated_models::models::JsonValue) -> String { value }
pub fn model_error(value: generated_models::models::JsonError) -> bool { value }
pub fn support_value() -> generated_models::JsonValue { generated_models::Nullable::Null }
pub fn support_error_kind(error: &generated_models::JsonError) -> generated_models::JsonErrorKind { error.kind }
"#).unwrap();
    let output = Command::new("cargo")
        .args(["check", "--offline", "--manifest-path"])
        .arg(consumer.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-json-names"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires tracked OpenRouter checkout and native Cargo"]
fn tracked_models_and_four_documents_pass_native_package_gates() {
    let checkout = PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT")
            .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into()),
    );
    let primary = checkout.join("projects/docs/openapi/openapi.yaml");
    assert!(primary.is_file(), "missing tracked OpenRouter checkout");
    let primary_contract = contract(&primary);
    let roots = [
        root(&primary_contract, "ORAnthropicNullableCaller"),
        root(&primary_contract, "AnthropicImageBlockParam"),
    ];
    let files = plan_models(&primary_contract, &roots).render().unwrap();
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let package = directory.path().join("rust");
    let consumer = directory.path().join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();

    let relatives = [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ];
    let mut oracles = Vec::new();
    for (index, relative) in relatives.iter().enumerate() {
        let path = checkout.join(relative);
        assert!(path.is_file(), "missing tracked input {}", path.display());
        let document = contract(&path);
        let oracle = document.document(document.entry()).unwrap().clone();
        std::fs::write(
            consumer.join(format!("input{index}.json")),
            serde_json::to_vec(&oracle).unwrap(),
        )
        .unwrap();
        oracles.push(oracle);
    }

    std::fs::write(consumer.join("Cargo.toml"), format!(
        "[package]\nname='rust-json-consumer'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\ngenerated_models={{package='generated-models',path={:?}}}\n",
        package
    )).unwrap();
    std::fs::write(consumer.join("src/lib.rs"), r##"
#[test]
fn models_and_exact_json_are_public_and_executable() {
    use generated_models::{Nullable, models::*, parse_json, stringify_json, JsonLimits};
    let caller = OrAnthropicNullableCaller::Value(
        OrAnthropicNullableCallerValue::CodeExecution20260120(
            AnthropicCodeExecution20260120Caller::new("tool".into())
        )
    );
    assert!(matches!(caller, Nullable::Value(_)));
    let image = AnthropicImageBlockParam::new(AnthropicImageBlockParamSource::Url(
        AnthropicUrlImageSource::new("https://example.test/image".into())
    ));
    assert_eq!(image.type_.wire_json(), "\"image\"");
    let exact = r#"{"index":9007199254740993,"zero":-0}"#;
    let limits = JsonLimits::default();
    assert_eq!(stringify_json(&parse_json(exact, limits).unwrap(), limits).unwrap(), exact);
    for index in 0..4 {
        let input = std::fs::read(format!("input{index}.json")).unwrap();
        let size = input.len();
        let limits = JsonLimits { max_input_bytes:size, max_output_bytes:size*2, max_work:size*8, ..JsonLimits::default() };
        let value = generated_models::parse_json_bytes(&input, limits).unwrap();
        let output = stringify_json(&value, limits).unwrap();
        assert_eq!(stringify_json(&parse_json(&output, limits).unwrap(), limits).unwrap(), output);
        std::fs::write(format!("output{index}.json"), output).unwrap();
    }
}
"##).unwrap();

    let target =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-json-model-plan");
    for (manifest, args) in [
        (
            consumer.join("Cargo.toml"),
            &["test", "--offline", "--quiet"][..],
        ),
        (
            package.join("Cargo.toml"),
            &["doc", "--offline", "--no-deps"][..],
        ),
    ] {
        let output = Command::new("cargo")
            .args(args)
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target)
            .current_dir(&consumer)
            .env("RUSTDOCFLAGS", "-D warnings")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for manifest in [package.join("Cargo.toml"), consumer.join("Cargo.toml")] {
        let output = Command::new("cargo")
            .args(["clippy", "--offline", "--all-targets", "--manifest-path"])
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target)
            .arg("--")
            .arg("-Dwarnings")
            .current_dir(&consumer)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let tree = Command::new("cargo")
        .args(["tree", "--offline", "--manifest-path"])
        .arg(consumer.join("Cargo.toml"))
        .current_dir(&consumer)
        .output()
        .unwrap();
    assert!(tree.status.success());
    assert!(!String::from_utf8(tree.stdout).unwrap().contains("suspect-"));
    for (index, oracle) in oracles.iter().enumerate() {
        let actual: Value = serde_json::from_slice(
            &std::fs::read(consumer.join(format!("output{index}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(&actual, oracle, "{} changed", relatives[index]);
    }
}
