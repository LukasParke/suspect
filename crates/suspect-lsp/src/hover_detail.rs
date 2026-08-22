//! Rich hover markdown renderers built directly from [`suspect_low::NodeRef`]
//! — no session or typed-view dependency. Produces structured markdown
//! tables so hover answers "what is this?" without navigating away.

use suspect_low::{NodeRef, ValueKind};

/// Attempts a rich markdown hover when the resolved target is under
/// `components/<section>/<Name>`.
#[must_use]
pub fn try_rich_hover(section: &str, name: &str, low: &suspect_low::LowDoc) -> Option<String> {
    if section != "schemas" {
        return None;
    }
    // Navigate to the component in the live document tree.
    let ptr =
        suspect_low::Pointer::from_tokens(vec!["components".into(), section.into(), name.into()]);
    let target = low.root().pointer(&ptr)?;
    Some(render_schema_node(&target, name))
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
