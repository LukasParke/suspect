//! Explicit acquisition followed by offline CLI generation/watch/comparison.

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_source::Uri;

fn run(root: &Path, arguments: &[&str], expected: i32) -> Output {
    let result = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(expected),
        "{arguments:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn acquired_remote_schema_drives_offline_codegen_session_and_comparison() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let model_uri = format!("{origin}/model.json");
    let model=br#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false}"#;
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut request = BufReader::new(&stream);
        let mut first = String::new();
        request.read_line(&mut first).unwrap();
        assert!(first.starts_with("GET /model.json "), "{first}");
        loop {
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            assert!(!line.is_empty());
        }
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",model.len()).unwrap();
        stream.write_all(model).unwrap();
    });
    let entry = root.join("api.json");
    let entry_uri = Uri::from_path(&entry).unwrap().to_string();
    let source = json!({"openapi":"3.1.0","info":{"title":"Pinned CLI","version":"1"},
        "servers":[{"url":"https://api.example.test/v1"}],"security":[{"Bearer":[]}],
        "components":{"securitySchemes":{"Bearer":{"type":"http","scheme":"bearer"}}},
        "paths":{"/message":{"get":{"operationId":"getMessage","responses":{"200":{"description":"Message",
            "content":{"application/json":{"schema":{"$ref":model_uri}}}}}}}}});
    let bytes = serde_json::to_vec(&source).unwrap();
    std::fs::write(&entry, &bytes).unwrap();
    let pin = |uri: &str, bytes: &[u8], remote: bool| {
        json!({"requested_uri":uri,"effective_uri":uri,
        "digest":format!("sha256-{:x}",Sha256::digest(bytes)),"media_type":"application/json", "via":if remote {"direct"}else{"local"},
        "redirects":[],"retrieved_at":"2026-09-10T00:00:00Z","attempts":usize::from(remote)})
    };
    std::fs::write(
        root.join("pins.json"),
        serde_json::to_vec(&json!({"manifest_version":1,"entry":entry_uri,
        "resources":[pin(&entry_uri,&bytes,false),pin(&model_uri,model,true)]}))
        .unwrap(),
    )
    .unwrap();
    let acquired = run(
        root,
        &[
            "acquire",
            "pins.json",
            "--cache-dir",
            "cache",
            "--insecure-test-origin",
            &origin,
            "--format",
            "json",
        ],
        0,
    );
    peer.join().unwrap(); // No server remains for compilation or cache verification.
    let acquisition: Value = serde_json::from_slice(&acquired.stdout).unwrap();
    assert_eq!(acquisition["networkRequests"], 1);
    assert_eq!(acquisition["documents"].as_array().unwrap().len(), 2);
    std::fs::remove_file(entry).unwrap();

    let direct = run(
        root,
        &[
            "codegen",
            "--pins",
            "pins.json",
            "--cache-dir",
            "cache",
            "--insecure-test-origin",
            &origin,
            "--profile",
            "typescript-http",
            "--package-name",
            "pinned-sdk",
            "--package-version",
            "1.0.0",
            "--out",
            "direct",
            "--format",
            "json",
        ],
        0,
    );
    let report: Value = serde_json::from_slice(&direct.stdout).unwrap();
    assert!(
        report["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "typescript/package.json")
    );
    let config = json!({"pins":{"manifest":"pins.json","cache_dir":"cache","insecure_test_origins":[origin]},
        "targets":[{"backend":"typescript-http","package_name":"pinned-sdk","package_version":"1.0.0"}]});
    std::fs::write(
        root.join("session.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let watched = run(
        root,
        &[
            "codegen-session",
            "--config",
            "session.json",
            "--out",
            "session",
            "--watch",
            "--max-iterations",
            "2",
            "--interval-ms",
            "25",
            "--format",
            "json",
        ],
        0,
    );
    let records = String::from_utf8(watched.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["sourceDocument"], entry_uri);
    assert_eq!(records[0]["delta"]["compiles"], 1);
    assert_eq!(records[1]["delta"]["compiles"], 0);
    assert_eq!(records[1]["delta"]["renders"], 0);
    assert_eq!(records[0]["revision"], records[1]["revision"]);
    let comparison = run(
        root,
        &[
            "codegen-compare",
            "--before",
            "session.json",
            "--after",
            "session.json",
            "--format",
            "json",
        ],
        0,
    );
    let comparison: Value = serde_json::from_slice(&comparison.stdout).unwrap();
    assert_eq!(comparison["before"]["entry"], entry_uri);
    assert_eq!(comparison["summary"]["unknowns"], 0);

    let cached = acquisition["documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|doc| doc["requestedUri"] == model_uri)
        .unwrap()["cachePath"]
        .as_str()
        .unwrap();
    std::fs::remove_file(root.join(cached)).unwrap();
    let rejected = run(
        root,
        &[
            "codegen-session",
            "--config",
            "session.json",
            "--out",
            "session",
            "--check",
            "--format",
            "json",
        ],
        1,
    );
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["diagnostics"][0]["code"], "pin-cache-missing");
}
