//! `$ref` engine benchmarks for `suspect-ref`: cold workspace loading of the
//! circular 2000x2000 fixture, warm memoized pointer resolution over sampled
//! schema pointers, and the cycle census.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_low::Pointer;
use suspect_ref::{Workspace, WorkspaceBuilder};

const CIRCULAR_FIXTURE: &str = "generated_2000x2000.yaml";
const SAMPLE_SIZE: usize = 100;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn build_workspace() -> Workspace {
    WorkspaceBuilder::new()
        .root(fixtures_dir())
        .build()
        .expect("workspace builds")
}

/// Evenly samples `SAMPLE_SIZE` pointers into `components.schemas`.
fn sample_schema_pointers(ws: &Workspace) -> Vec<Pointer> {
    let handle = ws.open(CIRCULAR_FIXTURE).expect("fixture opens");
    let schemas = handle
        .doc()
        .root()
        .get("components")
        .and_then(|c| c.get("schemas"))
        .expect("fixture has components.schemas");
    let entries = schemas.entries();
    assert!(entries.len() >= SAMPLE_SIZE, "not enough schemas to sample");
    let step = entries.len() / SAMPLE_SIZE;
    (0..SAMPLE_SIZE)
        .map(|i| Pointer::from_tokens(vec!["components".into(), "schemas".into(), entries[i * step].key.into()]))
        .collect()
}

fn bench_cold_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("ref/cold_load_all");
    group.bench_function("yaml_2000x2000_circular", |b| {
        b.iter(|| {
            let ws = build_workspace();
            let loaded = ws
                .load_all(CIRCULAR_FIXTURE)
                .expect("load_all succeeds");
            black_box((loaded, ws.len()));
        });
    });
    group.finish();
}

fn bench_warm_resolve(c: &mut Criterion) {
    let ws = build_workspace();
    ws.load_all(CIRCULAR_FIXTURE).expect("warmup load succeeds");
    let handle = ws.open(CIRCULAR_FIXTURE).expect("fixture opens");
    let pointers = sample_schema_pointers(&ws);

    // Warm the memo cache once outside timing so the bench measures the
    // memoized hot path.
    for p in &pointers {
        handle.resolve_pointer(handle.id(), p).expect("resolves");
    }

    let mut group = c.benchmark_group("ref/warm_resolve_pointer");
    group.throughput(criterion::Throughput::Elements(pointers.len() as u64));
    group.bench_function("schema_pointers_100_yaml_2000x2000", |b| {
        b.iter(|| {
            for p in &pointers {
                black_box(handle.resolve_pointer(handle.id(), p).is_ok());
            }
        });
    });
    group.finish();
}

fn bench_cycle_census(c: &mut Criterion) {
    let ws = build_workspace();
    ws.load_all(CIRCULAR_FIXTURE).expect("warmup load succeeds");
    let handle = ws.open(CIRCULAR_FIXTURE).expect("fixture opens");
    let expected = handle.cycles().cycles.len();
    assert!(expected > 0, "circular fixture must contain cycles");

    let mut group = c.benchmark_group("ref/cycle_census");
    group.bench_function("yaml_2000x2000_circular", |b| {
        b.iter(|| black_box(handle.cycles().cycles.len()));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_cold_load,
    bench_warm_resolve,
    bench_cycle_census
);
criterion_main!(benches);
