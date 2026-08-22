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

/// Loads a fixture; `None` when the (gitignored) fixture set is absent so
/// bench-smoke runs on clean checkouts stay green.
fn setup_doc(name: &str) -> Option<LowDoc> {
    let path = fixtures_dir().join(name);
    let bytes = std::fs::read(&path).ok()?;
    Some(LowDoc::parse(
        Uri::from_path(&path).expect("valid fixture URI"),
        Source::from_vec(bytes),
    ))
}

fn bench_query(c: &mut Criterion, label: &str, expr: &str, fixture: &str) {
    let Some(doc) = setup_doc(fixture) else {
        eprintln!(
            "jsonpath bench: skipping {label}; fixture {} absent",
            fixtures_dir().join(fixture).display()
        );
        return;
    };
    let query = Path::parse(expr).unwrap_or_else(|e| panic!("query {expr:?} must compile: {e}"));
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

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Loads a corpus document. Corpus files are gitignored but expected on
/// dev machines; a missing file panics with the path for clarity.
fn setup_corpus_doc(name: &str) -> LowDoc {
    let path = corpus_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read corpus {}: {e}", path.display()));
    let uri = Uri::from_path(&path).expect("valid corpus URI");
    LowDoc::parse(uri, Source::from_vec(bytes))
}

fn bench_corpus_query(c: &mut Criterion, expr: &str, doc_name: &str, sample_size: usize) {
    let doc = setup_corpus_doc(doc_name);
    let query = match Path::parse(expr) {
        Ok(q) => q,
        Err(e) => panic!("query {expr:?} must compile: {e}"),
    };
    let hits = query.query(doc.root()).len();
    // One-time setup report, not per-iteration output.
    println!("[jsonpath/corpus_query] {expr} over {doc_name}: {hits} matches");
    assert!(hits > 0, "query {expr:?} matched nothing in {doc_name}");

    let label = doc_name.trim_end_matches(".yaml");
    let bench_id = match expr {
        "$..['$ref']" => "descendant_ref_bracket",
        "$..schema" => "descendant_schema",
        "$.paths.*.get.responses" => "paths_star_get_responses",
        "$.components..properties.*" => "components_descendant_properties_star",
        other => panic!("add a bench id for query {other:?}"),
    };
    let mut group = c.benchmark_group("jsonpath/corpus_query");
    group.sample_size(sample_size);
    group.throughput(criterion::Throughput::Elements(hits as u64));
    group.bench_function(format!("{bench_id}/{label}"), |b| {
        b.iter(|| black_box(query.query(black_box(doc.root())).len()));
    });
    group.finish();
}

fn bench_corpus(c: &mut Criterion) {
    // Descendant scans over the 6.4 MB stripe spec may exceed 1s per eval,
    // so stripe-scale queries use the minimum sample count.
    bench_corpus_query(c, "$..['$ref']", "stripe.yaml", 10);
    bench_corpus_query(c, "$..schema", "stripe.yaml", 10);
    bench_corpus_query(c, "$.paths.*.get.responses", "api.github.com.yaml", 10);
    bench_corpus_query(c, "$.components..properties.*", "api.github.com.yaml", 10);
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

criterion_group!(benches, bench_all, bench_corpus);
criterion_main!(benches);
