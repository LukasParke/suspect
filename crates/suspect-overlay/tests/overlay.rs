use suspect_low::LowDoc;
use suspect_source::Source;

use suspect_overlay::{apply, validate_overlay, OverlayDoc};

fn parse_yaml(src: &str) -> LowDoc {
    LowDoc::parse("mem://t.yaml".into(), Source::from_vec(src.as_bytes().to_vec()))
}

const TARGET: &str = r#"
openapi: 3.1.0
info:
  title: Tic
  version: 1.0.0
paths:
  /pets:
    get:
      summary: List pets
      responses:
        '200':
          description: ok
"#;

const OVERLAY: &str = r#"
overlay: 1.0.0
info:
  title: Test overlay
  version: 1.0.0
extends: './target.yaml'
actions:
  - target: $.info
    update:
      title: Ticked
      x-generated: true
  - target: $.paths['/pets'].get
    update:
      description: Updated description
  - target: $.paths['/gone']
    update:
      get:
        summary: New
  - target: $.paths.*.get.responses
    update:
      '4XX':
        description: error
"#;

#[test]
fn overlay_parses_and_validates() {
    let doc = parse_yaml(OVERLAY);
    let overlay = OverlayDoc::parse(&doc).expect("valid overlay");
    assert_eq!(overlay.version(), Some("1.0.0"));
    assert_eq!(overlay.extends(), Some("./target.yaml"));
    assert_eq!(overlay.actions().len(), 4);
    let diags = validate_overlay(&overlay);
    // missing descriptions are advisory only
    assert!(diags.iter().all(|d| d.code == "overlay-action-missing-description"));
}

#[test]
fn apply_updates_merge_recursively() {
    let target = parse_yaml(TARGET);
    let overlay_doc = parse_yaml(OVERLAY);
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let result = apply(&overlay, target.root()).expect("apply succeeds");

    let out = result.output.to_yaml();
    assert!(out.contains("title: Ticked"), "info.title updated: {out}");
    assert!(out.contains("x-generated: true"), "new key appended: {out}");
    assert!(out.contains("Updated description"), "nested merge: {out}");
    assert!(out.contains("4XX:"), "responses updated via wildcard: {out}");
    assert!(out.contains("List pets"), "untouched content preserved: {out}");
    assert_eq!(result.applied_actions, 3, "/gone target does not exist and counts as unmatched");
    assert_eq!(result.unmatched_targets, vec!["$.paths['/gone']"]);
}

#[test]
fn remove_deletes_nodes() {
    let target = parse_yaml(TARGET);
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.0.0
info:
  title: strip
  version: 1.0.0
actions:
  - target: $.paths['/pets'].get.summary
    remove: true
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let result = apply(&overlay, target.root()).unwrap();
    let out = result.output.to_yaml();
    assert!(!out.contains("List pets"), "summary removed: {out}");
    assert!(out.contains("responses"), "rest intact: {out}");
}

#[test]
fn sequential_actions_chain() {
    let target = parse_yaml("info:\n  title: A\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.0.0
info:
  title: chain
  version: 1.0.0
actions:
  - target: $
    update:
      info:
        version: 2.0.0
  - target: $.info
    remove: true
  - target: $
    update:
      info:
        title: Reborn
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let result = apply(&overlay, target.root()).unwrap();
    let out = result.output.to_yaml();
    // info was removed, then re-created with only title
    assert!(out.contains("title: Reborn"));
    assert!(!out.contains("2.0.0"));
}

#[test]
fn array_append_via_update() {
    let target = parse_yaml("servers:\n  - url: https://a\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.0.0
info:
  title: servers
  version: 1.0.0
actions:
  - target: $.servers
    update:
      url: https://b
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let result = apply(&overlay, target.root()).unwrap();
    let out = result.output.to_yaml();
    assert!(out.contains("https://a"), "existing entry kept: {out}");
    assert!(out.contains("https://b"), "update appended to array: {out}");
}

#[test]
fn unmatched_targets_reported() {
    let target = parse_yaml(TARGET);
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.0.0
info:
  title: miss
  version: 1.0.0
actions:
  - target: $.nowhere.to.be.found
    update:
      x: 1
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let result = apply(&overlay, target.root()).unwrap();
    assert_eq!(result.applied_actions, 0);
    assert_eq!(result.unmatched_targets, vec!["$.nowhere.to.be.found"]);
}

#[test]
fn invalid_overlay_rejected() {
    let doc = parse_yaml("overlay: 1.0.0\n");
    let err = OverlayDoc::parse(&doc).unwrap_err();
    assert!(matches!(err, suspect_overlay::OverlayError::MissingField { field: "info.title" }));

    let doc = parse_yaml("overlay: 1.0.0\ninfo: {title: t, version: v}\nactions: []\n");
    let overlay = OverlayDoc::parse(&doc).unwrap();
    let diags = validate_overlay(&overlay);
    assert!(diags.iter().any(|d| d.code == "overlay-empty-actions"));
}

#[test]
fn scalar_target_rejected_at_apply() {
    let target = parse_yaml("info:\n  title: A\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.0.0
info:
  title: bad
  version: 1.0.0
actions:
  - target: $.info.title
    update:
      x: 1
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let err = apply(&overlay, target.root()).unwrap_err();
    assert!(matches!(err, suspect_overlay::OverlayError::TargetNotContainer { .. }));
}
