//! Single-file HTML reference emitter.

use super::{DocModel, escape_html};

/// Renders the full reference document as one self-contained HTML file.
#[must_use]
pub fn render_html(model: &DocModel) -> String {
    let mut html = String::with_capacity(64 * 1024);
    html.push_str(&format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{}</title>\n<style>\n{CSS}\n</style>\n</head>\n<body>\n",
        escape_html(&model.title)
    ));
    html.push_str(&format!("<h1>{}</h1>\n", escape_html(&model.title)));
    if let Some(description) = &model.description {
        html.push_str(&format!(
            "<section class=\"desc\"><p>{}</p></section>\n",
            escape_html(description)
        ));
    }
    html.push_str(&format!(
        "<p class=\"meta\">Version {}</p>\n",
        escape_html(&model.version)
    ));

    html.push_str("<h2 id=\"operations\">Operations</h2>\n");
    for group in &model.groups {
        html.push_str(&format!(
            "<h3 class=\"tag\">{}</h3>\n",
            escape_html(&group.tag)
        ));
        html.push_str(
            "<table class=\"ops\"><tr><th>Method</th><th>Path</th><th>Operation ID</th>\
             <th>Summary</th></tr>\n",
        );
        for op in &group.operations {
            let class = op.method.to_ascii_lowercase();
            html.push_str(&format!(
                "<tr><td><span class=\"method {class}\">{}</span></td>\
                 <td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>\n",
                escape_html(&op.method),
                escape_html(&op.path),
                escape_html(&op.operation_id),
                escape_html(&op.summary),
            ));
        }
        html.push_str("</table>\n");
    }

    if !model.schemas.is_empty() {
        html.push_str("<h2 id=\"schemas\">Schemas</h2>\n");
        for schema in &model.schemas {
            html.push_str(&format!(
                "<h3 class=\"schema-name\">{}</h3>\n",
                escape_html(&schema.name)
            ));
            if !schema.description.is_empty() {
                html.push_str(&format!("<p>{}</p>\n", escape_html(&schema.description)));
            }
            html.push_str(&render_properties_table(schema));
        }
    }

    html.push_str("</body>\n</html>\n");
    html
}

/// Renders a schema's properties as a table (same columns as hover).
fn render_properties_table(schema: &super::SchemaDoc) -> String {
    let mut md = String::from(
        "<table class=\"props\"><tr><th>Property</th><th>Type</th><th>Required</th>\
         <th>Description</th></tr>\n",
    );
    for prop in &schema.properties {
        let req = if prop.required { "✓" } else { "" };
        md.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{req}</td><td>{}</td></tr>\n",
            escape_html(&prop.name),
            escape_html(&prop.type_desc),
            escape_html(&prop.description),
        ));
    }
    md.push_str("</table>\n");
    md
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
.meta { color: #666; }
";

#[cfg(test)]
mod tests {
    use super::super::extract;
    use super::*;

    #[test]
    fn renders_operations_schemas_and_escapes_html() {
        let model = extract(&super::super::fixture::sample());
        let html = render_html(&model);

        assert!(html.contains("<h1>Test API</h1>"));
        assert!(html.contains("Test description"));
        assert!(html.contains("Version 1.4.2"));
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
