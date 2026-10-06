//! The three orphan features wired from the audit: stateful sequence
//! generation+execution, grammar-evolved fuzzing, and reverse
//! (spec drift from server source) — end to end through the real binary.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn suspect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
}

const API: &str = "openapi: 3.1.0
info: {title: Orphans, version: '1'}
paths:
  /users:
    post:
      operationId: createUser
      responses:
        '201': {description: ok}
  /users/{userId}/posts:
    post:
      operationId: createPost
      responses:
        '201': {description: ok}
  /users/{userId}/posts/{postId}:
    delete:
      operationId: deletePost
      responses:
        '204': {description: ok}
  /search:
    get:
      operationId: search
      parameters:
        - name: q
          in: query
          schema: {type: string}
      responses:
        '200': {description: ok}
";

/// A resource-ish server: POST mints ids, DELETE takes 204, GET takes 200.
struct ResourceServer {
    port: u16,
    paths: Arc<std::sync::Mutex<Vec<String>>>,
}

impl ResourceServer {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let next = Arc::new(AtomicU64::new(0));
        let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = Arc::clone(&paths);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let next = Arc::clone(&next);
                let recorder = Arc::clone(&recorder);
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let first = head.lines().next().unwrap_or_default().to_owned();
                    recorder.lock().unwrap().push(first.clone());
                    let (status, body) = if first.starts_with("POST") {
                        (
                            "201 Created",
                            format!(r#"{{"id":{}}}"#, next.fetch_add(1, Ordering::SeqCst) + 1),
                        )
                    } else if first.starts_with("DELETE") {
                        ("204 No Content", String::new())
                    } else {
                        ("200 OK", r#"{"results":[]}"#.to_owned())
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                });
            }
        });
        Self { port, paths }
    }
}

#[test]
fn stateful_generates_and_executes_resource_sequences() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    std::fs::write(root.join("api.yaml"), API).expect("api");
    let server = ResourceServer::spawn();
    let output = suspect()
        .arg("stateful")
        .arg(root.join("api.yaml"))
        .args(["--base-url", &format!("http://127.0.0.1:{}", server.port)])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "every sequence passes: {stdout} {stderr}"
    );
    for expected in ["createUser", "createPost", "deletePost"] {
        assert!(
            stdout.contains(&format!("{expected:<44}")),
            "sequence {expected} runs: {stdout}"
        );
    }
    assert!(
        stdout.contains("0 failed"),
        "the summary counts failures: {stdout}"
    );
    // The delete sequence proves id threading: DELETE /users/<id>/posts/<id>.
    let requests = server.paths.lock().unwrap().clone();
    let deletes: Vec<&String> = requests
        .iter()
        .filter(|line| line.starts_with("DELETE"))
        .collect();
    assert_eq!(
        deletes.len(),
        2,
        "the createPost teardown and the deletePost target each delete once"
    );
    assert!(
        !deletes[0].contains("{userId}") && !deletes[0].contains("{postId}"),
        "created ids substituted the path templates: {}",
        deletes[0]
    );
}

#[test]
fn stateful_emit_prints_sequences_without_running() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    std::fs::write(root.join("api.yaml"), API).expect("api");
    let output = suspect()
        .arg("stateful")
        .arg(root.join("api.yaml"))
        .arg("--emit")
        .output()
        .expect("run");
    assert!(output.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("emit prints sequences as JSON");
    assert!(
        parsed.as_array().is_some_and(|seqs| !seqs.is_empty()),
        "sequences are reviewable without a server: {parsed}"
    );
    assert!(
        parsed[0]["steps"][0]["id"].as_str().is_some(),
        "steps carry the ids param sources reference: {parsed}"
    );
}

#[test]
fn fuzz_evolved_runs_a_coverage_guided_campaign() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    std::fs::write(root.join("api.yaml"), API).expect("api");
    let server = ResourceServer::spawn();
    let output = suspect()
        .arg("fuzz")
        .arg(root.join("api.yaml"))
        .args(["--base-url", &format!("http://127.0.0.1:{}", server.port)])
        .args(["--filter", "search", "--evolved"])
        .args(["--rounds", "3", "--per-round", "4"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the campaign survives a benign server: {stdout} {stderr}"
    );
    assert!(
        stdout.contains("evolved fuzz complete: 12 requests"),
        "the campaign ran every planned request: {stdout}"
    );
    assert!(
        stdout.contains("corpus:"),
        "the campaign reports its corpus: {stdout}"
    );
}

#[test]
fn reverse_finds_undocumented_and_spec_only_endpoints() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    std::fs::write(
        src.join("main.rs"),
        r#"use axum::{routing::get, Router};
fn routes() -> Router {
    Router::new()
        .route("/pets", get(list_pets))
        .route("/secret-admin", get(admin))
}
"#,
    )
    .expect("source");
    std::fs::write(
        root.join("spec.yaml"),
        r#"openapi: 3.1.0
info: {title: P, version: "1"}
paths:
  /pets:
    get: {responses: {"200": {description: ok}}}
  /ghost:
    get: {responses: {"200": {description: ok}}}
"#,
    )
    .expect("spec");
    let output = suspect()
        .arg("reverse")
        .arg(&src)
        .arg(root.join("spec.yaml"))
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "the report is a report, not a failure: {stdout}"
    );
    assert!(
        stdout.contains("undocumented: GET /secret-admin"),
        "implemented-but-undocumented endpoints surface: {stdout}"
    );
    assert!(
        stdout.contains("spec-only: GET /ghost"),
        "documented-but-unimplemented endpoints surface: {stdout}"
    );
    assert!(
        stdout.contains("suggested spec fragment"),
        "undocumented endpoints come with a fix: {stdout}"
    );
    assert!(
        stdout.contains("1 undocumented, 1 spec-only"),
        "the summary counts both directions: {stdout}"
    );

    let output = suspect()
        .arg("reverse")
        .arg(&src)
        .arg(root.join("spec.yaml"))
        .args(["--format", "json"])
        .output()
        .expect("run");
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(
        parsed["undocumented"]
            .as_array()
            .is_some_and(|x| !x.is_empty()),
        "the JSON report carries the mismatches: {parsed}"
    );
}

/// Silence the unused warning when Path is only used in cfg(unix) tests.
#[allow(dead_code)]
fn _path_used(_: &Path) {}
