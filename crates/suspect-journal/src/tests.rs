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
