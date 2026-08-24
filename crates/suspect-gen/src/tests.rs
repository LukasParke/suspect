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

    // Rust mirrors the same schema set. Schema-level nullability wraps
    // the base type (and recurses into array items) in `Option`.
    assert_eq!(crate::rust_type(&string), "String");
    assert_eq!(crate::rust_type(&int), "i64");
    assert_eq!(crate::rust_type(&num), "f64");
    assert_eq!(crate::rust_type(&boolean), "bool");
    assert_eq!(crate::rust_type(&array), "Vec<i64>");
    assert_eq!(crate::rust_type(&r#ref), "Pet");
    assert_eq!(crate::rust_type(&object), "serde_json::Value");
    assert_eq!(
        crate::rust_type(&json!({"type": ["string", "null"]})),
        "Option<String>"
    );
}

#[test]
fn ts_type_parenthesizes_nullable_array_items() {
    // Type-array nullability in items must not leak `| null` into a
    // suffix position: `(string | null)[]`, never `string | null[]`.
    assert_eq!(
        crate::ts_type(&json!({
            "type": "array",
            "items": {"type": ["string", "null"]}
        })),
        "(string | null)[]"
    );

    // OpenAPI 3.0 sibling-nullability form on items behaves the same.
    assert_eq!(
        crate::ts_type(&json!({
            "type": "array",
            "items": {"type": "integer", "nullable": true}
        })),
        "(number | null)[]"
    );

    // Non-nullable items stay unparenthesized.
    assert_eq!(
        crate::ts_type(&json!({"type": "array", "items": {"type": "string"}})),
        "string[]"
    );
}

#[test]
fn rust_type_wraps_schema_level_nullability() {
    assert_eq!(
        crate::rust_type(&json!({"type": "string", "nullable": true})),
        "Option<String>"
    );
    assert_eq!(
        crate::rust_type(&json!({"type": ["integer", "null"]})),
        "Option<i64>"
    );
    // Nullable items recurse into Vec<Option<T>>.
    assert_eq!(
        crate::rust_type(&json!({
            "type": "array",
            "items": {"type": "integer", "nullable": true}
        })),
        "Vec<Option<i64>>"
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

#[test]
fn example_of_deep_ref_chain_yields_target_not_null() {
    // A chain of nine indirections exhausts the resolver bound; the
    // final target schema must still drive the example (padded "aa"),
    // instead of the whole node collapsing to null.
    let mut refs = serde_json::Map::new();
    for i in 1..=8 {
        refs.insert(
            format!("L{i}"),
            json!({"$ref": format!("#/components/schemas/L{}", i + 1)}),
        );
    }
    refs.insert("L9".into(), json!({"type": "string", "minLength": 2}));

    let out = crate::example_of(
        r##"{"$ref": "#/components/schemas/L1"}"##,
        &serde_json::Value::Object(refs).to_string(),
    );
    assert_eq!(out, r#""aa""#);
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
fn manifest_quoted_values_keep_hashes_and_backslashes() {
    // A '#' inside quotes is data, not an inline comment.
    let manifest =
        parse_manifest("[[output]]\ntemplate = \"t.j2\"\ntarget = \"gen #core.rs\"\n").unwrap();
    assert_eq!(manifest.outputs[0].target, "gen #core.rs");

    // Unquoted values still strip a whitespace-preceded '# comment'.
    let manifest = parse_manifest("[[output]]\ntemplate = t.j2\ntarget = gen.rs #core\n").unwrap();
    assert_eq!(manifest.outputs[0].target, "gen.rs");

    // Double-quoted backslash escapes are honored.
    let manifest = parse_manifest(
        "[[output]]\ntemplate = \"t.j2\"\ntarget = \"C:\\\\tools\\\\gen.rs\" # windows\n",
    )
    .unwrap();
    assert_eq!(manifest.outputs[0].target, r"C:\tools\gen.rs");

    // Single-quoted strings keep backslashes verbatim.
    let manifest =
        parse_manifest("[[output]]\ntemplate = \"t.j2\"\ntarget = 'C:\\tools\\gen.rs'\n").unwrap();
    assert_eq!(manifest.outputs[0].target, r"C:\tools\gen.rs");

    // A trailing comment after a quoted value is stripped.
    let manifest =
        parse_manifest("[[output]]\ntemplate = \"t.j2\"\ntarget = \"gen.rs\" # core outputs\n")
            .unwrap();
    assert_eq!(manifest.outputs[0].target, "gen.rs");
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

#[test]
fn render_manifest_rejects_targets_outside_out_root() {
    let dir = scratch_dir("target-traversal");
    let engine = engine_with(&[("m", "x")]);
    let make = |target: &str| Manifest {
        outputs: vec![OutputRule {
            template: "m".into(),
            target: target.into(),
        }],
    };
    let ctx = json!({});

    // Absolute targets are rejected outright.
    assert!(render_manifest(&engine, &make("/etc/passwd"), &ctx, &dir, false).is_err());

    // Relative escapes are rejected after lexical normalization.
    assert!(render_manifest(&engine, &make("../escape.txt"), &ctx, &dir, false).is_err());
    assert!(render_manifest(&engine, &make("a/../../escape.txt"), &ctx, &dir, false).is_err());

    // Nothing was written outside the root.
    let escape_path = dir.parent().unwrap().join("escape.txt");
    let _ = fs::remove_file(&escape_path);

    // A legitimate nested target still renders.
    let outcomes = render_manifest(&engine, &make("nested/deep/ok.txt"), &ctx, &dir, false)
        .expect("nested target is inside the root");
    assert_eq!(outcomes[0].reason, WriteReason::Created);
    assert_eq!(
        fs::read_to_string(dir.join("nested/deep/ok.txt")).unwrap(),
        "x"
    );

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
    let (spliced, count) = crate::orchestrate::splice_preserved_regions(&old, &new).unwrap();
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
    let (spliced, count) =
        crate::orchestrate::splice_preserved_regions(&indented_old, &new).unwrap();
    assert_eq!(count, 1);
    assert!(spliced.contains("keep_me();"));
}

#[test]
fn splice_ignores_lines_that_mention_markers() {
    // A user line that merely mentions the end marker must NOT close
    // the region; only a whole-line marker does.
    let old = format!(
        "{BEGIN}\ncustom_one();\n// see {END_MARK} documentation\nmore_user_code();\n{END}\n",
        BEGIN = BEGIN_MARK,
        END = END_MARK
    );
    let new = format!(
        "{BEGIN}\nauto_generated();\n{END}\n",
        BEGIN = BEGIN_MARK,
        END = END_MARK
    );
    let (spliced, count) = crate::orchestrate::splice_preserved_regions(&old, &new).unwrap();
    assert_eq!(count, 1);
    assert!(spliced.contains("custom_one();"));
    assert!(spliced.contains("more_user_code();"));
    assert!(!spliced.contains("auto_generated();"));
}

#[test]
fn splice_rejects_malformed_markers_naming_the_line() {
    let begin = BEGIN_MARK;
    let end = END_MARK;

    // END without an open region.
    let err =
        crate::orchestrate::splice_preserved_regions(&format!("a\n{end}\nb\n"), "").unwrap_err();
    assert!(err.0.contains("line 2"), "error names the line: {}", err.0);

    // BEGIN while a region is already open.
    let err = crate::orchestrate::splice_preserved_regions(&format!("{begin}\nx\n{begin}\n"), "")
        .unwrap_err();
    assert!(err.0.contains("line 3"), "error names the line: {}", err.0);

    // BEGIN never closed by end of input.
    let err = crate::orchestrate::splice_preserved_regions(&format!("h\n{begin}\nuser();\n"), "")
        .unwrap_err();
    assert!(err.0.contains("line 2"), "error names the line: {}", err.0);

    // Well-formed markers (comment-prefixed, indented) still parse.
    let ok = format!("    // {begin}\ncode();\n  # {end}\n");
    let fresh = format!("    // {begin}\nfresh();\n  # {end}\n");
    let (spliced, count) = crate::orchestrate::splice_preserved_regions(&ok, &fresh).unwrap();
    assert_eq!(count, 1);
    assert!(spliced.contains("code();"));
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

#[test]
fn unified_diff_merged_hunks_keep_trailing_context() {
    let old: Vec<String> = (1..=12).map(|i| format!("l{i}")).collect();
    let mut new = old.clone();
    new[2] = "X".into();
    new[5] = "Y".into();
    let diff = crate::orchestrate::unified_diff(
        &format!("{}\n", old.join("\n")),
        &format!("{}\n", new.join("\n")),
    );

    // Both changes land in one hunk (one header line; "@@" appears
    // twice within a single header, so count header lines instead).
    let headers = diff.lines().filter(|line| line.starts_with("@@ ")).count();
    assert_eq!(headers, 1);

    // ...whose trailing context survives the merge (context after the
    // second change must include l7/l8, not stop at the last edit).
    assert!(
        diff.contains("\n l7\n"),
        "missing trailing context:\n{diff}"
    );
    assert!(
        diff.contains("\n l8\n"),
        "missing trailing context:\n{diff}"
    );
}

#[test]
fn unified_diff_hunk_starting_on_insert_numbers_old_side() {
    // The hunk begins with an inserted line that has no old-side
    // position; the old start must come from the next positioned line
    // (`a` is old line 1), not the degenerate `-0,N`.
    let diff = crate::orchestrate::unified_diff("a\n", "X\na\n");
    assert!(
        diff.starts_with("@@ -1,1 +1,2 @@"),
        "wrong hunk header: {diff}"
    );
    assert!(diff.contains("+X"));
    assert!(diff.contains(" a\n"));
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
// ---------------------------------------------------------------- presets

use suspect_ir::{IrOperation, IrParameter, IrResponse, IrSchema, IrSpec, Method, ParamIn};

use crate::presets;

/// Builds a petstore-like spec mirroring the `suspect-ir` SPEC fixture.
fn petstore_spec() -> IrSpec {
    let mut spec = IrSpec {
        title: "Pets".into(),
        version: "2.1".into(),
        servers: vec!["https://api.example.com/v1".into()],
        ..IrSpec::default()
    };
    let pet_json = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type": "string"},
            "tag": {"type": "string"},
            "friend": {"$ref": "#/components/schemas/Pet"}
        }
    });
    spec.operations.push(IrOperation {
        id: Some("listPets".into()),
        method: Method::Get,
        path: "/pets".into(),
        summary: Some("list all pets".into()),
        description: None,
        tags: vec!["pets".into()],
        deprecated: false,
        parameters: vec![IrParameter {
            name: "limit".into(),
            location: ParamIn::Query,
            required: false,
            schema: Some(json!({"type": "integer"})),
        }],
        body_schema: None,
        responses: vec![
            IrResponse {
                status: Some(200),
                description: Some("page of pets".into()),
                schema: Some("PetList".into()),
            },
            IrResponse {
                status: None,
                description: Some("error".into()),
                schema: None,
            },
        ],
    });
    spec.operations.push(IrOperation {
        id: Some("createPet".into()),
        method: Method::Post,
        path: "/pets".into(),
        summary: Some("create a pet".into()),
        description: None,
        tags: vec!["pets".into()],
        deprecated: false,
        parameters: vec![],
        body_schema: Some("Pet".into()),
        responses: vec![IrResponse {
            status: Some(201),
            description: Some("created".into()),
            schema: None,
        }],
    });
    spec.operations.push(IrOperation {
        id: Some("showPetById".into()),
        method: Method::Get,
        path: "/pets/{petId}".into(),
        summary: Some("one pet".into()),
        description: None,
        tags: vec!["pets".into()],
        deprecated: false,
        parameters: vec![IrParameter {
            name: "petId".into(),
            location: ParamIn::Path,
            required: true,
            schema: Some(json!({"type": "string"})),
        }],
        body_schema: None,
        responses: vec![IrResponse {
            status: Some(200),
            description: Some("one pet".into()),
            schema: Some("Pet".into()),
        }],
    });
    spec.schemas.push(IrSchema {
        name: "Pet".into(),
        json: pet_json,
    });
    spec.schemas.push(IrSchema {
        name: "PetList".into(),
        json: json!({"type": "array", "items": {"$ref": "#/components/schemas/Pet"}}),
    });
    spec
}

/// Installs a preset's templates (with filters) into a fresh engine.
fn preset_engine(name: &str) -> MinijinjaEngine {
    let preset = presets::get(name).expect("preset exists");
    engine_with(preset.templates)
}

/// Renders every manifest output of `name` under `dir`.
fn render_preset_into(
    name: &str,
    dir: &std::path::Path,
    spec: &IrSpec,
) -> Vec<crate::orchestrate::RenderOutcome> {
    let preset = presets::get(name).expect("preset exists");
    let engine = preset_engine(name);
    let manifest = parse_manifest(preset.manifest_toml).expect("preset manifest parses");
    let ctx = (preset.ctx_builder)(spec);
    render_manifest(&engine, &manifest, &ctx, dir, false).expect("preset renders")
}

/// Counts opening and closing braces of `text`.
fn brace_balance(text: &str) -> (usize, usize) {
    (
        text.chars().filter(|c| *c == '{').count(),
        text.chars().filter(|c| *c == '}').count(),
    )
}

#[test]
fn presets_lookup_all_three_and_unknown_is_none() {
    for name in ["docs-md", "ts-sdk", "rust-sdk"] {
        let preset = presets::get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert!(!preset.templates.is_empty(), "{name} must bundle templates");
        // Every bundled template compiles with filters registered.
        let _engine = preset_engine(name);
        assert!(
            parse_manifest(preset.manifest_toml).is_ok(),
            "{name} manifest parses"
        );
    }
    assert!(presets::get("nope").is_none());
    assert!(presets::get("").is_none());
}

#[test]
fn docs_md_renders_petstore_docs() {
    let spec = petstore_spec();
    let engine = preset_engine("docs-md");
    let ctx = presets::base_context(&spec);

    // Context augmentations required by the contract.
    assert_eq!(ctx["base_url"], "https://api.example.com/v1");
    assert_eq!(ctx["operations_by_tag"][0]["tag"], "pets");
    assert_eq!(
        ctx["operations_by_tag"][0]["operations"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(ctx["schema_names"][0], "Pet");

    let index = engine.render("docs-md/index.md.j2", &ctx).unwrap();
    assert!(index.contains("# Pets 2.1"), "title heading present");
    assert!(
        index.contains("https://api.example.com/v1"),
        "base url present"
    );
    for id in ["listPets", "createPet", "showPetById"] {
        assert!(
            index.contains(&format!("— {id}\n")) || index.contains(&format!("— {id}")),
            "operation id {id} appears in a section heading"
        );
    }
    assert!(index.contains("`GET /pets`"), "method+path headings");
    assert!(index.contains("| limit | query | no |"), "parameters table");
    assert!(
        index.contains("**Request body:** `Pet`"),
        "request schema name"
    );
    assert!(
        index.contains("`200` — page of pets → `PetList`"),
        "response schemas"
    );

    let schemas = engine.render("docs-md/schema.md.j2", &ctx).unwrap();
    assert!(schemas.contains("## Pet\n"), "Pet section");
    for prop in ["name", "tag", "friend"] {
        assert!(schemas.contains(prop), "Pet property {prop} documented");
    }
    assert!(schemas.contains("## PetList\n"), "PetList section");
}

#[test]
fn ts_sdk_renders_client_models_deterministically() {
    let spec = petstore_spec();
    let engine = preset_engine("ts-sdk");
    let ctx = (presets::get("ts-sdk").unwrap().ctx_builder)(&spec);

    let client = engine.render("ts-sdk/client.ts.j2", &ctx).unwrap();
    let models = engine.render("ts-sdk/models.ts.j2", &ctx).unwrap();

    assert!(client.contains("class ApiClient"));
    for op in ["listPets", "createPet", "showPetById"] {
        assert!(
            client.contains(&format!("async {op}(")),
            "one method per operation ({op})"
        );
    }
    assert!(
        client.contains("URLSearchParams"),
        "query params via URLSearchParams"
    );
    assert!(
        client.contains("JSON.stringify(body)"),
        "body serialized when body_schema"
    );
    assert!(
        client.contains("import type"),
        "models referenced from client"
    );
    assert!(models.contains("export interface Pet {"));
    assert!(models.contains("name: string;"));
    assert!(models.contains("tag?: string;"), "optional property marked");
    assert!(
        models.contains("friend?: Pet;"),
        "$ref field typed as class name"
    );
    assert!(
        models.contains("export type PetList = Pet[];"),
        "array schema as T[]"
    );

    // No unresolved Jinja leftovers in either output.
    for out in [&client, &models] {
        assert!(!out.contains("{{"), "unresolved '{{' leftover: {out}");
        let (open, close) = brace_balance(out);
        assert_eq!(open, close, "brace balance violated");
    }

    // Deterministic re-render.
    let client2 = engine.render("ts-sdk/client.ts.j2", &ctx).unwrap();
    let models2 = engine.render("ts-sdk/models.ts.j2", &ctx).unwrap();
    assert_eq!(client, client2);
    assert_eq!(models, models2);
}

#[test]
#[allow(clippy::too_many_lines)]
fn rust_sdk_renders_and_compiles_standalone() {
    let spec = petstore_spec();
    let engine = preset_engine("rust-sdk");
    let ctx = (presets::get("rust-sdk").unwrap().ctx_builder)(&spec);

    let lib_rs = engine.render("rust-sdk/lib.rs.j2", &ctx).unwrap();
    let models_rs = engine.render("rust-sdk/models.rs.j2", &ctx).unwrap();

    assert!(lib_rs.contains("pub struct Client"));
    assert!(lib_rs.contains("pub base_url: String"));
    assert!(lib_rs.contains("pub struct HttpRequest"));
    assert!(lib_rs.contains("pub headers: Vec<(String, String)>"));
    assert!(lib_rs.contains("pub body: Option<String>"));
    assert!(
        lib_rs.contains("include!(\"models.rs\");"),
        "single-crate include"
    );
    for op in ["list_pets", "create_pet", "show_pet_by_id"] {
        assert!(lib_rs.contains(&format!("pub fn {op}(")), "builder fn {op}");
    }
    assert!(
        lib_rs.contains(BEGIN_MARK) && lib_rs.contains(END_MARK),
        "user-code markers"
    );
    assert!(models_rs.contains("pub struct Pet {"));
    assert!(
        models_rs.contains("pub name: String,"),
        "required field plain"
    );
    assert!(
        models_rs.contains("pub tag: Option<String>,"),
        "optional field Option<T>"
    );
    assert!(
        models_rs.contains("pub friend: Option<Box<Pet>>,"),
        "$ref via rust_type boxed"
    );
    assert!(
        models_rs.contains("pub type PetList = Vec<Pet>;"),
        "array schema as Vec<T>"
    );

    // Compile the generated SDK with a bare rustc invocation.
    let dir = scratch_dir("rust-sdk-compile");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        engine.render("rust-sdk/Cargo.toml.j2", &ctx).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("src/lib.rs"), &lib_rs).unwrap();
    fs::write(dir.join("src/models.rs"), &models_rs).unwrap();
    let output = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "-o",
            dir.join("lib.rlib").to_str().unwrap(),
            dir.join("src/lib.rs").to_str().unwrap(),
        ])
        .output()
        .expect("rustc runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && !stderr.contains("error"),
        "generated rust SDK failed to compile:\n{stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ts_sdk_preserved_user_code_survives_rerender() {
    let spec = petstore_spec();
    let dir = scratch_dir("ts-preserve");
    render_preset_into("ts-sdk", &dir, &spec);

    let client_path = dir.join("sdk/typescript/client.ts");
    let first = fs::read_to_string(&client_path).unwrap();
    // Simulate the user editing inside the preserved region.
    let edited = first.replace(
        "// suspect:begin:user-code",
        "// suspect:begin:user-code\ncustomUserHelper();",
    );
    assert_ne!(edited, first, "fresh client carries user-code markers");
    fs::write(&client_path, edited).unwrap();

    let outcomes = render_preset_into("ts-sdk", &dir, &spec);
    let client_outcome = outcomes
        .iter()
        .find(|o| o.path == client_path)
        .expect("client outcome reported");
    assert_eq!(client_outcome.reason, WriteReason::PreservedRegionsApplied);

    let rerendered = fs::read_to_string(&client_path).unwrap();
    assert!(
        rerendered.contains("customUserHelper();"),
        "user code survives regeneration verbatim"
    );
    assert!(
        rerendered.contains("class ApiClient"),
        "generated frame refreshed"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rust_sdk_preset_does_not_double_wrap_nullable_optionals() {
    // An optional property that is itself schema-level nullable must
    // render as a single `Option<String>`, never `Option<Option<..>>`.
    let mut spec = IrSpec {
        title: "Things".into(),
        version: "1.0".into(),
        ..IrSpec::default()
    };
    spec.schemas.push(IrSchema {
        name: "Thing".into(),
        json: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "integer"},
                "note": {"type": "string", "nullable": true}
            }
        }),
    });

    let engine = preset_engine("rust-sdk");
    let ctx = (presets::get("rust-sdk").unwrap().ctx_builder)(&spec);
    let models_rs = engine.render("rust-sdk/models.rs.j2", &ctx).unwrap();

    assert!(
        models_rs.contains("pub note: Option<String>,"),
        "nullable optional renders one Option:\n{models_rs}"
    );
    assert!(
        !models_rs.contains("Option<Option<"),
        "no double wrap:\n{models_rs}"
    );
    assert!(
        models_rs.contains("pub id: i64,"),
        "required non-nullable stays plain"
    );

    // Optional query parameters with nullable schemas stay single-wrapped.
    spec.operations.push(IrOperation {
        id: Some("listThings".into()),
        method: Method::Get,
        path: "/things".into(),
        summary: None,
        description: None,
        tags: vec![],
        deprecated: false,
        parameters: vec![IrParameter {
            name: "label".into(),
            location: ParamIn::Query,
            required: false,
            schema: Some(json!({"type": "string", "nullable": true})),
        }],
        body_schema: None,
        responses: vec![],
    });
    let ctx = (presets::get("rust-sdk").unwrap().ctx_builder)(&spec);
    let lib_rs = engine.render("rust-sdk/lib.rs.j2", &ctx).unwrap();
    assert!(
        lib_rs.contains("label: Option<String>") || lib_rs.contains("Option<String> label"),
        "query param single-wrapped:\n{lib_rs}"
    );
    assert!(!lib_rs.contains("Option<Option<"));
}

// --------------------------------------------------- docs-md render speed

/// Builds a synthetic spec with `ops` operations (10 tags, 5 parameters,
/// 3 responses each) and one schema per operation with 14 properties.
///
/// Deterministic: property names, types, and refs derive from indices so
/// the throughput fixture needs no corpus file.
fn synthetic_spec(ops: usize) -> IrSpec {
    let mut spec = IrSpec {
        title: "Synthetic".into(),
        version: "1.0".into(),
        servers: vec!["https://synthetic.invalid".into()],
        ..IrSpec::default()
    };
    for i in 0..ops {
        let schema_name = format!("Model{i}");
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for p in 0..14 {
            let prop = match p % 4 {
                0 => {
                    required.push(format!("field{p}"));
                    json!({"type": "string", "minLength": 2})
                }
                1 => json!({"type": "integer", "default": p}),
                2 => json!({"type": "array", "items": {"type": "boolean"}}),
                _ => json!({"$ref": format!("#/components/schemas/Model{}", (i + 1) % ops)}),
            };
            properties.insert(format!("field{p}"), prop);
        }
        spec.schemas.push(IrSchema {
            name: schema_name.clone(),
            json: json!({"type": "object", "required": required, "properties": properties}),
        });
        spec.operations.push(IrOperation {
            id: Some(format!("op{i}")),
            method: Method::Get,
            path: format!("/v1/resource{i}/{{id}}"),
            summary: Some(format!("Synthetic operation {i}")),
            description: None,
            tags: vec![format!("tag{}", i % 10)],
            deprecated: i % 25 == 0,
            parameters: vec![
                IrParameter {
                    name: "id".into(),
                    location: ParamIn::Path,
                    required: true,
                    schema: Some(json!({"type": "string"})),
                },
                IrParameter {
                    name: "limit".into(),
                    location: ParamIn::Query,
                    required: false,
                    schema: Some(json!({"type": "integer"})),
                },
                IrParameter {
                    name: "expand".into(),
                    location: ParamIn::Query,
                    required: false,
                    schema: None,
                },
                IrParameter {
                    name: "filter".into(),
                    location: ParamIn::Query,
                    required: false,
                    schema: Some(
                        json!({"$ref": format!("#/components/schemas/Model{}", (i + 7) % ops)}),
                    ),
                },
                IrParameter {
                    name: "X-Trace".into(),
                    location: ParamIn::Header,
                    required: false,
                    schema: Some(json!({"type": "string", "minLength": 4})),
                },
            ],
            body_schema: Some(schema_name.clone()),
            responses: vec![
                IrResponse {
                    status: Some(200),
                    description: Some("ok".into()),
                    schema: Some(schema_name),
                },
                IrResponse {
                    status: Some(400),
                    description: Some("bad request".into()),
                    schema: None,
                },
                IrResponse {
                    status: None,
                    description: Some("error".into()),
                    schema: None,
                },
            ],
        });
    }
    spec
}

/// The `docs-md` preset renders at memory-bandwidth-ish speed: table cells
/// and section fragments are precomputed in Rust and the page templates
/// are dumb printers. Guards against regressions reintroducing per-cell
/// filter calls or per-render context conversion into the hot path.
///
/// The ceiling is deliberately generous (>= 400 MB/s steady-state) so a
/// loaded CI machine still passes; the stripe measurement is ~an order of
/// magnitude above it.
#[test]
fn docs_md_render_throughput_stays_above_ceiling() {
    let spec = synthetic_spec(300);
    let engine = engine_with(presets::get("docs-md").unwrap().templates);
    let ctx = presets::base_context(&spec);

    let outputs = ["docs-md/index.md.j2", "docs-md/schema.md.j2"];
    let mut bytes_per_run = 0usize;
    for name in outputs {
        // Warmup also populates the context-conversion cache.
        bytes_per_run += engine.render(name, &ctx).expect("renders").len();
    }
    assert!(bytes_per_run > 200_000, "fixture too small to measure");

    let runs = 5;
    let start = std::time::Instant::now();
    for _ in 0..runs {
        for name in outputs {
            engine.render(name, &ctx).expect("renders");
        }
    }
    let mb_per_s = (bytes_per_run * runs) as f64 / (1048576.0 * start.elapsed().as_secs_f64());
    assert!(
        mb_per_s >= 400.0,
        "docs-md render throughput collapsed: {mb_per_s:.0} MB/s"
    );
}

/// Precomputed `docs-md` context keys exist alongside the shared keys.
#[test]
fn docs_md_context_precomputes_rows_and_fragments() {
    let spec = petstore_spec();
    let ctx = presets::base_context(&spec);

    let op = &ctx["operations_by_tag"][0]["operations"][0];
    assert_eq!(
        op["rows_params"][0],
        json!(["limit", "query", "no", "number"]),
        "parameter rows fully rendered"
    );
    assert!(
        op["fragment"]
            .as_str()
            .is_some_and(|f| f.contains("### `GET /pets`")),
        "operation fragment precomputed"
    );

    let pet = ctx["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Pet")
        .expect("Pet present");
    assert_eq!(
        pet["rows_props"][0],
        json!(["friend", "Pet", "no", "..."]),
        "property rows fully rendered in document order"
    );
    assert!(
        pet["fragment"]
            .as_str()
            .is_some_and(|f| f.contains("| friend | Pet | no |")),
        "schema fragment precomputed"
    );

    let list = ctx["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "PetList")
        .expect("PetList present");
    assert!(
        list["rows_props"].is_null(),
        "non-object schemas have no rows"
    );
    assert_eq!(list["type_str"], "Pet[]", "scalar branch type precomputed");
}
