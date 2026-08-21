//! Source-loading benchmarks for `suspect-source`: line-index
//! construction, BOM detection + UTF-16 transcoding at load, and URI
//! join resolution, profiled against real OpenAPI corpus files.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use suspect_source::{LineIndex, Source, Uri};

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

fn bench_line_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index");

    for (label, file) in [
        ("stripe_yaml", "stripe.yaml"),
        ("github_yaml", "api.github.com.yaml"),
    ] {
        let Some((_, bytes)) = read_corpus(file) else {
            continue;
        };
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(label, |b| {
            b.iter(|| black_box(LineIndex::new(black_box(&bytes))));
        });
    }
    group.finish();
}

fn bench_encoding_transcode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encoding_transcode");
    let Some((_, bytes)) = read_corpus("stripe.yaml") else {
        group.finish();
        return;
    };

    // 1 MB UTF-8 slice of stripe.yaml, encoded to UTF-16LE with a BOM so
    // `Source::from_vec` detects the encoding and transcodes on load.
    let slice = &bytes[..bytes.len().min(1024 * 1024)];
    let text = String::from_utf8_lossy(slice);
    let mut utf16le: Vec<u8> = Vec::with_capacity(text.len() * 2 + 2);
    utf16le.extend_from_slice(&[0xFF, 0xFE]); // UTF-16LE BOM
    for unit in text.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }

    group.sample_size(20);
    group.throughput(Throughput::Bytes(utf16le.len() as u64));
    group.bench_function("utf16le_to_utf8_1mb", |b| {
        b.iter(|| {
            let source = Source::from_vec(utf16le.clone());
            black_box(source.bytes().len());
        });
    });
    group.finish();
}

fn bench_uri_join(c: &mut Criterion) {
    let mut group = c.benchmark_group("uri_join");

    let base_path = corpus_dir().join("stripe.yaml");
    let base = Uri::from_path(&base_path)
        .unwrap_or_else(|e| panic!("failed to make URI for {}: {e}", base_path.display()));

    // 10k joins against fragment pointers, the shape `$ref` resolution hits.
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("x10_000_fragment_refs", |b| {
        b.iter(|| {
            for i in 0..10_000u32 {
                let reference = format!("#/components/schemas/thing_{i}");
                black_box(base.join(&reference).ok());
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_line_index, bench_encoding_transcode, bench_uri_join);
criterion_main!(benches);
