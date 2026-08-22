//! Linting benchmarks for `suspect-lint`: the full Spectral-style default
//! pack ([`Linter::spectral_default`]) run over real OpenAPI corpus
//! documents.
//!
//! Corpus files are gitignored; each document is skipped gracefully when
//! absent.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

/// `(file, criterion sample size)` — big documents get fewer samples so the
/// whole suite stays fast.
const CORPUS_DOCS: &[(&str, usize)] = &[
    ("stripe.yaml", 10),
    ("api.github.com.yaml", 10),
    ("digitalocean.yaml", 20),
    ("petstore-expanded.yaml", 100),
];

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Loads a corpus document as a `LowDoc`, skipping gracefully when absent or
/// unreadable.
fn load_corpus(name: &str) -> Option<LowDoc> {
    let path = Path::new(&*corpus_dir()).join(name);
    if !path.exists() {
        eprintln!(
            "[lint] skipping {name}: corpus file {} not found",
            path.display()
        );
        return None;
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[lint] skipping {name}: read failed ({e})");
            return None;
        }
    };
    let uri =
        Uri::from_path(&path).unwrap_or_else(|e| panic!("valid URI for {}: {e}", path.display()));
    Some(LowDoc::parse(uri, Source::from_vec(bytes)))
}

fn bench_corpus_lint(c: &mut Criterion) {
    let mut group = c.benchmark_group("corpus_lint");
    let linter = Linter::spectral_default();

    for (name, sample_size) in CORPUS_DOCS {
        let Some(doc) = load_corpus(name) else {
            continue;
        };

        // Warmup + sanity: the default pack must produce findings without
        // panicking on any of these documents.
        let count = linter.run(&doc).len();
        eprintln!("[lint] {name}: {count} findings (warmup)");

        group.sample_size(*sample_size);
        let stem = name.strip_suffix(".yaml").unwrap_or(name);
        group.bench_function(stem.to_string(), |b| {
            b.iter(|| black_box(linter.run(black_box(&doc)).len()))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_corpus_lint);
criterion_main!(benches);
