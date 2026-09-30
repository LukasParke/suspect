use suspect_low::LowDoc;
use suspect_source::Source;

use suspect_overlay::{OverlayDoc, apply, explain, synthesize_overlay, validate_overlay};

fn parse_yaml(src: &str) -> LowDoc {
    LowDoc::parse(
        "mem://t.yaml".into(),
        Source::from_vec(src.as_bytes().to_vec()),
    )
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
    assert!(
        diags
            .iter()
            .all(|d| d.code == "overlay-action-missing-description")
    );
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
    assert!(
        out.contains("4XX:"),
        "responses updated via wildcard: {out}"
    );
    assert!(
        out.contains("List pets"),
        "untouched content preserved: {out}"
    );
    assert_eq!(
        result.applied_actions, 3,
        "/gone target does not exist and counts as unmatched"
    );
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
    assert!(matches!(
        err,
        suspect_overlay::OverlayError::MissingField {
            field: "info.title"
        }
    ));

    let doc = parse_yaml("overlay: 1.0.0\ninfo: {title: t, version: v}\nactions: []\n");
    let overlay = OverlayDoc::parse(&doc).unwrap();
    let diags = validate_overlay(&overlay);
    assert!(diags.iter().any(|d| d.code == "overlay-empty-actions"));
}

#[test]
fn scalar_target_with_object_update_conflicts() {
    // Overlay 1.1: a primitive update replaces a primitive target, but an
    // object update on a primitive target is an incompatible combination.
    let target = parse_yaml("info:\n  title: A\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
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
    assert!(matches!(
        err,
        suspect_overlay::OverlayError::MergeConflict { .. }
    ));
}

#[test]
fn scalar_target_replaced_by_scalar_update() {
    // Overlay 1.1 §4.4.3: "If the target selects primitive nodes, the
    // value of this field MUST be a primitive value to replace each
    // selected node."
    let target = parse_yaml("info:\n  title: A\n  version: 1\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info:
  title: scalar replace
  version: 1.0.0
actions:
  - target: $.info.title
    update: B
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let applied = apply(&overlay, target.root()).unwrap();
    assert_eq!(
        applied.output.to_json(),
        r#"{"info":{"title":"B","version":1}}"#
    );
}

#[test]
fn copy_moves_and_renames_nodes() {
    // Overlay 1.1 §4.5.6.3 (Move Example): update-to-exist, copy,
    // remove-source.
    let target = parse_yaml(
        "openapi: 3.1.0\ninfo: {title: Example API, version: '1.0.0'}\npaths:\n  /items:\n    get:\n      responses:\n        '200': {description: OK}\n  /some-items:\n    delete:\n      responses:\n        '200': {description: OK}\n",
    );
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info:
  title: move
  version: 1.0.0
actions:
  - target: '$.paths'
    update: { "/new-items": {} }
  - target: '$.paths["/new-items"]'
    copy: '$.paths["/items"]'
  - target: '$.paths["/items"]'
    remove: true
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let applied = apply(&overlay, target.root()).unwrap();
    let out = applied.output.to_json();
    assert!(out.contains(r#""/new-items":{"get":{"responses""#), "{out}");
    assert!(!out.contains(r#""/items""#), "source path removed: {out}");
    assert!(
        out.contains(r#""/some-items""#),
        "unrelated path kept: {out}"
    );
}

#[test]
fn copy_onto_existing_node_merges_recursively() {
    // Overlay 1.1 §4.5.6.1: copy into an existing path merges (delete is
    // preserved, get is added).
    let target = parse_yaml(
        "openapi: 3.1.0\ninfo: {title: Example API, version: '1.0.0'}\npaths:\n  /items:\n    get:\n      responses:\n        '200': {description: OK}\n  /some-items:\n    delete:\n      responses:\n        '200': {description: OK}\n",
    );
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info: {title: copy, version: 1.0.0}
actions:
  - target: '$.paths["/some-items"]'
    copy: '$.paths["/items"]'
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let applied = apply(&overlay, target.root()).unwrap();
    let out = applied.output.to_json();
    assert!(out.contains(r#""/some-items":{"delete""#), "{out}");
    assert!(out.contains(r#""get":{"responses""#), "{out}");
}

#[test]
fn copy_source_must_resolve() {
    let target = parse_yaml("paths: {}\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info: {title: bad, version: 1.0.0}
actions:
  - target: '$.paths'
    copy: '$.paths["/missing"]'
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let err = apply(&overlay, target.root()).unwrap_err();
    assert!(matches!(
        err,
        suspect_overlay::OverlayError::CopySourceUnresolved { .. }
    ));
}

#[test]
fn array_update_concatenates_instead_of_nesting() {
    // Overlay 1.1 §4.4.3: "An array value of the update or copy property
    // is concatenated with an array value of the target property."
    let target = parse_yaml("tags: [a, b]\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info: {title: concat, version: 1.0.0}
actions:
  - target: $.tags
    update: [c]
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let applied = apply(&overlay, target.root()).unwrap();
    assert_eq!(applied.output.to_json(), r#"{"tags":["a","b","c"]}"#);
}

#[test]
fn merge_property_arrays_concatenate() {
    // Overlay 1.1 §4.4.3: inside a recursive merge, an array-valued
    // property of update concatenates with the target's array.
    let target = parse_yaml("info:\n  tags: [a]\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info: {title: m, version: 1.0.0}
actions:
  - target: $.info
    update:
      tags: [b]
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let applied = apply(&overlay, target.root()).unwrap();
    assert_eq!(applied.output.to_json(), r#"{"info":{"tags":["a","b"]}}"#);
}

#[test]
fn explain_reports_per_action_deltas() {
    let target = parse_yaml("info:\n  title: A\n  version: 1\n");
    let overlay_doc = parse_yaml(
        r#"
overlay: 1.1.0
info: {title: e, version: 1.0.0}
actions:
  - target: $.info.title
    update: B
  - target: $.info.missing
    update: x
  - target: $.info.version
    remove: true
"#,
    );
    let overlay = OverlayDoc::parse(&overlay_doc).unwrap();
    let steps = explain(&overlay, target.root()).unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].kind, "update");
    assert_eq!(steps[0].matches, 1);
    assert_eq!(steps[0].before.as_deref(), Some(r#""A""#));
    assert_eq!(steps[0].after.as_deref(), Some(r#""B""#));
    assert_eq!(steps[1].matches, 0, "zero-match actions are legal");
    assert_eq!(steps[2].kind, "remove");
    assert_eq!(
        steps[2].after.as_deref(),
        None,
        "removed nodes have no after"
    );
}

#[test]
fn synthesized_overlay_round_trips_old_into_new() {
    let old = parse_yaml(
        r#"
openapi: 3.1.0
info: {title: API, version: '1.0.0', description: old}
paths:
  /pets:
    get:
      summary: List pets
      deprecated: true
  /gone:
    get: {summary: vanishing}
  /kept:
    get: {summary: stays}
components:
  schemas:
    Pet:
      type: object
      properties: {name: {type: string}, kind: {type: string}}
    Old:
      type: string
"#,
    );
    let new = parse_yaml(
        r#"
openapi: 3.1.0
info: {title: API, version: '1.0.0', description: new description}
paths:
  /pets:
    get:
      summary: List pets
      deprecated: false
      parameters: [{name: limit, in: query, schema: {type: integer}}]
  /kept:
    get: {summary: stays}
  /added:
    post: {summary: fresh}
components:
  schemas:
    Pet:
      type: object
      properties: {name: {type: string}, weight: {type: number}}
"#,
    );
    let overlay = synthesize_overlay(old.root(), new.root(), "evolve API");
    // The synthesized document is itself a valid overlay.
    let overlay_doc = parse_yaml(&overlay.to_yaml());
    let parsed = OverlayDoc::parse(&overlay_doc).unwrap();
    assert!(parsed.version() == Some("1.1.0"));

    // Applying it to old must produce new (modulo key ordering).
    let applied = apply(&parsed, old.root()).unwrap();
    let result: serde_json::Value = serde_json::from_str(&applied.output.to_json()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&new_owned(new.root()).to_json()).unwrap();
    assert_eq!(
        result,
        expected,
        "round trip must converge: {}",
        applied.output.to_json()
    );
}

fn new_owned(root: suspect_low::NodeRef<'_>) -> suspect_overlay::Value {
    suspect_overlay::Value::from_node(root)
}

#[test]
fn synthesized_overlay_reports_action_kinds() {
    let old = parse_yaml("info: {title: A, note: drop}\npaths: {}\n");
    let new = parse_yaml("info: {title: B}\npaths: {}\nextra: 1\n");
    let overlay = synthesize_overlay(old.root(), new.root(), "d");
    let yaml = overlay.to_yaml();
    assert!(yaml.contains("overlay: 1.1.0"), "{yaml}");
    assert!(yaml.contains("remove: true"), "{yaml}");
    assert!(yaml.contains("title: B"), "{yaml}");
}

#[test]
fn a_fragment_reference_stays_quoted_in_yaml_output() {
    // Regression, found against a real 63k-line specification: a value
    // beginning with `#` was emitted unquoted, and in YAML a `#` preceded
    // by a space starts a comment. `$ref: #/components/headers/X` therefore
    // round-tripped to a null value, silently corrupting every component
    // reference in the published document.
    let tree = suspect_overlay::Value::Object(vec![(
        "ref".into(),
        suspect_overlay::Value::Str("#/components/headers/X-Thing".into()),
    )]);
    let yaml = tree.to_yaml();
    assert!(
        yaml.contains("$ref: \"#/components/headers/X-Thing\"")
            || yaml.contains("ref: \"#/components/headers/X-Thing\""),
        "a fragment reference must be quoted: {yaml}"
    );
    // And it must survive a YAML parse with its value intact.
    let doc = suspect_low::LowDoc::parse(
        "mem://quoting.yaml".into(),
        suspect_source::Source::from_vec(yaml.clone().into_bytes()),
    );
    assert!(
        doc.syntax_errors().is_empty(),
        "the emitted YAML must parse"
    );
    assert_eq!(
        doc.root().get("ref").and_then(|node| node.as_str()),
        Some("#/components/headers/X-Thing"),
        "the value must survive the round trip: {yaml}"
    );
}

#[test]
fn values_that_yaml_would_misread_are_quoted() {
    for (input, why) in [
        (
            "#/components/schemas/Pet",
            "a space-preceded # is a comment",
        ),
        ("value: with colon", "a colon-space ends the scalar"),
        ("a # b", "an embedded comment marker"),
        ("*anchor", "an alias indicator"),
        ("&anchor", "an anchor indicator"),
        ("- item", "a sequence indicator"),
        ("? key", "a mapping indicator"),
        ("|", "a literal block indicator"),
        (">", "a folded block indicator"),
        ("!tag", "a tag indicator"),
        ("%directive", "a directive indicator"),
        ("@text", "a reserved indicator"),
        ("`tick", "a reserved indicator"),
        ("''", "quotes need care"),
    ] {
        let tree = suspect_overlay::Value::Str(input.into());
        let yaml = tree.to_yaml();
        let doc = suspect_low::LowDoc::parse(
            "mem://quoting.yaml".into(),
            suspect_source::Source::from_vec(yaml.clone().into_bytes()),
        );
        assert!(
            doc.syntax_errors().is_empty(),
            "{why}: {input:?} emitted {yaml:?}"
        );
        assert_eq!(
            doc.root().as_str(),
            Some(input),
            "{why}: {input:?} emitted {yaml:?} and did not round trip"
        );
    }
}
