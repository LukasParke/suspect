//! Real command-line interpretation, unnamed-operation and cached configuration paths.
use serde_json::{Value, json};
use std::{path::Path, process::Command};

fn command(root: &Path, args: &[&str], exit: i32) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture() -> Value {
    json!({"openapi":"3.1.2","info":{"title":"API title","version":"API version"},
        "servers":[{"url":"https://example.test/v1"}],"security":[],"paths":{
        "/blob":{"get":{"operationId":"downloadBlob","responses":{"200":{"description":"Bytes",
            "content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}},
        "/json":{"get":{"operationId":"jsonValue","responses":{"200":{"description":"Value",
            "content":{"application/json":{"schema":{"type":"string"}}}}}}},
        "/health":{"get":{"responses":{"204":{"description":"No payload"}}}}
    }})
}

fn generate(root: &Path, extra: &[&str], exit: i32) -> Value {
    let mut args = vec![
        "codegen",
        "api.json",
        "--profile",
        "go-http",
        "--package-name",
        "example.test/options-sdk",
        "--package-version",
        "1.2.3",
        "--out",
        "generated",
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    command(root, &args, exit)
}

#[test]
fn public_codegen_keeps_standard_bytes_and_explicit_profile_configuration_distinct() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    let source = serde_json::to_vec_pretty(&fixture()).unwrap();
    std::fs::write(&path, &source).unwrap();
    let ordinary = generate(root.path(), &["--operation-id", "downloadBlob"], 1);
    assert_eq!(ordinary["status"], "failed");
    assert_eq!(ordinary["compatibilityProfiles"], json!([]));
    assert!(
        ordinary["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |finding| finding["pointer"]
                    .as_str()
                    .is_some_and(|pointer| pointer.starts_with(
                        "/paths/~1blob/get/responses/200/content/application~1octet-stream"
                    ))
                    && finding["range"]["end"].as_u64() > finding["range"]["start"].as_u64()
            )
    );
    assert!(!root.path().join("generated").exists());
    let args = [
        "--operation-id",
        "downloadBlob",
        "--compatibility-profile",
        "legacy-binary-string-v1",
    ];
    let enabled = generate(root.path(), &args, 0);
    assert_eq!(
        enabled["compatibilityProfiles"],
        json!(["legacy-binary-string-v1"])
    );
    let output = root.path().join("generated/go/operations.go");
    let before = std::fs::read(&output).unwrap();
    let time = std::fs::metadata(&output).unwrap().modified().unwrap();
    let mut check = args.to_vec();
    check.push("--check");
    assert_eq!(generate(root.path(), &check, 0)["status"], "current");
    assert_eq!(
        std::fs::metadata(&output).unwrap().modified().unwrap(),
        time
    );
    generate(root.path(), &["--operation-id", "downloadBlob"], 1);
    assert_eq!(std::fs::read(output).unwrap(), before);
    assert_eq!(std::fs::read(path).unwrap(), source);
    let invalid = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root.path())
        .args([
            "codegen",
            "api.json",
            "--profile",
            "go-http",
            "--package-name",
            "example.test/options-sdk",
            "--package-version",
            "1.2.3",
            "--out",
            "invalid-output",
            "--compatibility-profile",
            "legacy-binary-string-v2",
        ])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(!root.path().join("invalid-output").exists());
}

#[test]
fn anonymous_unnamed_operations_use_the_canonical_expanded_typescript_pipeline() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture();
    source["paths"]
        .as_object_mut()
        .unwrap()
        .retain(|key, _| key == "/health");
    std::fs::write(root.path().join("api.json"), source.to_string()).unwrap();
    let report = command(
        root.path(),
        &[
            "codegen",
            "api.json",
            "--profile",
            "typescript-http",
            "--package-name",
            "@fixture/health",
            "--package-version",
            "0.1.0",
            "--out",
            "generated",
            "--format",
            "json",
        ],
        0,
    );
    assert_eq!(report["operations"][0]["operationId"], Value::Null);
    assert_eq!(report["operations"][0]["method"], "GET");
    assert_eq!(report["operations"][0]["path"], "/health");
    assert_eq!(report["operations"][0]["pointer"], "/paths/~1health/get");
    assert!(
        root.path()
            .join("generated/typescript/http-manifest.json")
            .is_file()
    );
    assert_eq!(report["compatibilityProfiles"], json!([]));
}

#[test]
fn unsupported_native_import_names_fail_before_creating_output() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture().to_string();
    std::fs::write(root.path().join("api.json"), &source).unwrap();
    let report = generate(
        root.path(),
        &[
            "--operation-id",
            "jsonValue",
            "--import-name",
            "ignored_import",
        ],
        1,
    );
    assert_eq!(report["status"], "failed");
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "sdk-package"
                && finding["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("import_name")))
    );
    assert!(!root.path().join("generated").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("api.json")).unwrap(),
        source
    );
}

#[test]
fn session_preview_and_comparison_retain_and_compare_explicit_interpretation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("api.json"), fixture().to_string()).unwrap();
    let mut config = json!({"spec":"api.json","operation_ids":["jsonValue"],"targets":[{
        "backend":"go-http","package_name":"example.test/options-sdk","package_version":"1.2.3"}]});
    std::fs::write(root.path().join("before.json"), config.to_string()).unwrap();
    config["compatibility_profiles"] = json!(["legacy-binary-string-v1"]);
    std::fs::write(root.path().join("after.json"), config.to_string()).unwrap();
    let preview = |config: &str| {
        command(
            root.path(),
            &[
                "codegen-session",
                "--config",
                config,
                "--out",
                "preview",
                "--preview",
                "--format",
                "json",
            ],
            1,
        )
    };
    let ordinary = preview("before.json");
    let enabled = preview("after.json");
    assert_eq!(ordinary["status"], "drift");
    assert_eq!(
        enabled["compatibilityProfiles"],
        json!(["legacy-binary-string-v1"])
    );
    assert_ne!(ordinary["revision"], enabled["revision"]);
    assert!(!root.path().join("preview").exists());
    let changed = command(
        root.path(),
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
    assert!(
        changed["wire"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "wire-interpretation-profile-changed")
    );
    assert_eq!(
        changed["after"]["generation"]["compatibility_profiles"],
        json!(["legacy-binary-string-v1"])
    );
    let stable = command(
        root.path(),
        &[
            "codegen-compare",
            "--before",
            "after.json",
            "--after",
            "after.json",
            "--format",
            "json",
        ],
        0,
    );
    assert_eq!(stable["summary"]["unknowns"], 0);
    config["operation_ids"] = json!(["downloadBlob"]);
    std::fs::write(root.path().join("bytes.json"), config.to_string()).unwrap();
    assert_eq!(preview("bytes.json")["status"], "drift");
    config["compatibility_profiles"] = json!(["unversioned-binary"]);
    std::fs::write(root.path().join("bad.json"), config.to_string()).unwrap();
    assert_eq!(preview("bad.json")["status"], "planning-error");
    let discovery = command(root.path(), &["codegen-profiles", "--format", "json"], 0);
    assert_eq!(
        discovery["compatibilityProfiles"],
        json!(["legacy-binary-string-v1"])
    );
}
