//! Rich hover markdown renderers built directly from [`suspect_low::NodeRef`]
//! — no session or typed-view dependency. Produces structured markdown so
//! hover answers "what is this?" without navigating away.
//!
//! Every card shares one frame, so the whole hover surface reads as one
//! design rather than five: a `### \`name\`` heading, one italic subtitle
//! naming what the card is, prose where prose belongs (OpenAPI
//! `description` fields are CommonMark and render as markdown, never as
//! code), `| Field | Value |` tables for facts, and fenced code only for
//! values that *are* code.

use suspect_low::{NodeRef, ValueKind};

/// The heading + italic-subtitle frame every hover card opens with.
fn heading(name: &str, subtitle: &str) -> String {
    format!("### `{name}`\n\n*{subtitle}*")
}

/// Reflows a value for a markdown table cell: a newline would break the
/// row and a pipe would break the columns, so both are neutralised. The
/// words survive, and so does the table — inline markdown (bold, code,
/// links) keeps rendering inside the cell.
fn cell(text: &str) -> String {
    text.replace(['\r', '\n'], " ").replace('|', "\\|")
}

/// A table cell holding a literal value, in code font.
fn code_cell(text: &str) -> String {
    format!("`{}`", cell(text))
}

/// A `| Field | Value |` table from label/value facts; empty when there
/// are no facts, so cards never carry a bare header row.
fn facts_table(rows: &[(&str, String)]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut md = String::from("\n\n| Field | Value |\n|---|---|");
    for (label, value) in rows {
        md.push_str(&format!("\n| {label} | {value} |"));
    }
    md
}

/// Reads a prose field (description, summary): the value decoded, not its
/// YAML source. A description written as a block scalar is CommonMark
/// once decoded; read raw it carries the `>-` marker and the original
/// indentation, which is source, not prose. Empty text reads as absent —
/// an empty string contributes no card section.
fn prose_field(node: &NodeRef<'_>, key: &str) -> Option<String> {
    prose(&node.get(key)?)
}

/// Decodes one string node's prose, as [`prose_field`] reads it.
fn prose(node: &NodeRef<'_>) -> Option<String> {
    if node.kind() != ValueKind::Str {
        return None;
    }
    let text = String::from_utf8_lossy(&node.decoded_scalar())
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

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
        "parameters" => render_parameter(target, name, "parameter"),
        "headers" => render_parameter(target, name, "header"),
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
    let mut md = format!("### 🔑 `{name}`\n\n*security scheme*");
    if let Some(d) = prose_field(scheme, "description") {
        md.push_str(&format!("\n\n{d}"));
    }
    let mut rows: Vec<(&str, String)> = Vec::new();
    for (field, label) in [
        ("type", "Type"),
        ("in", "In"),
        ("scheme", "Scheme"),
        ("bearerFormat", "Bearer format"),
        ("openIdConnectUrl", "OIDC URL"),
    ] {
        if let Some(v) = scheme.get(field).and_then(|n| n.as_str()) {
            rows.push((label, code_cell(v)));
        }
    }
    md.push_str(&facts_table(&rows));
    if let Some(flows) = scheme.get("flows") {
        for (flow_name, label) in [
            ("authorizationCode", "Authorization code"),
            ("clientCredentials", "Client credentials"),
            ("implicit", "Implicit"),
            ("password", "Password"),
        ] {
            let Some(flow) = flows.get(flow_name) else {
                continue;
            };
            md.push_str(&format!("\n\n**{label} flow:**"));
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
        }
    }
    md
}

/// Parameter/header: location, requirement, schema summary.
///
/// `what` names the card ("parameter" or "header") since the renderer
/// serves both component sections and the `in` value alone does not
/// distinguish them.
pub fn render_parameter(param: &NodeRef<'_>, name: &str, what: &str) -> String {
    let mut subtitle = param
        .get("in")
        .and_then(|n| n.as_str())
        .map_or_else(|| what.to_owned(), |loc| format!("`{loc}` {what}"));
    if param.get("required").and_then(|n| n.as_bool()) == Some(true) {
        subtitle.push_str(" — required");
    }
    let mut md = heading(name, &subtitle);
    if let Some(d) = prose_field(param, "description") {
        md.push_str(&format!("\n\n{d}"));
    }
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(schema) = param.get("schema") {
        if let Some(ts) = get_type(&schema) {
            rows.push(("Type", code_cell(&render_types(&ts))));
        }
        if let Some(default) = schema.get("default") {
            let dv = suspect_overlay::Value::from_node(default).to_json();
            rows.push(("Default", code_cell(&dv)));
        }
        if let Some(enum_) = schema.get("enum") {
            let vals: Vec<String> = enum_
                .items()
                .iter()
                .map(|v| code_cell(&String::from_utf8_lossy(v.scalar_bytes())))
                .collect();
            if !vals.is_empty() {
                rows.push(("Enum", vals.join(" · ")));
            }
        }
    }
    md.push_str(&facts_table(&rows));
    md
}

/// Response: description, headers, content types.
pub fn render_response(response: &NodeRef<'_>, name: &str) -> String {
    let mut md = heading(name, "response");
    if let Some(d) = prose_field(response, "description") {
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
                .map_or_else(|| "—".to_owned(), |t| code_cell(&render_types(&t)));
            md.push_str(&format!("\n| {} | {ts} |", code_cell(h.key)));
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
    let mut md = heading(name, "example");
    if let Some(s) = prose_field(example, "summary") {
        md.push_str(&format!("\n\n**{s}**"));
    }
    if let Some(d) = prose_field(example, "description") {
        md.push_str(&format!("\n\n{d}"));
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
    let mut subtitle = String::from("request body");
    if body.get("required").and_then(|n| n.as_bool()) == Some(true) {
        subtitle.push_str(" — required");
    }
    let mut md = heading(name, &subtitle);
    if let Some(d) = prose_field(body, "description") {
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
    // The subtitle names the state first — "deprecated schema" reads as a
    // fact; `~~deprecated~~` next to a heading reads as damaged text.
    let mut subtitle = if is_deprecated(schema) {
        "deprecated schema".to_owned()
    } else {
        "schema".to_owned()
    };
    if let Some(ts) = get_type(schema) {
        subtitle.push_str(&format!(" — `{}`", render_types(&ts)));
    }
    let mut md = heading(name, &subtitle);

    // The description is CommonMark: raw, so the author's markdown
    // renders in the card the way it renders in generated docs.
    if let Some(desc) = prose_field(schema, "description") {
        md.push_str(&format!("\n\n{desc}"));
    }

    // Enum values
    if let Some(enum_node) = schema.get("enum") {
        let vals: Vec<String> = enum_node
            .items()
            .iter()
            .map(|v| code_cell(&String::from_utf8_lossy(v.scalar_bytes())))
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
                let type_str = get_type(&prop_schema)
                    .map_or_else(|| "—".to_owned(), |t| code_cell(&render_types(&t)));
                // The cell carries the description reflowed onto one line
                // with its pipes escaped; decoding first keeps block-scalar
                // descriptions from arriving as their YAML source.
                let desc = prop_schema
                    .get("description")
                    .map(|d| cell(&prose(&d).unwrap_or_default()))
                    .unwrap_or_default();
                md.push_str(&format!(
                    "\n| {} | {type_str} | {req} | {desc} |",
                    code_cell(pname)
                ));
            }
        }
    }

    push_constraints(schema, &mut md);
    md
}

/// Renders structured markdown for an operation node.
#[must_use]
pub fn render_operation_node(op: &NodeRef<'_>, method: &str, path: &str) -> String {
    let mut md = heading(&format!("{} {path}", method.to_uppercase()), "operation");

    if let Some(s) = prose_field(op, "summary") {
        md.push_str(&format!("\n\n**{s}**"));
    }
    if let Some(d) = prose_field(op, "description") {
        md.push_str(&format!("\n\n{d}"));
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
                    .map_or_else(|| "—".to_owned(), |t| code_cell(&render_types(&t)));
                let req = if p.get("required").and_then(|n| n.as_bool()) == Some(true) {
                    "✓"
                } else {
                    ""
                };
                md.push_str(&format!(
                    "\n| {} | {} | {ts} | {req} |",
                    code_cell(name),
                    code_cell(loc)
                ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_low::LowDoc;
    use suspect_source::{Source, Uri};

    /// Renders the schema card for `name` in a parsed component document.
    fn schema_card(text: &str, name: &str) -> String {
        let low = LowDoc::parse(
            Uri::parse("file:///t.yaml").unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        );
        let ptr = suspect_low::Pointer::from_tokens(vec![
            "components".into(),
            "schemas".into(),
            name.into(),
        ]);
        let target = low.root().pointer(&ptr).unwrap();
        render_schema_node(&target, name)
    }

    /// The card frame: heading, italic subtitle, prose, table — in that
    /// order, so the whole hover surface reads as one design.
    #[test]
    fn schema_card_opens_with_the_heading_frame() {
        let md = schema_card(
            "components:\n  schemas:\n    Pet:\n      type: object\n",
            "Pet",
        );
        assert!(md.starts_with("### `Pet`\n\n*schema — `object`*"), "{md}");
    }

    /// OpenAPI `description` fields are CommonMark: the card body carries
    /// them raw, so markdown renders as markdown — never inside a code
    /// fence or inline code where it would show as source.
    #[test]
    fn the_schema_description_renders_as_markdown() {
        let md = schema_card(
            "components:\n  schemas:\n    Pet:\n      type: object\n      description: A **pet** with a [docs page](https://example.com).\n",
            "Pet",
        );
        let at = md.find("A **pet**").expect("description present");
        let before = &md[..at];
        assert!(
            !before.ends_with("```\n"),
            "description must not open a code fence: {md}"
        );
        assert!(
            md.contains("A **pet** with a [docs page](https://example.com)."),
            "{md}"
        );
    }

    /// A multi-line property description reflows onto one table row: the
    /// newline would split the row and leave every later property
    /// rendered as raw pipe-delimited text — the "descriptions show as
    /// plain text" failure mode.
    #[test]
    fn multiline_property_descriptions_keep_the_table_intact() {
        let md = schema_card(
            "components:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        name:\n          type: string\n          description: |-\n            First line of the\n            description body.\n        age:\n          type: integer\n",
            "Pet",
        );
        let property_rows = md
            .lines()
            .filter(|l| l.starts_with("| `"))
            .collect::<Vec<_>>();
        assert_eq!(property_rows.len(), 2, "{md}");
        assert!(md.contains("First line of the description body."), "{md}");
        assert!(property_rows[1].contains("`age`"), "{md}");
        assert!(property_rows[1].contains("`integer`"), "{md}");
    }

    /// A `type: [string, null]` value contains a pipe, which would break
    /// a table cell column; cells escape it instead.
    #[test]
    fn union_types_survive_table_cells() {
        let md = schema_card(
            "components:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        name:\n          type: [string, \"null\"]\n",
            "Pet",
        );
        assert!(md.contains("`string \\| null`"), "{md}");
    }
}
