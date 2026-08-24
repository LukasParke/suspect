//! Gateway integration tests.
//!
//! Every test drives a real server: the router is built in-process, bound
//! to an ephemeral port via `TcpListener::bind("127.0.0.1:0")`, and hit
//! over an actual TCP connection through the crate's own hyper transport.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use bytes::Bytes;
use suspect_journal::{
    Body, CassetteEntry, CassetteHeader, Journal, REDACTED, Record, VecSink, Verdict,
    read_cassette, write_cassette,
};
use tower::ServiceExt;

use crate::{
    FaultConfig, GatewayConfig, Mode, ReplayIndex, build_router, scenario::scenario_router,
};
/// Spec fixture mirroring `suspect-ir`'s test spec but with fully
/// resolvable local refs so mock synthesis has real schemas to work from.
const SPEC: &str = r#"
openapi: 3.1.0
info:
  title: Pets
  version: "1.0"
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - name: limit
          in: query
          required: true
          schema: { type: integer }
      responses:
        '200':
          description: page of pets
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PetList'
  /pets/{petId}:
    get:
      operationId: showPetById
      parameters:
        - name: petId
          in: path
          required: true
          schema: { type: string }
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
  /breeds/{name}:
    get:
      operationId: showBreedByName
      parameters:
        - name: name
          in: path
          required: true
          schema:
            type: string
            enum: ["rex+a"]
      responses:
        '200':
          description: breed
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
        age: { type: integer }
        tags:
          type: array
          items: { type: string }
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
"#;

/// Writes [`SPEC`] into `dir` and returns the entry-document path.
fn write_spec(dir: &Path) -> PathBuf {
    std::fs::write(dir.join("spec.yaml"), SPEC).expect("write spec");
    dir.join("spec.yaml")
}

/// Builds a journal over a shared in-memory sink.
fn journal() -> (Arc<tokio::sync::Mutex<Journal>>, VecSink) {
    let sink = VecSink::default();
    (
        Arc::new(tokio::sync::Mutex::new(Journal::new(Box::new(
            sink.clone(),
        )))),
        sink,
    )
}

/// Binds `app` on an ephemeral port and serves it; returns the base URL.
async fn spawn_app(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

/// Builds and serves the gateway for `cfg`; returns its base URL.
async fn serve_gateway(cfg: &GatewayConfig) -> String {
    let (journal, _sink) = journal();
    let app = build_router(cfg, journal).await.expect("router");
    spawn_app(app).await
}

/// Performs one request through the crate's hyper transport.
async fn request(
    base: &str,
    method: &str,
    path_and_query: &str,
) -> (u16, Vec<(String, String)>, Bytes) {
    let reply = crate::proxy::fetch_upstream(base, method, path_and_query, &[], Bytes::new())
        .await
        .expect("request");
    (reply.status, reply.headers, reply.body)
}

/// Header value lookup helper.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Spawns an upstream that answers every path with one canned JSON body.
async fn canned_upstream(status: u16, body: &'static str) -> String {
    let app = Router::new().fallback(move || async move {
        (
            StatusCode::from_u16(status).unwrap(),
            [("content-type", "application/json")],
            body,
        )
    });
    spawn_app(app).await
}

/// Mock gateway over the shared spec fixture.
async fn mock_gateway(faults: FaultConfig) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: write_spec(dir.path()),
        port: 0,
        faults,
    };
    let base = serve_gateway(&cfg).await;
    // Hold the tempdir alive for the duration of the test.
    (base, dir)
}

#[tokio::test]
async fn mock_synthesizes_pet_for_parameterized_get() {
    let (base, _dir) = mock_gateway(FaultConfig::default()).await;
    let (status, headers, body) = request(&base, "GET", "/pets/42").await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
    let pet: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(pet["name"], "");
    assert_eq!(pet["age"], 0);
    assert_eq!(pet["tags"], serde_json::json!([""]));
}

#[tokio::test]
async fn unknown_path_returns_404_problem_json() {
    let (base, _dir) = mock_gateway(FaultConfig::default()).await;
    let (status, headers, body) = request(&base, "GET", "/definitely/not/here").await;

    assert_eq!(status, 404);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(parsed["title"], "Operation not found");
}

#[tokio::test]
async fn fault_injection_is_deterministic_at_100_pct() {
    let faults = FaultConfig {
        delay_ms: 1,
        delay_pct: 100,
        error_status: Some(503),
        error_pct: 100,
    };
    let (base, _dir) = mock_gateway(faults).await;

    // The counter-based roll means every single request faults identically;
    // no RNG, no flakes, reproducible across runs.
    for _ in 0..3 {
        let (status, _, _) = request(&base, "GET", "/pets").await;
        assert_eq!(status, 503);
    }
}

#[tokio::test]
async fn record_mode_writes_readable_cassette() {
    let upstream = canned_upstream(200, r#"{"ok":true}"#).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cassette = dir.path().join("rec.cassette");
    let cfg = GatewayConfig {
        mode: Mode::Record {
            upstream,
            cassette: cassette.clone(),
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    let (status, _, body) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], br#"{"ok":true}"#);

    let file = std::fs::File::open(&cassette).expect("cassette exists");
    let (hdr, entries) = read_cassette(file).expect("readable cassette");
    assert_eq!(hdr.format, suspect_journal::CASSETTE_FORMAT);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].method, "GET");
    assert!(entries[0].url.ends_with("/pets/42"));
    assert_eq!(entries[0].response_body.text(), Some(r#"{"ok":true}"#));
}

#[tokio::test]
async fn replay_serves_recorded_entry_binary_safe() {
    // A non-UTF-8 payload forces base64 encoding in the cassette; replay
    // must decode it back to identical raw bytes.
    let raw = [0x00u8, 0xff, 0x80, b'a', b'b', 0x0a];
    let entry = CassetteEntry {
        id: 1,
        method: "GET".to_owned(),
        url: "http://api.example.com/pets/42".to_owned(),
        status: 200,
        request_headers: vec![],
        request_body: Body::from_bytes(b""),
        response_headers: vec![(
            "content-type".to_owned(),
            "application/octet-stream".to_owned(),
        )],
        response_body: Body::from_bytes(&raw),
        duration_ms: 1.5,
    };
    let cassette_header = CassetteHeader {
        format: suspect_journal::CASSETTE_FORMAT.to_owned(),
        version: suspect_journal::CASSETTE_VERSION,
        recorded_at_ms: Journal::now_ms(),
        source: "test".to_owned(),
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let cassette = dir.path().join("replay.cassette");
    write_cassette(
        &mut std::fs::File::create(&cassette).expect("create"),
        &cassette_header,
        &[entry],
    )
    .expect("write cassette");

    let cfg = GatewayConfig {
        mode: Mode::Replay {
            cassette: cassette.clone(),
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    let (status, headers, body) = request(&base, "GET", "/pets/42?full=1").await;
    assert_eq!(status, 200);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(&body[..], &raw);
}

#[tokio::test]
async fn replay_miss_returns_404_problem_json() {
    let entry = CassetteEntry {
        id: 1,
        method: "GET".to_owned(),
        url: "http://api.example.com/pets/1".to_owned(),
        status: 200,
        request_headers: vec![],
        request_body: Body::from_bytes(b""),
        response_headers: vec![],
        response_body: Body::from_bytes(b"{}"),
        duration_ms: 1.0,
    };
    let cassette_header = CassetteHeader {
        format: suspect_journal::CASSETTE_FORMAT.to_owned(),
        version: suspect_journal::CASSETTE_VERSION,
        recorded_at_ms: Journal::now_ms(),
        source: "test".to_owned(),
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let cassette = dir.path().join("replay.cassette");
    write_cassette(
        &mut std::fs::File::create(&cassette).expect("create"),
        &cassette_header,
        &[entry],
    )
    .expect("write cassette");

    let cfg = GatewayConfig {
        mode: Mode::Replay { cassette },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    let (status, headers, body) = request(&base, "GET", "/nowhere").await;
    assert_eq!(status, 404);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["title"], "Replay miss");
}

#[tokio::test]
async fn scenario_serves_in_order_mismatch_and_exhaustion() {
    use crate::scenario::StepExpect;
    let steps = vec![
        StepExpect {
            method: "GET".to_owned(),
            path_suffix: "/pets/42".to_owned(),
            status: 200,
            body: serde_json::json!({"name": "Rex"}),
        },
        StepExpect {
            method: "POST".to_owned(),
            path_suffix: "/pets".to_owned(),
            status: 201,
            body: serde_json::json!({"created": true}),
        },
    ];
    let base = spawn_app(scenario_router(steps)).await;

    // In-order success.
    let (status, _, body) = request(&base, "GET", "/api/pets/42").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], br#"{"name":"Rex"}"#);
    let (status, _, body) = request(&base, "POST", "/pets").await;
    assert_eq!(status, 201);
    assert_eq!(&body[..], br#"{"created":true}"#);

    // Past the end: 410 Gone.
    let (status, headers, body) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 410);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["title"], "Scenario exhausted");
}

#[tokio::test]
async fn scenario_mismatch_rejects_without_consuming_step() {
    use crate::scenario::StepExpect;
    let steps = vec![StepExpect {
        method: "GET".to_owned(),
        path_suffix: "/pets/42".to_owned(),
        status: 200,
        body: serde_json::json!({"name": "Rex"}),
    }];
    let base = spawn_app(scenario_router(steps)).await;

    // Wrong method: 400 naming expected vs got...
    let (status, _, body) = request(&base, "DELETE", "/pets/42").await;
    assert_eq!(status, 400);
    let detail = String::from_utf8_lossy(&body).into_owned();
    assert!(detail.contains("GET"), "detail names expected: {detail}");
    assert!(detail.contains("DELETE"), "detail names got: {detail}");

    // ...and the step is still pending, so the right request succeeds.
    let (status, _, body) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], br#"{"name":"Rex"}"#);
}

#[tokio::test]
async fn validate_observe_journals_invalid_without_altering_response() {
    let upstream = canned_upstream(200, r#"{"age":3}"#).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Validate {
            upstream,
            enforce: false,
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let (journal_handle, sink) = journal();
    let app = build_router(&cfg, journal_handle).await.expect("router");
    let base = spawn_app(app).await;

    // Upstream's {"age":3} violates Pet (missing required `name`) but is
    // passed through byte-for-byte.
    let (status, headers, body) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
    assert_eq!(&body[..], br#"{"age":3}"#);

    let verdicts = sink
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(t) => Some(t.verdict),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(verdicts.len(), 1);
    match &verdicts[0] {
        Verdict::Invalid(violations) => {
            assert!(
                violations.iter().any(|v| v.message.contains("name")),
                "violation names the missing field: {violations:?}"
            );
        }
        other => panic!("expected Invalid verdict, got {other:?}"),
    }
}

#[tokio::test]
async fn validate_enforce_rejects_invalid_request_before_upstream() {
    // Upstream would answer anything with 200; the enforced request must
    // never reach it.
    let upstream = canned_upstream(200, "{}").await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Validate {
            upstream,
            enforce: true,
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    // listPets declares required integer query parameter `limit`.
    let (status, headers, body) = request(&base, "GET", "/pets").await;
    assert_eq!(status, 400);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["title"], "Request failed validation");
    let violations = parsed["violations"].as_array().expect("violations array");
    assert!(
        violations
            .iter()
            .any(|v| v["message"].as_str().unwrap_or("").contains("limit")),
        "names missing limit: {violations:?}"
    );

    // With a valid value the exchange goes through.
    let (status, _, _) = request(&base, "GET", "/pets?limit=5").await;
    assert_eq!(status, 200);
}
/// Builds a minimal cassette entry for index-level replay tests.
fn cassette_entry(id: u64, method: &str, url: &str) -> CassetteEntry {
    CassetteEntry {
        id,
        method: method.to_owned(),
        url: url.to_owned(),
        status: 200,
        request_headers: vec![],
        request_body: Body::from_bytes(b""),
        response_headers: vec![],
        response_body: Body::from_bytes(b"{}"),
        duration_ms: 1.0,
    }
}

#[test]
fn fault_roll_is_pure_function_of_input() {
    // Same request, same roll — no shared counter, no concurrency drift.
    for path in ["/pets", "/pets/42?full=1", "/nowhere"] {
        let expected = crate::fault_roll("GET", path);
        for _ in 0..3 {
            assert_eq!(crate::fault_roll("GET", path), expected);
            assert_eq!(
                crate::fault_roll("POST", path) % 100,
                crate::fault_roll("POST", path)
            );
        }
    }
    // Rolls stay in range and spread across distinct requests.
    let rolls: std::collections::HashSet<u64> = (0..30)
        .map(|i| crate::fault_roll("GET", &format!("/pets/{i}")))
        .collect();
    assert!(rolls.iter().all(|r| *r < 100));
    assert!(
        rolls.len() >= 10,
        "hash roll should vary by input: {rolls:?}"
    );
}

#[tokio::test]
async fn proxied_requests_carry_upstream_host_header() {
    // Capture the Host header as received on the wire upstream.
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen_for_app = Arc::clone(&seen);
    let app = Router::new().fallback(move |headers: axum::http::HeaderMap| {
        let seen = Arc::clone(&seen_for_app);
        async move {
            seen.lock().expect("lock").push(
                headers
                    .get(axum::http::header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
            );
            (StatusCode::OK, "up")
        }
    });
    let upstream = spawn_app(app).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Proxy {
            upstream: upstream.clone(),
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    let (status, _, _) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 200);

    let hosts = seen.lock().expect("lock");
    assert_eq!(hosts.len(), 1);
    assert_eq!(
        hosts[0],
        upstream.strip_prefix("http://").expect("authority"),
        "upstream must receive the upstream authority as Host"
    );
}

#[tokio::test]
async fn oversized_request_body_is_rejected_with_413() {
    let upstream = canned_upstream(200, r#"{"ok":true}"#).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Proxy { upstream },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    // One byte past the cap must be rejected, not silently truncated to
    // an empty body that gets forwarded upstream.
    let big = Bytes::from(vec![b'a'; crate::proxy::MAX_BODY + 1]);
    let reply = crate::proxy::fetch_upstream(&base, "GET", "/pets/42", &[], big)
        .await
        .expect("reply");
    assert_eq!(reply.status, 413);
    assert_eq!(
        header(&reply.headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&reply.body).expect("problem json");
    assert_eq!(parsed["title"], "Payload too large");
}

#[tokio::test]
async fn record_mode_redacts_credentials_in_cassette() {
    // Upstream sets a cookie and returns a body with a secret token; the
    // client sends Authorization and Cookie headers plus a secret body.
    let upstream = spawn_app(Router::new().fallback(|| async move {
        (
            StatusCode::OK,
            [
                ("content-type", "application/json"),
                ("set-cookie", "session=abc123"),
            ],
            r#"{"token":"s3cret","name":"pet"}"#,
        )
    }))
    .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cassette = dir.path().join("rec.cassette");
    let cfg = GatewayConfig {
        mode: Mode::Record {
            upstream,
            cassette: cassette.clone(),
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let base = serve_gateway(&cfg).await;

    let reply = crate::proxy::fetch_upstream(
        &base,
        "GET",
        "/pets/42",
        &[
            ("authorization".to_owned(), "Bearer topsecret".to_owned()),
            ("cookie".to_owned(), "k=v".to_owned()),
        ],
        Bytes::from_static(br#"{"password":"hunter2"}"#),
    )
    .await
    .expect("reply");
    assert_eq!(reply.status, 200);

    let (_, entries) =
        read_cassette(std::fs::File::open(&cassette).expect("cassette exists")).expect("readable");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let find_header = |name: &str| {
        entry
            .request_headers
            .iter()
            .chain(entry.response_headers.iter())
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(find_header("authorization"), Some(REDACTED));
    assert_eq!(find_header("cookie"), Some(REDACTED));
    assert_eq!(find_header("set-cookie"), Some(REDACTED));

    let request_text = entry.request_body.text().expect("utf8 request body");
    assert!(request_text.contains(REDACTED), "{request_text}");
    assert!(!request_text.contains("hunter2"), "{request_text}");
    let response_text = entry.response_body.text().expect("utf8 response body");
    assert!(response_text.contains(REDACTED), "{response_text}");
    assert!(!response_text.contains("s3cret"), "{response_text}");
}

#[tokio::test]
async fn non_utf8_header_values_are_preserved_lossily() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let (journal_handle, sink) = journal();
    let router = build_router(&cfg, journal_handle).await.expect("router");

    // A header value with non-UTF-8 bytes must survive collection with a
    // replacement character instead of being dropped entirely.
    let mut request = axum::http::Request::builder()
        .method("GET")
        .uri("/pets/42")
        .body(axum::body::Body::empty())
        .expect("request");
    request.headers_mut().insert(
        "x-trace",
        axum::http::HeaderValue::from_bytes(&[0xff, b'a']).expect("opaque header value"),
    );
    let response = router.oneshot(request).await.expect("response");
    assert_eq!(response.status(), 200);

    let values: Vec<Option<String>> = sink
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(t) => Some(
                t.request_headers
                    .into_iter()
                    .find(|(n, _)| n == "x-trace")
                    .map(|(_, v)| v),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec![Some("\u{fffd}a".to_owned())]);
}

#[tokio::test]
async fn validate_treats_plus_as_literal_in_path_segments() {
    let upstream = canned_upstream(200, "{}").await;
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Validate {
            upstream,
            enforce: false,
        },
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let (journal_handle, sink) = journal();
    let app = build_router(&cfg, journal_handle).await.expect("router");
    let base = spawn_app(app).await;

    // `/breeds/{name}` declares enum ["rex+a"]; `+` in a path segment is
    // literal, so `rex+a` matches the enum and no violation is reported.
    let (status, _, _) = request(&base, "GET", "/breeds/rex+a").await;
    assert_eq!(status, 200);

    let verdicts: Vec<Verdict> = sink
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(t) => Some(t.verdict),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 1);
    assert!(
        matches!(&verdicts[0], Verdict::Pass),
        "`+` must not decode to space in paths: {:?}",
        verdicts[0]
    );
}

#[test]
fn replay_fallback_is_method_aware() {
    let entries = vec![
        cassette_entry(1, "POST", "http://api.example.com/pets/1"),
        cassette_entry(2, "GET", "http://api.example.com/pets/2"),
    ];
    let index = ReplayIndex::new(&entries);

    assert!(index.lookup("POST", "/pets/1").is_some());
    assert!(index.lookup("GET", "/pets/2").is_some());
    // A GET must never fall back onto a recorded POST exchange...
    assert!(index.lookup("GET", "/pets/1").is_none());
    assert!(index.lookup("GET", "/pets/1?full=1").is_none());
    // ...but same-method query-less fallback keeps working.
    assert!(index.lookup("get", "/pets/2?full=1").is_some());
}

#[test]
fn replay_keys_normalize_percent_escapes() {
    let entries = vec![cassette_entry(
        1,
        "GET",
        "http://api.example.com/pets/a%2fb",
    )];
    let index = ReplayIndex::new(&entries);

    // Escape-hex casing differs between recording and live request; both
    // must hit exactly (no fallback needed).
    assert!(index.lookup("GET", "/pets/a%2Fb").is_some());
    assert!(index.lookup("GET", "/pets/a%2fb").is_some());
    assert!(index.lookup("GET", "/pets/a/b").is_none());
}

#[tokio::test]
async fn undeclared_method_returns_journaled_405() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: write_spec(dir.path()),
        port: 0,
        faults: FaultConfig::default(),
    };
    let (journal_handle, sink) = journal();
    let app = build_router(&cfg, journal_handle).await.expect("router");
    let base = spawn_app(app).await;

    // /pets/{petId} only declares GET; POST must yield a problem+json 405
    // that is journaled like every other served exchange.
    let (status, headers, body) = request(&base, "POST", "/pets/42").await;
    assert_eq!(status, 405);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("problem json");
    assert_eq!(parsed["title"], "Method not allowed");

    let statuses: Vec<Option<u16>> = sink
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(t) => Some(t.status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, vec![Some(405)]);
}
