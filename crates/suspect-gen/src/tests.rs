//! Tests for `suspect-gen`: filters, engines, manifest parsing, and
//! preservation-aware output orchestration.

use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::orchestrate::{
    BEGIN_MARK, END_MARK, Manifest, OutputRule, WriteReason, parse_manifest, render_manifest,
};
use crate::{FilterRegistry, MinijinjaEngine, TemplateEngine};

/// Builds a sandboxed engine with the built-in filters registered.
fn engine_with(templates: &[(&str, &str)]) -> MinijinjaEngine {
    let mut engine = MinijinjaEngine::new();
    for (name, src) in templates {
        engine.add_template(name, src).expect("template compiles");
    }
    FilterRegistry::register(&mut engine);
    engine
}

/// Creates a unique scratch directory under the OS temp dir.
fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("suspect-gen-test-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir created");
    dir
}

// ------------------------------------------------------------- case maps

#[test]
fn case_conversions_round_trip() {
    assert_eq!(crate::filters::to_snake_case("petStoreId"), "pet_store_id");
    assert_eq!(crate::filters::to_camel_case("pet_store_id"), "petStoreId");
    assert_eq!(crate::filters::to_pascal_case("pet_store_id"), "PetStoreId");
    assert_eq!(crate::filters::to_kebab_case("PetStoreId"), "pet-store-id");
    assert_eq!(
        crate::filters::to_constant_case("pet-store-id"),
        "PET_STORE_ID"
    );

    // Round trip: snake -> camel -> snake is stable.
    let snake = "http_server_v2";
    let camel = crate::filters::to_camel_case(snake);
    assert_eq!(camel, "httpServerV2");
    assert_eq!(crate::filters::to_snake_case(&camel), snake);

    // Acronyms split on the lower boundary.
    assert_eq!(
        crate::filters::to_snake_case("parseHTTPResponse"),
        "parse_http_response"
    );
}

// ------------------------------------------------------------ type maps

#[test]
fn ts_and_rust_type_mappings() {
    let string = json!({"type": "string"});
    let int = json!({"type": "integer"});
    let num = json!({"type": "number"});
    let boolean = json!({"type": "boolean"});
    let array = json!({"type": "array", "items": {"type": "integer"}});
    let r#ref = json!({"$ref": "#/components/schemas/Pet"});
    let object = json!({"type": "object", "properties": {"a": {"type": "string"}}});

    assert_eq!(crate::ts_type(&string), "string");
    assert_eq!(crate::ts_type(&int), "number");
    assert_eq!(crate::ts_type(&num), "number");
    assert_eq!(crate::ts_type(&boolean), "boolean");
    assert_eq!(crate::ts_type(&array), "number[]");
    assert_eq!(crate::ts_type(&r#ref), "Pet");
    assert_eq!(crate::ts_type(&object), "Record<string, unknown>");

    // Nullable via type array and via nullable flag.
    assert_eq!(
        crate::ts_type(&json!({"type": ["string", "null"]})),
        "string | null"
    );
    assert_eq!(
        crate::ts_type(&json!({"type": "string", "nullable": true})),
        "string | null"
    );

    // Rust mirrors the same schema set; no Option wrapping without
    // requiredness context.
    assert_eq!(crate::rust_type(&string), "String");
    assert_eq!(crate::rust_type(&int), "i64");
    assert_eq!(crate::rust_type(&num), "f64");
    assert_eq!(crate::rust_type(&boolean), "bool");
    assert_eq!(crate::rust_type(&array), "Vec<i64>");
    assert_eq!(crate::rust_type(&r#ref), "Pet");
    assert_eq!(crate::rust_type(&object), "serde_json::Value");
    assert_eq!(
        crate::rust_type(&json!({"type": ["string", "null"]})),
        "String"
    );
}

// -------------------------------------------------------------- examples

#[test]
fn example_of_is_deterministic() {
    let refs = json!({
        "Address": {"type": "object", "properties": {"city": {"type": "string", "minLength": 4}}}
    })
    .to_string();
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {"type": "integer"},
            "name": {"type": "string", "minLength": 3},
            "kind": {"enum": ["cat", "dog"]},
            "alive": {"type": "boolean"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "home": {"$ref": "#/components/schemas/Address"},
            "nick": {"default": "spot"}
        }
    })
    .to_string();

    let first = crate::example_of(&schema, &refs);
    let second = crate::example_of(&schema, &refs);
    assert_eq!(first, second);

    let value: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(value["id"], 0);
    // minLength honored by padding 'a'.
    assert_eq!(value["name"], json!("aaa"));
    assert_eq!(value["home"]["city"], json!("aaaa"));
    // enum[0] beats type default.
    assert_eq!(value["kind"], json!("cat"));
    assert_eq!(value["alive"], json!(false));
    // arrays synthesize exactly one element.
    assert_eq!(value["tags"], json!([""]));
    // default keyword wins over "".
    assert_eq!(value["nick"], json!("spot"));
}

// ---------------------------------------------------------------- engine

#[test]
fn minijinja_render_with_registered_filter() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("greet", "Hello {{ name | PascalCase }}!")
        .unwrap();
    FilterRegistry::register(&mut engine);

    let out = engine
        .render("greet", &json!({ "name": "pet_store" }))
        .unwrap();
    assert_eq!(out, "Hello PetStore!");

    // Unknown template errors cleanly.
    assert!(engine.render("missing", &json!({})).is_err());
}

// -------------------------------------------------------------- manifest

#[test]
fn manifest_parse_two_outputs() {
    let text = r#"
# generation manifest
[[output]]
template = "models.ts.j2"
target = "src/gen/models.ts"

[[output]]
template = "client.rs.j2"
target = "src/gen/{{ package }}_client.rs"

[other_table]
ignored = true
"#;
    let manifest = parse_manifest(text).unwrap();
    assert_eq!(
        manifest.outputs,
        vec![
            OutputRule {
                template: "models.ts.j2".into(),
                target: "src/gen/models.ts".into(),
            },
            OutputRule {
                template: "client.rs.j2".into(),
                target: "src/gen/{{ package }}_client.rs".into(),
            },
        ]
    );
    assert!(parse_manifest("[[output]]\ntemplate = \"x.j2\"").is_err());
}

#[test]
fn render_manifest_created_changed_unchanged() {
    let dir = scratch_dir("created-changed-unchanged");
    let engine = engine_with(&[("m", "v={{ version }}")]);
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "m".into(),
            target: "out/a.txt".into(),
        }],
    };
    let ctx1 = json!({ "version": 1 });
    let ctx2 = json!({ "version": 2 });

    // Run 1: file does not exist -> Created and written.
    let outcomes = render_manifest(&engine, &manifest, &ctx1, &dir, false).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::Created);
    assert!(outcomes[0].wrote);
    assert_eq!(fs::read_to_string(dir.join("out/a.txt")).unwrap(), "v=1");

    // Run 2: identical content -> Unchanged, not rewritten.
    let outcomes = render_manifest(&engine, &manifest, &ctx1, &dir, false).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::Unchanged);
    assert!(!outcomes[0].wrote);

    // Run 3: different context -> Changed.
    let outcomes = render_manifest(&engine, &manifest, &ctx2, &dir, false).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::Changed);
    assert!(outcomes[0].wrote);
    assert_eq!(fs::read_to_string(dir.join("out/a.txt")).unwrap(), "v=2");

    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- preserve regions

#[test]
fn preserve_region_splice_keeps_user_code() {
    let old = format!(
        "// header\n{BEGIN}\ncustom_user_line();\n{END}\n// footer\n",
        BEGIN = BEGIN_MARK,
        END = END_MARK
    );
    let new = format!(
        "// new header\n{BEGIN}\nauto_generated();\n{END}\n// new footer\n",
        BEGIN = BEGIN_MARK,
        END = END_MARK
    );
    let (spliced, count) = crate::orchestrate::splice_preserved_regions(&old, &new);
    assert_eq!(count, 1);
    assert!(spliced.contains("custom_user_line();"));
    assert!(!spliced.contains("auto_generated();"));
    assert!(spliced.starts_with("// new header\n"));
    assert!(spliced.ends_with("// new footer\n"));

    // Indented markers keep their prefix line intact.
    let indented_old = format!(
        "    // {BEGIN}\n    keep_me();\n    // {END}\n",
        BEGIN = BEGIN_MARK,
        END = END_MARK
    );
    let (spliced, count) = crate::orchestrate::splice_preserved_regions(&indented_old, &new);
    assert_eq!(count, 1);
    assert!(spliced.contains("keep_me();"));
}

#[test]
fn render_manifest_applies_preserved_regions() {
    let dir = scratch_dir("preserved-reason");
    let tpl = format!("gen\n{BEGIN_MARK}\nplaceholder\n{END_MARK}\ntail");
    let engine = engine_with(&[("m", tpl.as_str())]);
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "m".into(),
            target: "b.txt".into(),
        }],
    };
    fs::write(
        dir.join("b.txt"),
        format!("{BEGIN_MARK}\nUSER CODE\n{END_MARK}\nold tail"),
    )
    .unwrap();

    let outcomes = render_manifest(&engine, &manifest, &json!({}), &dir, false).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::PreservedRegionsApplied);
    assert!(outcomes[0].wrote);
    let written = fs::read_to_string(dir.join("b.txt")).unwrap();
    assert!(written.contains("USER CODE"));
    assert!(!written.contains("placeholder"));

    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ diff

#[test]
fn diff_only_produces_hunks_and_writes_nothing() {
    let dir = scratch_dir("diff-only");
    let engine = engine_with(&[("m", "alpha\nbeta\ngamma")]);
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "m".into(),
            target: "d.txt".into(),
        }],
    };

    // Created target in diff-only mode: diff present, nothing on disk.
    let outcomes = render_manifest(&engine, &manifest, &json!({}), &dir, true).unwrap();
    assert!(matches!(
        outcomes[0],
        crate::RenderOutcome { wrote: false, .. }
    ));
    assert_eq!(outcomes[0].reason, WriteReason::Created);
    let diff = outcomes[0].diff.as_deref().unwrap();
    assert!(diff.contains("@@ -0,0 +1,3 @@"));
    assert!(diff.contains("+gamma"));
    assert!(!dir.join("d.txt").exists());

    // Existing divergent file: hunk shows removals and additions, still no write.
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("d.txt"), "alpha\ndelta\ngamma\n").unwrap();
    let outcomes = render_manifest(&engine, &manifest, &json!({}), &dir, true).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::Changed);
    assert!(!outcomes[0].wrote);
    let diff = outcomes[0].diff.as_deref().unwrap();
    assert!(diff.contains("-delta"));
    assert!(diff.contains("+beta"));
    assert_eq!(
        fs::read_to_string(dir.join("d.txt")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );

    // Identical content: no diff at all.
    fs::write(dir.join("d.txt"), "alpha\nbeta\ngamma").unwrap();
    let outcomes = render_manifest(&engine, &manifest, &json!({}), &dir, true).unwrap();
    assert_eq!(outcomes[0].reason, WriteReason::Unchanged);
    assert_eq!(outcomes[0].diff, None);

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- mermaid

#[test]
fn mermaid_refs_reuses_edges_and_walks_fallback() {
    // Edges list reused verbatim when present.
    let spec = json!({
        "title": "demo",
        "schema_edges": { "PetList": ["Pet"], "Pet": ["Category"] }
    });
    let mermaid = crate::mermaid_refs(&spec.to_string());
    assert!(mermaid.starts_with("flowchart TD\n"));
    assert!(mermaid.contains("PetList --> Pet"));
    assert!(mermaid.contains("Pet --> Category"));

    // Fallback: walk $refs inside schemas when edges are absent.
    let spec = json!({
        "schemas": [
            {"name": "PetList", "json": {"type": "array", "items": {"$ref": "#/components/schemas/Pet"}}},
            {"name": "Pet", "json": {"$ref": "#/components/schemas/Pet"}}
        ]
    });
    let mermaid = crate::mermaid_refs(&spec.to_string());
    assert!(mermaid.contains("PetList --> Pet"));
    assert!(!mermaid.contains("Pet --> Pet")); // self edges dropped
}
