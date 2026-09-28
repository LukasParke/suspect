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
    let original_doc = low.root().syntax().doc().uri().clone();
    let shared_refs = swagger_shared_refs(components);
    // `definitions/*` instances.
    if let Some(definitions) = low.root().get("definitions") {
        for entry in definitions.entries() {
            let Some(schema) = entry.value else {
                continue;
            };
            let original_doc = low.root().syntax().doc().uri().clone();
            check_one_doc(
                "2.0",
                schema,
                schema.byte_range(),
                &mut out,
                false,
                &original_doc,
                shared_refs.as_ref(),
            );
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
                    check_one_doc(
                        "2.0",
                        schema,
                        span,
                        &mut out,
                        false,
                        &original_doc,
                        shared_refs.as_ref(),
                    );
                }
            }
            if let Some(responses) = op.get("responses") {
                for response in responses.entries() {
                    let Some(resp) = response.value else {
                        continue;
                    };
                    if let Some(schema) = resp.get("schema") {
                        let span = schema.byte_range();
                        check_one_doc(
                            "2.0",
                            schema,
                            span,
                            &mut out,
                            false,
                            &original_doc,
                            shared_refs.as_ref(),
                        );
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn check_schema_instances(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    check_instances_inner(api, out, false);
}

/// The opt-in `format` assertion pass: identical walk, but schemas compile
/// with `format_assertion: true` so declared formats validate instances
/// (RFC 2020-12 makes them annotations by default).
pub(crate) fn check_format_assertions(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    check_instances_inner(api, out, true);
}

fn check_instances_inner(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>, format_assertion: bool) {
    // One shared components-document fallback per run: the scan is
    // computed once, not per instance check (quadratic on large
    // component sections otherwise).
    let shared_refs = shared_component_refs(api);
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
                check_one(
                    api,
                    schema,
                    schema.byte_range(),
                    out,
                    format_assertion,
                    shared_refs.as_ref(),
                );
            }
        }
    }
    // Swagger 2.0: same guarantee over `definitions`.
    if let Some(definitions) = root.get("definitions") {
        for entry in definitions.entries() {
            if let Some(schema) = entry.value {
                check_one(
                    api,
                    schema,
                    schema.byte_range(),
                    out,
                    format_assertion,
                    shared_refs.as_ref(),
                );
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
                    check_one(
                        api,
                        schema,
                        span,
                        out,
                        format_assertion,
                        shared_refs.as_ref(),
                    );
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
    format_assertion: bool,
    shared_refs: Option<&suspect_schema::DocumentRefs<'_>>,
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
    let original_doc = api.root().syntax().doc().uri().clone();
    check_one_doc(
        version,
        schema_node,
        span,
        out,
        format_assertion,
        &original_doc,
        shared_refs,
    );
}

/// The 2.0 battery's shared fallback, built from the already-extracted
/// `definitions` map.
fn swagger_shared_refs(
    components: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<suspect_schema::DocumentRefs<'static>> {
    let json = serde_json::json!({ "definitions": components }).to_string();
    let uri = suspect_source::Uri::parse("mem://swagger-definitions.json").ok()?;
    let doc: &'static suspect_low::LowDoc = Box::leak(Box::new(suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(json.into_bytes()),
    )));
    suspect_schema::DocumentRefs::scan(doc.root(), usize::MAX).ok()
}

/// The enclosing-document `$ref` fallback: a synthetic document holding
/// just `components/schemas`, scanned once and shared by every compile.
/// `NodeRef` is covariant, so the long-lived root coerces into each
/// per-instance compile's shorter lifetime.
fn shared_component_refs(api: &OpenApi<'_>) -> Option<suspect_schema::DocumentRefs<'static>> {
    let components = document_components(api);
    let json = serde_json::json!({ "components": { "schemas": components } }).to_string();
    let uri = suspect_source::Uri::parse("mem://instance-components.json").ok()?;
    let doc: &'static suspect_low::LowDoc = Box::leak(Box::new(suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(json.into_bytes()),
    )));
    suspect_schema::DocumentRefs::scan(doc.root(), usize::MAX).ok()
}

/// The document's `components/schemas` map as JSON, so the wrapper the
/// schema compiles from can resolve `#/components/schemas/...` refs.
fn document_components(api: &OpenApi<'_>) -> std::collections::BTreeMap<String, serde_json::Value> {
    api.root()
        .get("components")
        .and_then(|c| c.get("schemas"))
        .map(|schemas| {
            schemas
                .entries()
                .iter()
                .filter_map(|entry| {
                    let schema = entry.value?;
                    let json = OvValue::from_node(schema).to_json();
                    let value = serde_json::from_str(&json).ok()?;
                    Some((entry.key.to_owned(), value))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn check_one_doc(
    version: &str,
    schema_node: NodeRef<'_>,
    span: std::ops::Range<usize>,
    out: &mut Vec<Diagnostic>,
    format_assertion: bool,
    original_doc: &suspect_source::Uri,
    shared_refs: Option<&suspect_schema::DocumentRefs<'_>>,
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
    // Local `$ref`s resolve against the shared components document via
    // compile_with_document_root, so component graphs — including
    // recursive schemas — validate without pre-inlining. Refs inside
    // examples are data (a string) and stay as-is.
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
        let Ok(compiled) = suspect_schema::Compiler::new(suspect_schema::Config {
            format_assertion,
            ..suspect_schema::Config::default()
        })
        // Components live at the wrapper root, so `$ref`s into the
        // component graph — recursive ones included — resolve here.
        .compile_with_document_root(schema_node, shared_refs) else {
            // Malformed schema keywords surface through the schema battery;
            // instance checking has nothing to say.
            continue;
        };
        for error in compiled.validate(instance_node) {
            out.push(super::diag_for(
                original_doc,
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
                out.push(super::diag_for(
                    original_doc,
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
            let Ok(compiled) = suspect_schema::Compiler::new(suspect_schema::Config {
                format_assertion,
                ..suspect_schema::Config::default()
            })
            .compile(schema_node) else {
                continue;
            };
            for error in compiled.validate(instance_node) {
                out.push(super::diag_for(
                    original_doc,
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
