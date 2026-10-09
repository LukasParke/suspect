//! Reading recorded traffic files and rendering HAR archives.

use std::io::Write;

use suspect_cli::commands::har::{HarEntry, build_archive};
use suspect_cli::commands::traffic_file::{iso8601, read_traffic_lines};

#[test]
fn journals_and_cassettes_read_line_by_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("traffic.jsonl");
    let mut file = std::fs::File::create(&path).expect("create");
    // Cassette header, a cassette entry, a journal traffic record, a
    // journal meta record — in one mixed file.
    let lines_json = [
        r#"{"format":"suspect.cassette.v1","version":1}"#,
        r#"{"id":1,"method":"GET","url":"http://x/identity","status":200,"request_headers":[["accept","application/json"]],"request_body":{"encoding":"utf8","content":"{}","sha256":"x"},"response_headers":[["content-type","application/json"]],"response_body":{"encoding":"utf8","content":"{}"},"duration_ms":1.5}"#,
        r#"{"kind":"traffic","ts_ms":1751779000000,"id":2,"correlation":"t/1","method":"GET","url":"http://x/hubs","status":404,"request_headers":[],"response_headers":[],"duration_ms":2.0,"verdict":{"pass":[]}}"#,
        r#"{"kind":"meta","ts_ms":1,"component":"gateway","msg":"started"}"#,
    ];
    for line in lines_json {
        writeln!(file, "{line}").unwrap();
    }
    drop(file);

    let lines = read_traffic_lines(&path).expect("reads");
    assert_eq!(lines.len(), 2, "headers and meta records are skipped");
    let first = &lines[0];
    assert_eq!(first.method, "GET");
    assert_eq!(first.url, "http://x/identity");
    assert_eq!(first.status, 200);
    assert_eq!(
        first.request_headers,
        vec![("accept".to_owned(), "application/json".to_owned())]
    );
    assert!(first.response_body.is_some(), "cassette bodies are kept");
    assert_eq!(first.ts_ms, 0, "cassette entries carry order, not clock");
    let second = &lines[1];
    assert_eq!(second.status, 404);
    assert!(second.response_body.is_none(), "journals carry no bodies");
    assert_eq!(second.ts_ms, 1_751_779_000_000);
}

#[test]
fn iso8601_arithmetic_is_exact() {
    assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
    // 2026-10-09T00:00:00Z = 1791504000 seconds.
    assert_eq!(iso8601(1_791_504_000_000), "2026-10-09T00:00:00.000Z");
    assert_eq!(iso8601(1_791_504_000_123), "2026-10-09T00:00:00.123Z");
    // Leap-year boundary: 2024-02-29T12:34:56Z = 1709210096.
    assert_eq!(iso8601(1_709_210_096_000), "2024-02-29T12:34:56.000Z");
}

#[test]
fn har_document_shape() {
    let entry = HarEntry {
        method: "GET".to_owned(),
        url: "http://x/identity?check=1".to_owned(),
        status: 200,
        request_headers: vec![("accept".to_owned(), "application/json".to_owned())],
        response_headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        response_body: Some((suspect_journal::BodyEncoding::Utf8, "{}".to_owned())),
        duration_ms: 3.25,
        ts_ms: 1_791_504_000_000,
    };
    let archive = build_archive(&[entry]);
    let log = &archive["log"];
    assert_eq!(log["version"], "1.2");
    let har_entry = &log["entries"][0];
    assert_eq!(har_entry["request"]["method"], "GET");
    assert_eq!(har_entry["request"]["queryString"][0]["name"], "check");
    assert_eq!(har_entry["response"]["status"], 200);
    assert_eq!(
        har_entry["response"]["content"]["mimeType"],
        "application/json"
    );
    assert_eq!(har_entry["response"]["content"]["text"], "{}");
    assert_eq!(har_entry["startedDateTime"], "2026-10-09T00:00:00.000Z");
    assert_eq!(har_entry["timings"]["wait"], 3.25);
}
