//! Consumer behavior through Contract -> generated Rust package with the
//! optional `serde-json` feature: adapter fields, exact numbers, absence/null,
//! mutated-model rejection, ambiguous oneOf rejection, and dependency-free
//! builds with the feature off.
use serde_json::json;
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::rust_codecs::{CodecConfig, CodecPlan, plan_codecs};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn fixture(f: impl FnOnce(Arc<Contract>)) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "openapi":"3.1.0","info":{"title":"Serde contract","version":"1"},"paths":{},
            "components":{"schemas":{
                "Area":{"type":"object","required":["big","tiny","expo"],
                    "properties":{"big":{"type":"integer"},"tiny":{"type":"number"},
                        "expo":{"type":"number"},"opt":{"type":"string"},"maybe":{"type":["string","null"]}}},
                "Alias":{"type":"string","minLength":2},
                "Exclusive":{"oneOf":[{"type":"integer"},{"type":"number"}]},
                "Result":{"type":"object","required":["ok"],"properties":{"ok":{"$ref":"#/components/schemas/Alias"}}},
                "Node":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/Node"}}}
            }}
        }))
        .unwrap(),
    )
    .unwrap();
    f(contract(&path));
}

fn cargo(dir: &Path, manifest: &Path, args: &[&str]) {
    let mut command = Command::new("cargo");
    command
        .arg(args[0])
        .args(["--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .args(&args[1..])
        .arg("--target-dir")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-serde"))
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "fixture {} {args:?}\n{}{}",
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn native(plan: &CodecPlan, consumer_lib: &str) {
    // Cross-binary serialization with sibling native suites.
    static NATIVE_CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _cache = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render(), dir.path()).unwrap();
    let generated = dir.path().join("rust");
    let mut tree = Command::new("cargo");
    tree.args(["tree", "--offline", "--edges", "normal", "--manifest-path"])
        .arg(generated.join("Cargo.toml"));
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        tree.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    let tree = tree.output().unwrap();
    assert!(
        tree.status.success(),
        "{}",
        String::from_utf8_lossy(&tree.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&tree.stdout).lines().count(),
        1,
        "default package must have no active normal dependency"
    );
    // Feature off: the package must compile and test with no serde dependency.
    cargo(dir.path(), &generated.join("Cargo.toml"), &["test"]);
    // Feature on: package tests, doctests and docs.
    cargo(
        dir.path(),
        &generated.join("Cargo.toml"),
        &["test", "--features", "serde-json"],
    );
    cargo(
        dir.path(),
        &generated.join("Cargo.toml"),
        &["test", "--doc", "--features", "serde-json"],
    );
    cargo(
        dir.path(),
        &generated.join("Cargo.toml"),
        &["doc", "--no-deps", "--features", "serde-json"],
    );
    // Native consumer: serde_json arbitrary_precision on the caller side, codec
    // adapter fields on a wrapper struct. Named `Result` model avoids namespace
    // issues through explicit module paths.
    let consumer = dir.path().join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"serde-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[features]\ncaller-precision=[\"serde_json/arbitrary_precision\"]\n[dependencies]\nserde-generated={package=\"generated-models\",path=\"../rust\",features=[\"serde-json\"]}\nserde={version=\"=1.0.229\",features=[\"derive\"]}\nserde_json={version=\"=1.0.151\",features=[\"raw_value\"]}\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("src/lib.rs"),
        format!("#![cfg(test)]\n{consumer_lib}"),
    )
    .unwrap();
    cargo(dir.path(), &consumer.join("Cargo.toml"), &["test"]);
    cargo(
        dir.path(),
        &consumer.join("Cargo.toml"),
        &["test", "--features", "caller-precision"],
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn serde_json_adapters_preserve_exact_numbers_and_validation() {
    fixture(|contract| {
        let plan = plan_codecs(
            contract.clone(),
            contract.schema_roots(),
            CodecConfig::default(),
        )
        .unwrap();
        native(
            &plan,
            r##"
use serde_generated::{codecs::{AreaCodec, AliasCodec, ExclusiveCodec, ResultCodec}, models::{Area, Alias, Exclusive, Result as ApiResult}, Presence};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Wrapper {
    #[serde(with = "AreaCodec")]
    area: Area,
    #[serde(with = "AliasCodec")]
    alias: Alias,
    #[serde(with = "ResultCodec")]
    outcome: ApiResult,
    #[serde(with = "ExclusiveCodec")]
    winner: Exclusive,
}

#[test] fn exact_numbers_round_trip_through_the_validated_codec() {
    let input = r#"{"area":{"big":9007199254740993,"tiny":1e-400,"expo":1E+400,"maybe":null},"alias":"ok","outcome":{"ok":"yes"},"winner":1.5}"#;
    let wrapper: Wrapper = serde_json::from_str(input).unwrap();
    let output = serde_json::to_string(&wrapper).unwrap();
    // Object keys are emitted in codec (sorted) order; number tokens are verbatim.
    assert_eq!(
        output,
        r#"{"area":{"big":9007199254740993,"expo":1E+400,"maybe":null,"tiny":1e-400},"alias":"ok","outcome":{"ok":"yes"},"winner":1.5}"#
    );
}

#[test] fn absence_is_distinct_from_null_and_omitted_on_output() {
    let input = r#"{"area":{"big":7,"tiny":2.5,"expo":1e-400},"alias":"ok","outcome":{"ok":"yes"},"winner":1.5}"#;
    let wrapper: Wrapper = serde_json::from_str(input).unwrap();
    assert!(matches!(wrapper.area.maybe, Presence::Absent));
    assert!(wrapper.area.opt.is_none());
    let output = serde_json::to_string(&wrapper).unwrap();
    assert_eq!(
        output,
        r#"{"area":{"big":7,"expo":1e-400,"tiny":2.5},"alias":"ok","outcome":{"ok":"yes"},"winner":1.5}"#
    );
}

#[test] fn mutated_invalid_models_and_ambiguous_unions_are_rejected() {
    let input = r#"{"area":{"big":7,"tiny":2.5,"expo":1e-400},"alias":"ok","outcome":{"ok":"yes"},"winner":1.5}"#;
    let mut wrapper: Wrapper = serde_json::from_str(input).unwrap();
    // Serialization always re-runs the validated encode, so post-deserialization
    // mutation cannot smuggle invalid data into JSON.
    wrapper.alias.clear();
    assert!(serde_json::to_string(&wrapper).is_err());
    // `1` validates both oneOf branches; the codec rejects the ambiguity and the
    // adapter surfaces it as a deserialization error.
    assert!(serde_json::from_str::<Wrapper>(
        r#"{"area":{"big":7,"tiny":2.5,"expo":1e-400},"alias":"ok","outcome":{"ok":"yes"},"winner":1}"#
    )
    .is_err());
}
#[test]
fn recursive_values_and_json_rejections_go_through_real_codecs() {
    #[derive(Serialize, Deserialize)]
    struct Tree {
        #[serde(with="serde_generated::codecs::NodeCodec")]
        node: serde_generated::models::Node,
    }
    let mut value = r#"{"label":"leaf"}"#.to_owned();
    for _ in 0..30 {value=format!("{{\"label\":\"parent\",\"child\":{value}}}");}
    let source=format!("{{\"node\":{value}}}");
    let tree: Tree = serde_json::from_str(&source).unwrap();
    let encoded = serde_json::to_string(&tree).unwrap();
    let repeated: Tree = serde_json::from_str(&encoded).unwrap();
    assert_eq!(encoded,serde_json::to_string(&repeated).unwrap());
    let mut cursor = &tree.node;
    let mut depth = 0;
    while let Some(child) = &cursor.child {cursor=child;depth+=1;}
    assert_eq!(depth,30);
    assert_eq!(cursor.label,"leaf");
    for invalid in [r#"{"node":{"label":"a","label":"b"}}"#,r#"{"node":{"label":"a","n":01}}"#,r#"{"node":{"label":"a","n":1.}}"#] {
        assert!(serde_json::from_str::<Tree>(invalid).is_err());
    }
    assert!(serde_json::from_slice::<Tree>(b"{\"node\":{\"label\":\"\xff\"}}").is_err());
    for _ in 0..200 {value=format!("{{\"label\":\"parent\",\"child\":{value}}}");}
    assert!(serde_json::from_str::<Tree>(&format!("{{\"node\":{value}}}")).is_err());
}
"##,
        );
    });
}
