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

use suspect_ir::IrSpec;

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
/// under `"other"`), `schema_names`, and `schema_refs` (component name to
/// schema JSON, used by the `example_of` filter).
fn common_context(spec: &IrSpec) -> serde_json::Value {
    let mut ctx = serde_json::to_value(spec).expect("IrSpec serializes to JSON");
    let obj = ctx.as_object_mut().expect("IrSpec serializes to an object");

    obj.insert(
        "base_url".to_owned(),
        serde_json::Value::String(spec.servers.first().cloned().unwrap_or_default()),
    );

    let mut groups: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
    for op in &spec.operations {
        let tag = op
            .tags
            .first()
            .cloned()
            .unwrap_or_else(|| "other".to_owned());
        let serialized = serde_json::to_value(op).expect("operation serializes to JSON");
        match groups.iter_mut().find(|(t, _)| *t == tag) {
            Some((_, ops)) => ops.push(serialized),
            None => groups.push((tag, vec![serialized])),
        }
    }
    let operations_by_tag = groups
        .into_iter()
        .map(|(tag, operations)| serde_json::json!({ "tag": tag, "operations": operations }))
        .collect::<Vec<_>>();
    obj.insert(
        "operations_by_tag".to_owned(),
        serde_json::Value::Array(operations_by_tag),
    );

    let names: Vec<String> = spec.schemas.iter().map(|s| s.name.clone()).collect();
    obj.insert(
        "schema_names".to_owned(),
        serde_json::Value::Array(names.into_iter().map(serde_json::Value::String).collect()),
    );

    let mut refs = serde_json::Map::new();
    for s in &spec.schemas {
        refs.insert(s.name.clone(), s.json.clone());
    }
    obj.insert("schema_refs".to_owned(), serde_json::Value::Object(refs));

    ctx
}

/// Builds the render context used by every shipped preset.
///
/// Exposed as a free function so callers can inspect or extend the exact
/// context the presets render against.
#[must_use]
pub fn base_context(spec: &IrSpec) -> serde_json::Value {
    common_context(spec)
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
            ctx_builder: common_context,
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
