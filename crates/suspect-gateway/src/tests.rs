//! Gateway integration tests.
//!
//! Every test drives a real server: the router is built in-process, bound
//! to an ephemeral port via `TcpListener::bind("127.0.0.1:0")`, and hit
//! over an actual TCP connection through the crate's own hyper transport.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::http::Request;
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
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Pet'
      responses:
        '201':
          description: created pet
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
    };
    let base = serve_gateway(&cfg).await;
    // Hold the tempdir alive for the duration of the test.
    (base, dir)
}

#[tokio::test]
async fn mock_synthesizes_pet_for_parameterized_get() {
    let (base, _dir) = mock_gateway(FaultConfig::default()).await;
    // Unknown ids fall back to the synthesized example (mocks stay
    // always-answer); POST seeds the store and the created id is then
    // served from it.
    let (status, _, _) = request(&base, "GET", "/pets/42").await;
    assert_eq!(status, 200, "unknown id falls back to example");

    let (post_status, _, post_body) = request(&base, "POST", "/pets").await;
    assert_eq!(post_status, 201);
    let created: serde_json::Value = serde_json::from_slice(&post_body).expect("json");
    let id = created
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("1")
        .to_owned();

    let (status, headers, body) = request(&base, "GET", &format!("/pets/{id}")).await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
    let pet: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(pet["id"], id);
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
    };
    let (journal_handle, sink) = journal();
    let router = build_router(&cfg, journal_handle).await.expect("router");

    // A header value with non-UTF-8 bytes must survive collection with a
    // replacement character instead of being dropped entirely.
    let mut request = axum::http::Request::builder()
        .method("GET")
        .uri("/pets")
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
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

/// Startup must stay fast: building the mock router over a generated
/// ~500-operation spec has to finish well under two seconds. This is a
/// generous regression bound (debug-build test time included), not a
/// benchmark — the release target is <1s on real corpora.
#[tokio::test]
async fn mock_startup_over_500_ops_stays_under_budget() {
    let dir = tempfile::tempdir().expect("tempdir");
    const OPS: usize = 500;
    let mut spec =
        String::from("openapi: 3.1.0\ninfo:\n  title: Bulk\n  version: \"1.0\"\npaths:\n");
    for i in 0..OPS {
        spec.push_str(&format!(
            concat!(
                "  /items/item{i}:\n",
                "    get:\n",
                "      operationId: getItem{i}\n",
                "      responses:\n",
                "        '200':\n",
                "          description: ok\n",
                "          content:\n",
                "            application/json:\n",
                "              schema:\n",
                "                $ref: '#/components/schemas/Item'\n"
            ),
            i = i
        ));
    }
    spec.push_str(
        "components:\n  schemas:\n    Item:\n      type: object\n      properties:\n        id: { type: integer }\n        name: { type: string }\n        tags:\n          type: array\n          items: { type: string }\n",
    );
    std::fs::write(dir.path().join("spec.yaml"), &spec).expect("write generated spec");

    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: dir.path().join("spec.yaml"),
        port: 0,
        faults: FaultConfig::default(),
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
    };
    let start = std::time::Instant::now();
    let (journal, _sink) = journal();
    let app = build_router(&cfg, journal).await.expect("router");
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "mock startup over {OPS} ops took {elapsed:?}; budget is 1500ms"
    );
    // The router actually serves: one synthesized response proves the
    // fast startup did not skip precompilation.
    let base = spawn_app(app).await;
    let (status, _, body) = request(&base, "GET", "/items/item7").await;
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(parsed["id"], 0);
}
#[tokio::test]
async fn debug_stateful_flow() {
    let (base, _dir) = mock_gateway(FaultConfig::default()).await;
    let (s1, _, b1) = request(&base, "GET", "/pets/42").await;
    println!(
        "GET /pets/42 -> {s1} {}",
        String::from_utf8_lossy(&b1)
            .chars()
            .take(80)
            .collect::<String>()
    );
    let (s2, _, b2) = request(&base, "POST", "/pets").await;
    println!(
        "POST /pets -> {s2} {}",
        String::from_utf8_lossy(&b2)
            .chars()
            .take(80)
            .collect::<String>()
    );
    let (s3, _, b3) = request(&base, "GET", "/pets/1").await;
    println!(
        "GET /pets/1 -> {s3} {}",
        String::from_utf8_lossy(&b3)
            .chars()
            .take(80)
            .collect::<String>()
    );
}

#[tokio::test]
async fn gateway_serves_its_own_contract_at_openapi_json() {
    let dir = tempfile::tempdir().unwrap();
    let spec = dir.path().join("openapi.yaml");
    std::fs::write(
        &spec,
        "openapi: 3.1.0\ninfo: {title: Contract API, version: '2.1.0'}\npaths:\n  /pets:\n    get:\n      operationId: listPets\n      summary: List pets\n      responses:\n        '200': {description: ok}\ncomponents:\n  schemas:\n    Pet: {type: object, properties: {id: {type: string}}}\n",
    )
    .unwrap();
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: spec.clone(),
        port: 0,
        faults: FaultConfig::default(),
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
    };
    let journal = Arc::new(tokio::sync::Mutex::new(Journal::new(Box::new(
        suspect_journal::StdoutSink,
    ))));
    let router = build_router(&cfg, journal).await.expect("router");
    let response = router
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let contract: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(contract["info"]["title"], "Contract API");
    assert_eq!(contract["info"]["version"], "2.1.0");
    assert_eq!(contract["paths"]["/pets"]["get"]["operationId"], "listPets");
    assert!(
        contract["components"]["schemas"]["Pet"]["properties"]["id"].is_object(),
        "the served contract carries the schema graph"
    );
}

/// `allOf` compositions synthesize as the union-merge of their members:
/// the spec-heavy composition pattern mocks as the merge the wire
/// actually carries, not `null`.
#[test]
fn synth_example_merges_allof_members() {
    use crate::mock::{SchemaRefs, synth_example};
    use serde_json::json;

    let mut refs = SchemaRefs::new();
    refs.insert(
        "MediaContainer".to_owned(),
        json!({
            "type": "object",
            "properties": {
                "size": { "type": "integer" },
                "machineIdentifier": { "type": "string" }
            }
        }),
    );
    refs.insert(
        "Extra".to_owned(),
        json!({
            "type": "object",
            "properties": { "platform": { "type": "string" } }
        }),
    );

    let schema = json!({
        "type": "object",
        "properties": {
            "MediaContainer": {
                "allOf": [
                    { "$ref": "#/components/schemas/MediaContainer" },
                    { "$ref": "#/components/schemas/Extra" },
                    { "type": "object", "properties": { "Directory": { "type": "array", "items": { "type": "string" } } } }
                ]
            }
        }
    });

    let synthesized = synth_example(&schema, &refs, 0);
    let container = synthesized
        .pointer("/MediaContainer")
        .expect("the composed member synthesizes an object");
    assert!(
        container.get("size").is_some() && container.get("machineIdentifier").is_some(),
        "the first member's properties survive the merge: {container}"
    );
    assert!(
        container.get("platform").is_some(),
        "the second member's properties merge in: {container}"
    );
    assert!(
        container.get("Directory").is_some(),
        "the inline member's properties merge in: {container}"
    );
}

// ------------------------------------------------ contract conformance

/// Media types, status ranges, shared responses and bodiless replies.
const MEDIA_SPEC: &str = r##"
openapi: 3.1.0
info: { title: Media, version: "1" }
paths:
  /reports/{reportId}:
    get:
      operationId: showReport
      parameters:
        - { name: reportId, in: path, required: true, schema: { type: string } }
      responses:
        '200':
          description: a JSON:API document
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Report' }
        '404':
          $ref: '#/components/responses/NotFound'
  /summary:
    get:
      operationId: summary
      responses:
        2XX:
          description: some success
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Report' }
  /ping:
    get:
      operationId: ping
      responses:
        '204': { description: pong }
  /problems:
    get:
      operationId: problems
      responses:
        '404':
          $ref: '#/components/responses/NotFound'
components:
  responses:
    NotFound:
      description: nothing there
      content:
        application/problem+json:
          schema: { $ref: '#/components/schemas/Problem' }
  schemas:
    Report:
      type: object
      required: [id]
      properties:
        id: { type: string, example: r1 }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string, example: Not found }
"##;

/// A gateway over `spec` in `mode`, with its journal sink.
async fn gateway_over(spec: &str, mode: Mode) -> (String, VecSink, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), spec).expect("write spec");
    let cfg = GatewayConfig {
        mode,
        spec: dir.path().join("spec.yaml"),
        port: 0,
        faults: FaultConfig::default(),
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: false,
    };
    let (journal_handle, sink) = journal();
    let app = build_router(&cfg, journal_handle).await.expect("router");
    (spawn_app(app).await, sink, dir)
}

/// An upstream answering every path with one status, content type and body.
async fn typed_upstream(status: u16, content_type: &'static str, body: &'static str) -> String {
    let app = Router::new().fallback(move || async move {
        (
            StatusCode::from_u16(status).unwrap(),
            [("content-type", content_type)],
            body,
        )
    });
    spawn_app(app).await
}

/// The violation messages of the single journaled exchange.
fn journaled_violations(sink: &VecSink) -> Vec<String> {
    let verdicts: Vec<Verdict> = sink
        .records()
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(traffic) => Some(traffic.verdict),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    match verdicts.into_iter().next().unwrap() {
        Verdict::Invalid(violations) => violations.into_iter().map(|v| v.message).collect(),
        Verdict::Pass => Vec::new(),
        other => panic!("unexpected verdict {other:?}"),
    }
}

#[tokio::test]
async fn mock_answers_in_the_declared_media_type() {
    let (base, _sink, _dir) = gateway_over(MEDIA_SPEC, Mode::Mock).await;
    let (status, headers, body) = request(&base, "GET", "/reports/7").await;
    assert_eq!(status, 200);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/vnd.api+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed, serde_json::json!({"id": "r1"}));
}

#[tokio::test]
async fn mock_serves_a_shared_component_response() {
    let (base, _sink, _dir) = gateway_over(MEDIA_SPEC, Mode::Mock).await;
    let (status, headers, body) = request(&base, "GET", "/problems").await;
    assert_eq!(status, 404);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed, serde_json::json!({"title": "Not found"}));
}

#[tokio::test]
async fn mock_answers_a_status_range_with_a_status_in_it() {
    let (base, _sink, _dir) = gateway_over(MEDIA_SPEC, Mode::Mock).await;
    let (status, _, body) = request(&base, "GET", "/summary").await;
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed, serde_json::json!({"id": "r1"}));
}

#[tokio::test]
async fn mock_sends_no_body_for_a_response_that_declares_none() {
    let (base, _sink, _dir) = gateway_over(MEDIA_SPEC, Mode::Mock).await;
    let (status, headers, body) = request(&base, "GET", "/ping").await;
    assert_eq!(status, 204);
    assert!(body.is_empty(), "{body:?}");
    assert_eq!(header(&headers, "content-type"), None);
}

/// The mock and the conformance check read one description: whatever the
/// mock answers, the check accepts.
#[tokio::test]
async fn the_mock_conforms_to_the_description_it_serves() {
    for spec_text in [SPEC, MEDIA_SPEC] {
        let (base, _sink, dir) = gateway_over(spec_text, Mode::Mock).await;
        let spec = suspect_ir::IrSpec::from_file(&dir.path().join("spec.yaml")).expect("spec");
        let schemas = suspect_runtime::Schemas::from_spec(&spec);
        for op in &spec.operations {
            // Any value serves for a path parameter except where the
            // fixture constrains it.
            let path = op
                .path
                .replace("{name}", "rex+a")
                .replace("{petId}", "1")
                .replace("{reportId}", "1");
            let (status, headers, body) = request(&base, op.method.as_str(), &path).await;
            let check = suspect_runtime::check_response(
                &op.responses,
                &schemas,
                suspect_runtime::Reply {
                    method: op.method.as_str(),
                    status,
                    headers: &headers,
                    body: &body,
                },
                suspect_runtime::Conformance::Strict,
            );
            assert!(
                check.conforms(),
                "{} {} answered {status}: {:?}",
                op.method,
                op.path,
                check.violations
            );
        }
    }
}

#[tokio::test]
async fn validate_journals_an_undeclared_status() {
    let upstream = typed_upstream(418, "application/json", "{}").await;
    let (base, sink, _dir) = gateway_over(
        MEDIA_SPEC,
        Mode::Validate {
            upstream,
            enforce: false,
        },
    )
    .await;
    let (status, _, _) = request(&base, "GET", "/reports/7").await;
    assert_eq!(status, 418, "the upstream reply is passed through");
    let violations = journaled_violations(&sink);
    assert!(
        violations
            .iter()
            .any(|message| message.contains("status 418 is not declared")),
        "{violations:?}"
    );
}

#[tokio::test]
async fn validate_journals_an_undeclared_content_type() {
    let upstream = typed_upstream(200, "application/json", r#"{"id": "r1"}"#).await;
    let (base, sink, _dir) = gateway_over(
        MEDIA_SPEC,
        Mode::Validate {
            upstream,
            enforce: false,
        },
    )
    .await;
    let _ = request(&base, "GET", "/reports/7").await;
    let violations = journaled_violations(&sink);
    assert!(
        violations
            .iter()
            .any(|message| message.contains("`application/json` is not declared")),
        "{violations:?}"
    );
}

#[tokio::test]
async fn validate_checks_a_structured_json_body_and_locates_the_violation() {
    let upstream = typed_upstream(200, "application/vnd.api+json", r#"{"id": 7}"#).await;
    let (base, sink, _dir) = gateway_over(
        MEDIA_SPEC,
        Mode::Validate {
            upstream,
            enforce: false,
        },
    )
    .await;
    let _ = request(&base, "GET", "/reports/7").await;
    let records = sink.records();
    let pointers: Vec<String> = records
        .into_iter()
        .filter_map(|record| match record {
            Record::Traffic(traffic) => match traffic.verdict {
                Verdict::Invalid(violations) => Some(violations),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .map(|violation| violation.pointer)
        .collect();
    assert_eq!(pointers, ["/id"]);
}

/// A CRUD resource whose description departs from the mock's defaults: a
/// vendor JSON media type everywhere and a `DELETE` that answers `200`
/// with the removed resource.
const CRUD_SPEC: &str = r##"
openapi: 3.1.0
info: { title: Widgets, version: "1" }
paths:
  /widgets:
    post:
      operationId: createWidget
      requestBody:
        content:
          application/vnd.api+json:
            schema: { $ref: '#/components/schemas/Widget' }
      responses:
        '201':
          description: created
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Widget' }
  /widgets/{widgetId}:
    parameters:
      - { name: widgetId, in: path, required: true, schema: { type: string } }
    get:
      operationId: showWidget
      responses:
        '200':
          description: one widget
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Widget' }
    put:
      operationId: replaceWidget
      responses:
        '200':
          description: replaced
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Widget' }
    delete:
      operationId: deleteWidget
      responses:
        '200':
          description: the removed widget
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Widget' }
components:
  schemas:
    Widget:
      type: object
      required: [id, name]
      properties:
        id: { type: string, example: w1 }
        name: { type: string, example: gear }
"##;

/// One request with a JSON body through the crate's hyper transport.
async fn send(
    base: &str,
    method: &str,
    path: &str,
    body: &'static str,
) -> (u16, Vec<(String, String)>, Bytes) {
    let headers = [("content-type".to_owned(), "application/json".to_owned())];
    let reply = crate::proxy::fetch_upstream(
        base,
        method,
        path,
        &headers,
        Bytes::from_static(body.as_bytes()),
    )
    .await
    .expect("request");
    (reply.status, reply.headers, reply.body)
}

/// The stateful mock creates, reads, replaces and removes resources; each
/// of those replies is still the one the description declares.
#[tokio::test]
async fn the_stateful_mock_conforms_through_a_resource_lifecycle() {
    let (base, _sink, dir) = gateway_over(CRUD_SPEC, Mode::Mock).await;
    let spec = suspect_ir::IrSpec::from_file(&dir.path().join("spec.yaml")).expect("spec");
    let schemas = suspect_runtime::Schemas::from_spec(&spec);
    let conforms = |id: &str, status: u16, headers: &[(String, String)], body: &[u8]| {
        let op = spec
            .operation(suspect_ir::OpSelector::Id(id))
            .expect("operation");
        let check = suspect_runtime::check_response(
            &op.responses,
            &schemas,
            suspect_runtime::Reply {
                method: op.method.as_str(),
                status,
                headers,
                body,
            },
            suspect_runtime::Conformance::Strict,
        );
        assert!(
            check.conforms(),
            "{id} answered {status} {:?}: {:?}",
            String::from_utf8_lossy(body),
            check.violations
        );
    };

    let (status, headers, body) = send(&base, "POST", "/widgets", r#"{"name": "cog"}"#).await;
    conforms("createWidget", status, &headers, &body);
    let created: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let id = created["id"].as_str().expect("a synthesized id").to_owned();
    let path = format!("/widgets/{id}");

    let (status, headers, body) = request(&base, "GET", &path).await;
    conforms("showWidget", status, &headers, &body);

    let (status, headers, body) =
        send(&base, "PUT", &path, r#"{"id": "w9", "name": "sprocket"}"#).await;
    conforms("replaceWidget", status, &headers, &body);

    let (status, headers, body) = request(&base, "DELETE", &path).await;
    conforms("deleteWidget", status, &headers, &body);
}

// ------------------------------------------------------ enforcing mock

/// A mock gateway over `spec` that rejects requests breaking the contract.
async fn enforcing_mock(spec: &str) -> (String, VecSink, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), spec).expect("write spec");
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: dir.path().join("spec.yaml"),
        port: 0,
        faults: FaultConfig::default(),
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
        enforce_mock: true,
    };
    let (journal_handle, sink) = journal();
    let app = build_router(&cfg, journal_handle).await.expect("router");
    (spawn_app(app).await, sink, dir)
}

/// A consumer that sends what the contract forbids is told so, instead of
/// being answered as if the request were fine.
#[tokio::test]
async fn an_enforcing_mock_rejects_a_request_that_breaks_the_contract() {
    let (base, sink, _dir) = enforcing_mock(SPEC).await;
    // `listPets` requires an integer `limit`.
    let (status, headers, body) = request(&base, "GET", "/pets").await;
    assert_eq!(status, 400);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    let problem: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(problem["title"], "Request failed validation");
    assert!(
        problem["violations"]
            .as_array()
            .is_some_and(|violations| violations
                .iter()
                .any(|v| v["message"].as_str().unwrap_or("").contains("limit"))),
        "{problem}"
    );
    assert!(
        journaled_violations(&sink)
            .iter()
            .any(|message| message.contains("limit")),
        "the rejection is journaled as invalid"
    );

    // A request the contract allows is answered as before.
    let (status, _, _) = request(&base, "GET", "/pets?limit=5").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_mock_answers_every_request_unless_told_to_enforce() {
    let (base, _dir) = mock_gateway(FaultConfig::default()).await;
    let (status, _, _) = request(&base, "GET", "/pets").await;
    assert_eq!(status, 200, "the plain mock always answers");
}

/// When the description declares how it rejects a request, the enforcing
/// mock rejects that way, so its reply is still one the contract declares.
#[tokio::test]
async fn an_enforcing_mock_rejects_with_the_declared_response() {
    const SPEC_WITH_REJECTION: &str = r##"
openapi: 3.1.0
info: { title: Widgets, version: "1" }
paths:
  /widgets:
    post:
      operationId: createWidget
      requestBody:
        required: true
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Widget' }
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Widget' }
        '422':
          description: not a widget
          content:
            application/problem+json:
              schema: { $ref: '#/components/schemas/Problem' }
components:
  schemas:
    Widget:
      type: object
      required: [name]
      properties:
        name: { type: string, example: gear }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string, example: Unprocessable }
"##;
    let (base, _sink, dir) = enforcing_mock(SPEC_WITH_REJECTION).await;
    let (status, headers, body) = send(&base, "POST", "/widgets", r#"{"name": 7}"#).await;
    assert_eq!(status, 422);
    assert_eq!(
        header(&headers, "content-type"),
        Some("application/problem+json")
    );
    // The rejection is itself a reply the description declares.
    let spec = suspect_ir::IrSpec::from_file(&dir.path().join("spec.yaml")).expect("spec");
    let op = spec
        .operation(suspect_ir::OpSelector::Id("createWidget"))
        .expect("operation");
    let check = suspect_runtime::check_response(
        &op.responses,
        &suspect_runtime::Schemas::from_spec(&spec),
        suspect_runtime::Reply {
            method: "POST",
            status,
            headers: &headers,
            body: &body,
        },
        suspect_runtime::Conformance::Strict,
    );
    assert!(check.conforms(), "{:?}", check.violations);

    let (status, _, _) = send(&base, "POST", "/widgets", r#"{"name": "cog"}"#).await;
    assert_eq!(status, 201);
}

/// A response that declares headers is mocked with them, so a consumer
/// sees the shape and the reply still satisfies a strict contract check.
#[tokio::test]
async fn mock_sends_the_headers_a_response_declares() {
    const TICKETS: &str = r##"
openapi: 3.1.0
info: { title: Tickets, version: "1" }
paths:
  /status:
    get:
      operationId: status
      responses:
        '200':
          description: current status
          headers:
            X-Rate-Limit-Remaining:
              required: true
              schema: { type: integer }
            X-Region:
              schema: { type: string, example: eu-west }
            X-Trace:
              $ref: '#/components/headers/Trace'
          content:
            application/json:
              schema: { type: object }
components:
  headers:
    Trace:
      required: true
      schema: { type: string, example: trace-1 }
"##;
    let (base, _sink, dir) = gateway_over(TICKETS, Mode::Mock).await;
    let (status, headers, body) = request(&base, "GET", "/status").await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "x-rate-limit-remaining"), Some("0"));
    assert_eq!(header(&headers, "x-region"), Some("eu-west"));
    assert_eq!(header(&headers, "x-trace"), Some("trace-1"));

    let spec = suspect_ir::IrSpec::from_file(&dir.path().join("spec.yaml")).expect("spec");
    let op = spec
        .operation(suspect_ir::OpSelector::Id("status"))
        .expect("operation");
    let check = suspect_runtime::check_response(
        &op.responses,
        &suspect_runtime::Schemas::from_spec(&spec),
        suspect_runtime::Reply {
            method: "GET",
            status,
            headers: &headers,
            body: &body,
        },
        suspect_runtime::Conformance::Strict,
    );
    assert!(check.conforms(), "{:?}", check.violations);
}

// ------------------------------------------- real-world descriptions

/// How the mock fares against real-world descriptions: for every operation
/// of every corpus document, the response the mock would serve is held to
/// the strict contract check.
///
/// An explicit gate (`cargo xtask fetch-corpus`, then `cargo test -p
/// suspect-gateway -- --ignored --nocapture corpus_mock`): it reads tens
/// of megabytes of descriptions. What still fails is a description whose
/// own `example` contradicts its schema, or a `oneOf` whose branches
/// overlap; the bound keeps the mock from sliding back to replies its own
/// description forbids.
#[test]
#[ignore = "reads the fetched corpus; run explicitly"]
fn corpus_mock_responses_conform() {
    /// Largest share of a document's operations whose mock reply may break
    /// the contract.
    const TOLERATED: f64 = 0.05;
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let entries = std::fs::read_dir(&corpus)
        .unwrap_or_else(|_| panic!("no corpus/: run `cargo xtask fetch-corpus` first"));
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    let mut failing_documents = Vec::new();
    for path in paths {
        let Ok(spec) = suspect_ir::IrSpec::from_file(&path) else {
            continue; // not OpenAPI 3.x
        };
        let schemas = suspect_runtime::Schemas::from_spec(&spec);
        let mocks = crate::mock::compile_all(&spec);
        let mut broken = 0usize;
        let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
        for op in &spec.operations {
            let Some(reply) = mocks
                .get(&(op.method, op.path.clone()))
                .and_then(|compiled| crate::mock::best(compiled))
            else {
                continue;
            };
            let headers: Vec<(String, String)> = reply
                .content_type
                .iter()
                .map(|value| ("content-type".to_owned(), value.clone()))
                .chain(reply.headers.iter().cloned())
                .collect();
            let check = suspect_runtime::check_response(
                &op.responses,
                &schemas,
                suspect_runtime::Reply {
                    method: op.method.as_str(),
                    status: reply.status.unwrap_or(200),
                    headers: &headers,
                    body: &reply.body,
                },
                suspect_runtime::Conformance::Strict,
            );
            if !check.conforms() {
                broken += 1;
                // Group by the message with its location's indices erased.
                let first = &check.violations[0];
                let shape: String = first
                    .message
                    .chars()
                    .map(|c| if c.is_ascii_digit() { '#' } else { c })
                    .take(90)
                    .collect();
                *reasons.entry(shape).or_default() += 1;
            }
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        eprintln!(
            "{name:<28} operations: {:<5} mock replies that break the contract: {broken}",
            spec.operations.len()
        );
        let mut top: Vec<_> = reasons.into_iter().collect();
        top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (reason, count) in top.iter().take(6) {
            eprintln!("    {count:>5}  {reason}");
        }
        if broken as f64 > TOLERATED * spec.operations.len() as f64 {
            failing_documents.push(format!("{name}: {broken} of {}", spec.operations.len()));
        }
    }
    assert!(
        failing_documents.is_empty(),
        "more than {TOLERATED} of the mock's own replies break the contract in: {failing_documents:?}"
    );
}

/// Schema shapes real descriptions use that an example has to satisfy:
/// unions, constants, bounds, formats, recursion, untyped objects and an
/// integer id on a created resource.
const SHAPES_SPEC: &str = r##"
openapi: 3.1.0
info: { title: Shapes, version: "1" }
paths:
  /shapes:
    get:
      operationId: listShapes
      responses:
        '200':
          description: every shape
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Shapes' }
    post:
      operationId: createShape
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Counter' }
  /shapes/{shapeId}:
    get:
      operationId: showShape
      parameters:
        - { name: shapeId, in: path, required: true, schema: { type: integer } }
      responses:
        '200':
          description: one
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Counter' }
components:
  schemas:
    Counter:
      type: object
      required: [id, count]
      additionalProperties: false
      properties:
        id: { type: integer, minimum: 1 }
        count: { type: integer, minimum: 3, maximum: 9 }
    Shapes:
      type: object
      required: [union, nullable_first, constant, bounded, exclusive, when, day, key, mail, link, several, untyped, tree, open, either, code, version, archived]
      properties:
        union:
          oneOf:
            - { $ref: '#/components/schemas/Counter' }
            - { type: string }
        nullable_first:
          anyOf:
            - { type: "null" }
            - { type: string, minLength: 2 }
        constant: { const: fixed }
        bounded: { type: number, minimum: 5, maximum: 6 }
        exclusive: { type: integer, exclusiveMinimum: 10 }
        when: { type: string, format: date-time }
        day: { type: string, format: date }
        key: { type: string, format: uuid }
        mail: { type: string, format: email }
        link: { type: string, format: uri }
        several: { type: array, minItems: 2, items: { type: integer } }
        untyped:
          # `ghost` is required but never declared: any value satisfies it.
          required: [inner, ghost]
          properties:
            inner: { type: boolean }
        tree: { $ref: '#/components/schemas/Node' }
        open: { type: object, additionalProperties: { type: string } }
        either: { type: [integer, "null"] }
        code: { type: string, pattern: '^[A-Z]{3}$' }
        version: { type: string, pattern: '^[0-9]+\.[0-9]+$' }
        archived:
          allOf:
            - { $ref: '#/components/schemas/Node' }
            - { required: [parent] }
    Node:
      type: object
      required: [name, children]
      properties:
        name: { type: string }
        children:
          type: array
          items: { $ref: '#/components/schemas/Node' }
        parent: { $ref: '#/components/schemas/Node' }
"##;

/// Whatever the mock answers for the shapes fixture satisfies the schema
/// it was synthesized from, through the same check contract tests apply.
#[tokio::test]
async fn mock_examples_satisfy_the_schemas_they_come_from() {
    let (base, _sink, dir) = gateway_over(SHAPES_SPEC, Mode::Mock).await;
    let spec = suspect_ir::IrSpec::from_file(&dir.path().join("spec.yaml")).expect("spec");
    let schemas = suspect_runtime::Schemas::from_spec(&spec);
    let conforms = |id: &str, status: u16, headers: &[(String, String)], body: &[u8]| {
        let op = spec
            .operation(suspect_ir::OpSelector::Id(id))
            .expect("operation");
        let check = suspect_runtime::check_response(
            &op.responses,
            &schemas,
            suspect_runtime::Reply {
                method: op.method.as_str(),
                status,
                headers,
                body,
            },
            suspect_runtime::Conformance::Strict,
        );
        assert!(
            check.conforms(),
            "{id} answered {status} {}: {:?}",
            String::from_utf8_lossy(body),
            check.violations
        );
    };

    let (status, headers, body) = request(&base, "GET", "/shapes").await;
    conforms("listShapes", status, &headers, &body);
    let shapes: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(shapes["constant"], "fixed");
    assert_eq!(shapes["several"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        shapes["tree"]["children"],
        serde_json::json!([]),
        "a recursive schema ends in an empty list, not in `null`"
    );
    assert!(
        shapes["tree"].get("parent").is_none(),
        "an optional recursive property is left out rather than nulled"
    );

    // A created resource keeps the id type its schema declares.
    let (status, headers, body) = send(&base, "POST", "/shapes", "{}").await;
    conforms("createShape", status, &headers, &body);
    let created: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let id = created["id"].as_u64().expect("an integer id");
    let (status, headers, body) = request(&base, "GET", &format!("/shapes/{id}")).await;
    conforms("showShape", status, &headers, &body);
}
