//! Standalone native checks for the generated, dependency-free Rust JSON runtime.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[path = "../src/rust_models/runtime.rs"]
#[allow(dead_code)]
mod support;
pub use support::{JsonNonNullValue, JsonNumber, JsonValue, Nullable};
#[path = "../src/rust_codecs/json_runtime.rs"]
mod json;

use json::{JsonErrorKind, JsonLimits, parse_json, parse_json_bytes, stringify_json};

fn roundtrip(input: &str, expected: &str) {
    let value = parse_json(input, JsonLimits::default()).unwrap();
    assert_eq!(
        stringify_json(&value, JsonLimits::default()).unwrap(),
        expected
    );
}

#[test]
fn exact_numbers_unicode_duplicates_and_syntax_are_explicit() {
    roundtrip(
        "[-0,9007199254740992,9007199254740993,1E+400,1e-400]",
        "[-0,9007199254740992,9007199254740993,1E+400,1e-400]",
    );
    roundtrip(
        r#"{"music":"\uD834\uDD1E","solidus":"\/","é":"é"}"#,
        r#"{"music":"𝄞","solidus":"/","é":"é"}"#,
    );

    let duplicate = parse_json(r#"{"x":1,"x":2}"#, JsonLimits::default()).unwrap_err();
    assert_eq!(duplicate.kind, JsonErrorKind::DuplicateKey);
    for input in [
        "01",
        "1.",
        "1e",
        "[1,]",
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
    ] {
        assert_eq!(
            parse_json(input, JsonLimits::default()).unwrap_err().kind,
            JsonErrorKind::Syntax,
            "{input}"
        );
    }
    let invalid = parse_json_bytes(&[b'"', 0xff, b'"'], JsonLimits::default()).unwrap_err();
    assert_eq!(invalid.kind, JsonErrorKind::InvalidUtf8);
}

#[test]
fn byte_depth_and_explicit_scan_traversal_budgets_are_enforced() {
    let defaults = JsonLimits::default();
    let input = JsonLimits {
        max_input_bytes: 1,
        ..defaults
    };
    assert_eq!(
        parse_json("null", input).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    let depth = JsonLimits {
        max_depth: 1,
        ..defaults
    };
    assert_eq!(
        parse_json("[[]]", depth).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    let work = JsonLimits {
        max_work: 2,
        ..defaults
    };
    assert_eq!(
        parse_json("true", work).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    let value = parse_json(r#"{"long":"payload"}"#, defaults).unwrap();
    let output = JsonLimits {
        max_output_bytes: 4,
        ..defaults
    };
    assert_eq!(
        stringify_json(&value, output).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );
    let write_depth = JsonLimits {
        max_depth: 0,
        ..defaults
    };
    assert_eq!(
        stringify_json(&value, write_depth).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );
    let write_work = JsonLimits {
        max_work: 1,
        ..defaults
    };
    assert_eq!(
        stringify_json(&value, write_work).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    let impossible_depth = JsonLimits {
        max_depth: usize::MAX,
        ..defaults
    };
    assert_eq!(
        parse_json("null", impossible_depth).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );
    assert_eq!(
        stringify_json(&value, impossible_depth).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    // Numeric scanning spends work incrementally, before reaching or owning
    // the end of an attacker-controlled token.
    let long_number = "1".repeat(100_000);
    let tiny_work = JsonLimits {
        max_input_bytes: long_number.len(),
        max_work: 8,
        ..defaults
    };
    assert_eq!(
        parse_json(&long_number, tiny_work).unwrap_err().kind,
        JsonErrorKind::ResourceLimit
    );

    // Non-ASCII decoding is linear and each encoded byte is charged once.
    let unicode = format!("\"{}\"", "é".repeat(10_000));
    let bounded = JsonLimits {
        max_input_bytes: unicode.len(),
        max_work: unicode.len() + 2,
        ..defaults
    };
    parse_json(&unicode, bounded).unwrap();
}

#[test]
fn prioritized_openrouter_payload_examples_roundtrip_exactly() {
    // These payload shapes come from the four first normalized OpenRouter roots.
    // JSON runtime round-tripping is intentionally schema-neutral; codec tests
    // will separately establish validity and convert them to native models.
    let cases = [
        (
            "ORAnthropicNullableCaller",
            r#"{"tool_id":"tool","type":"code_execution_20260120"}"#,
        ),
        (
            "AnthropicImageBlockParam",
            r#"{"source":{"data":"AA==","media_type":"image/png","type":"base64"},"type":"image"}"#,
        ),
        ("ChatChoice.index", "9007199254740993e+400"),
        (
            "ImageGenerationServerToolConfig",
            r#"{"custom":[true,null,{"scale":-0}],"model":"openai/gpt-5-image","output_compression":85.00,"quality":"high"}"#,
        ),
    ];
    for (root, oracle) in cases {
        let value = parse_json(oracle, JsonLimits::default())
            .unwrap_or_else(|error| panic!("{root}: {error}"));
        assert_eq!(
            stringify_json(&value, JsonLimits::default()).unwrap(),
            oracle,
            "{root}"
        );
    }
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT; missing tracked inputs fail"]
fn all_tracked_openrouter_documents_roundtrip_against_json_value_oracle() {
    let root = PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT")
            .expect("set OPENROUTER_WEB_ROOT to the tracked source checkout"),
    );
    for relative in [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ] {
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "missing required tracked input: {}",
            path.display()
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap())
            .expect("tracked document must normalize");
        // Contract supplies the canonical YAML/JSON normalization. serde_json
        // is the independent JSON grammar/value oracle after that boundary.
        let oracle = contract.document(contract.entry()).unwrap();
        let input = serde_json::to_string(oracle).unwrap();
        let budget = input.len().checked_mul(8).unwrap();
        let limits = JsonLimits {
            max_input_bytes: input.len(),
            max_output_bytes: input.len().checked_mul(2).unwrap(),
            max_work: budget,
            ..JsonLimits::default()
        };
        let parsed = parse_json(&input, limits)
            .unwrap_or_else(|error| panic!("{relative}: parse failed: {error}"));
        let output = stringify_json(&parsed, limits)
            .unwrap_or_else(|error| panic!("{relative}: write failed: {error}"));
        assert_eq!(
            parse_json(&output, limits)
                .and_then(|value| stringify_json(&value, limits))
                .unwrap(),
            output,
            "{relative}: output is unstable"
        );
        let actual: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(&actual, oracle, "{relative}: normalized document changed");
    }
}

#[test]
#[ignore = "requires native Cargo"]
fn generated_style_package_has_clean_docs_consumer_and_dependency_tree() {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("generated");
    let consumer = directory.path().join("consumer");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rust_models/runtime.rs"),
        package.join("src/support.rs"),
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rust_codecs/json_runtime.rs"),
        package.join("src/json.rs"),
    )
    .unwrap();
    std::fs::write(package.join("Cargo.toml"), "[package]\nname='generated-exact-json'\nversion='0.0.0'\nedition='2024'\nrust-version='1.88'\n[workspace]\n").unwrap();
    std::fs::write(package.join("src/lib.rs"), r#"#![forbid(unsafe_code)]
mod support;
pub use support::{ExtraFieldError, JsonInteger, JsonNonNullValue, JsonNumber, JsonValue, Never, Nullable, NumberError, Presence};
pub mod json;
"#).unwrap();
    std::fs::write(consumer.join("Cargo.toml"), format!("[package]\nname='exact-json-consumer'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nsdk={{package='generated-exact-json',path={:?}}}\n", package)).unwrap();
    std::fs::write(consumer.join("src/lib.rs"), r##"#[test]
fn public_api() {
    let limits = sdk::json::JsonLimits::default();
    let value = sdk::json::parse_json(r#"{"index":9007199254740993,"zero":-0}"#, limits).unwrap();
    assert_eq!(sdk::json::stringify_json(&value, limits).unwrap(), r#"{"index":9007199254740993,"zero":-0}"#);
}
"##).unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-json");
    for (manifest, args) in [
        (
            consumer.join("Cargo.toml"),
            vec!["test", "--offline", "--quiet"],
        ),
        (
            package.join("Cargo.toml"),
            vec!["doc", "--offline", "--no-deps"],
        ),
    ] {
        let mut command = Command::new("cargo");
        command
            .args(args)
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target);
        command.env("RUSTDOCFLAGS", "-D warnings");
        let output = command.output().unwrap();
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
        .output()
        .unwrap();
    assert!(tree.status.success());
    let tree = String::from_utf8(tree.stdout).unwrap();
    assert!(
        !tree.contains("suspect-"),
        "generated dependency tree leaked compiler crates:\n{tree}"
    );
}
