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

use crate::diagnostic::{Diagnostic, Severity};

/// Validates `definitions/*` instances of a Swagger 2.0 document — the
/// 2.0 counterpart of the components-schemas walk below.
pub(crate) fn check_swagger_definition_instances(
    low: &suspect_low::LowDoc,
    components: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // `definitions/*` instances.
    if let Some(definitions) = low.root().get("definitions") {
        for entry in definitions.entries() {
            let Some(schema) = entry.value else {
                continue;
            };
            check_one_doc("2.0", schema, schema.byte_range(), components, &mut out);
        }
    }
    // Inline operation schemas: non-body 2.0 parameters carry their
    // constraint keywords directly on the parameter object, so the
    // parameter itself is the schema (name/in are unknown keywords the
    // compiler skips). Responses carry `schema` directly.
    let Some(paths) = low.root().get("paths") else {
        return out;
    };
    for path_entry in paths.entries() {
        let Some(path_item) = path_entry.value else {
            continue;
        };
        for method in ["get", "put", "post", "delete", "options", "head", "patch"] {
            let Some(op) = path_item.get(method) else {
                continue;
            };
            if let Some(params) = op.get("parameters") {
                for param in params.items() {
                    let schema = param.get("schema").unwrap_or(param);
                    let span = schema.byte_range();
                    check_one_doc("2.0", schema, span, components, &mut out);
                }
            }
            if let Some(responses) = op.get("responses") {
                for response in responses.entries() {
                    let Some(resp) = response.value else {
                        continue;
                    };
                    if let Some(schema) = resp.get("schema") {
                        let span = schema.byte_range();
                        check_one_doc("2.0", schema, span, components, &mut out);
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn check_schema_instances(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    // The contract index is reference-closure based; components schemas
    // unreferenced from paths never register. Instance checking is a
    // whole-document guarantee, so walk the raw tree instead: every
    // `components/schemas/*` entry (3.x) or `definitions/*` entry (2.0),
    // plus inline schemas under operations.
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
    // Swagger 2.0: same guarantee over `definitions`.
    if let Some(definitions) = root.get("definitions") {
        for entry in definitions.entries() {
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
    let version = if api
        .root()
        .get("openapi")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.starts_with("3.0"))
    {
        "3.0"
    } else {
        "3.1"
    };
    check_one_doc(
        version,
        schema_node,
        span,
        &std::collections::BTreeMap::new(),
        out,
    );
}

fn check_one_doc(
    version: &str,
    schema_node: NodeRef<'_>,
    span: std::ops::Range<usize>,
    components: &std::collections::BTreeMap<String, serde_json::Value>,
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
    if version.starts_with("3.0") || version == "2.0" {
        translate_nullable(&mut schema);
    }
    let schema = inline_component_refs(&schema, components, &mut Vec::new(), 0);
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
    for (_label, value, kind) in &instances {
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
            out.push(super::diag_at(
                schema_node,
                "oas-schema-instance-invalid",
                Severity::Warning,
                span.clone(),
                format!("{kind} violates the schema: {}", error.message),
            ));
        }
    }
    // contentMediaType + contentSchema: a string instance whose declared
    // media type is JSON decodes to a value that must satisfy the content
    // schema. Evaluated in a second wrapper (decoded value + content
    // schema).
    if let (Some(media), Some(content_schema)) = (
        schema.get("contentMediaType").and_then(|v| v.as_str()),
        schema.get("contentSchema"),
    ) && media.starts_with("application/json")
    {
        let content_json = serde_json::to_string(content_schema).unwrap_or_default();
        for (_label, value, kind) in &instances {
            let Some(text) = value.as_str() else {
                continue;
            };
            let Ok(decoded) = serde_json::from_str::<serde_json::Value>(text) else {
                out.push(super::diag_at(
                    schema_node,
                    "oas-schema-instance-invalid",
                    Severity::Warning,
                    span.clone(),
                    format!("{kind} is not decodable {media}: invalid JSON"),
                ));
                continue;
            };
            let value_json = serde_json::to_string(&decoded).unwrap_or_default();
            let wrapper = format!(r#"{{"schema": {content_json}, "instance": {value_json}}}"#);
            let Ok(uri) = suspect_source::Uri::parse("mem://content-schema.json") else {
                continue;
            };
            let doc = suspect_low::LowDoc::parse(
                uri,
                suspect_source::Source::from_vec(wrapper.into_bytes()),
            );
            let (Some(schema_node), Some(instance_node)) =
                (doc.root().get("schema"), doc.root().get("instance"))
            else {
                continue;
            };
            let Ok(compiled) = suspect_schema::Compiler::new(suspect_schema::Config::default())
                .compile(schema_node)
            else {
                continue;
            };
            for error in compiled.validate(instance_node) {
                out.push(super::diag_at(
                    schema_node,
                    "oas-schema-instance-invalid",
                    Severity::Warning,
                    span.clone(),
                    format!(
                        "{kind} decoded as {media} violates the content schema: {}",
                        error.message
                    ),
                ));
            }
        }
    }
}

/// Substitutes `#/components/schemas/<name>` references in a schema tree
/// with the referenced component JSON (depth-capped; recursive refs
/// collapse to permissive beyond the cap).
fn inline_component_refs(
    value: &serde_json::Value,
    components: &std::collections::BTreeMap<String, serde_json::Value>,
    seen: &mut Vec<String>,
    depth: usize,
) -> serde_json::Value {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return serde_json::Value::Bool(true);
    }
    match value {
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some(target) = map.get("$ref").and_then(|r| r.as_str())
                && let Some(name) = target.strip_prefix("#/components/schemas/")
                && let Some(component) = components.get(name)
            {
                if seen.iter().any(|s| s == name) {
                    return serde_json::Value::Bool(true);
                }
                seen.push(name.to_owned());
                let resolved = inline_component_refs(component, components, seen, depth + 1);
                seen.pop();
                return resolved;
            }
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                out.insert(
                    key.clone(),
                    inline_component_refs(child, components, seen, depth),
                );
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| inline_component_refs(item, components, seen, depth))
                .collect(),
        ),
        other => other.clone(),
    }
}
