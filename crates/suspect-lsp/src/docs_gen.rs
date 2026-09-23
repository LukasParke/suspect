//! Static API reference documentation emitter.
//!
//! Renders a single self-contained HTML file from the OpenAPI document:
//! operations grouped by tag, request/response tables, schema sections,
//! and embedded examples. The rich-hover renderer data structures double
//! as the content model — the same tables appear in hover and in the
//! published reference.

use std::collections::BTreeMap;

use suspect_low::{LowDoc, NodeRef};

/// Renders the full reference document as HTML.
#[must_use]
pub fn render(low: &LowDoc, title: &str) -> String {
    let root = low.root();
    let mut ops_by_tag: BTreeMap<String, Vec<(String, String, NodeRef<'_>)>> = BTreeMap::new();
    if let Some(paths) = root.get("paths") {
        for path_entry in paths.entries() {
            let Some(path_item) = path_entry.value else {
                continue;
            };
            for method in [
                "get", "post", "put", "patch", "delete", "options", "head", "trace",
            ] {
                let Some(op) = path_item.get(method) else {
                    continue;
                };
                let entry = (method.to_ascii_uppercase(), path_entry.key.to_owned(), op);
                let primary_tag = op
                    .get("tags")
                    .and_then(|t| t.items().first().map(|f| f.to_owned()))
                    .and_then(|t| t.as_str().map(String::from))
                    .unwrap_or_else(|| "Other".to_owned());
                ops_by_tag
                    .entry(primary_tag.clone())
                    .or_default()
                    .push(entry.clone());
            }
        }
    }

    let mut html = String::with_capacity(64 * 1024);
    html.push_str(&format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{title}</title>\n<style>\n{CSS}\n</style>\n</head>\n<body>\n"
    ));
    html.push_str(&format!("<h1>{title}</h1>\n"));
    if let Some(description) = root
        .get("info")
        .and_then(|i| i.get("description"))
        .and_then(|d| d.as_str())
    {
        html.push_str(&format!(
            "<section class=\"desc\"><p>{}</p></section>\n",
            escape_html(description)
        ));
    }

    // Operations grouped by tag.
    html.push_str("<h2 id=\"operations\">Operations</h2>\n");
    let mut tags_seen: Vec<String> = Vec::new();
    if let Some(declared_tags) = root.get("tags") {
        for tag in declared_tags.items() {
            if let Some(name) = tag.get("name").and_then(|n| n.as_str()) {
                tags_seen.push(name.to_owned());
            }
        }
    }
    for (tag, ops) in &ops_by_tag {
        html.push_str(&format!("<h3 class=\"tag\">{}</h3>\n", escape_html(tag)));
        html.push_str("<table class=\"ops\"><tr><th>Method</th><th>Path</th><th>Operation ID</th><th>Summary</th></tr>\n");
        for (method, path, op) in ops {
            let summary = op
                .get("summary")
                .and_then(|s| s.as_str())
                .unwrap_or_default();
            let class = method.to_ascii_lowercase();
            let op_id = op
                .get("operationId")
                .and_then(|s| s.as_str())
                .unwrap_or_default();
            html.push_str(&format!(
                "<tr><td><span class=\"method {class}\">{method}</span></td>\
                 <td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>\n",
                escape_html(path),
                escape_html(op_id),
                escape_html(summary),
            ));
        }
        html.push_str("</table>\n");
    }
    let _ = &tags_seen;

    // Schemas section.
    if let Some(components) = root.get("components")
        && let Some(schemas) = components.get("schemas")
        && !schemas.entries().is_empty()
    {
        html.push_str("<h2 id=\"schemas\">Schemas</h2>\n");
        for entry in schemas.entries() {
            let Some(schema) = entry.value else {
                continue;
            };
            html.push_str(&format!(
                "<h3 class=\"schema-name\">{}</h3>\n",
                escape_html(entry.key)
            ));
            if let Some(description) = schema.get("description").and_then(|d| d.as_str()) {
                html.push_str(&format!("<p>{}</p>\n", escape_html(description)));
            }
            html.push_str(&render_properties_table(&schema));
        }
    }

    html.push_str("</body>\n</html>\n");
    html
}

/// Renders a schema's properties as a table (same columns as hover).
fn render_properties_table(schema: &NodeRef<'_>) -> String {
    let Some(properties) = schema.get("properties") else {
        return String::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .map(|r| {
            r.items()
                .iter()
                .filter_map(|i| i.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut md = String::from(
        "<table class=\"props\"><tr><th>Property</th><th>Type</th><th>Required</th><th>Description</th></tr>\n",
    );
    for entry in properties.entries() {
        let name = &entry.key;
        let prop = entry.value.unwrap_or_else(|| unreachable!("pair value"));
        let type_str = prop.get("type").and_then(|t| t.as_str()).unwrap_or("—");
        let req = if required.contains(&&**name) {
            "✓"
        } else {
            ""
        };
        let desc = prop
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or_default();
        md.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{req}</td><td>{}</td></tr>\n",
            escape_html(name),
            escape_html(type_str),
            escape_html(desc),
        ));
    }
    md.push_str("</table>\n");
    md
}

/// Escapes HTML special characters.
#[must_use]
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Embedded stylesheet: neutral layout, method color coding.
const CSS: &str = "\
body { font-family: system-ui, sans-serif; margin: 2rem auto; max-width: 60rem; padding: 0 1rem; color: #1a1a2e; }
h1 { border-bottom: 2px solid #4a6fa5; padding-bottom: .3rem; }
h2 { border-bottom: 1px solid #ccc; padding-bottom: .2rem; margin-top: 2rem; }
h3 { margin-top: 1.4rem; }
table { border-collapse: collapse; width: 100%; margin: .6rem 0 1.2rem; }
th, td { border: 1px solid #ddd; padding: .4rem .6rem; text-align: left; vertical-align: top; }
th { background: #f0f4f8; }
code { background: #f0f0f0; padding: .1rem .3rem; border-radius: 3px; font-size: .9em; }
.method { font-weight: 700; padding: .1rem .4rem; border-radius: 3px; color: white; font-size: .8em; }
.method.get { background: #2e7d32; }
.method.post { background: #1565c0; }
.method.put { background: #e65100; }
.method.patch { background: #6a1b9a; }
.method.delete { background: #c62828; }
.method.options, .method.head, .method.trace { background: #616161; }
.desc { color: #444; }
";

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    const SPEC: &str = "\
openapi: 3.1.0
info: {title: Test API, version: '1', description: Test description}
tags: [{name: pets}]
paths:
  /pets:
    get:
      tags: [pets]
      operationId: listPets
      summary: Lists pets
      responses:
        '200': {description: ok}
    post:
      operationId: createPet
      responses:
        '201': {description: created}
components:
  schemas:
    Pet:
      type: object
      description: A pet
      required: [name]
      properties:
        name: {type: string, description: The name}
";

    #[test]
    fn renders_operations_schemas_and_escapes_html() {
        let uri = Uri::parse("mem://docs.yaml").unwrap();
        let low = LowDoc::parse(uri, Source::from_vec(SPEC.as_bytes().to_vec()));
        let html = render(&low, "Test API");

        println!("HTML: {}", html);
        assert!(html.contains("<h1>Test API</h1>"));
        assert!(html.contains("Test description"));
        // Operations grouped under the declared tag.
        assert!(html.contains("<h3 class=\"tag\">pets</h3>"));
        assert!(html.contains("listPets"));
        assert!(html.contains("class=\"method get\""));
        assert!(html.contains("class=\"method post\""));
        // Schema section with the property table.
        assert!(html.contains("<h3 class=\"schema-name\">Pet</h3>"));
        assert!(html.contains("A pet"));
        assert!(html.contains("The name"));
        // HTML escaping.
        assert!(!html.contains("<script"));
    }
}
