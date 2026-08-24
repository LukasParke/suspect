//! Built-in generation presets.
//!
//! A [`Preset`] bundles starter templates, a manifest, and an IR-to-context
//! builder for a known target. Three presets ship out of the box:
//!
//! - `docs-md` — Markdown API documentation (`docs/api/`)
//! - `ts-sdk` — a fetch-based TypeScript client (`sdk/typescript/`)
//! - `rust-sdk` — a zero-dependency request-builder SDK (`sdk/rust/`)
//!
//! All templates render through [`FilterRegistry`](crate::FilterRegistry)
//! filters and embed preservation markers around user-owned sections where
//! regeneration must not clobber hand-written code.

use rayon::prelude::*;

use suspect_ir::{IrSpec, ParamIn};

use crate::TemplateEngine;
/// A bundled generation preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// `(template_name, template_source)` pairs installed into the engine.
    pub templates: &'static [(&'static str, &'static str)],
    /// Starter manifest text referencing the bundled templates.
    pub manifest_toml: &'static str,
    /// Builds the render context from a normalized spec.
    pub ctx_builder: fn(&IrSpec) -> serde_json::Value,
}

/// `docs-md` templates: an index page (per-tag TOC + per-operation
/// sections) and a schema reference page.
pub static DOCS_MD_TEMPLATES: &[(&str, &str)] = &[
    (
        "docs-md/index.md.j2",
        include_str!("../presets/docs-md/index.md.j2"),
    ),
    (
        "docs-md/schema.md.j2",
        include_str!("../presets/docs-md/schema.md.j2"),
    ),
    (
        "docs-md/partials/op.md.j2",
        include_str!("../presets/docs-md/partials/op.md.j2"),
    ),
    (
        "docs-md/partials/schema-body.md.j2",
        include_str!("../presets/docs-md/partials/schema-body.md.j2"),
    ),
];

/// Starter manifest for the `docs-md` preset.
pub static DOCS_MD_MANIFEST: &str = include_str!("../presets/docs-md/manifest.toml");

/// `ts-sdk` templates: a typed fetch client plus model interfaces.
pub static TS_SDK_TEMPLATES: &[(&str, &str)] = &[
    (
        "ts-sdk/models.ts.j2",
        include_str!("../presets/ts-sdk/models.ts.j2"),
    ),
    (
        "ts-sdk/client.ts.j2",
        include_str!("../presets/ts-sdk/client.ts.j2"),
    ),
];

/// Starter manifest for the `ts-sdk` preset.
pub static TS_SDK_MANIFEST: &str = include_str!("../presets/ts-sdk/manifest.toml");

/// `rust-sdk` templates: package manifest, models, and the client lib.
pub static RUST_SDK_TEMPLATES: &[(&str, &str)] = &[
    (
        "rust-sdk/Cargo.toml.j2",
        include_str!("../presets/rust-sdk/Cargo.toml.j2"),
    ),
    (
        "rust-sdk/lib.rs.j2",
        include_str!("../presets/rust-sdk/lib.rs.j2"),
    ),
    (
        "rust-sdk/models.rs.j2",
        include_str!("../presets/rust-sdk/models.rs.j2"),
    ),
];

/// Starter manifest for the `rust-sdk` preset.
pub static RUST_SDK_MANIFEST: &str = include_str!("../presets/rust-sdk/manifest.toml");

/// Serializes `spec` and augments it with the shared derived context keys.
///
/// Adds `base_url` (first server or empty string), `operations_by_tag`
/// (`[{tag, operations}]` in document order, untagged operations grouped
/// under `"other"`), `schema_names`, `schema_refs` (component name to
/// schema JSON, used by the `example_of` filter), `schema_examples`, and
/// the precomputed `docs-md` render aids (`rows_params`, `rows_props`,
/// per-entity `fragment` strings, `type_str`, `example_str`).
fn common_context(spec: &IrSpec) -> serde_json::Value {
    let mut ctx = serde_json::to_value(spec).expect("IrSpec serializes to JSON");
    let obj = ctx.as_object_mut().expect("IrSpec serializes to an object");

    obj.insert(
        "base_url".to_owned(),
        serde_json::Value::String(spec.servers.first().cloned().unwrap_or_default()),
    );

    let mut refs = serde_json::Map::new();
    for s in &spec.schemas {
        refs.insert(s.name.clone(), s.json.clone());
    }
    obj.insert(
        "schema_refs".to_owned(),
        serde_json::Value::Object(refs.clone()),
    );

    // One-pass deep examples per component.
    let examples = crate::examples_for_components(&refs);
    obj.insert(
        "schema_examples".to_owned(),
        serde_json::Value::Object(examples.clone()),
    );

    // `docs-md` hot path: every per-cell string and each operation /
    // schema section fragment is precomputed here, in parallel, so the
    // page templates are dumb printers (loops + variable output only,
    // zero filters in hot loops).
    let mut engine = crate::MinijinjaEngine::new();
    crate::FilterRegistry::register(&mut engine);
    for (name, src) in DOCS_MD_TEMPLATES {
        engine
            .add_template(name, src)
            .expect("bundled docs-md template compiles");
    }

    let ops: Vec<serde_json::Value> = spec
        .operations
        .par_iter()
        .map(|op| {
            let mut v = serde_json::to_value(op).expect("operation serializes to JSON");
            v.as_object_mut()
                .expect("operation serializes to an object")
                .insert(
                    "rows_params".to_owned(),
                    serde_json::Value::Array(op.parameters.iter().map(param_table_row).collect()),
                );
            // The partials end in a newline, which minijinja trims from
            // top-level renders (`keep_trailing_newline` defaults to
            // false); restore it so stored fragments match included
            // output byte-for-byte.
            let fragment = format!(
                "{}\n",
                engine
                    .render_once(
                        OP_FRAGMENT_TEMPLATE,
                        &serde_json::json!({ "op": v.clone() }),
                    )
                    .expect("operation fragment renders")
            );
            v.as_object_mut()
                .expect("operation serializes to an object")
                .insert("fragment".to_owned(), serde_json::Value::String(fragment));
            v
        })
        .collect();

    let mut groups: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
    for (op, serialized) in spec.operations.iter().zip(ops) {
        let tag = op
            .tags
            .first()
            .cloned()
            .unwrap_or_else(|| "other".to_owned());
        match groups.iter_mut().find(|(t, _)| *t == tag) {
            Some((_, group_ops)) => group_ops.push(serialized),
            None => groups.push((tag, vec![serialized])),
        }
    }
    let operations_by_tag = groups
        .into_iter()
        .map(|(tag, operations)| {
            // Precomputed TOC lines and section bodies per tag: the page
            // templates print these instead of looping per operation.
            let mut toc = String::from("\n");
            let mut sections = String::from("\n");
            for op in &operations {
                let o = op.as_object().expect("operation serializes to an object");
                toc.push_str("\n- `");
                toc.push_str(o["method"].as_str().unwrap_or_default());
                toc.push(' ');
                toc.push_str(o["path"].as_str().unwrap_or_default());
                toc.push('`');
                if let Some(id) = o["id"].as_str() {
                    toc.push_str(" \u{2014} ");
                    toc.push_str(id);
                }
                toc.push('\n');
                sections.push('\n');
                sections.push_str(o["fragment"].as_str().unwrap_or_default());
            }
            serde_json::json!({
                "tag": tag,
                "operations": operations,
                "toc": toc,
                "sections": sections,
            })
        })
        .collect::<Vec<_>>();
    obj.insert(
        "operations_by_tag".to_owned(),
        serde_json::Value::Array(operations_by_tag),
    );

    let schemas: Vec<serde_json::Value> = spec
        .schemas
        .par_iter()
        .map(|s| {
            let mut v = serde_json::to_value(s).expect("schema serializes to JSON");
            let (rows_props, type_str) = schema_rows_and_type(s);
            let example_str = examples
                .get(&s.name)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            {
                let entry = v.as_object_mut().expect("schema serializes to an object");
                entry.insert("rows_props".to_owned(), rows_props);
                entry.insert("type_str".to_owned(), type_str);
                entry.insert("example_str".to_owned(), example_str);
            }
            let fragment = format!(
                "{}\n",
                engine
                    .render_once(SCHEMA_BODY_TEMPLATE, &serde_json::json!({ "s": v.clone() }))
                    .expect("schema fragment renders")
            );
            v.as_object_mut()
                .expect("schema serializes to an object")
                .insert("fragment".to_owned(), serde_json::Value::String(fragment));
            v
        })
        .collect();
    obj.insert("schemas".to_owned(), serde_json::Value::Array(schemas));

    let names: Vec<String> = spec.schemas.iter().map(|s| s.name.clone()).collect();
    obj.insert(
        "schema_names".to_owned(),
        serde_json::Value::Array(names.into_iter().map(serde_json::Value::String).collect()),
    );

    ctx
}

/// Template name of the per-operation section fragment.
const OP_FRAGMENT_TEMPLATE: &str = "docs-md/partials/op.md.j2";

/// Template name of the per-schema reference body fragment.
const SCHEMA_BODY_TEMPLATE: &str = "docs-md/partials/schema-body.md.j2";

/// Builds one fully rendered parameter-table row `[name, in, required,
/// type]`; the `type` cell applies [`crate::ts_type`] to the materialized
/// parameter schema (or `"-"` when absent).
fn param_table_row(p: &suspect_ir::IrParameter) -> serde_json::Value {
    let type_str = p
        .schema
        .as_ref()
        .map(crate::ts_type)
        .unwrap_or_else(|| "-".to_owned());
    serde_json::json!([
        p.name,
        match p.location {
            ParamIn::Query => "query",
            ParamIn::Header => "header",
            ParamIn::Path => "path",
            ParamIn::Cookie => "cookie",
        },
        if p.required { "yes" } else { "no" },
        type_str,
    ])
}

/// Precomputes `(rows_props, type_str)` for one schema.
///
/// `rows_props` is an array of fully rendered property-table rows
/// `[name, type, required, example]` for object schemas with a non-empty
/// `properties` map ([`crate::ts_type`] and [`crate::scalar_example`]
/// applied here, once per cell); it is `null` otherwise. `type_str`
/// carries [`crate::ts_type`] of the whole schema for the scalar branch.
fn schema_rows_and_type(s: &suspect_ir::IrSchema) -> (serde_json::Value, serde_json::Value) {
    let props = s
        .json
        .get("properties")
        .and_then(serde_json::Value::as_object);
    let is_object = s.json.get("type").and_then(serde_json::Value::as_str) == Some("object");
    let rows = if is_object {
        match props.filter(|p| !p.is_empty()) {
            None => serde_json::Value::Null,
            Some(props) => {
                let required = s.json.get("required").and_then(serde_json::Value::as_array);
                serde_json::Value::Array(
                    props
                        .iter()
                        .map(|(pname, prop)| {
                            let req = required.is_some_and(|r| {
                                r.iter().any(|v| v.as_str() == Some(pname.as_str()))
                            });
                            serde_json::json!([
                                pname,
                                crate::ts_type(prop),
                                if req { "yes" } else { "no" },
                                crate::scalar_example(prop),
                            ])
                        })
                        .collect(),
                )
            }
        }
    } else {
        serde_json::Value::Null
    };
    let type_str = if rows.is_null() {
        serde_json::Value::String(crate::ts_type(&s.json))
    } else {
        serde_json::Value::Null
    };
    (rows, type_str)
}

/// Builds the render context used by every shipped preset.
///
/// Exposed as a free function so callers can inspect or extend the exact
/// context the presets render against.
#[must_use]
pub fn base_context(spec: &IrSpec) -> serde_json::Value {
    common_context(spec)
}

/// Builds the render context for the `docs-md` preset.
///
/// Identical to [`base_context`] except for keys the shipped `docs-md`
/// pages never read (`operations`, `schema_refs`, `schema_examples`,
/// `servers`) and the raw `schemas[].json` payloads (consumed while the
/// fragments above are rendered; finished pages print `fragment`
/// strings). Trimming these duplicates cuts the one-time context
/// conversion that dominates render startup on large specs.
#[must_use]
pub fn docs_md_context(spec: &IrSpec) -> serde_json::Value {
    let mut ctx = common_context(spec);
    if let Some(obj) = ctx.as_object_mut() {
        obj.remove("operations");
        obj.remove("schema_refs");
        obj.remove("schema_examples");
        obj.remove("servers");
    }
    // The raw schema payloads are only consumed while fragments are
    // rendered above; the finished pages print `fragment` strings.
    if let Some(schemas) = ctx
        .get_mut("schemas")
        .and_then(serde_json::Value::as_array_mut)
    {
        for schema in schemas {
            schema
                .as_object_mut()
                .expect("schema serializes to an object")
                .remove("json");
        }
    }
    ctx
}

/// Looks up a preset by name.
///
/// Known names: `"docs-md"`, `"ts-sdk"`, `"rust-sdk"`. Returns `None` for
/// anything else.
#[must_use]
pub fn get(name: &str) -> Option<Preset> {
    match name {
        "docs-md" => Some(Preset {
            templates: DOCS_MD_TEMPLATES,
            manifest_toml: DOCS_MD_MANIFEST,
            ctx_builder: docs_md_context,
        }),
        "ts-sdk" => Some(Preset {
            templates: TS_SDK_TEMPLATES,
            manifest_toml: TS_SDK_MANIFEST,
            ctx_builder: common_context,
        }),
        "rust-sdk" => Some(Preset {
            templates: RUST_SDK_TEMPLATES,
            manifest_toml: RUST_SDK_MANIFEST,
            ctx_builder: common_context,
        }),
        _ => None,
    }
}
