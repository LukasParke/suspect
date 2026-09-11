//! Explicit runtime credential policy reaches the canonical CLI/compare seam.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

fn run(root: &Path, args: &[&str], code: i32) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn environment_policy_is_explicit_in_reports_and_cannot_be_ignored_before_writes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("api.json"),
        json!({
            "openapi":"3.1.2","info":{"title":"Explicit client defaults","version":"1"},
            "servers":[{"url":"https://source.example.test/v1"}],
            "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
            "paths":{"/value":{"get":{"operationId":"value","security":[{"apiKey":[]}],
                "responses":{"204":{"description":"Done"}}}}}
        })
        .to_string(),
    )
    .unwrap();
    let before = json!({"spec":"api.json","targets":[{
        "backend":"go-http","package_name":"example.com/env-policy","package_version":"0.1.0"
    }]});
    let policy = json!({"version":"v1","schemes":{"not-declared":"RUNTIME_CREDENTIAL"}});
    let mut after = before.clone();
    after["credential_env"] = policy.clone();
    std::fs::write(root.join("before.json"), before.to_string()).unwrap();
    std::fs::write(root.join("after.json"), after.to_string()).unwrap();
    let rejected = run(
        root,
        &[
            "codegen-session",
            "--config",
            "after.json",
            "--out",
            "unwritten",
            "--format",
            "json",
        ],
        1,
    );
    assert_eq!(rejected["status"], "planning-error");
    assert!(
        rejected["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("sdk-credential-env-")))
    );
    assert!(!root.join("unwritten").exists());
    let compared = run(
        root,
        &[
            "codegen-compare",
            "--before",
            "before.json",
            "--after",
            "after.json",
            "--format",
            "json",
        ],
        1,
    );
    assert_eq!(compared["wire"], json!([]));
    assert_eq!(compared["after"]["generation"]["credential_env"], policy);
    assert!(
        compared["native"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["code"] == "native-credential-env-changed")
    );
    assert!(!root.join("unwritten").exists());
}

fn output_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    files
}

fn generate_with_canary(root: &Path, config: &str, out: &str, canary: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .env("OPENROUTER_API_KEY", canary)
        .args([
            "codegen-session",
            "--config",
            config,
            "--out",
            out,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(canary));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(canary));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn configured_generation_reports_names_only_and_disabling_restores_all_ordinary_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("api.json"),
        json!({
            "openapi":"3.1.2","info":{"title":"Source-bound defaults","version":"1"},
            "servers":[{"url":"https://source.example.test/v1"}],
            "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
            "security":[{"apiKey":[]}],
            "paths":{"/key":{"get":{"operationId":"getCurrentKey",
                "responses":{"204":{"description":"Done"}}}}}
        })
        .to_string(),
    )
    .unwrap();
    let ordinary = json!({"spec":"api.json","targets":[{
        "backend":"python-http","package_name":"openrouter","package_version":"0.1.0",
        "import_name":"openrouter"
    }]});
    let policy = json!({"version":"v1","schemes":{"apiKey":"OPENROUTER_API_KEY"}});
    let mut configured = ordinary.clone();
    configured["credential_env"] = policy.clone();
    std::fs::write(root.join("ordinary.json"), ordinary.to_string()).unwrap();
    std::fs::write(root.join("configured.json"), configured.to_string()).unwrap();
    let canaries = [
        "ControlledGeneratorCanaryAlpha7291",
        "ControlledGeneratorCanaryBeta5463",
    ];
    let first = generate_with_canary(root, "configured.json", "configured", canaries[0]);
    assert_eq!(first["credentialEnv"], policy);
    let configured_files = output_files(&root.join("configured"));
    assert!(
        configured_files
            .keys()
            .any(|path| path.ends_with("_credential_env.py"))
    );
    let different_environment =
        generate_with_canary(root, "configured.json", "second-process", canaries[1]);
    assert_eq!(first["revision"], different_environment["revision"]);
    assert_eq!(configured_files, output_files(&root.join("second-process")));
    let unconfigured = generate_with_canary(root, "ordinary.json", "ordinary", canaries[0]);
    assert!(unconfigured.get("credentialEnv").is_none());
    assert_ne!(first["revision"], unconfigured["revision"]);
    let ordinary_files = output_files(&root.join("ordinary"));
    assert!(
        !ordinary_files
            .keys()
            .any(|path| path.ends_with("_credential_env.py"))
    );
    let disabled = generate_with_canary(root, "ordinary.json", "configured", canaries[1]);
    assert_eq!(disabled["revision"], unconfigured["revision"]);
    assert!(disabled.get("credentialEnv").is_none());
    assert_eq!(ordinary_files, output_files(&root.join("configured")));
    for content in configured_files.values().chain(ordinary_files.values()) {
        let text = String::from_utf8_lossy(content);
        assert!(canaries.iter().all(|value| !text.contains(value)));
    }
}
