//! Low-level document model benchmarks for `suspect-low`: parse, JSON
//! Pointer lookups, mapping-entry iteration, and duplicate-key reporting
//! over the generated OpenAPI 3.1 fixtures.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_low::{LowDoc, Pointer};
use suspect_source::{Source, Uri};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Parses a YAML fixture once for use as benchmark state.
fn setup_doc(name: &str) -> LowDoc {
    let path = fixtures_dir().join(name);
    let source = Source::from_vec(
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display())),
    );
    LowDoc::parse(Uri::from_path(&path).expect("valid fixture URI"), source)
}

/// RFC 6901 fragment pointer to `#/paths/<key>/get`, with `~1` escaping.
fn path_get_pointer(path_key: &str) -> Pointer {
    Pointer::parse(&format!("#/paths/{}/get", path_key.replace('/', "~1")))
        .expect("constructed pointer must parse")
}

fn bench_parse(c: &mut Criterion) {
    let path = fixtures_dir().join("generated_1000x1000.yaml");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    let uri = Uri::from_path(&path).expect("valid fixture URI");

    let mut group = c.benchmark_group("low/parse");
    group.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
    group.bench_function("yaml/1000x1000/autodetect", |b| {
        b.iter(|| {
            let doc = LowDoc::parse(black_box(uri.clone()), Source::from_vec(bytes.clone()));
            black_box(doc.sniff_family())
        });
    });
    group.finish();
}

fn bench_pointer_lookups(c: &mut Criterion) {
    let doc = setup_doc("generated_100x100.yaml");
    // Collect real path keys at setup so pointers hit live data.
    let keys: Vec<String> = doc
        .root()
        .get("paths")
        .map(|paths| paths.entries().into_iter().map(|e| e.key.to_owned()).collect())
        .unwrap_or_default();
    assert!(!keys.is_empty(), "fixture must contain paths");
    let pointers: Vec<Pointer> = keys.iter().take(100).map(|k| path_get_pointer(k)).collect();

    let mut group = c.benchmark_group("low/pointer_lookup");
    group.throughput(criterion::Throughput::Elements(pointers.len() as u64));
    group.bench_function("paths_100_get_yaml_100x100", |b| {
        b.iter(|| {
            for p in &pointers {
                black_box(doc.root().pointer(p));
            }
        });
    });
    group.finish();
}

fn bench_entries_iteration(c: &mut Criterion) {
    let doc = setup_doc("generated_100x100.yaml");
    let schemas = doc
        .root()
        .get("components")
        .and_then(|c| c.get("schemas"))
        .expect("fixture must have components.schemas");

    let mut group = c.benchmark_group("low/entries");
    group.bench_function("components_schemas_yaml_100x100", |b| {
        b.iter(|| {
            let entries = schemas.entries();
            black_box(entries.len())
        });
    });
    group.finish();
}

fn bench_duplicate_keys(c: &mut Criterion) {
    let doc = setup_doc("generated_100x100.yaml");
    let root = doc.root();

    let mut group = c.benchmark_group("low/duplicate_keys");
    group.bench_function("root_yaml_100x100", |b| {
        b.iter(|| black_box(root.duplicate_keys().len()));
    });
    group.finish();
}

criterion_group!(benches, bench_parse, bench_pointer_lookups, bench_entries_iteration, bench_duplicate_keys);
criterion_main!(benches);
