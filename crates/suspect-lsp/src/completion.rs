//! Context-aware key and `$ref`-target completion.

use suspect_low::NodeRef;
use suspect_ref::Workspace;
use suspect_source::Uri;
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};

use crate::navigation::node_at;

/// Keys valid inside an operation object.
pub const OPERATION_KEYS: &[&str] = &[
    "tags",
    "summary",
    "description",
    "externalDocs",
    "operationId",
    "parameters",
    "requestBody",
    "responses",
    "callbacks",
    "deprecated",
    "security",
    "servers",
];

/// Keys valid inside a schema object.
pub const SCHEMA_KEYS: &[&str] = &[
    "type", "format", "title", "description", "default", "example", "enum", "const", "required",
    "properties", "items", "prefixItems", "additionalProperties", "patternProperties", "allOf",
    "anyOf", "oneOf", "not", "$ref", "$defs", "discriminator", "xml", "deprecated", "readOnly",
    "writeOnly", "minLength", "maxLength", "pattern", "minimum", "maximum", "minItems",
    "maxItems",
];

const METHODS: &[&str] =
    &["get", "put", "post", "delete", "options", "head", "patch", "trace"];

/// Keys whose value is always a schema — seeing one among the ancestor keys
/// means we are completing inside a schema.
const SCHEMA_PARENT_KEYS: &[&str] = &[
    "properties",
    "items",
    "prefixItems",
    "additionalProperties",
    "patternProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "$defs",
];

/// Component sections that hold named entries addressable via `$ref`.
const COMPONENT_SECTIONS: &[&str] = &[
    "schemas",
    "responses",
    "parameters",
    "examples",
    "requestBodies",
    "headers",
    "securitySchemes",
    "links",
    "callbacks",
    "pathItems",
];

/// What kind of completion applies at `offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// Mapping-key position: offer these keys.
    Keys(&'static [&'static str]),
    /// Inside a `$ref` string value: offer component pointers.
    Refs,
    /// No opinion.
    None,
}

#[must_use]
pub fn context_at(low: &suspect_low::LowDoc, offset: usize) -> CompletionContext {
    let Some(node) = node_at(low, offset) else { return CompletionContext::None };

    // Inside a `$ref` string value?
    {
        let mut cur = node;
        while let Some(parent) = cur.parent() {
            if parent.kind() == SyntaxKind::Pair
                && let Some(key) = parent.child_by_field("key")
                    && key.scalar_bytes() == b"$ref"
                        && let Some(value) = parent.child_by_field("value") {
                            let (vr, nr) = (value.byte_range(), node.byte_range());
                            if vr.start <= nr.start && nr.end <= vr.end {
                                return CompletionContext::Refs;
                            }
                        }
            cur = parent;
        }
    }

    // Mapping-key position: the node (or an ancestor) is the key side of a
    // pair whose key range contains the offset.
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair {
            if let Some(key) = n.child_by_field("key") {
                let kr = key.byte_range();
                if kr.start <= offset && offset <= kr.end {
                    return key_context(low, n);
                }
            }
            break;
        }
        cur = n.parent();
    }
    CompletionContext::None
}

/// Classifies the mapping that owns a key-position pair.
fn key_context(low: &suspect_low::LowDoc, pair: suspect_syntax::SNode<'_>) -> CompletionContext {
    // The owning mapping is the pair's structural ancestor.
    let mut mapping = pair.parent();
    while let Some(m) = mapping {
        if matches!(m.kind(), SyntaxKind::Mapping) {
            break;
        }
        mapping = m.parent();
    }
    let Some(mapping) = mapping else { return CompletionContext::None };
    let ptr = NodeRef::new(mapping.content()).path_from_root();
    let tokens = ptr.tokens();

    if tokens.first().is_some_and(|t| t.as_ref() == "paths") && tokens.len() >= 3 {
        let method = tokens[2].as_ref();
        if METHODS.contains(&method) {
            return CompletionContext::Keys(OPERATION_KEYS);
        }
    }
    if tokens.len() >= 2
        && tokens[0].as_ref() == "components"
        && tokens[1].as_ref() == "schemas"
    {
        return CompletionContext::Keys(SCHEMA_KEYS);
    }
    // Any ancestor key that is schema-valued puts us in schema context.
    let mut cur = Some(pair);
    while let Some(n) = cur {
        if n.kind() == SyntaxKind::Pair
            && let Some(key) = n.child_by_field("key") {
                let k = String::from_utf8_lossy(key.scalar_bytes());
                if SCHEMA_PARENT_KEYS.contains(&k.as_ref()) {
                    return CompletionContext::Keys(SCHEMA_KEYS);
                }
            }
        cur = n.parent();
    }
    let _ = low;
    CompletionContext::None
}

/// All `#/components/...` pointers across loaded documents. Same-document
/// candidates are fragment-only; others use a path relative to `current`
/// (`other.yaml#/components/schemas/Name`).
#[must_use]
pub fn ref_candidates(ws: &Workspace, current: &Uri) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for uri in ws.uris() {
        let Some(h) = ws.get(&uri) else { continue };
        let Some(components) = h.doc().root().get("components") else { continue };
        for section in COMPONENT_SECTIONS {
            let Some(sec_node) = components.get(section) else { continue };
            for entry in sec_node.entries() {
                let prefix = if uri == *current {
                    "#/".to_owned()
                } else {
                    format!("{}#/", relative_ref(current, &uri))
                };
                out.push(format!("{prefix}components/{section}/{}", entry.key));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Relative path reference from document `from`'s directory to `to`.
fn relative_ref(from: &Uri, to: &Uri) -> String {
    let (Some(f), Some(t)) = (from.as_path(), to.as_path()) else {
        return to.as_str().to_owned();
    };
    let fdir = f.parent().unwrap_or(std::path::Path::new("."));
    if let Ok(rel) = t.strip_prefix(fdir) {
        return rel.to_string_lossy().into_owned();
    }
    let fc: Vec<_> = fdir.components().collect();
    let tc: Vec<_> = t.components().collect();
    let common = fc.iter().zip(tc.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        fc[common..].iter().map(|_| "..".to_owned()).collect();
    parts.extend(
        tc[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        t.file_name().map_or_else(|| t.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
    } else {
        parts.join("/")
    }
}

/// Builds completion items for a key list.
#[must_use]
pub fn key_items(keys: &'static [&'static str]) -> Vec<CompletionItem> {
    keys.iter()
        .map(|k| CompletionItem {
            label: (*k).to_owned(),
            kind: Some(CompletionItemKind::PROPERTY),
            ..CompletionItem::default()
        })
        .collect()
}

/// Builds completion items for `$ref` pointer candidates.
#[must_use]
pub fn ref_items(candidates: Vec<String>) -> Vec<CompletionItem> {
    candidates
        .into_iter()
        .map(|c| CompletionItem {
            label: c,
            kind: Some(CompletionItemKind::MODULE),
            ..CompletionItem::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_ref::{Workspace, WorkspaceBuilder};

    fn low_of(text: &str) -> suspect_low::LowDoc {
        let uri = Uri::parse("file:///mem/doc.yaml").unwrap();
        suspect_low::LowDoc::parse(uri, suspect_source::Source::from_vec(text.as_bytes().to_vec()))
    }

    #[test]
    fn operation_key_context() {
        let text = "openapi: 3.1.0\npaths:\n  /pets:\n    get:\n      summary: x\n";
        let low = low_of(text);
        let off = text.find("summary").unwrap() + 2;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(OPERATION_KEYS));
    }

    #[test]
    fn schema_key_context_via_properties_parent() {
        let text = "components:\n  schemas:\n    Pet:\n      properties:\n        name:\n          type: string\n";
        let low = low_of(text);
        let off = text.find("type").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn schema_key_context_under_components_schemas() {
        let text = "components:\n  schemas:\n    Pet:\n      required: true\n";
        let low = low_of(text);
        let off = text.find("required").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::Keys(SCHEMA_KEYS));
    }

    #[test]
    fn ref_value_context() {
        let text = "components:\n  schemas:\n    A:\n      $ref: '#/x'\n";
        let low = low_of(text);
        let off = text.find("#/x").unwrap();
        assert_eq!(context_at(&low, off), CompletionContext::Refs);
    }

    #[test]
    fn no_context_outside_known_shapes() {
        let text = "info:\n  title: T\n";
        let low = low_of(text);
        let off = text.find("title").unwrap() + 1;
        assert_eq!(context_at(&low, off), CompletionContext::None);
    }

    fn workspace(dir: &std::path::Path) -> Arc<Workspace> {
        std::fs::write(
            dir.join("main.yaml"),
            "components:
  responses:
    Err:
      description: e
",
        )
        .unwrap();
        std::fs::write(
            dir.join("schemas.yaml"),
            "components:
  schemas:
    Pet:
      type: object
    PetList:
      type: array
",
        )
        .unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        ws.load_all("schemas.yaml").unwrap();
        Arc::new(ws)
    }

    #[test]
    fn ref_candidates_same_and_cross_file() {
        let dir = std::env::temp_dir().join("suspect-lsp-completion");
        std::fs::create_dir_all(&dir).unwrap();
        let ws = workspace(&dir);
        let main_uri = Uri::from_path(&dir.join("main.yaml")).unwrap();
        let cands = ref_candidates(&ws, &main_uri);
        assert!(cands.contains(&"#/components/responses/Err".to_owned()), "{cands:?}");
        assert!(cands.contains(&"schemas.yaml#/components/schemas/Pet".to_owned()));
        assert!(cands.contains(&"schemas.yaml#/components/schemas/PetList".to_owned()));
        // From the other document, same-file candidates are fragment-only.
        let schemas_uri = Uri::from_path(&dir.join("schemas.yaml")).unwrap();
        let cands2 = ref_candidates(&ws, &schemas_uri);
        assert!(cands2.contains(&"#/components/schemas/Pet".to_owned()), "{cands2:?}");
        assert!(cands2.contains(&"main.yaml#/components/responses/Err".to_owned()));
    }

    #[test]
    fn item_kinds_match_context() {
        let keys = key_items(OPERATION_KEYS);
        assert!(keys.iter().all(|i| i.kind == Some(CompletionItemKind::PROPERTY)));
        let refs = ref_items(vec!["#/components/schemas/Pet".to_owned()]);
        assert_eq!(refs[0].kind, Some(CompletionItemKind::MODULE));
    }
}
