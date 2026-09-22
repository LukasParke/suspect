//! Canonical Rust profile selection, artifact ownership and native package consumption.

use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args([
            "codegen",
            "api.json",
            "--profile",
            "rust-http",
            "--package-name",
            "fixture-rust-sdk",
            "--package-version",
            "0.0.0",
            "--out",
            "output",
            "--format",
            "json",
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn report(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Rust SDK","version":"1"},
    "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
    "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
    "paths":{"/credits":{"get":{"operationId":"getCredits","responses":{
        "200":{"description":"balance","content":{"application/json":{"schema":{"type":"number"}}}}
    }}}}})
}

#[test]
fn rust_profile_preserves_owned_output_and_reports_selected_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api.json"), fixture().to_string()).unwrap();
    assert_eq!(report(&run(dir.path(), &["--check"]), 1)["status"], "drift");
    assert!(!dir.path().join("output").exists());
    let generated = report(&run(dir.path(), &[]), 0);
    assert_eq!(generated["profile"], "rust-http");
    assert_eq!(generated["releaseReady"], false);
    assert_eq!(
        generated["operations"][0]["pointer"],
        "/paths/~1credits/get"
    );
    let root = dir.path().join("output/rust");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("http-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["package"]["name"], "fixture-rust-sdk");
    assert_eq!(manifest["package"]["crate"], "fixture_rust_sdk");
    assert_eq!(
        report(&run(dir.path(), &["--check"]), 0)["status"],
        "current"
    );
    let file = root.join("src/operations/get_credits.rs");
    std::fs::write(&file, "caller-owned edit").unwrap();
    assert_eq!(report(&run(dir.path(), &[]), 1)["status"], "conflict");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "caller-owned edit");
}

#[test]
fn rust_profile_rejects_unsupported_selected_contracts_without_changing_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    let mut spec = fixture();
    spec["components"]["securitySchemes"]["digest"] = json!({"type":"http","scheme":"digest"});
    spec["paths"]["/unsupported"] = json!({"get":{"operationId":"unsupported","security":[{"digest":[]}],"responses":{"204":{"description":"empty"}}}});
    std::fs::write(&path, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
    report(&run(dir.path(), &["--operation-id", "getCredits"]), 0);
    let file = dir.path().join("output/rust/src/operations/get_credits.rs");
    let original = std::fs::read(&file).unwrap();
    let failed = report(&run(dir.path(), &[]), 1);
    assert!(
        failed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "http-security-http-scheme"
                && d["pointer"] == "/components/securitySchemes/digest/scheme"
                && d["line"].as_u64().unwrap() > 1)
    );
    let located = failed["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "http-security-http-scheme")
        .unwrap();
    let start = located["range"]["start"].as_u64().unwrap() as usize;
    let end = located["range"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&std::fs::read(&path).unwrap()[start..end], b"\"digest\"");
    assert_eq!(std::fs::read(&file).unwrap(), original);
    let missing = report(&run(dir.path(), &["--operation-id", "getCredit"]), 1);
    assert_eq!(missing["diagnostics"][0]["code"], "sdk-operation-not-found");
}

#[test]
#[ignore = "requires native Cargo and pinned HTTP dependencies"]
fn rust_cli_output_is_a_native_feature_gated_cargo_package() {
    let temporary = tempfile::tempdir().unwrap();
    let dir = temporary.keep();
    std::fs::write(dir.join("api.json"), fixture().to_string()).unwrap();
    report(&run(&dir, &[]), 0);
    for args in [
        vec!["check"],
        vec!["test", "--doc", "--features", "http"],
        vec!["check", "--all-features"],
        vec!["doc", "--no-deps", "--all-features"],
    ] {
        let output = Command::new("cargo")
            .args(args)
            .args(["--offline", "--quiet", "--manifest-path"])
            .arg(dir.join("output/rust/Cargo.toml"))
            .arg("--target-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-http-cli"))
            .env_remove("RUST_MIN_STACK")
            .env("RUSTFLAGS", "-D warnings")
            .env("RUSTDOCFLAGS", "-D warnings")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture {}\n{}{}",
            dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
