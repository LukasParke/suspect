//! Generation hot-path benchmark for `suspect-gen`.
//!
//! Renders one medium-sized template 200 times per iteration over a
//! synthetic context, exercising the sandboxed engine plus the built-in
//! filter set end to end.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_gen::{FilterRegistry, MinijinjaEngine, TemplateEngine};

/// A synthetic context large enough to make rendering, not lookup, the cost.
fn synthetic_ctx() -> serde_json::Value {
    let models: Vec<serde_json::Value> = (0..24)
        .map(|i| {
            serde_json::json!({
                "name": format!("Model{i}"),
                "table": format!("model_{i}"),
                "fields": [
                    {"key": "id", "ts": "number"},
                    {"key": "label", "ts": "string | null"}
                ]
            })
        })
        .collect();
    serde_json::json!({ "package": "gen", "models": models })
}

/// Medium template mixing loops and several built-in filters.
const MEDIUM_TEMPLATE: &str = r#"// generated for {{ package }}
{% for m in models %}
export interface {{ m.name | PascalCase }} {
{% for f in m.fields %}  {{ f.key | snake_case }}: {{ f.ts }};
{% endfor %}}
export const {{ m.name | constant_case }}_TABLE = "{{ m.table | kebab_case }}";
{% endfor %}
"#;

/// Renders the medium template 200 times per measured iteration.
fn bench_render_medium_x200(c: &mut Criterion) {
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template("medium", MEDIUM_TEMPLATE)
        .expect("template compiles");
    FilterRegistry::register(&mut engine);
    let ctx = synthetic_ctx();

    c.bench_function("gen/render_medium_x200", |b| {
        b.iter(|| {
            for _ in 0..200 {
                black_box(engine.render("medium", black_box(&ctx)).expect("renders"));
            }
        })
    });
}

criterion_group!(benches, bench_render_medium_x200);
criterion_main!(benches);
