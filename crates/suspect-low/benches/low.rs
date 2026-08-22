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

/// Parses a YAML fixture once; `None` when the gitignored fixtures are
/// absent (clean checkouts, CI).
fn setup_doc(name: &str) -> Option<LowDoc> {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path).ok()?;
    Some(LowDoc::parse(
        Uri::from_path(&path).expect("valid fixture URI"),
        Source::from_vec(bytes),
    ))
}

/// RFC 6901 fragment pointer to `#/paths/<key>/get`, with `~1` escaping.
fn path_get_pointer(path_key: &str) -> Pointer {
    Pointer::parse(&format!("#/paths/{}/get", path_key.replace('/', "~1")))
        .expect("constructed pointer must parse")
}

fn bench_parse(c: &mut Criterion) {
    let path = fixtures_dir().join("generated_1000x1000.yaml");
    let Some(bytes) = std::fs::read(&path).ok() else {
        eprintln!("low bench: skipping; {} absent", path.display());
        return;
    };
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
    let Some(doc) = setup_doc("generated_100x100.yaml") else {
        eprintln!("low bench: skipping; fixture absent");
        return;
    };
    // Collect real path keys at setup so pointers hit live data.
    let keys: Vec<String> = doc
        .root()
        .get("paths")
        .map(|paths| {
            paths
                .entries()
                .into_iter()
                .map(|e| e.key.to_owned())
                .collect()
        })
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
    let Some(doc) = setup_doc("generated_100x100.yaml") else {
        eprintln!("low bench: skipping; fixture absent");
        return;
    };
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
    let Some(doc) = setup_doc("generated_100x100.yaml") else {
        eprintln!("low bench: skipping; fixture absent");
        return;
    };
    let root = doc.root();

    let mut group = c.benchmark_group("low/duplicate_keys");
    group.bench_function("root_yaml_100x100", |b| {
        b.iter(|| black_box(root.duplicate_keys().len()));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_parse,
    bench_pointer_lookups,
    bench_entries_iteration,
    bench_duplicate_keys,
    bench_corpus_low,
    bench_corpus_traverse
);
criterion_main!(benches);

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Reads a gitignored corpus file; returns `None` (with a note) when the
/// corpus is not checked out so benchmarks skip instead of panicking.
fn read_corpus(name: &str) -> Option<(PathBuf, Vec<u8>)> {
    let path = corpus_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some((path, bytes)),
        Err(e) => {
            eprintln!("skipping corpus benchmark for {}: {e}", path.display());
            None
        }
    }
}

fn bench_corpus_low(c: &mut Criterion) {
    let mut group = c.benchmark_group("corpus_low");
    group.sample_size(10);

    for (label, file) in [
        ("stripe_yaml", "stripe.yaml"),
        ("github_yaml", "api.github.com.yaml"),
        ("kubernetes_yaml", "kubernetes-swagger.yaml"),
    ] {
        let Some((path, bytes)) = read_corpus(file) else {
            continue;
        };
        let uri = Uri::from_path(&path)
            .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", path.display()));
        group.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
        group.bench_function(label, |b| {
            b.iter(|| {
                let doc = LowDoc::parse(black_box(uri.clone()), Source::from_vec(bytes.clone()));
                black_box(doc.syntax_errors().len());
            });
        });
    }
    group.finish();
}

fn bench_corpus_traverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("corpus_traverse");
    let Some((path, bytes)) = read_corpus("stripe.yaml") else {
        group.finish();
        return;
    };
    let uri = Uri::from_path(&path)
        .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", path.display()));
    let doc = LowDoc::parse(uri, Source::from_vec(bytes));
    group.sample_size(20);

    // Count component schemas and walk the first 100 schemas' properties:
    // measures repeated mapping-entry iteration over a real spec shape.
    group.bench_function("stripe_schemas_walk_100", |b| {
        b.iter(|| {
            let root = doc.root();
            let mut visited = 0usize;
            if let Some(schemas) = root.get("components").and_then(|c| c.get("schemas")) {
                for entry in schemas.entries() {
                    visited += 1;
                    if visited > 100 {
                        break;
                    }
                    if let Some(props) = entry.value.and_then(|v| v.get("properties")) {
                        visited += props.entries().len();
                    }
                }
            }
            black_box(visited);
        });
    });

    group.bench_function("stripe_root_duplicate_keys", |b| {
        b.iter(|| black_box(doc.root().duplicate_keys().len()));
    });
    group.finish();
}
