//! Structured Markdown emitter — one file per tag and per schema, with
//! YAML frontmatter, ready for MDX/MDsvex pipelines.
//!
//! Output layout (relative to `--out`):
//!
//! ```text
//! index.md                 overview, servers, table of contents
//! toc.json                 machine-readable table of contents
//! operations/<slug>.md     one page per tag
//! schemas/<Name>.md        one page per component schema
//! ```
//!
//! Pages carry `title`/`description` frontmatter and standard Markdown
//! tables (no HTML), so any MDX/MDsvex site can consume them directly or
//! import the tables as components.

use std::collections::BTreeMap;
use std::path::Path;

use super::{DocModel, slugify};

/// What one Markdown render wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownReport {
    /// Files written, relative to the output directory.
    pub files: Vec<String>,
}

/// Renders the reference as structured Markdown into `out_dir`.
///
/// # Errors
/// Propagates filesystem failures.
pub fn render_markdown(model: &DocModel, out_dir: &Path) -> std::io::Result<MarkdownReport> {
    std::fs::create_dir_all(out_dir)?;
    std::fs::create_dir_all(out_dir.join("operations"))?;
    std::fs::create_dir_all(out_dir.join("schemas"))?;
    let mut files = Vec::new();

    // Per-tag pages first so the index can link them. Slugs are
    // deduplicated within their own directory namespace.
    let mut issued_operation_slugs: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let mut group_links: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for group in &model.groups {
        let slug = unique_slug(&slugify(&group.tag), &mut issued_operation_slugs);
        let file = format!("operations/{slug}.md");
        let count = group.operations.len();
        std::fs::write(out_dir.join(&file), render_group(model, group, &file))?;
        files.push(file.clone());
        group_links.insert(group.tag.clone(), (file, count));
    }

    // Per-schema pages.
    let mut schema_links: BTreeMap<String, String> = BTreeMap::new();
    for schema in &model.schemas {
        let file = format!("schemas/{}.md", schema.name);
        std::fs::write(out_dir.join(&file), render_schema(schema))?;
        files.push(file.clone());
        schema_links.insert(schema.name.clone(), file);
    }

    // Index.
    let index = render_index(model, &group_links, &schema_links);
    std::fs::write(out_dir.join("index.md"), &index)?;
    files.push("index.md".to_owned());

    // Machine-readable table of contents.
    let toc = toc_json(model, &group_links, &schema_links);
    std::fs::write(out_dir.join("toc.json"), &toc)?;
    files.push("toc.json".to_owned());

    files.sort();
    Ok(MarkdownReport { files })
}

/// Deduplicates a slug against already-issued ones with numeric suffixes.
fn unique_slug(base: &str, issued: &mut std::collections::BTreeSet<String>) -> String {
    let mut candidate = base.to_owned();
    let mut suffix = 2;
    while issued.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    issued.insert(candidate.clone());
    candidate
}

/// YAML frontmatter + body.
fn frontmatter(title: &str, description: &str) -> String {
    let mut out = String::from("---\ntitle: ");
    out.push_str(&frontmatter_scalar(title));
    out.push('\n');
    if !description.is_empty() {
        out.push_str("description: ");
        out.push_str(&frontmatter_scalar(description));
        out.push('\n');
    }
    out.push_str("---\n\n");
    out
}

/// Frontmatter scalars: quoted, with internal quotes doubled (YAML '' escape).
fn frontmatter_scalar(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The overview page.
fn render_index(
    model: &DocModel,
    group_links: &BTreeMap<String, (String, usize)>,
    schema_links: &BTreeMap<String, String>,
) -> String {
    let mut out = frontmatter(&model.title, model.description.as_deref().unwrap_or(""));
    out.push_str(&format!("# {}\n\n", markdown_escape(&model.title)));
    if !model.version.is_empty() {
        out.push_str(&format!(
            "**Version:** {}\n\n",
            markdown_escape(&model.version)
        ));
    }
    if let Some(description) = &model.description {
        out.push_str(description);
        out.push_str("\n\n");
    }
    if !model.servers.is_empty() {
        out.push_str("## Servers\n\n");
        for server in &model.servers {
            out.push_str(&format!("- `{}`\n", markdown_escape(server)));
        }
        out.push('\n');
    }
    out.push_str("## Operations\n\n");
    if group_links.is_empty() {
        out.push_str("_This API declares no operations._\n\n");
    } else {
        out.push_str("| Tag | Operations | Docs |\n|---|---|---|\n");
        for group in &model.groups {
            let Some((file, count)) = group_links.get(&group.tag) else {
                continue;
            };
            out.push_str(&format!(
                "| {} | {count} | [{}](/{file}) |\n",
                markdown_escape(&group.tag),
                markdown_escape(file.trim_end_matches(".md")),
            ));
        }
        out.push('\n');
    }
    if !schema_links.is_empty() {
        out.push_str("## Schemas\n\n| Schema | Docs |\n|---|---|\n");
        for (name, file) in schema_links {
            out.push_str(&format!(
                "| `{}` | [{}](/{file}) |\n",
                markdown_escape(name),
                markdown_escape(file.trim_end_matches(".md")),
            ));
        }
        out.push('\n');
    }
    out
}

/// One tag's operations page.
fn render_group(model: &DocModel, group: &super::OpGroup, file: &str) -> String {
    let mut out = frontmatter(
        &format!("{} — operations", group.tag),
        &format!("{} operations in {}", group.operations.len(), model.title),
    );
    out.push_str(&format!("# {} operations\n\n", markdown_escape(&group.tag)));
    for op in &group.operations {
        out.push_str(&format!(
            "## {} {}\n\n",
            op.method,
            markdown_escape(&op.path)
        ));
        if !op.summary.is_empty() {
            out.push_str(&op.summary);
            out.push_str("\n\n");
        }
        if !op.description.is_empty() {
            out.push_str(&op.description);
            out.push_str("\n\n");
        }
        if !op.operation_id.is_empty() {
            out.push_str(&format!(
                "**Operation ID:** `{}`\n\n",
                markdown_escape(&op.operation_id)
            ));
        }
        if op.deprecated {
            out.push_str(
                "> **DEPRECATED** — this operation may be removed in a future version.\n\n",
            );
        }
        if !op.parameters.is_empty() {
            out.push_str("### Parameters\n\n");
            out.push_str("| Name | In | Type | Required | Description |\n|---|---|---|---|---|\n");
            for param in &op.parameters {
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} | {} |\n",
                    markdown_escape(&param.name),
                    markdown_escape(&param.location),
                    markdown_escape(&param.type_desc),
                    if param.required { "yes" } else { "no" },
                    markdown_escape(&param.description),
                ));
            }
            out.push('\n');
        }
        if let Some(body) = &op.request_body {
            out.push_str("### Request body\n\n");
            out.push_str(&format!(
                "Content type `{}`, type `{}`{}\n\n",
                markdown_escape(&body.content_type),
                markdown_escape(&body.type_desc),
                if body.required {
                    ", required"
                } else {
                    ", optional"
                },
            ));
        }
        if !op.responses.is_empty() {
            out.push_str("### Responses\n\n");
            out.push_str("| Status | Description | Content type | Type |\n|---|---|---|---|\n");
            for response in &op.responses {
                out.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    markdown_escape(&response.status),
                    markdown_escape(&response.description),
                    response
                        .content_type
                        .as_deref()
                        .map(markdown_escape)
                        .unwrap_or_else(|| "—".to_owned()),
                    response
                        .type_desc
                        .as_deref()
                        .map(markdown_escape)
                        .unwrap_or_else(|| "—".to_owned()),
                ));
            }
            out.push('\n');
        }
    }
    let _ = file;
    out
}

/// One schema page.
fn render_schema(schema: &super::SchemaDoc) -> String {
    let mut out = frontmatter(&schema.name, &schema.description);
    out.push_str(&format!("# `{}`\n\n", markdown_escape(&schema.name)));
    if !schema.description.is_empty() {
        out.push_str(&schema.description);
        out.push_str("\n\n");
    }
    if schema.properties.is_empty() {
        out.push_str("_No declared properties._\n");
        return out;
    }
    out.push_str("## Properties\n\n");
    out.push_str("| Property | Type | Required | Description |\n|---|---|---|---|\n");
    for prop in &schema.properties {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            markdown_escape(&prop.name),
            markdown_escape(&prop.type_desc),
            if prop.required { "yes" } else { "no" },
            markdown_escape(&prop.description),
        ));
    }
    out.push('\n');
    out
}

/// `toc.json`: the machine-readable table of contents for site builders.
fn toc_json(
    model: &DocModel,
    group_links: &BTreeMap<String, (String, usize)>,
    schema_links: &BTreeMap<String, String>,
) -> String {
    let operations: Vec<serde_json::Value> = model
        .groups
        .iter()
        .filter_map(|group| {
            group_links.get(&group.tag).map(|(file, count)| {
                serde_json::json!({
                    "tag": group.tag,
                    "file": file,
                    "operationCount": count,
                })
            })
        })
        .collect();
    let schemas: Vec<serde_json::Value> = schema_links
        .iter()
        .map(|(name, file)| {
            serde_json::json!({
                "name": name,
                "file": file,
            })
        })
        .collect();
    let value = serde_json::json!({
        "title": model.title,
        "version": model.version,
        "description": model.description,
        "servers": model.servers,
        "operations": operations,
        "schemas": schemas,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned())
}

/// Escapes pipes so table cells survive Markdown rendering.
fn markdown_escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::super::{extract, fixture};
    use super::*;

    #[test]
    fn emits_index_group_and_schema_pages_with_frontmatter() {
        let model = extract(&fixture::sample());
        let dir = tempfile::tempdir().unwrap();
        let report = render_markdown(&model, dir.path()).unwrap();

        assert!(report.files.contains(&"index.md".to_owned()));
        assert!(report.files.contains(&"operations/pets.md".to_owned()));
        assert!(report.files.contains(&"operations/adoption.md".to_owned()));
        assert!(report.files.contains(&"schemas/Pet.md".to_owned()));
        assert!(report.files.contains(&"toc.json".to_owned()));

        let index = std::fs::read_to_string(dir.path().join("index.md")).unwrap();
        assert!(index.starts_with("---\ntitle: 'Test API'\n"));
        assert!(index.contains("**Version:** 1.4.2"));
        assert!(index.contains("[operations/pets](/operations/pets.md)"));
        assert!(index.contains("[schemas/Pet](/schemas/Pet.md)"));

        let pets = std::fs::read_to_string(dir.path().join("operations/pets.md")).unwrap();
        assert!(pets.contains("## GET /pets"));
        assert!(pets.contains("## POST /pets"));
        assert!(pets.contains("**Operation ID:** `listPets`"));
        assert!(pets.contains("| `limit` | query | integer (int32) | no |"));
        assert!(pets.contains("| `cursor` | query | string | no | Continuation cursor |"));
        // Request body and response reference the schema by name.
        assert!(pets.contains("type `Pet`, required"));
        assert!(pets.contains("| 201 | Created | application/json | Pet |"));

        let pet = std::fs::read_to_string(dir.path().join("schemas/Pet.md")).unwrap();
        assert!(pet.contains("title: 'Pet'"));
        assert!(pet.contains("| `name` | string | yes | The name |"));
        assert!(pet.contains("| `kind` | string (enum) | no | Species |"));

        // toc.json is valid JSON with the expected entries.
        let toc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("toc.json")).unwrap())
                .unwrap();
        assert_eq!(toc["title"], "Test API");
        assert_eq!(toc["operations"][0]["tag"], "pets");
        assert_eq!(toc["schemas"][0]["name"], "Pet");
    }

    #[test]
    fn slugged_tags_never_collide() {
        let mut issued = std::collections::BTreeSet::new();
        assert_eq!(unique_slug(&slugify("Pets"), &mut issued), "pets");
        assert_eq!(unique_slug(&slugify("Pets"), &mut issued), "pets-2");
    }
}
