//! Schema-instance validation: `example`, `examples`, and `default`
//! values validated against their schemas with the JSON Schema compiler.
//!
//! The compiler evaluates schema and instance in one shared lifetime, so
//! each check materializes a wrapper document
//! `{"schema": <schema>, "instance": <value>}` and validates the instance
//! subtree. Schema and instance both live in the spec document, so the
//! wrapper embeds their serialized forms; schemas are small and instances
//! are rarer still, so per-check compilation is cheap.

use suspect_low::NodeRef;
use suspect_oas::OpenApi;
use suspect_overlay::Value as OvValue;

use crate::checks::diag;
use crate::diagnostic::{Diagnostic, Severity};

pub(crate) fn check_schema_instances(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    // The contract index is reference-closure based; components schemas
    // unreferenced from paths never register. Instance checking is a
    // whole-document guarantee, so walk the raw tree instead: every
    // `components/schemas/*` entry, plus inline schemas under operations.
    let root = api.root();
    if let Some(components) = root.get("components")
        && let Some(schemas) = components.get("schemas")
    {
        for entry in schemas.entries() {
            if let Some(schema) = entry.value {
                check_one(api, schema, schema.byte_range(), out);
            }
        }
    }
    if let Some(paths) = root.get("paths") {
        for path_entry in paths.entries() {
            let Some(path_item) = path_entry.value else {
                continue;
            };
            for method in [
                "get", "put", "post", "delete", "options", "head", "patch", "trace",
            ] {
                let Some(op) = path_item.get(method) else {
                    continue;
                };
                for schema in inline_operation_schemas(&op) {
                    let span = schema.byte_range();
                    check_one(api, schema, span, out);
                }
            }
        }
    }
}

/// Rewrites every `nullable: true` in the schema tree into a type union
/// (`type: [T, "null"]`), the 2020-12 spelling of the same assertion.
fn translate_nullable(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let nullable = map
                .get("nullable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if nullable && let Some(serde_json::Value::String(single)) = map.get("type") {
                let widened = serde_json::Value::Array(vec![
                    serde_json::Value::String(single.clone()),
                    serde_json::Value::String("null".into()),
                ]);
                map.insert("type".into(), widened);
            }
            for child in map.values_mut() {
                translate_nullable(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                translate_nullable(item);
            }
        }
        _ => {}
    }
}

/// Inline schema positions inside one operation: request body content
/// schemas and response content schemas.
fn inline_operation_schemas<'a>(op: &NodeRef<'a>) -> Vec<NodeRef<'a>> {
    let mut out = Vec::new();
    let mut push_content = |owner: Option<NodeRef<'a>>| {
        if let Some(content) = owner.and_then(|o| o.get("content")) {
            for media in content.entries() {
                if let Some(media_value) = media.value
                    && let Some(schema) = media_value.get("schema")
                {
                    out.push(schema);
                }
            }
        }
    };
    push_content(op.get("requestBody"));
    if let Some(responses) = op.get("responses") {
        for response in responses.entries() {
            if let Some(resp) = response.value {
                push_content(Some(resp));
            }
        }
    }
    out
}

/// Validates one schema's declared instances (examples, defaults) against
/// the schema itself.
fn check_one(
    api: &OpenApi<'_>,
    schema_node: NodeRef<'_>,
    span: std::ops::Range<usize>,
    out: &mut Vec<Diagnostic>,
) {
    let schema_json = OvValue::from_node(schema_node).to_json();
    let Ok(mut schema) = serde_json::from_str::<serde_json::Value>(&schema_json) else {
        return;
    };
    if !schema.is_object() {
        return;
    }
    // OAS 3.0's `nullable` widens the declared type; the 2020-12 compiler
    // ignores unknown keywords, so translate it into a type union before
    // the wrapper compiles — otherwise `default: null` falsely violates
    // `type: string`.
    if api
        .root()
        .get("openapi")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.starts_with("3.0"))
    {
        translate_nullable(&mut schema);
    }
    let mut instances: Vec<(String, &serde_json::Value, &str)> = Vec::new();
    if let Some(default) = schema.get("default") {
        instances.push(("default".into(), default, "default"));
    }
    if let Some(example) = schema.get("example") {
        instances.push(("example".into(), example, "example"));
    }
    if let Some(examples) = schema.get("examples").and_then(|e| e.as_array()) {
        for (idx, example) in examples.iter().enumerate() {
            instances.push((format!("examples[{idx}]"), example, "example"));
        }
    }
    if instances.is_empty() {
        return;
    }
    let schema_json = serde_json::to_string(&schema).unwrap_or_default();
    for (_label, value, kind) in instances {
        // `$ref` values inside examples are data (a string), not
        // references — the wrapper serializes them as-is and the schema
        // sees a string. `default` is checked against the schema's
        // non-$ref assertions only: a `$ref`-bearing schema cannot be
        // inlined into the wrapper without a resolver, so skip those.
        if schema.get("$ref").is_some() {
            continue;
        }
        let value_json = serde_json::to_string(value).unwrap_or_default();
        let wrapper = format!(r#"{{"schema": {schema_json}, "instance": {value_json}}}"#);
        let Ok(uri) = suspect_source::Uri::parse("mem://schema-instance.json") else {
            continue;
        };
        let doc =
            suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(wrapper.into_bytes()));
        if !doc.syntax_errors().is_empty() {
            continue;
        }
        let (Some(schema_node), Some(instance_node)) =
            (doc.root().get("schema"), doc.root().get("instance"))
        else {
            continue;
        };
        let Ok(compiled) =
            suspect_schema::Compiler::new(suspect_schema::Config::default()).compile(schema_node)
        else {
            // Malformed schema keywords surface through the schema battery;
            // instance checking has nothing to say.
            continue;
        };
        for error in compiled.validate(instance_node) {
            out.push(diag(
                api,
                "oas-schema-instance-invalid",
                Severity::Warning,
                span.clone(),
                format!("{kind} violates the schema: {}", error.message),
            ));
        }
    }
}
