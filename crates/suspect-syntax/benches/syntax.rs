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
use suspect_syntax::{Format, SourceDoc};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
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

criterion_group!(benches, bench_all);
criterion_main!(benches);
