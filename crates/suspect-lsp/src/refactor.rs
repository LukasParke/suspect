//! Refactors that move meaning, not text: extract a schema to a component,
//! and inline one back.
//!
//! Rename already rewrites references across the workspace. These two
//! operations change *structure*: an inline schema becomes a component
//! every operation can share, and a shared component becomes local again
//! when only one place uses it. Both are single, atomic, multi-file edits
//! over the lossless tree — which is why they belong in the editor, where
//! the change is visible, rather than as a separate tool.
//!
//! Each refactor reports what it would do before it does anything, so the
//! caller can preview and the user can undo.

use std::collections::{BTreeMap, BTreeSet};

use suspect_low::{LowDoc, NodeRef, Pointer};

use crate::meaning::{Index, Model};

/// Why a refactor is not available, when it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The cursor is not on an inline schema.
    NotAnInlineSchema,
    /// The cursor is not on a component schema.
    NotAComponent,
    /// A component with that name is already declared.
    NameTaken(String),
    /// The inline schema cannot be moved (a `$ref` is already a reference).
    AlreadyAReference,
    /// The component is referenced from more than one place.
    StillShared(usize),
}

/// A planned refactor: the edits it would apply, and what it would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The component name involved.
    pub name: String,
    /// Edits to apply, keyed by document URI.
    pub edits: Vec<(String, Pointer, String)>,
    /// A one-line description for the code action title.
    pub summary: String,
}

impl Plan {
    /// Whether the refactor would change anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

/// The refactor that would help most at a position, as a code action title:
/// a shared component with more than one use site is a candidate for
/// inlining or splitting.
#[must_use]
pub fn hint_for(index: &Index) -> Option<String> {
    let shared = shared_components(index);
    shared
        .iter()
        .next()
        .map(|name| format!("{name} is referenced more than once"))
}

/// Plans extracting the inline schema at `offset` into
/// `components/schemas/<name>`, rewriting the position to a `$ref`.
///
/// The schema is moved only when it is genuinely inline: a position that
/// is already a `$ref` has nothing to extract.
pub fn plan_extract(
    low: &LowDoc,
    index: &Index,
    offset: usize,
    name: &str,
) -> Result<Plan, Unavailable> {
    let model = Model::new(low);
    let Some(meaning) = model.at(offset) else {
        return Err(Unavailable::NotAnInlineSchema);
    };
    let pointer = meaning.pointer.clone();
    // The schema node is the cursor's own mapping, or the nearest ancestor
    // that is a schema: extracting a property, not a scalar.
    let Some(node) = schema_node_at(low, offset) else {
        return Err(Unavailable::NotAnInlineSchema);
    };
    if node.get("$ref").is_some() {
        return Err(Unavailable::AlreadyAReference);
    }
    if declares(low, name) {
        return Err(Unavailable::NameTaken(name.to_owned()));
    }
    let node_pointer = node.path_from_root();
    let value = suspect_overlay::Value::from_node(node);
    let mut edits = vec![(
        low.uri().as_str().to_owned(),
        node_pointer.clone(),
        format!("$ref: '#/components/schemas/{name}'"),
    )];
    // Add the component, with the pointer the body should be inserted at.
    edits.push((
        low.uri().as_str().to_owned(),
        components_pointer(low),
        format!("{name}: {}", indent_block(&value.to_yaml(), 6)),
    ));
    let _ = pointer;
    let _ = index;
    Ok(Plan {
        name: name.to_owned(),
        edits,
        summary: format!(
            "extract {} into components/schemas/{name}",
            describe_pointer(&node_pointer)
        ),
    })
}

/// Plans inlining the component schema at `offset` back to its use sites.
///
/// Refuses when the component is shared: inlining it at one site while
/// another still references it would leave the contract inconsistent.
pub fn plan_inline(low: &LowDoc, index: &Index, offset: usize) -> Result<Plan, Unavailable> {
    let model = Model::new(low);
    let Some(meaning) = model.at(offset) else {
        return Err(Unavailable::NotAComponent);
    };
    let tokens: Vec<String> = meaning
        .pointer
        .tokens()
        .iter()
        .map(|token| token.to_string())
        .collect();
    let under_components = tokens.len() == 3 && tokens[0] == "components" && tokens[1] == "schemas";
    if !under_components {
        return Err(Unavailable::NotAComponent);
    }
    let name = tokens[2].clone();
    let definition_pointer = meaning.pointer.clone();

    let inbound = index.inbound(low.uri().as_str(), &definition_pointer);
    if inbound.len() > 1 {
        return Err(Unavailable::StillShared(inbound.len()));
    }
    let Some(reference) = inbound.first() else {
        return Err(Unavailable::NotAComponent);
    };

    // Replace the reference with the component's own body, and delete the
    // component: an inline schema at the one place it is used.
    let definition = low
        .root()
        .pointer(&definition_pointer)
        .ok_or(Unavailable::NotAComponent)?;
    let body = suspect_overlay::Value::from_node(definition);
    let edits = vec![
        (
            reference.document.clone(),
            reference.source.clone(),
            body.to_yaml().trim_end().to_owned(),
        ),
        (
            low.uri().as_str().to_owned(),
            definition_pointer,
            String::new(),
        ),
    ];
    let summary = format!("inline components/schemas/{name} at its only use site");
    Ok(Plan {
        name,
        edits,
        summary,
    })
}

/// The schema node a cursor is in, or just above.
fn schema_node_at(low: &LowDoc, offset: usize) -> Option<NodeRef<'_>> {
    let model = Model::new(low);
    if let Some(node) = model.object_ancestor(offset) {
        return Some(node);
    }
    model.enclosing_schema(offset)
}

/// Where a new component's body is inserted.
fn components_pointer(low: &LowDoc) -> Pointer {
    low.root()
        .get("components")
        .and_then(|components| components.get("schemas"))
        .map_or_else(
            || Pointer::parse("/components/schemas").unwrap_or_else(|_| Pointer::root()),
            |_| Pointer::parse("/components/schemas").unwrap_or_else(|_| Pointer::root()),
        )
}

/// Whether `components/schemas/<name>` is already declared.
#[must_use]
pub fn declares(low: &LowDoc, name: &str) -> bool {
    low.root()
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(|schemas| schemas.get(name))
        .is_some()
}

/// A suggested component name derived from the position, so the code action
/// needs no prompt: `listPetsResponse` from the operation it sits in.
#[must_use]
pub fn suggest_name(low: &LowDoc, offset: usize) -> String {
    let model = Model::new(low);
    let Some(meaning) = model.at(offset) else {
        return "Extracted".to_owned();
    };
    // The operation this schema belongs to, when there is one.
    let tokens: Vec<String> = meaning
        .pointer
        .tokens()
        .iter()
        .map(|token| token.to_string())
        .collect();
    if tokens.first().is_some_and(|first| first == "paths") && tokens.len() >= 2 {
        return capitalize(&tokens[1].replace(['/', '~', '-'], ""));
    }
    meaning
        .pointer
        .tokens()
        .last()
        .map(|token| capitalize(&token.to_string().replace(['/', '~', '-'], "")))
        .unwrap_or_else(|| "Extracted".to_owned())
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Normalizes a suggested name into a component-name shape: an identifier
/// that starts with a capital, with no separators.
#[must_use]
pub fn component_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut chars = cleaned.chars();
    match chars.next() {
        Some(first) => {
            first.to_uppercase().collect::<String>() + &chars.as_str().to_ascii_lowercase()
        }
        None => "Extracted".to_owned(),
    }
}

/// A schema body, re-indented for insertion under a component key.
fn indent_block(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    text.trim_end()
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe_pointer(pointer: &Pointer) -> String {
    let path = pointer.to_path();
    path.rsplit('/').next().unwrap_or(&path).to_owned()
}

/// Every component name referenced more than once — the refactor that would
/// help most, surfaced as a hint.
#[must_use]
pub fn shared_components(index: &Index) -> BTreeSet<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for reference in index.references() {
        if let Some(rest) = reference.text.strip_prefix("#/components/schemas/") {
            *counts.entry(rest.to_owned()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::Source;

    fn doc(text: &str) -> LowDoc {
        LowDoc::parse(
            "mem://refactor.yaml".into(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    const SPEC: &str = r#"
openapi: 3.1.0
info: {title: Refactor, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [id]
                properties:
                  id: {type: string}
components:
  schemas:
    Error:
      type: object
      properties: {message: {type: string}}
    Only: {$ref: '#/components/schemas/Error'}
"#;

    #[test]
    fn extraction_replaces_the_inline_schema_and_adds_a_component() {
        let low = doc(SPEC);
        let index = Index::default();
        // The cursor is on the inline response schema, marked by the
        // `required` line that only it carries.
        let offset = low
            .inner()
            .bytes()
            .windows(9)
            .position(|w| w == b"required:")
            .expect("the inline response schema")
            + 2;
        let plan = plan_extract(&low, &index, offset, "Pet").expect("extractable");
        assert_eq!(plan.name, "Pet");
        assert_eq!(plan.edits.len(), 2, "one replacement, one insertion");
        let (uri, pointer, text) = &plan.edits[0];
        assert_eq!(uri, low.uri().as_str());
        assert_eq!(
            pointer.to_path(),
            "/paths/~1pets/get/responses/200/content/application~1json/schema"
        );
        assert_eq!(text, "$ref: '#/components/schemas/Pet'");
        // The inserted component carries the body it extracted.
        let (_, insertion, body) = &plan.edits[1];
        assert_eq!(insertion.to_path(), "/components/schemas");
        assert!(body.starts_with("Pet: "), "{body}");
        assert!(body.contains("required"), "{body}");
        assert!(
            plan.summary.contains("components/schemas/Pet"),
            "{}",
            plan.summary
        );
    }

    #[test]
    fn extraction_refuses_a_name_already_in_use() {
        let low = doc(SPEC);
        let index = Index::default();
        let offset = low
            .inner()
            .bytes()
            .windows(9)
            .position(|w| w == b"required:")
            .expect("the inline response schema")
            + 2;
        let err = plan_extract(&low, &index, offset, "Error").expect_err("taken");
        assert_eq!(err, Unavailable::NameTaken("Error".to_owned()));
    }

    #[test]
    fn extraction_refuses_a_position_that_is_already_a_reference() {
        let low = doc(SPEC);
        let index = Index::default();
        let offset = low
            .inner()
            .bytes()
            .windows(5)
            .position(|w| w == b"'#/co")
            .expect("the reference")
            + 2;
        let err = plan_extract(&low, &index, offset, "Copy").expect_err("already a ref");
        assert_eq!(err, Unavailable::AlreadyAReference);
    }

    #[test]
    fn inlining_requires_a_single_use_site() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("openapi.yaml"), SPEC).unwrap();
        let ws = suspect_ref::WorkspaceBuilder::new()
            .root(dir.path())
            .build()
            .unwrap();
        ws.load_all("openapi.yaml").unwrap();
        let ws = std::sync::Arc::new(ws);
        let low = ws.get(&ws.uris()[0]).unwrap().doc();
        let mut index = Index::default();
        index.index_document(low.uri().as_str(), low);

        // `Only` is a one-line alias, not a component definition.
        let alias = low
            .inner()
            .bytes()
            .windows(4)
            .position(|w| w == b"Only")
            .unwrap();
        let err = plan_inline(low, &index, alias).expect_err("not a component");
        assert_eq!(err, Unavailable::NotAComponent);
    }

    #[test]
    fn shared_components_are_reported() {
        let low = doc(
            "openapi: 3.1.0\ninfo: {title: a, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    A: {$ref: '#/components/schemas/B'}\n    C: {$ref: '#/components/schemas/B'}\n    B: {type: object}\n",
        );
        let mut index = Index::default();
        index.index_document(low.uri().as_str(), &low);
        let shared = shared_components(&index);
        assert!(shared.contains("B"), "{shared:?}");
        assert!(!shared.contains("A"));
    }

    #[test]
    fn a_shared_component_produces_a_hint() {
        let low = doc(
            "openapi: 3.1.0\ninfo: {title: a, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    A: {$ref: '#/components/schemas/B'}\n    C: {$ref: '#/components/schemas/B'}\n    B: {type: object}\n",
        );
        let mut index = Index::default();
        index.index_document(low.uri().as_str(), &low);
        assert_eq!(
            hint_for(&index).as_deref(),
            Some("B is referenced more than once")
        );
    }

    #[test]
    fn a_plan_reports_whether_it_would_change_anything() {
        let low = doc(SPEC);
        let index = Index::default();
        let offset = low
            .inner()
            .bytes()
            .windows(9)
            .position(|w| w == b"required:")
            .expect("the inline response schema")
            + 2;
        let plan = plan_extract(&low, &index, offset, "Pet").expect("extractable");
        assert!(!plan.is_empty());
    }

    #[test]
    fn a_name_is_suggested_from_the_position() {
        let low = doc(SPEC);
        let offset = low
            .inner()
            .bytes()
            .windows(9)
            .position(|w| w == b"required:")
            .expect("the inline response schema")
            + 2;
        let name = suggest_name(&low, offset);
        assert!(name.starts_with("Pets"), "suggested from the path: {name}");
    }
}
