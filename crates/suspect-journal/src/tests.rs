use crate::{
    Body, BodyEncoding, CASSETTE_FORMAT, CASSETTE_VERSION, CassetteEntry, CassetteHeader, Journal,
    Level, REDACTED, Record, Redactor, TrafficRecord, VecSink, Verdict, read_cassette, sha256_hex,
    write_cassette,
};

fn traffic(correlation: &str) -> TrafficRecord {
    TrafficRecord {
        ts_ms: 1_700_000_000_000,
        id: 0,
        correlation: correlation.to_owned(),
        method: "POST".to_owned(),
        url: "https://api.example.com/v1/pets".to_owned(),
        status: Some(201),
        request_headers: vec![
            ("Authorization".to_owned(), "Bearer sekrit".to_owned()),
            ("X-Custom".to_owned(), "visible".to_owned()),
        ],
        response_headers: vec![("Set-Cookie".to_owned(), "sid=xyz".to_owned())],
        duration_ms: 12.5,
        verdict: Verdict::Pass,
    }
}

#[test]
fn journal_lines_parse_and_sequence() {
    let sink = VecSink::default();
    let mut journal = Journal::new(Box::new(sink.clone()));
    journal.traffic(traffic("wf/step"));
    journal.log(Level::Info, "test", "hello", serde_json::json!({ "k": 1 }));
    let recs = sink.records();
    assert_eq!(recs.len(), 2);
    match &recs[0] {
        Record::Traffic(t) => {
            assert_eq!(t.id, 0);
            assert_eq!(t.correlation, "wf/step");
            assert_eq!(t.method, "POST");
        }
        other => panic!("expected traffic, got {other:?}"),
    }
    match &recs[1] {
        Record::Log(l) => {
            assert_eq!(l.msg, "hello");
            assert_eq!(l.level, Level::Info);
        }
        other => panic!("expected log, got {other:?}"),
    }
}

#[test]
fn redactor_scrubs_headers_and_json_keys() {
    let mut r = Redactor::new();
    r.deny_header("x-trace-token");
    let headers = r.headers(&[
        ("authorization".to_owned(), "Bearer x".to_owned()),
        ("X-Trace-Token".to_owned(), "abc".to_owned()),
        ("Accept".to_owned(), "application/json".to_owned()),
    ]);
    assert_eq!(headers[0].1, REDACTED);
    assert_eq!(headers[1].1, REDACTED);
    assert_eq!(headers[2].1, "application/json");

    let body = r.json_body(r#"{"user":"a","password":"p","nested":{"api_key":"k"},"n":3}"#);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["password"], REDACTED);
    assert_eq!(v["nested"]["api_key"], REDACTED);
    assert_eq!(v["user"], "a");
    // Non-JSON passes through untouched.
    assert_eq!(r.json_body("<html>ok</html>"), "<html>ok</html>");
}

#[test]
fn traffic_through_journal_is_redacted() {
    let sink = VecSink::default();
    let mut journal = Journal::new(Box::new(sink.clone()));
    journal.traffic(traffic("c"));
    let recs = sink.records();
    match &recs[0] {
        Record::Traffic(t) => {
            assert!(
                t.request_headers
                    .iter()
                    .any(|(k, v)| k == "Authorization" && v == REDACTED),
                "authorization must be redacted: {t:?}"
            );
            assert!(
                t.response_headers
                    .iter()
                    .any(|(k, v)| k == "Set-Cookie" && v == REDACTED)
            );
            assert!(
                t.request_headers
                    .iter()
                    .any(|(k, v)| k == "X-Custom" && v == "visible"),
                "non-denied headers stay visible"
            );
        }
        other => panic!("expected traffic, got {other:?}"),
    }
}

#[test]
fn run_summary_and_meta_roundtrip() {
    let sink = VecSink::default();
    let mut journal = Journal::new(Box::new(sink.clone()));
    journal.emit(Journal::meta(
        "gateway",
        "starting",
        serde_json::json!({ "port": 8080 }),
    ));
    journal.run_summary("test", 3, 1, 0, 250.5);
    let recs = sink.records();
    match &recs[0] {
        Record::Meta(m) => {
            assert_eq!(m.component, "gateway");
            assert_eq!(m.fields["port"], 8080);
        }
        other => panic!("expected meta, got {other:?}"),
    }
    match &recs[1] {
        Record::RunSummary(s) => {
            assert_eq!(s.run_kind, "test");
            assert_eq!((s.passed, s.failed, s.skipped), (3, 1, 0));
            assert_eq!(s.duration_ms, 250.5);
        }
        other => panic!("expected run summary, got {other:?}"),
    }
}

#[test]
fn cassette_roundtrips_binary_and_text_bodies() {
    let header = CassetteHeader {
        format: CASSETTE_FORMAT.to_owned(),
        version: CASSETTE_VERSION,
        recorded_at_ms: 42,
        source: "test".to_owned(),
    };
    let entries = vec![
        CassetteEntry {
            id: 1,
            method: "GET".to_owned(),
            url: "http://x/y".to_owned(),
            status: 200,
            request_headers: vec![],
            request_body: Body::from_bytes(b""),
            response_headers: vec![],
            response_body: Body::from_bytes(b"{\"a\":1}"),
            duration_ms: 1.0,
        },
        CassetteEntry {
            id: 2,
            method: "POST".to_owned(),
            url: "http://x/bin".to_owned(),
            status: 201,
            request_headers: vec![],
            request_body: Body::from_bytes(&[0u8, 159, 146, 150]),
            response_headers: vec![],
            response_body: Body::from_bytes(b"ok"),
            duration_ms: 2.5,
        },
    ];
    let mut buf: Vec<u8> = Vec::new();
    write_cassette(&mut buf, &header, &entries).unwrap();
    let (read_header, read_entries) = read_cassette(buf.as_slice()).unwrap();
    assert_eq!(read_header.format, CASSETTE_FORMAT);
    assert_eq!(read_header.version, CASSETTE_VERSION);
    assert_eq!(read_entries.len(), 2);
    assert_eq!(read_entries[0].response_body.text(), Some("{\"a\":1}"));
    assert_eq!(
        read_entries[1].request_body.bytes(),
        vec![0u8, 159, 146, 150]
    );
    assert_eq!(read_entries[0].request_body.sha256, sha256_hex(b""));
}

#[test]
fn cassette_rejects_bad_format_version_and_ids() {
    let bad_format = format!(
        "{{\"format\":\"other\",\"version\":{CASSETTE_VERSION},\"recorded_at_ms\":0,\"source\":\"s\"}}\n"
    );
    assert!(read_cassette(bad_format.as_bytes()).is_err());
    let newer = format!(
        "{{\"format\":\"{CASSETTE_FORMAT}\",\"version\":99,\"recorded_at_ms\":0,\"source\":\"s\"}}\n"
    );
    assert!(read_cassette(newer.as_bytes()).is_err());
    let header = CassetteHeader {
        format: CASSETTE_FORMAT.to_owned(),
        version: CASSETTE_VERSION,
        recorded_at_ms: 0,
        source: String::new(),
    };
    let entry = CassetteEntry {
        id: 5,
        method: "GET".to_owned(),
        url: String::new(),
        status: 200,
        request_headers: vec![],
        request_body: Body::from_bytes(b""),
        response_headers: vec![],
        response_body: Body::from_bytes(b""),
        duration_ms: 0.0,
    };
    let mut buf: Vec<u8> = Vec::new();
    write_cassette(&mut buf, &header, &[entry]).unwrap();
    assert!(read_cassette(buf.as_slice()).is_err());
}

#[test]
fn body_hash_detects_tampering() {
    let original = Body::from_bytes(br#"{"status":"ok"}"#);
    let tampered = Body {
        content: r#"{"status":"evil"}"#.to_owned(),
        sha256: original.sha256.clone(),
        encoding: BodyEncoding::Utf8,
    };
    assert_ne!(sha256_hex(tampered.bytes().as_slice()), tampered.sha256);
}

fn valid_entry(id: u64) -> CassetteEntry {
    CassetteEntry {
        id,
        method: "GET".to_owned(),
        url: "http://x/y".to_owned(),
        status: 200,
        request_headers: vec![],
        request_body: Body::from_bytes(b"in"),
        response_headers: vec![],
        response_body: Body::from_bytes(b"out"),
        duration_ms: 1.0,
    }
}

fn header_line() -> String {
    format!(
        "{{\"format\":\"{CASSETTE_FORMAT}\",\"version\":{CASSETTE_VERSION},\
         \"recorded_at_ms\":0,\"source\":\"s\"}}\n"
    )
}

fn cassette_with_entries(entries: &[serde_json::Value]) -> Vec<u8> {
    let mut buf = header_line().into_bytes();
    for entry in entries {
        buf.extend_from_slice(serde_json::to_string(entry).unwrap().as_bytes());
        buf.push(b'\n');
    }
    buf
}

#[test]
fn log_and_meta_fields_are_redacted() {
    let sink = VecSink::default();
    let mut journal = Journal::new(Box::new(sink.clone()));
    journal.log(
        Level::Info,
        "gateway",
        "login",
        serde_json::json!({ "user": "a", "password": "hunter2",
            "nested": { "api_key": "k" } }),
    );
    journal.emit(Journal::meta(
        "gateway",
        "starting",
        serde_json::json!({ "token": "t" }),
    ));
    let recs = sink.records();
    match &recs[0] {
        Record::Log(l) => {
            assert_eq!(l.fields["user"], "a");
            assert_eq!(l.fields["password"], REDACTED);
            assert_eq!(l.fields["nested"]["api_key"], REDACTED);
        }
        other => panic!("expected log, got {other:?}"),
    }
    match &recs[1] {
        Record::Meta(m) => assert_eq!(m.fields["token"], REDACTED),
        other => panic!("expected meta, got {other:?}"),
    }
    for rec in &recs {
        let line = serde_json::to_string(rec).unwrap();
        assert!(
            !line.contains("hunter2") && !line.contains("\"t\""),
            "credential leaked into sink output: {line}"
        );
    }
}

#[test]
fn json_key_redaction_is_case_insensitive() {
    let mut r = Redactor::new();
    let body = r.json_body(r#"{"Password":"p","User":"a","NESTED":{"Token":"t"}}"#);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["Password"], REDACTED);
    assert_eq!(v["NESTED"]["Token"], REDACTED);
    assert_eq!(v["User"], "a");

    // A denylist key added in mixed case still matches any casing.
    r.deny_json_key("SECRET");
    let mut value = serde_json::json!({ "secret": "s", "SeCreT": "s2", "keep": 1 });
    r.json_value(&mut value);
    assert_eq!(value["secret"], REDACTED);
    assert_eq!(value["SeCreT"], REDACTED);
    assert_eq!(value["keep"], 1);
}

#[test]
fn read_cassette_rejects_bad_base64_and_bad_sha() {
    // Declared base64 whose content does not decode strictly.
    let mut corrupt = serde_json::to_value(valid_entry(1)).unwrap();
    corrupt["response_body"]["encoding"] = serde_json::json!("base64");
    corrupt["response_body"]["content"] = serde_json::json!("!!!not base64!!!");
    assert!(read_cassette(cassette_with_entries(&[corrupt]).as_slice()).is_err());

    // Valid base64 whose stored hash belongs to different bytes.
    let mismatched = CassetteEntry {
        response_body: Body {
            encoding: BodyEncoding::Base64,
            content: "YWJj".to_owned(), // base64 of "abc"
            sha256: sha256_hex(b"xyz"),
        },
        ..valid_entry(1)
    };
    let line = serde_json::to_value(&mismatched).unwrap();
    assert!(read_cassette(cassette_with_entries(&[line]).as_slice()).is_err());

    // Hash that is not 64 hex characters.
    let short_hash = CassetteEntry {
        response_body: Body {
            sha256: "abcd".to_owned(),
            ..Body::from_bytes(b"out")
        },
        ..valid_entry(1)
    };
    let line = serde_json::to_value(&short_hash).unwrap();
    assert!(read_cassette(cassette_with_entries(&[line]).as_slice()).is_err());
}

#[test]
fn cassette_rejects_zero_version_and_out_of_range_entry_values() {
    let zero_version = format!(
        "{{\"format\":\"{CASSETTE_FORMAT}\",\"version\":0,\
         \"recorded_at_ms\":0,\"source\":\"s\"}}\n"
    );
    assert!(read_cassette(zero_version.as_bytes()).is_err());

    let header = CassetteHeader {
        format: CASSETTE_FORMAT.to_owned(),
        version: CASSETTE_VERSION,
        recorded_at_ms: 0,
        source: String::new(),
    };
    let low_status = CassetteEntry {
        status: 99,
        ..valid_entry(1)
    };
    let mut buf: Vec<u8> = Vec::new();
    assert!(write_cassette(&mut buf, &header, &[low_status]).is_err());

    let high_status = CassetteEntry {
        status: 600,
        ..valid_entry(1)
    };
    let mut buf: Vec<u8> = Vec::new();
    assert!(write_cassette(&mut buf, &header, &[high_status]).is_err());

    let nan_duration = CassetteEntry {
        duration_ms: f64::NAN,
        ..valid_entry(1)
    };
    let mut buf: Vec<u8> = Vec::new();
    assert!(write_cassette(&mut buf, &header, &[nan_duration]).is_err());

    // Reader enforces the same ranges against hand-written JSONL.
    let mut bad_status = serde_json::to_value(valid_entry(1)).unwrap();
    bad_status["status"] = serde_json::json!(42);
    assert!(read_cassette(cassette_with_entries(&[bad_status]).as_slice()).is_err());

    // NaN serializes to JSON null; the reader must reject it too.
    let mut bad_duration = serde_json::to_value(valid_entry(1)).unwrap();
    bad_duration["duration_ms"] = serde_json::Value::Null;
    assert!(read_cassette(cassette_with_entries(&[bad_duration]).as_slice()).is_err());
}
