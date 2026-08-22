//! Overlay apply benchmarks for `suspect-overlay`: a small overlay (10
//! update actions + 1 remove action) applied to the 100x100 YAML fixture.
//!
//! Both documents are parsed once at setup; the timed section measures
//! only [`apply`].

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_low::LowDoc;
use suspect_overlay::{OverlayDoc, apply};
use suspect_source::{Source, Uri};

const OVERLAY_TEXT: &str = r#"
overlay: 1.0.0
info:
  title: bench overlay
  version: "1.0.0"
extends: generated_100x100.yaml
actions:
  - target: "$.paths['/items/item-0'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-1'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-2'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-3'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-4'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-5'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-6'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-7'].get"
    update:
      deprecated: true
      description: overlaid by bench
  - target: "$.paths['/items/item-8'].post.responses"
    update:
      "202":
        description: added by bench
  - target: "$.info"
    update:
      summary: overlaid summary
  - target: "$.tags"
    remove: true
"#;

/// A 10-action update-only overlay aimed at real DigitalOcean API paths
/// (targets that exist in the corpus document).
const CORPUS_OVERLAY_TEXT: &str = r#"
overlay: 1.0.0
info:
  title: bench corpus overlay
  version: "1.0.0"
actions:
  - target: "$.info"
    update:
      summary: overlaid by corpus bench
  - target: "$.servers[0]"
    update:
      description: production (overlaid)
  - target: "$.paths['/v2/account'].get"
    update:
      deprecated: true
  - target: "$.paths['/v2/actions'].get"
    update:
      deprecated: true
  - target: "$.paths['/v2/1-clicks'].get"
    update:
      deprecated: true
  - target: "$.paths['/v2/account/keys'].get"
    update:
      deprecated: true
  - target: "$.tags"
    update:
      - name: overlaid-tag
        description: added by corpus bench
  - target: "$.info.license"
    update:
      name: Apache 2.0 (overlaid)
  - target: "$.info.contact"
    update:
      name: Bench Overlay Team
  - target: "$.security"
    remove: true
"#;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn bench_all(c: &mut Criterion) {
    let path = fixtures_dir().join("generated_100x100.yaml");
    let target = LowDoc::parse(
        Uri::from_path(&path).expect("valid fixture URI"),
        Source::from_vec(
            std::fs::read(&path)
                .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display())),
        ),
    );
    let overlay_doc = LowDoc::parse(
        Uri::parse("memory://bench-overlay.yaml").expect("valid URI"),
        Source::from_vec(OVERLAY_TEXT.as_bytes().to_vec()),
    );
    let overlay = OverlayDoc::parse(&overlay_doc).expect("overlay parses");

    // Sanity check before timing: every action must hit.
    let applied = apply(&overlay, target.root()).expect("apply succeeds");
    assert_eq!(applied.applied_actions, 11, "all actions must match");
    assert!(applied.unmatched_targets.is_empty());

    let mut group = c.benchmark_group("overlay/apply");
    group.bench_function("11_actions_over_yaml_100x100", |b| {
        b.iter(|| {
            let applied =
                apply(black_box(&overlay), black_box(target.root())).expect("apply succeeds");
            black_box(applied.output.to_json().len())
        });
    });

    group.finish();
}

/// Applies the 10-action corpus overlay to `digitalocean.yaml`.
fn bench_corpus_apply(c: &mut Criterion) {
    let path = corpus_dir().join("digitalocean.yaml");
    if !path.exists() {
        eprintln!(
            "[overlay] skipping corpus_apply: corpus file {} not found",
            path.display()
        );
        return;
    }
    let target = LowDoc::parse(
        Uri::from_path(&path).expect("valid corpus URI"),
        Source::from_path(&path).expect("corpus file reads"),
    );
    let overlay_doc = LowDoc::parse(
        Uri::parse("memory://bench-corpus-overlay.yaml").expect("valid URI"),
        Source::from_vec(CORPUS_OVERLAY_TEXT.as_bytes().to_vec()),
    );
    let overlay = OverlayDoc::parse(&overlay_doc).expect("overlay parses");

    // Sanity check before timing: apply must succeed; most actions should
    // hit the real document.
    let applied = apply(&overlay, target.root()).expect("apply succeeds");
    assert_eq!(
        applied.applied_actions, 10,
        "all 10 corpus actions must match"
    );
    assert!(applied.unmatched_targets.is_empty());

    let mut group = c.benchmark_group("overlay/corpus_apply");
    group.sample_size(20);
    group.bench_function("10_actions_over_digitalocean_yaml", |b| {
        b.iter(|| {
            let applied =
                apply(black_box(&overlay), black_box(target.root())).expect("apply succeeds");
            black_box(applied.output.to_json().len())
        });
    });
    group.finish();
}

criterion_group!(benches, bench_all, bench_corpus_apply);
criterion_main!(benches);
