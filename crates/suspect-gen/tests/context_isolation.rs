//! Context isolation at the public template-rendering boundary.

use serde_json::json;
use suspect_gen::{GenError, MinijinjaEngine, TemplateEngine};

#[test]
fn rendering_observes_changes_to_the_current_spec() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("operation", "{{ api.title }}: {{ operation.id }}")
        .unwrap();
    let mut context = json!({
        "api": { "title": "First API" },
        "operation": { "id": "listModels" }
    });

    assert_eq!(
        engine.render("operation", &context).unwrap(),
        "First API: listModels"
    );
    context["api"]["title"] = json!("Second API");
    context["operation"]["id"] = json!("getModel");
    assert_eq!(
        engine.render("operation", &context).unwrap(),
        "Second API: getModel"
    );
}

#[test]
fn prepared_specs_can_be_reused_without_sharing_values() {
    let mut engine = MinijinjaEngine::new();
    engine.add_template("title", "{{ title }}").unwrap();
    let engine: &dyn TemplateEngine = &engine;
    let first = json!({ "title": "First API" });
    let second = json!({ "title": "Second API" });
    let first = engine.prepare_context(&first);
    let second = engine.prepare_context(&second);

    for _ in 0..3 {
        assert_eq!(
            engine.render_prepared("title", &first).unwrap(),
            "First API"
        );
        assert_eq!(
            engine.render_prepared("title", &second).unwrap(),
            "Second API"
        );
    }
}

#[test]
fn successive_spec_values_do_not_reuse_an_earlier_render() {
    let mut engine = MinijinjaEngine::new();
    engine.add_template("title", "{{ title }}").unwrap();

    for title in ["First API", "Second API", "Third API"] {
        let context = json!({ "title": title });
        assert_eq!(engine.render("title", &context).unwrap(), title);
    }
}

#[test]
fn prepared_specs_can_render_concurrently() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("operation", "{{ api }}: {{ operation }}")
        .unwrap();
    let engine: &dyn TemplateEngine = &engine;
    let contexts = [
        json!({ "api": "First API", "operation": "listModels" }),
        json!({ "api": "Second API", "operation": "getModel" }),
    ];
    let expected = ["First API: listModels", "Second API: getModel"];

    std::thread::scope(|scope| {
        for (context, expected) in contexts.iter().zip(expected) {
            let prepared = engine.prepare_context(context);
            scope.spawn(move || {
                for _ in 0..100 {
                    assert_eq!(
                        engine.render_prepared("operation", &prepared).unwrap(),
                        expected
                    );
                    assert_eq!(engine.render("operation", context).unwrap(), expected);
                }
            });
        }
    });
}

/// An existing adapter implementing only the original engine contract.
struct LegacyEngine(MinijinjaEngine);

impl TemplateEngine for LegacyEngine {
    fn render(&self, name: &str, ctx: &serde_json::Value) -> Result<String, GenError> {
        self.0.render(name, ctx)
    }

    fn add_template(&mut self, name: &str, source: &str) -> Result<(), GenError> {
        self.0.add_template(name, source)
    }
}

#[test]
fn existing_engine_implementations_support_prepared_rendering() {
    let mut engine = LegacyEngine(MinijinjaEngine::new());
    engine.add_template("title", "{{ title }}").unwrap();
    let context = json!({ "title": "Compatible API" });
    let prepared = engine.prepare_context(&context);

    assert_eq!(
        engine.render_prepared("title", &prepared).unwrap(),
        "Compatible API"
    );
    assert_eq!(
        engine.0.render_prepared("title", &prepared).unwrap(),
        "Compatible API"
    );
}

#[test]
fn json_roundtrips_preserve_exact_numbers_and_literal_object_keys() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("json", "{{ schema | tojson }}")
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(
        r#"{
            "native": 18446744073709551615,
            "integer": 1844674407370955161600000000000000000000000,
            "decimal": 0.123456789012345678901234567890,
            "exponent": 1e500,
            "nested": [1, {"$serde_json::private::Number": "2"}]
        }"#,
    )
    .unwrap();
    let context = json!({ "schema": schema });
    let prepared = engine.prepare_context(&context);

    for output in [
        engine.render("json", &context).unwrap(),
        engine.render_prepared("json", &prepared).unwrap(),
    ] {
        let rendered: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(rendered, context["schema"]);
    }
}

#[test]
fn ordinary_json_numbers_keep_template_arithmetic() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("arithmetic", "{{ count + 1 }} / {{ weight + 0.25 }}")
        .unwrap();
    let context = json!({ "count": 41, "weight": 1.25 });
    assert_eq!(engine.render("arithmetic", &context).unwrap(), "42 / 1.5");
    let prepared = engine.prepare_context(&context);
    assert_eq!(
        engine.render_prepared("arithmetic", &prepared).unwrap(),
        "42 / 1.5"
    );
}

#[test]
fn wide_numbers_render_exactly_and_reject_unsupported_arithmetic() {
    let mut engine = MinijinjaEngine::new();
    engine.add_template("literal", "{{ number }}").unwrap();
    engine
        .add_template("arithmetic", "{{ number + 1 }}")
        .unwrap();
    for literal in [
        "1844674407370955161600000000000000000000000",
        "0.123456789012345678901234567890",
        "1e+500",
    ] {
        let number: serde_json::Value = serde_json::from_str(literal).unwrap();
        let context = json!({ "number": number });
        let prepared = engine.prepare_context(&context);
        assert_eq!(engine.render("literal", &context).unwrap(), literal);
        assert_eq!(
            engine.render_prepared("literal", &prepared).unwrap(),
            literal
        );
        assert!(engine.render("arithmetic", &context).is_err());
        assert!(engine.render_prepared("arithmetic", &prepared).is_err());
    }
}

#[test]
fn schema_example_filters_preserve_numeric_literals() {
    let mut engine = MinijinjaEngine::new();
    suspect_gen::FilterRegistry::register(&mut engine);
    engine
        .add_template(
            "example",
            "{{ schema | scalar_example }} / {{ schema | example_of }}",
        )
        .unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(r#"{"type":"number","example":0.123456789012345678901234567890}"#)
            .unwrap();
    let context = json!({ "schema": schema });
    assert_eq!(
        engine.render("example", &context).unwrap(),
        "0.123456789012345678901234567890 / 0.123456789012345678901234567890"
    );
}

#[test]
fn json_output_supports_indentation_and_html_safe_escaping() {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template(
            "script.html",
            "<script>{{ payload | tojson(indent=2) }}</script>",
        )
        .unwrap();
    let context = json!({ "payload": { "text": "</script>'&<script>" } });
    let rendered = engine.render("script.html", &context).unwrap();
    assert_eq!(rendered.matches("</script>").count(), 1);
    assert!(rendered.contains("\n  \"text\": "));
    assert!(rendered.contains(r"\u003c/script\u003e\u0027\u0026\u003cscript\u003e"));
    let json: serde_json::Value = serde_json::from_str(
        rendered
            .strip_prefix("<script>")
            .unwrap()
            .strip_suffix("</script>")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json, context["payload"]);
}

#[test]
#[ignore = "requires SUSPECT_OPENROUTER_SPEC pointing to the local OpenRouter OpenAPI JSON"]
fn openrouter_docs_contexts_remain_isolated() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_SPEC")
        .expect("set SUSPECT_OPENROUTER_SPEC to openrouter-web/openrouter-openapi.json");
    let spec = suspect_ir::IrSpec::from_file(std::path::Path::new(&path)).unwrap();
    assert_eq!(spec.title, "OpenRouter API");
    let preset = suspect_gen::presets::get("docs-md").unwrap();
    let mut engine = MinijinjaEngine::new();
    suspect_gen::FilterRegistry::register(&mut engine);
    for (name, source) in preset.templates {
        engine.add_template(name, source).unwrap();
    }
    let context = (preset.ctx_builder)(&spec);
    let mut revised = context.clone();
    revised["title"] = json!("Alternate API");
    let contexts = [context, revised];
    let templates = ["docs-md/index.md.j2", "docs-md/schema.md.j2"];
    let expected: Vec<Vec<String>> = contexts
        .iter()
        .map(|context| {
            templates
                .iter()
                .map(|name| engine.render(name, context).unwrap())
                .collect()
        })
        .collect();
    assert!(expected[0][0].starts_with("# OpenRouter API "));
    assert!(expected[1][0].starts_with("# Alternate API "));
    assert!(
        expected
            .iter()
            .flatten()
            .all(|output| !output.contains("$serde_json::private::Number"))
    );
    let prepared: Vec<_> = contexts
        .iter()
        .map(|context| engine.prepare_context(context))
        .collect();
    let engine: &dyn TemplateEngine = &engine;

    std::thread::scope(|scope| {
        for (context, expected) in prepared.iter().zip(&expected) {
            scope.spawn(move || {
                for _ in 0..3 {
                    for (name, expected) in templates.iter().zip(expected) {
                        assert_eq!(engine.render_prepared(name, context).unwrap(), *expected);
                    }
                }
            });
        }
    });

    let bytes: usize = expected[0].iter().map(String::len).sum();
    let runs = 10;
    let start = std::time::Instant::now();
    for _ in 0..runs {
        for name in templates {
            std::hint::black_box(engine.render_prepared(name, &prepared[0]).unwrap());
        }
    }
    let mib_per_s = (bytes * runs) as f64 / (1048576.0 * start.elapsed().as_secs_f64());
    eprintln!("OpenRouter prepared docs: {bytes} bytes/run, {mib_per_s:.0} MiB/s");
}
