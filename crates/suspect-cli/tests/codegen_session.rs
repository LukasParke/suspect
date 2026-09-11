//! Public CLI process seam: readonly preview, persistent refresh and ownership.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api.json"), spec("first").to_string()).unwrap();
    std::fs::write(dir.path().join("session.json"),json!({"spec":"api.json","targets":[{"backend":"typescript-http","package_name":"test-sdk","package_version":"0.0.0"},{"backend":"rust-http","package_name":"test-sdk","package_version":"0.0.0"}],"owner":"session-test"}).to_string()).unwrap();
    dir
}
fn spec(description: &str) -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Session","version":"1"},"servers":[{"url":"https://example.test/v1"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/items":{"get":{"operationId":"getItems","description":description,"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"object","required":["value"],"properties":{"value":{"type":"string"}}}}}}}}}}})
}
fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_suspect"));
    command
        .args([
            "codegen-session",
            "--config",
            "session.json",
            "--out",
            "output",
            "--format",
            "json",
        ])
        .current_dir(root);
    command
}
fn run(root: &Path, args: &[&str]) -> (i32, Vec<Value>) {
    let output = command(root).args(args).output().unwrap();
    let lines = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (output.status.code().unwrap(), lines)
}
#[test]
fn preview_is_readonly_and_write_check_preserve_user_edits() {
    let root = setup();
    let (code, records) = run(root.path(), &["--preview"]);
    assert_eq!(code, 1);
    let preview = &records[0];
    assert_eq!(preview["format"], "suspect.sdk.session.v1");
    assert_eq!(preview["status"], "drift");
    assert!(!preview["artifacts"].as_array().unwrap().is_empty());
    assert!(!root.path().join("output").exists());
    let (code, records) = run(root.path(), &[]);
    assert_eq!(code, 0);
    assert_eq!(records[0]["delta"]["compiles"], 1);
    assert_eq!(records[0]["delta"]["renders"], 2);
    assert!(root.path().join("output/rust/Cargo.toml").is_file());
    assert_eq!(run(root.path(), &["--check"]).0, 0);
    std::fs::write(root.path().join("output/typescript/models.ts"), "user work").unwrap();
    let (code, records) = run(root.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(records[0]["status"], "write-conflict");
    assert_eq!(
        std::fs::read_to_string(root.path().join("output/typescript/models.ts")).unwrap(),
        "user work"
    );
}
#[test]
fn watch_reuses_one_contract_and_publishes_source_changes() {
    let root = setup();
    let mut child = command(root.path())
        .args(["--watch", "--interval-ms", "100", "--max-iterations", "8"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let first: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(first["status"], "written");
    line.clear();
    reader.read_line(&mut line).unwrap();
    let unchanged: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(unchanged["delta"]["compiles"], 0);
    assert_eq!(unchanged["delta"]["renders"], 0);
    assert_eq!(unchanged["delta"]["cache_hits"], 1);
    std::fs::write(root.path().join("api.json"), spec("second").to_string()).unwrap();
    let mut changed = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        };
        let record: Value = serde_json::from_str(&line).unwrap();
        if record["delta"]["compiles"] == 1 {
            changed = Some(record);
        }
    }
    assert!(child.wait().unwrap().success());
    let changed = changed.expect("changed source notification");
    assert_eq!(changed["stats"]["compiles"], 2);
    assert_eq!(changed["status"], "written");
    assert!(
        std::fs::read_to_string(root.path().join("output/typescript/operations.ts"))
            .unwrap()
            .contains("second")
    );
}
#[test]
fn malformed_config_and_unsupported_source_have_explicit_diagnostics() {
    let root = setup();
    std::fs::write(
        root.path().join("session.json"),
        "{\"spec\":\"api.json\",\"targets\":[],\"typo\":true}",
    )
    .unwrap();
    let (code, records) = run(root.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(records[0]["status"], "planning-error");
    assert!(!root.path().join("output").exists());
    let root = setup();
    let mut value = spec("bad");
    value["security"] = json!([{"missing":[]}]);
    std::fs::write(root.path().join("api.json"), value.to_string()).unwrap();
    let (code, records) = run(root.path(), &["--preview"]);
    assert_eq!(code, 1);
    assert!(
        records[0]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |finding| finding["code"] == "http-security-scheme-unresolved"
                    && finding["source"]["pointer"] == "/security/0/missing"
            )
    );
}

#[test]
fn readonly_watch_publishes_cached_reverts_with_the_same_drift_paths() {
    let root = setup();
    let mut child = command(root.path())
        .args([
            "--watch",
            "--preview",
            "--interval-ms",
            "50",
            "--max-iterations",
            "12",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let read = |reader: &mut BufReader<std::process::ChildStdout>| {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).expect("watch result")
    };
    let first = read(&mut reader);
    assert_eq!(first["status"], "drift");
    std::fs::write(root.path().join("api.json"), spec("second").to_string()).unwrap();
    let changed = read(&mut reader);
    assert_ne!(first["revision"], changed["revision"]);
    std::fs::write(root.path().join("api.json"), spec("first").to_string()).unwrap();
    let reverted = read(&mut reader);
    assert_eq!(reverted["delta"]["compiles"], 0);
    assert_eq!(reverted["delta"]["renders"], 0);
    assert_eq!(reverted["revision"], first["revision"]);
    assert_eq!(reverted["artifacts"], first["artifacts"]);
    assert_eq!(reverted["changedArtifacts"], changed["changedArtifacts"]);
    assert_eq!(child.wait().unwrap().code(), Some(1));
    assert!(!root.path().join("output").exists());
}

#[test]
fn watch_detects_creation_of_an_initially_missing_example_file() {
    let root = setup();
    let mut document = spec("missing example");
    document["paths"]["/items"]["get"]["responses"]["200"]["content"]["application/json"]["examples"] =
        json!({"external":{"$ref":"example.json#/entry"}});
    std::fs::write(root.path().join("api.json"), document.to_string()).unwrap();
    let mut child = command(root.path())
        .args([
            "--watch",
            "--preview",
            "--interval-ms",
            "25",
            "--max-iterations",
            "40",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let first: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(first["status"], "drift");
    std::fs::write(
        root.path().join("example.json"),
        r#"{"entry":{"value":{"value":"recovered example"}}}"#,
    )
    .unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let recovered: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(recovered["delta"]["compiles"], 1);
    assert_ne!(recovered["revision"], first["revision"]);
    assert!(
        recovered["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "typescript/examples.json"
                && file["content"]
                    .as_str()
                    .unwrap()
                    .contains("recovered example"))
    );
    assert_eq!(child.wait().unwrap().code(), Some(1));
}
