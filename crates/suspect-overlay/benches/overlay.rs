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

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
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

criterion_group!(benches, bench_all);
criterion_main!(benches);
