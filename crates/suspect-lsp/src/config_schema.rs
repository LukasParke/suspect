//! Knowledge about suspect's own configuration files, so that editing them
//! in an editor is as good as editing an OpenAPI document.
//!
//! Two files get first-class treatment: `.suspect.yaml` (the settings the
//! CLI, CI and the language server all read) and `suspect.project.json`
//! (the project manifest). The tables are `suspect_config::schema`'s, the
//! same rows the CLI parses with and the JSON Schemas are generated from,
//! so hover and completion here cannot disagree with them. Validation is
//! `suspect_config::schema::problems`, run by the check pass.
//!
//! Everything keys off the document's *file name*, so a `.suspect.yaml`
//! belonging to some unrelated tool never gets this treatment.

use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};

use crate::navigation::node_at;

pub use suspect_config::schema::problems::{
    FileKind, Problem, TARGET_SCOPE, children, closest, kind_for, kind_of, problems,
};
pub use suspect_config::schema::{Kind, Setting as Field};

/// The field at a dot path, if the schema has one.
#[must_use]
pub fn lookup(kind: FileKind, path: &str) -> Option<&'static Field> {
    suspect_config::schema::problems::field(kind, path)
}

/// The fields valid inside one `codegen` target, as paths from the root.
#[must_use]
pub fn target_children(scope: &str) -> Vec<&'static Field> {
    children(FileKind::Project, scope)
}

/// How a path reads in a message: `codegen[]` is noise in prose.
fn display_path(path: &str) -> String {
    suspect_config::schema::problems::display_path(path)
}

/// A mapping key's text, without the quotes a JSON document puts around it.
fn key_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(&text);
    trimmed.to_owned()
}

fn join(scope: &str, key: &str) -> String {
    if scope.is_empty() {
        key.to_owned()
    } else {
        format!("{scope}.{key}")
    }
}

/// The dot path of the configuration key under `offset`, and its byte range.
///
/// Recognises the key of the pair the cursor is in, whether the cursor is on
/// the key text or on the value beside it — the same generosity
/// [`crate::navigation::ref_value_node`] applies to `$ref`.
#[must_use]
pub fn key_path_at(low: &LowDoc, offset: usize) -> Option<(String, std::ops::Range<usize>)> {
    let mut cur = node_at(low, offset)?;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            // Walking up from the smallest containing node, the first pair
            // reached is the innermost one — so its key is the answer
            // whether the cursor is on the key text or on the value beside
            // it. Descending into the value first loses the key entirely.
            let key = cur.child_by_field("key")?;
            let range = key.byte_range();
            let name = key_text(key.scalar_bytes());
            return Some((join(&scope_of(low, range.start)?, &name), range));
        }
        cur = cur.parent()?;
    }
}

/// Like [`key_path_at`], but only when the cursor is inside the pair's
/// *value*.
///
/// Enumeration values apply to values, not keys, and a cursor on the blank
/// line after `min_severity: warning` is still inside that pair's line span
/// without being anywhere near its value. Offering severities there would be
/// right about the document and wrong about the user's intent, which is to
/// add the next key.
#[must_use]
pub fn value_key_path_at(low: &LowDoc, offset: usize) -> Option<(String, std::ops::Range<usize>)> {
    let node = node_at(low, offset)?;
    let mut cur = node;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            let key = cur.child_by_field("key")?;
            let range = key.byte_range();
            let inside_value = cur.child_by_field("value").is_some_and(|value| {
                let vr = value.byte_range();
                vr.start <= offset && offset < vr.end.max(vr.start + 1)
            });
            if inside_value {
                let name = key_text(key.scalar_bytes());
                return Some((join(&scope_of(low, range.start)?, &name), range));
            }
        }
        cur = cur.parent()?;
    }
}

/// The dot path of the mapping (or list-of-targets) a byte sits in.
fn scope_of(low: &LowDoc, offset: usize) -> Option<String> {
    let mut cur = node_at(low, offset)?;
    loop {
        if cur.kind() == SyntaxKind::Mapping {
            let node = NodeRef::new(cur);
            if node.kind() == ValueKind::Object {
                let mut tokens: Vec<String> = node
                    .path_from_root()
                    .tokens()
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                // A mapping inside a sequence is one `codegen` target, so
                // mark the list it belongs to; otherwise its keys would be
                // looked up against the wrong scope.
                let in_target_list = cur
                    .parent()
                    .filter(|p| p.kind() == SyntaxKind::Sequence)
                    .and_then(|s| s.parent())
                    .is_some_and(|p| p.kind() == SyntaxKind::Pair);
                if in_target_list && !tokens.is_empty() {
                    tokens[0] = format!("{}[]", tokens[0]);
                }
                return Some(tokens.join("."));
            }
        }
        cur = cur.parent()?;
    }
}

/// Hover text for the configuration key under `offset`, if there is one.
#[must_use]
pub fn hover(kind: FileKind, low: &LowDoc, offset: usize) -> Option<String> {
    let (path, _) = key_path_at(low, offset)?;
    let field = lookup(kind, &path).or_else(|| closest_field(kind, &path))?;
    Some(hover_markdown(kind, field))
}

/// The hover body for a field.
///
/// The same card frame as the OpenAPI hover surfaces: a `### \`key\``
/// heading, an italic subtitle carrying the qualified path, prose, and
/// a `| Field | Value |` table for the type/default/allowed facts.
#[must_use]
pub fn hover_markdown(kind: FileKind, field: &Field) -> String {
    let name = if field.path.is_empty() {
        // The root field describes the whole file; its key *is* the file.
        kind.display()
    } else {
        field.path.rsplit('.').next().unwrap_or(field.path)
    };
    let mut out = format!("### `{name}`");
    if !field.path.is_empty() {
        out.push_str(&format!("\n\n*`{}`*", display_path(field.path)));
    }
    out.push_str(&format!("\n\n{}", field.doc));
    out.push_str("\n\n| Field | Value |\n|---|---|");
    out.push_str(&format!("\n| Type | {} |", field.kind.type_name()));
    if let Some(default) = field.default {
        out.push_str(&format!("\n| Default | `{default}` |"));
    }
    if !field.values().is_empty() {
        out.push_str(&format!(
            "\n| One of | {} |",
            field
                .values()
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    out.push_str(&format!("\n\n---\n\n*{}*", kind.display()));
    out
}

/// A field whose path is the longest known prefix of `path`, so hovering a
/// section header still says something useful.
fn closest_field(kind: FileKind, path: &str) -> Option<&'static Field> {
    let mut best: Option<&'static Field> = None;
    for field in kind.table() {
        if field.path.is_empty() {
            continue;
        }
        if path == field.path
            || path.starts_with(&format!("{}.", field.path))
                && best.is_none_or(|b| b.path.len() < field.path.len())
        {
            best = Some(field);
        }
    }
    best
}

/// Completions for a configuration document: the permitted values when the
/// cursor is on an enum value, otherwise the keys valid in the enclosing
/// scope that are not already present.
#[must_use]
pub fn completions(kind: FileKind, low: &LowDoc, offset: usize) -> Vec<CompletionItem> {
    if let Some((path, _)) = value_key_path_at(low, offset)
        && let Some(field) = lookup(kind, &path)
        && !field.values().is_empty()
    {
        return field
            .values()
            .iter()
            .map(|value| CompletionItem {
                label: (*value).to_owned(),
                detail: Some(format!("{} — {}", field.doc, display_path(&path))),
                kind: Some(CompletionItemKind::VALUE),
                sort_text: Some(format!("0{value}")),
                ..CompletionItem::default()
            })
            .collect();
    }
    let scope = scope_of(low, offset).unwrap_or_default();
    let present: Vec<String> = node_at(low, offset)
        .and_then(|n| {
            let mut cur = n;
            loop {
                if cur.kind() == SyntaxKind::Mapping
                    && NodeRef::new(cur).kind() == ValueKind::Object
                {
                    return Some(NodeRef::new(cur));
                }
                cur = cur.parent()?;
            }
        })
        .map(|node| node.entries().iter().map(|e| e.key.to_owned()).collect())
        .unwrap_or_default();
    let candidates = if scope == TARGET_SCOPE {
        target_children(&scope)
    } else {
        children(kind, &scope)
    };
    candidates
        .into_iter()
        .filter(|f| !f.path.is_empty())
        .filter(|f| {
            let name = f.path.rsplit('.').next().unwrap_or(f.path);
            !present.iter().any(|p| p == name)
        })
        .map(|f| CompletionItem {
            label: f.path.rsplit('.').next().unwrap_or(f.path).to_owned(),
            detail: Some(f.doc.to_owned()),
            kind: Some(CompletionItemKind::PROPERTY),
            sort_text: Some(format!("1{}", f.path)),
            ..CompletionItem::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    fn doc(name: &str, text: &str) -> LowDoc {
        LowDoc::parse(
            Uri::parse(&format!("file:///w/{name}")).unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn offset(text: &str, needle: &str) -> usize {
        text.find(needle).expect("needle")
    }

    const SETTINGS_YAML: &str = "\
lint:
  min_severity: warning
  ruleset: ./rules.yaml
validate:
  strict_format: true
format:
  json: false
  yaml: true
docs:
  style: sveltekit
codegen:
  package_version: 1.1.1
";

    #[test]
    fn hover_describes_a_key() {
        let low = doc(".suspect.yaml", SETTINGS_YAML);
        let md = hover(
            FileKind::Settings,
            &low,
            offset(SETTINGS_YAML, "min_severity"),
        )
        .expect("hover");
        assert!(md.starts_with("### `min_severity`"), "{md}");
        assert!(md.contains("*`lint.min_severity`*"), "{md}");
        assert!(md.contains("Minimum severity"), "{md}");
        assert!(md.contains("| Type | string |"), "{md}");
        assert!(
            md.contains("| One of | `error` · `warning` · `info` · `hint` |"),
            "{md}"
        );
        assert!(md.ends_with("\n\n---\n\n*.suspect.yaml*"), "{md}");
    }

    #[test]
    fn hover_works_from_the_value_side_too() {
        let low = doc(".suspect.yaml", SETTINGS_YAML);
        let md =
            hover(FileKind::Settings, &low, offset(SETTINGS_YAML, "sveltekit")).expect("hover");
        assert!(md.contains("Output style"), "{md}");
    }

    #[test]
    fn hover_describes_a_manifest_key() {
        let text = "{\n  \"entry\": \"a.yaml\"\n}\n";
        let low = doc("suspect.project.json", text);
        let md = hover(FileKind::Project, &low, offset(text, "entry")).expect("hover");
        assert!(md.contains("Entry OpenAPI document"), "{md}");
        assert!(md.contains("suspect.project.json"), "{md}");
    }

    #[test]
    fn manifest_policy_keys_hover_and_complete() {
        // The old policy sections are ignored: hover says so and where the key
        // belongs; completion still names the keys.
        let text = "{\"entry\": \"a.yaml\", \"lint\": {\"min_severity\": \"warning\"}, \
                    \"validate\": {\"strict_format\": true}, \"editor\": {\"inlay_hints\": {}}}";
        let low = doc("suspect.project.json", text);
        let md = hover(FileKind::Project, &low, offset(text, "min_severity")).expect("lint hover");
        assert!(
            md.contains("Ignored") && md.contains(".suspect.yaml"),
            "{md}"
        );
        let md =
            hover(FileKind::Project, &low, offset(text, "strict_format")).expect("validate hover");
        assert!(md.contains("format"), "{md}");
        let md =
            hover(FileKind::Project, &low, offset(text, "editor")).expect("editor section hover");
        assert!(md.contains("Ignored"), "{md}");

        // Completion inside the lint section offers the policy keys.
        let items = completions(FileKind::Project, &low, offset(text, "min_severity"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        for expected in ["ruleset", "rules", "recommended"] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
        }
    }

    #[test]
    fn completion_offers_the_keys_still_missing_from_a_section() {
        let low = doc(".suspect.yaml", "lint:\n  min_severity: warning\n");
        let items = completions(
            FileKind::Settings,
            &low,
            offset("lint:\n  min_severity", "min"),
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["ruleset", "design", "rules", "recommended"],
            "{labels:?}"
        );
    }

    #[test]
    fn completion_offers_every_root_key_when_empty() {
        let low = doc(".suspect.yaml", "");
        let items = completions(FileKind::Settings, &low, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        for expected in ["lint", "validate", "format", "docs", "codegen"] {
            assert!(
                labels.contains(&expected),
                "{expected} missing from {labels:?}"
            );
        }
    }

    #[test]
    fn completion_offers_the_permitted_values_of_an_enum() {
        let low = doc(".suspect.yaml", "lint:\n  min_severity: warn\n");
        let items = completions(
            FileKind::Settings,
            &low,
            offset("lint:\n  min_severity: warn", "warn"),
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["error", "warning", "info", "hint"],
            "{labels:?}"
        );
    }

    #[test]
    fn completion_offers_target_keys_inside_a_codegen_entry() {
        let text = "{\"codegen\": [{\"name\": \"go\", \"profilee\": \"go-http\"}]}";
        let low = doc("suspect.project.json", text);
        let found = problems(FileKind::Project, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].suggestion.as_deref(), Some("profile"));

        let items = completions(FileKind::Project, &low, offset(text, "go-http"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"package_name"), "{labels:?}");
        assert!(!labels.contains(&"name"), "already present: {labels:?}");
    }
}
