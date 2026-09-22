//! Rich hover markdown renderers built directly from [`suspect_low::NodeRef`]
//! — no session or typed-view dependency. Produces structured markdown
//! tables so hover answers "what is this?" without navigating away.

use suspect_low::{NodeRef, ValueKind};

/// Attempts a rich markdown hover when the resolved target is under
/// `components/<section>/<Name>`.
#[must_use]
pub fn try_rich_hover(section: &str, name: &str, low: &suspect_low::LowDoc) -> Option<String> {
    // Navigate to the component in the live document tree.
    let ptr =
        suspect_low::Pointer::from_tokens(vec!["components".into(), section.into(), name.into()]);
    let target = low.root().pointer(&ptr)?;
    Some(render_component(&target, section, name))
}

/// Dispatches the rich renderer for one component node — works against
/// any document, so cross-file `$ref` targets render as richly as local
/// ones.
#[must_use]
pub fn render_component(target: &NodeRef<'_>, section: &str, name: &str) -> String {
    match section {
        "schemas" => render_schema_node(target, name),
        "securitySchemes" => render_security_scheme(target, name),
        "parameters" | "headers" => render_parameter(target, name),
        "responses" => render_response(target, name),
        "examples" => render_example(target, name),
        "requestBodies" => render_request_body(target, name),
        _ => String::new(),
    }
}

/// True when `section` has a rich renderer.
#[must_use]
pub fn has_renderer(section: &str) -> bool {
    matches!(
        section,
        "schemas"
            | "securitySchemes"
            | "parameters"
            | "headers"
            | "responses"
            | "examples"
            | "requestBodies"
    )
}

/// Security scheme: type/in/scheme table plus flows.
pub fn render_security_scheme(scheme: &NodeRef<'_>, name: &str) -> String {
    let mut md = format!("**🔑 {name}**");
    if let Some(d) = scheme.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{d}"));
    }
    md.push_str("\n\n| Field | Value |");
    md.push_str("\n|---|---|");
    for (field, label) in [
        ("type", "Type"),
        ("in", "In"),
        ("scheme", "Scheme"),
        ("bearerFormat", "Bearer format"),
        ("openIdConnectUrl", "OIDC URL"),
    ] {
        if let Some(v) = scheme.get(field).and_then(|n| n.as_str()) {
            md.push_str(&format!("\n| {label} | `{v}` |"));
        }
    }
    if let Some(flows) = scheme.get("flows") {
        for (flow_name, _flow) in [
            ("authorizationCode", "Authorization code"),
            ("clientCredentials", "Client credentials"),
            ("implicit", "Implicit"),
            ("password", "Password"),
        ] {
            if let Some(flow) = flows.get(flow_name) {
                md.push_str(&format!("\n\n**{} flow:**", flow_name));
                if let Some(url) = flow.get("tokenUrl").and_then(|n| n.as_str()) {
                    md.push_str(&format!("\n- token: `{url}`"));
                }
                if let Some(url) = flow.get("authorizationUrl").and_then(|n| n.as_str()) {
                    md.push_str(&format!("\n- authorize: `{url}`"));
                }
                if let Some(scopes) = flow.get("scopes") {
                    let scope_names: Vec<&str> = scopes.entries().iter().map(|e| e.key).collect();
                    if !scope_names.is_empty() {
                        md.push_str(&format!(
                            "\n- scopes: {}",
                            scope_names
                                .iter()
                                .map(|s| format!("`{s}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                }
                let _ = flow; // entries used above via get
            }
        }
    }
    md
}

/// Parameter/header: location, requirement, schema summary.
pub fn render_parameter(param: &NodeRef<'_>, name: &str) -> String {
    let mut md = format!("**{name}**");
    let loc = param.get("in").and_then(|n| n.as_str());
    if let Some(loc) = loc {
        md.push_str(&format!(" (`in: {loc}`)"));
    }
    if param.get("required").and_then(|n| n.as_bool()) == Some(true) {
        md.push_str(" — *required*");
    }
    if let Some(d) = param.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{d}"));
    }
    if let Some(schema) = param.get("schema") {
        if let Some(ts) = get_type(&schema) {
            md.push_str(&format!("\n\n**Type:** {}", render_types(&ts)));
        }
        if let Some(default) = schema.get("default") {
            let dv = suspect_overlay::Value::from_node(default).to_json();
            md.push_str(&format!("\n\n**Default:** `{dv}`"));
        }
        if let Some(enum_) = schema.get("enum") {
            let vals: Vec<String> = enum_
                .items()
                .iter()
                .map(|v| format!("`{}`", String::from_utf8_lossy(v.scalar_bytes())))
                .collect();
            if !vals.is_empty() {
                md.push_str(&format!("\n\n**Enum:** {}", vals.join(" · ")));
            }
        }
    }
    md
}

/// Response: description, headers, content types.
pub fn render_response(response: &NodeRef<'_>, name: &str) -> String {
    let mut md = format!("**{name}**");
    if let Some(d) = response.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{d}"));
    }
    if let Some(headers) = response.get("headers")
        && !headers.entries().is_empty()
    {
        md.push_str("\n\n**Headers:**");
        md.push_str("\n\n| Name | Type |");
        md.push_str("\n|---|---|");
        for h in headers.entries() {
            let ts = h
                .value
                .and_then(|v| v.get("schema"))
                .and_then(|s| get_type(&s))
                .map_or_else(|| "—".to_owned(), |t| render_types(&t));
            md.push_str(&format!("\n| `{}` | {ts} |", h.key));
        }
    }
    if let Some(content) = response.get("content")
        && !content.entries().is_empty()
    {
        md.push_str("\n\n**Content:** ");
        let media: Vec<String> = content
            .entries()
            .iter()
            .map(|e| format!("`{}`", e.key))
            .collect();
        md.push_str(&media.join(", "));
    }
    md
}

/// Example: summary, description, pretty value.
pub fn render_example(example: &NodeRef<'_>, name: &str) -> String {
    let mut md = format!("**{name}**");
    if let Some(s) = example.get("summary").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{s}"));
    }
    if let Some(d) = example.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n---\n\n{d}"));
    }
    if let Some(value) = example.get("value") {
        let json = suspect_overlay::Value::from_node(value).to_json_pretty();
        md.push_str(&format!("\n\n```json\n{json}\n```"));
    }
    if let Some(external) = example.get("externalValue").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n*External:* `{external}`"));
    }
    md
}

/// Request body: required + content types.
pub fn render_request_body(body: &NodeRef<'_>, name: &str) -> String {
    let mut md = format!("**{name}**");
    if body.get("required").and_then(|n| n.as_bool()) == Some(true) {
        md.push_str(" — *required*");
    }
    if let Some(d) = body.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{d}"));
    }
    if let Some(content) = body.get("content") {
        md.push_str("\n\n**Content:** ");
        let media: Vec<String> = content
            .entries()
            .iter()
            .map(|e| {
                let type_str = e
                    .value
                    .and_then(|v| v.get("schema"))
                    .and_then(|s| get_type(&s))
                    .map_or_else(|| "—".to_owned(), |t| render_types(&t));
                format!("`{}` ({type_str})", e.key)
            })
            .collect();
        md.push_str(&media.join(", "));
    }
    md
}

/// Renders structured markdown for a schema node.
#[must_use]
pub fn render_schema_node(schema: &NodeRef<'_>, name: &str) -> String {
    let mut md = String::new();

    // Title + type badge
    md.push_str(&format!("**{name}**"));
    if let Some(ts) = get_type(schema) {
        md.push_str(&format!(" `{}`", render_types(&ts)));
    }
    if is_deprecated(schema) {
        md.push_str(" ~~deprecated~~");
    }

    if let Some(desc) = string_value(schema, "description") {
        md.push_str(&format!("\n\n{desc}"));
    }

    // Enum values
    if let Some(enum_node) = schema.get("enum") {
        let vals: Vec<String> = enum_node
            .items()
            .iter()
            .map(|v| format!("`{}`", String::from_utf8_lossy(v.scalar_bytes())))
            .collect();
        if !vals.is_empty() {
            md.push_str("\n\n**Enum:** ");
            md.push_str(&vals.join(" · "));
        }
    }

    // Properties table
    if let Some(props) = schema
        .get("properties")
        .filter(|p| matches!(p.kind(), suspect_low::ValueKind::Object))
    {
        let required = schema
            .get("required")
            .map(|r| {
                r.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let entries = props.entries();
        if !entries.is_empty() {
            md.push_str("\n\n| Property | Type | Required | Description |");
            md.push_str("\n|---|---|---|---|");
            for entry in entries {
                let pname = entry.key;
                let req = if required.iter().any(|r| r.as_str() == pname) {
                    "✓"
                } else {
                    ""
                };
                let prop_schema = entry.value.unwrap();
                let type_str =
                    get_type(&prop_schema).map_or_else(|| "—".to_owned(), |t| render_types(&t));
                let desc = prop_schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                md.push_str(&format!("\n| `{pname}` | {type_str} | {req} | {desc} |"));
            }
        }
    }

    push_constraints(schema, &mut md);
    md
}

/// Renders structured markdown for an operation node.
#[must_use]
pub fn render_operation_node(op: &NodeRef<'_>, method: &str, path: &str) -> String {
    let mut md = format!("**{}** `{path}`", method.to_uppercase());

    if let Some(s) = op.get("summary").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n{s}"));
    }
    if let Some(d) = op.get("description").and_then(|n| n.as_str()) {
        md.push_str(&format!("\n\n---\n\n{d}"));
    }
    if op.get("deprecated").and_then(|n| n.as_bool()) == Some(true) {
        md.push_str("\n\n⚠ *Deprecated*");
    }

    // Parameters
    if let Some(params) = op.get("parameters") {
        let items = params.items();
        if !items.is_empty() {
            md.push_str("\n\n**Parameters:**\n");
            md.push_str("\n| Name | In | Type | Required |");
            md.push_str("\n|---|---|---|---|");
            for p in items {
                let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let loc = p.get("in").and_then(|n| n.as_str()).unwrap_or("?");
                let ts = p
                    .get("schema")
                    .and_then(|s| get_type(&s))
                    .map_or_else(|| "—".to_owned(), |t| render_types(&t));
                let req = if p.get("required").and_then(|n| n.as_bool()) == Some(true) {
                    "✓"
                } else {
                    ""
                };
                md.push_str(&format!("\n| `{name}` | {loc} | {ts} | {req} |"));
            }
        }
    }

    // Responses
    if let Some(responses) = op.get("responses") {
        let codes: Vec<String> = responses
            .entries()
            .iter()
            .map(|e| format!("**{}**", e.key))
            .collect();
        if !codes.is_empty() {
            md.push_str(&format!("\n\n**Responses:** {}", codes.join(", ")));
        }
    }

    md
}

// ---- helpers ----

/// Gets the declared type set from a schema node as a bitmask-compatible
/// string list.
fn get_type(schema: &NodeRef<'_>) -> Option<Vec<String>> {
    match schema.get("type") {
        Some(t) => match t.kind() {
            ValueKind::Str => t.as_str().map(|s| vec![s.to_owned()]),
            ValueKind::Array => Some(
                t.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect(),
            ),
            _ => None,
        },
        None => {
            // Infer from sibling keywords
            let mut types = Vec::new();
            if schema.get("properties").is_some() || schema.get("required").is_some() {
                types.push("object".to_owned());
            }
            if schema.get("items").is_some() || schema.get("prefixItems").is_some() {
                types.push("array".to_owned());
            }
            (!types.is_empty()).then_some(types)
        }
    }
}

fn is_deprecated(schema: &NodeRef<'_>) -> bool {
    schema
        .get("deprecated")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
}

fn string_value(schema: &NodeRef<'_>, key: &str) -> Option<String> {
    schema.get(key).and_then(|n| n.as_str()).map(String::from)
}

fn render_types(types: &[String]) -> String {
    types.join(" | ")
}

fn push_constraints(schema: &NodeRef<'_>, md: &mut String) {
    let mut parts = Vec::new();
    macro_rules! num_constraint {
        ($key:expr, $label:expr) => {
            if let Some(v) = schema.get($key).and_then(|n| n.as_f64()) {
                parts.push(format!("{}: {}", $label, v));
            }
        };
    }
    num_constraint!("minimum", "min");
    num_constraint!("exclusiveMinimum", "min (excl)");
    num_constraint!("maximum", "max");
    num_constraint!("exclusiveMaximum", "max (excl)");
    num_constraint!("multipleOf", "multiple of");
    num_constraint!("minLength", "minLength");
    num_constraint!("maxLength", "maxLength");
    if let Some(p) = schema.get("pattern").and_then(|n| n.as_str()) {
        parts.push(format!("pattern: `{p}`"));
    }
    num_constraint!("minItems", "minItems");
    num_constraint!("maxItems", "maxItems");
    if schema.get("uniqueItems").and_then(|n| n.as_bool()) == Some(true) {
        parts.push("unique items".to_owned());
    }
    if let Some(f) = schema.get("format").and_then(|n| n.as_str()) {
        parts.push(format!("format: `{f}`"));
    }
    if !parts.is_empty() {
        md.push_str("\n\n**Constraints:** ");
        md.push_str(&parts.join(" · "));
    }
}
