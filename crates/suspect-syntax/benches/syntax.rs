//! CST parsing benchmarks for `suspect-syntax`.
//!
//! Fixture bytes are read once at setup; each iteration clones the byte
//! buffer into a fresh `Source` so only parsing (tree-sitter + line index)
//! is measured. Clone cost for the largest fixture (~2.7 MB) is a few
//! hundred microseconds of memcpy against multi-millisecond parses.
//!
//! `with_format` variants skip format sniffing; `autodetect` variants run
//! [`SourceDoc::parse`] with its detection heuristic.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_source::{Source, Uri};
use suspect_syntax::{Edit, Format, SourceDoc};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

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

fn bench_parse(c: &mut Criterion, label: &str, fixture: &str, format: Option<Format>) {
    let path = fixtures_dir().join(fixture);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    let uri = Uri::from_path(&path)
        .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", path.display()));

    let mut group = c.benchmark_group(label);
    group.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
    group.bench_function(if format.is_some() { "with_format" } else { "autodetect" }, |b| {
        b.iter(|| {
            let source = Source::from_vec(bytes.clone());
            let doc = match format {
                Some(f) => SourceDoc::with_format(black_box(uri.clone()), source, f),
                None => SourceDoc::parse(black_box(uri.clone()), source),
            };
            // Touch the tree without walking it: keep the timed section
            // limited to parsing + line-index construction.
            black_box(doc.root().byte_range());
        });
    });
    group.finish();
}

fn bench_all(c: &mut Criterion) {
    bench_parse(c, "syntax/json/100x100", "generated_100x100.json", None);
    bench_parse(
        c,
        "syntax/json/1000x1000",
        "generated_1000x1000.json",
        Some(Format::Json),
    );
    bench_parse(
        c,
        "syntax/yaml/100x100",
        "generated_100x100.yaml",
        Some(Format::Yaml),
    );
    bench_parse(c, "syntax/yaml/1000x1000", "generated_1000x1000.yaml", None);
    bench_parse(
        c,
        "syntax/yaml/2000x2000-circular",
        "generated_2000x2000.yaml",
        Some(Format::Yaml),
    );
}

criterion_group!(benches, bench_all, bench_corpus_parse, bench_incremental_reparse);
criterion_main!(benches);

fn bench_corpus_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("corpus_parse");

    // (label, file, format, sample size). Large real-world specs use a
    // small sample size to keep suite runtime bounded.
    let cases: &[(&str, &str, Option<Format>, usize)] = &[
        ("stripe_yaml", "stripe.yaml", Some(Format::Yaml), 10),
        ("stripe_sdk_yaml", "stripe-sdk.yaml", Some(Format::Yaml), 10),
        ("github_yaml", "api.github.com.yaml", Some(Format::Yaml), 10),
        ("kubernetes_yaml", "kubernetes-swagger.yaml", Some(Format::Yaml), 10),
        ("stripe_yaml_autodetect", "stripe.yaml", None, 10),
        ("kubernetes_yaml_autodetect", "kubernetes-swagger.yaml", None, 10),
        ("petstore_expanded_yaml", "petstore-expanded.yaml", Some(Format::Yaml), 100),
    ];

    for (label, file, format, sample_size) in cases {
        let Some((path, bytes)) = read_corpus(file) else {
            continue;
        };
        let uri = Uri::from_path(&path)
            .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", path.display()));
        group.sample_size(*sample_size);
        group.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
        group.bench_function(*label, |b| {
            b.iter(|| {
                let source = Source::from_vec(bytes.clone());
                let doc = match format {
                    Some(f) => SourceDoc::with_format(black_box(uri.clone()), source, *f),
                    None => SourceDoc::parse(black_box(uri.clone()), source),
                };
                black_box(doc.root().byte_range());
            });
        });
    }
    group.finish();
}

fn bench_incremental_reparse(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_reparse");
    let Some((path, bytes)) = read_corpus("stripe.yaml") else {
        group.finish();
        return;
    };
    let uri = Uri::from_path(&path)
        .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", path.display()));
    let doc = SourceDoc::with_format(uri, Source::from_vec(bytes.clone()), Format::Yaml);

    // Insert 1 KB of valid YAML at 40 % of the document.
    let offset = bytes.len() * 2 / 5;
    let mut payload = Vec::with_capacity(1024);
    payload.extend_from_slice(b"\n# incremental-reparse benchmark insertion\nbenchmark_note:\n");
    while payload.len() < 1024 {
        payload.extend_from_slice(b"  padding_key: padding value for the benchmark insertion\n");
    }
    payload.truncate(1024);
    let edit = Edit::from_bytes(&doc, offset, offset, payload.len());

    // The edited buffer is prepared once; reparse is idempotent given the
    // same inputs, so every iteration replays the identical edit.
    let mut edited = Vec::with_capacity(bytes.len() + payload.len());
    edited.extend_from_slice(&bytes[..offset]);
    edited.extend_from_slice(&payload);
    edited.extend_from_slice(&bytes[offset..]);

    group.sample_size(20);
    group.throughput(criterion::Throughput::Bytes(bytes.len() as u64));
    group.bench_function("stripe_1kb_insert", |b| {
        b.iter(|| {
            let reparse =
                doc.reparse(Source::from_vec(edited.clone()), std::slice::from_ref(&edit));
            black_box(reparse.root().byte_range());
        });
    });
    group.bench_function("full_reparse_control", |b| {
        b.iter(|| {
            let reparse = doc.reparse(Source::from_vec(bytes.clone()), &[]);
            black_box(reparse.root().byte_range());
        });
    });
    group.finish();
}
