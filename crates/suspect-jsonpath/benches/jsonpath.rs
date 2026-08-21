//! JSONPath benchmarks for `suspect-jsonpath`: one compiled query evaluated
//! repeatedly over parsed LowDoc roots from the generated fixtures.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_jsonpath::Path;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn setup_doc(name: &str) -> LowDoc {
    let path = fixtures_dir().join(name);
    let source = Source::from_vec(
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display())),
    );
    LowDoc::parse(Uri::from_path(&path).expect("valid fixture URI"), source)
}

fn bench_query(c: &mut Criterion, label: &str, expr: &str, fixture: &str) {
    let doc = setup_doc(fixture);
    let query = Path::parse(expr)
        .unwrap_or_else(|e| panic!("query {expr:?} must compile: {e}"));
    // Sanity-check the query matches something before timing it.
    let hits = query.query(doc.root()).len();
    assert!(hits > 0, "query {expr:?} matched nothing in {fixture}");

    let mut group = c.benchmark_group(label);
    group.throughput(criterion::Throughput::Elements(hits as u64));
    group.bench_function("query", |b| {
        b.iter(|| black_box(query.query(black_box(doc.root())).len()));
    });
    group.finish();
}

fn bench_all(c: &mut Criterion) {
    bench_query(
        c,
        "jsonpath/paths_star_get/yaml_100x100",
        "$.paths.*.get",
        "generated_100x100.yaml",
    );
    bench_query(
        c,
        "jsonpath/descendant_star/json_100x100",
        "$..*",
        "generated_100x100.json",
    );
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
