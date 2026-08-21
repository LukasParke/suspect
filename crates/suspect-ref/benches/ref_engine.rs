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

/// Corpus documents profiled alongside the synthetic fixtures. All are
/// self-contained single files; gitignored but present on dev machines.
const CORPUS_DOCS: &[&str] = &[
    "stripe.yaml",
    "stripe-sdk.yaml",
    "api.github.com.yaml",
    "kubernetes-swagger.yaml",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn corpus_path(name: &str) -> Option<PathBuf> {
    let p = corpus_dir().join(name);
    if p.exists() {
        Some(p)
    } else {
        eprintln!(
            "[ref_engine] skipping {}: corpus file {} not found",
            name,
            p.display()
        );
        None
    }
}

fn build_corpus_workspace() -> Workspace {
    WorkspaceBuilder::new()
        .root(corpus_dir())
        .max_doc_size(256 << 20)
        .build()
        .expect("workspace builds")
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
        .map(|i| {
            Pointer::from_tokens(vec![
                "components".into(),
                "schemas".into(),
                entries[i * step].key.into(),
            ])
        })
        .collect()
}

fn bench_cold_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("ref/cold_load_all");
    group.bench_function("yaml_2000x2000_circular", |b| {
        b.iter(|| {
            let ws = build_workspace();
            let loaded = ws.load_all(CIRCULAR_FIXTURE).expect("load_all succeeds");
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

/// Cold load: fresh workspace + full parse + edge scan per iteration.
///
/// stripe*.yaml hit a known scanner limitation: `$ref: >-` folded-scalar
/// values are read raw, so those refs misclassify as external and
/// `load_all` aborts. For such docs we fall back to cold `open` + full
/// edge scan (the same parse+scan work minus the BFS tail).
fn bench_corpus_cold_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("ref/corpus_load");
    group.sample_size(10);
    for name in CORPUS_DOCS {
        let Some(path) = corpus_path(name) else {
            continue;
        };
        let entry = path.file_name().unwrap().to_str().unwrap().to_owned();
        // Probe once outside the timed loop; also fails fast on real errors.
        let probe = build_corpus_workspace();
        match probe.load_all(&entry) {
            Ok(loaded) => eprintln!(
                "[ref/corpus_load] {name}: {loaded} docs, {} edges",
                probe.open(&entry).expect("reopens").edges().len()
            ),
            Err(e) => eprintln!(
                "[ref/corpus_load] {name}: load_all unavailable ({e}); \
                 benching cold open + edge scan instead"
            ),
        }
        let load_all_ok = probe.load_all(&entry).is_ok();
        drop(probe);

        let label = name.trim_end_matches(".yaml");
        group.bench_function(label.to_owned(), |b| {
            if load_all_ok {
                b.iter(|| {
                    let ws = build_corpus_workspace();
                    let loaded = ws.load_all(&entry).expect("load_all succeeds");
                    black_box((loaded, ws.len(), ws.stats().edges));
                });
            } else {
                b.iter(|| {
                    let ws = build_corpus_workspace();
                    let edges = ws.open(&entry).expect("cold open succeeds").edges().len();
                    black_box((ws.len(), edges));
                });
            }
        });
    }
    group.finish();
}

/// Cycle census over real specs (warm: document already loaded). Census
/// needs only the single doc's edges, so a `load_all` failure on stripe
/// (folded-scalar refs misread as external) does not block it.
fn bench_corpus_cycles(c: &mut Criterion) {
    for name in ["stripe.yaml", "api.github.com.yaml"] {
        let Some(path) = corpus_path(name) else {
            continue;
        };
        let entry = path.file_name().unwrap().to_str().unwrap().to_owned();
        let ws = build_corpus_workspace();
        if let Err(e) = ws.load_all(&entry) {
            eprintln!("[ref/corpus_cycles] {name}: load_all unavailable ({e})");
        }
        let handle = ws.open(&entry).expect("corpus doc opens");

        let mut group = c.benchmark_group("ref/corpus_cycles");
        group.sample_size(10);
        group.bench_function(name.trim_end_matches(".yaml").to_owned(), |b| {
            b.iter(|| black_box(handle.cycles().cycles.len()));
        });
        group.finish();
    }
}

/// Warm memoized pointer resolution over 50 sampled component schemas in
/// stripe.yaml. Pointers are collected once, outside the timed region.
fn bench_corpus_warm_resolve(c: &mut Criterion) {
    const WARM_SAMPLES: usize = 50;
    let Some(path) = corpus_path("stripe.yaml") else {
        return;
    };
    let entry = path.file_name().unwrap().to_str().unwrap().to_owned();
    let ws = build_corpus_workspace();
    if let Err(e) = ws.load_all(&entry) {
        eprintln!(
            "[ref/corpus_resolve] {entry}: load_all unavailable ({e}); \
             resolving within the single loaded doc"
        );
    }
    let handle = ws.open(&entry).expect("stripe.yaml opens");
    let schemas = handle
        .doc()
        .root()
        .get("components")
        .and_then(|c| c.get("schemas"))
        .expect("stripe.yaml has components.schemas");
    let keys: Vec<&str> = schemas.entries().iter().map(|e| e.key).collect();
    assert!(
        keys.len() >= WARM_SAMPLES,
        "stripe.yaml has too few schemas"
    );
    let step = keys.len() / WARM_SAMPLES;
    let pointers: Vec<Pointer> = (0..WARM_SAMPLES)
        .map(|i| {
            Pointer::from_tokens(vec![
                "components".into(),
                "schemas".into(),
                keys[i * step].into(),
            ])
        })
        .collect();

    // Warm the memo table so the bench measures the memoized hot path.
    for p in &pointers {
        handle
            .resolve_pointer(handle.id(), p)
            .unwrap_or_else(|e| panic!("sampled schema pointer fails to resolve: {e}"));
    }

    let mut group = c.benchmark_group("ref/corpus_resolve");
    group.throughput(criterion::Throughput::Elements(pointers.len() as u64));
    group.bench_function("stripe_yaml_schema_pointers_50", |b| {
        b.iter(|| {
            for p in &pointers {
                black_box(handle.resolve_pointer(handle.id(), p).is_ok());
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_cold_load,
    bench_warm_resolve,
    bench_cycle_census,
    bench_corpus_cold_load,
    bench_corpus_cycles,
    bench_corpus_warm_resolve
);
criterion_main!(benches);
