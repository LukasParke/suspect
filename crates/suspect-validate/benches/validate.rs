//! Semantic-validation benchmarks for `suspect-validate`: the full check
//! battery ([`suspect_validate::validate_entry`]) over real OpenAPI corpus
//! documents, with a preloaded `Session` built outside the timed section.
//!
//! Corpus files are gitignored; each document is skipped gracefully when
//! absent.

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;

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

fn corpus_path(name: &str) -> Option<PathBuf> {
    let p = corpus_dir().join(name);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("[validate] skipping {name}: corpus file {} not found", p.display());
        None
    }
}

fn bench_corpus_validate(c: &mut Criterion) {
    let mut group = c.benchmark_group("corpus_validate");

    for (name, sample_size) in CORPUS_DOCS {
        if corpus_path(name).is_none() {
            continue;
        }
        let ws = Arc::new(
            WorkspaceBuilder::new()
                .root(corpus_dir())
                .max_doc_size(256 << 20)
                .build()
                .expect("workspace builds"),
        );
        let session = Session::new(ws);

        // Preload + sanity check outside timing. A document that fails to
        // load as an OpenAPI model is skipped with a note.
        let expected = match suspect_validate::validate_entry(&session, name) {
            Ok(diags) => diags.len(),
            Err(e) => {
                eprintln!("[validate] skipping {name}: does not load as OpenAPI ({e})");
                continue;
            }
        };

        group.sample_size(*sample_size);
        let stem = name.strip_suffix(".yaml").unwrap_or(name);
        group.bench_function(stem.to_string(), |b| {
            b.iter(|| {
                let diags = suspect_validate::validate_entry(
                    black_box(&session),
                    name,
                )
                .expect("validates after warmup");
                debug_assert_eq!(diags.len(), expected, "diagnostic count is stable");
                black_box(diags.len())
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_corpus_validate);
criterion_main!(benches);
