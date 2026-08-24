//! In-process mock-serving latency benchmark.
//!
//! Drives the gateway router with `tower::ServiceExt::oneshot` — no
//! sockets, no kernel networking — so the measurement isolates routing,
//! dispatch, and precompiled-body serving from TCP overhead.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body as AxumBody;
use axum::http::Request;
use criterion::{Criterion, criterion_group, criterion_main};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Spec fixture (same shape as the test suite's).
const SPEC: &str = r#"
openapi: 3.1.0
info:
  title: Pets
  version: "1.0"
paths:
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
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
"#;

struct BenchSpec {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn write_spec() -> BenchSpec {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), SPEC).expect("write spec");
    BenchSpec {
        path: dir.path().join("spec.yaml"),
        _dir: dir,
    }
}

fn bench_mock_get(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let spec = write_spec();
    let cfg = suspect_gateway::GatewayConfig {
        mode: suspect_gateway::Mode::Mock,
        spec: spec.path.clone(),
        port: 0,
        faults: suspect_gateway::FaultConfig::default(),
    };
    let router = rt
        .block_on(suspect_gateway::build_router(
            &cfg,
            Arc::new(tokio::sync::Mutex::new(suspect_journal::Journal::new(
                Box::new(suspect_journal::VecSink::default()),
            ))),
        ))
        .expect("router");

    let mut group = c.benchmark_group("gateway-mock");
    group.bench_function("get_pet_by_id", |b| {
        b.iter(|| {
            let request = Request::builder()
                .uri("/pets/42")
                .body(AxumBody::empty())
                .expect("request");
            // Router is an Arc internally, so cloning per iteration is cheap.
            let router = router.clone();
            let bytes = rt.block_on(async move {
                let response = router.oneshot(request).await.expect("response");
                let (_, body) = response.into_parts();
                body.collect().await.expect("body").to_bytes()
            });
            std::hint::black_box(bytes);
        })
    });
    group.finish();
}

criterion_group!(benches, bench_mock_get);
criterion_main!(benches);
