//! Typed-model traversal benchmarks for `suspect-oas`.
//!
//! Measures `Session::load` (workspace load + family sniff + typed root
//! construction) followed by a full walk of the lazy views: operations with
//! `operation_id` resolution, path iteration, component schema enumeration,
//! and `info`/`servers`/`tags` access. Corpus files are read once at setup;
//! missing corpus files skip the group gracefully.

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn read_corpus(name: &str) -> Option<(PathBuf, Vec<u8>)> {
    let path = corpus_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some((path, bytes)),
        Err(e) => {
            eprintln!("skipping: corpus file {} unavailable ({e})", path.display());
            None
        }
    }
}

/// Full typed walk: operations -> operation_ids, paths, schemas,
/// info/servers/tags. Returns aggregate counts so nothing is DCE'd.
fn walk(api: &suspect_oas::OpenApi<'_>) -> (usize, usize, usize) {
    let mut op_ids = 0usize;
    for op in api.operations() {
        if op.operation_id().is_some() {
            op_ids += 1;
        }
        black_box(op.responses().map(|r| r.len()));
        black_box(op.parameters().len());
    }

    let mut path_count = 0usize;
    if let Some(paths) = api.paths() {
        for (key, item) in paths.iter() {
            black_box(key);
            black_box(item.parameters().len());
            path_count += 1;
        }
    }

    let schema_count = api.components().map(|c| c.schemas().len()).unwrap_or(0);

    // Root-level metadata access.
    black_box(api.info().and_then(|i| i.title()).map(str::len));
    black_box(api.servers().len());
    black_box(api.tags().len());

    (op_ids, path_count, schema_count)
}

fn bench_typed_traversal(c: &mut Criterion) {
    for (name, file, sample) in [
        ("typed_traversal/stripe_yaml", "stripe.yaml", Some(10)),
        (
            "typed_traversal/github_yaml",
            "api.github.com.yaml",
            Some(10),
        ),
    ] {
        let Some((_, bytes)) = read_corpus(file) else {
            continue;
        };
        let entry = file.to_owned();
        let ws = WorkspaceBuilder::new()
            .root(corpus_dir())
            .build()
            .unwrap_or_else(|e| panic!("workspace build failed: {e}"));
        let ws = Arc::new(ws);
        // Warm the workspace cache so the timed section measures session
        // load + view traversal on an already-materialized document set.
        if ws.load_all(&entry).is_err() {
            eprintln!("skipping {name}: {file} failed to load into workspace");
            continue;
        }

        let mut g = c.benchmark_group(name);
        g.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
        if let Some(n) = sample {
            g.sample_size(n);
        }
        g.bench_function("load_and_walk", |b| {
            b.iter(|| {
                let session = Session::new(Arc::clone(&ws));
                let api = session
                    .load(black_box(&entry))
                    .expect("entry loads as OpenAPI");
                black_box(walk(&api))
            })
        });
        g.finish();

        // One-shot sanity report of what the walk observes.
        let session = Session::new(Arc::clone(&ws));
        let api = session.load(&entry).expect("entry loads as OpenAPI");
        let (ops, paths, schemas) = walk(&api);
        eprintln!("{name}: operations={ops} paths={paths} schemas={schemas}");
    }
}

criterion_group!(benches, bench_typed_traversal);
criterion_main!(benches);
