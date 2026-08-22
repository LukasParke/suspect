//! Rename support for component keys (`components/<section>/<Name>`) and
//! the workspace-wide `$ref` rewrites that follow.
//!
//! Scope (v1): only named component entries are renameable. Path keys under
//! `paths` are deliberately *not* renameable — a path key is textually
//! self-contained (it *is* the path), and renaming it would also require
//! rewriting every operationId, tag reference, and external link that
//! spells the path out. That is deferred; `prepare_rename` reports `None`
//! for path positions so the server answers honestly instead of producing
//! a partial edit.

use std::collections::HashMap;

use suspect_low::{NodeRef, Pointer};
use suspect_ref::{ParsedRef, Workspace};
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{TextEdit, Url, WorkspaceEdit};

use crate::navigation::{node_at, value_anchor};
use crate::state::{OpenDoc, lsp_range};

/// Component sections whose named entries can be renamed.
const RENAMEABLE_SECTIONS: &[&str] = &[
    "schemas",
    "parameters",
    "responses",
    "headers",
    "examples",
    "requestBodies",
    "securitySchemes",
    "links",
    "callbacks",
    "pathItems",
];

/// The renameable component key found under a cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderRange {
    /// Byte range of the key token inside the home document.
    pub range: std::ops::Range<usize>,
    /// The current name (used as the editor's rename placeholder).
    pub placeholder: String,
}

/// Internal record of where a renameable key lives.
struct KeySite {
    /// Byte range of the key token.
    range: std::ops::Range<usize>,
    /// Component section (`schemas`, `parameters`, ...).
    section: String,
    /// Current name of the component.
    name: String,
}

/// Returns the placeholder for a rename at `offset`, but only when the
/// position sits on the **key** of a named entry under one of
/// the renameable component sections inside `components`. Positions inside schema
/// bodies, `$ref` values, path keys, or anywhere else yield `None`.
#[must_use]
pub fn prepare_rename(doc: &OpenDoc, offset: usize) -> Option<PlaceholderRange> {
    key_site(&doc.low, offset).map(|site| PlaceholderRange {
        range: site.range,
        placeholder: site.name,
    })
}

/// Locates the innermost mapping pair whose **key** spans `offset` and
/// verifies it names an entry in a renameable component section.
fn key_site(low: &suspect_low::LowDoc, offset: usize) -> Option<KeySite> {
    let node = node_at(low, offset)?;
    // Climb to the pair whose key token contains the cursor; positions on
    // values, colons, or between keys never find one.
    let mut cur = node;
    let key = loop {
        if cur.kind() == SyntaxKind::Pair
            && let Some(k) = cur.child_by_field("key")
        {
            let kr = k.byte_range();
            if kr.start <= offset && offset <= kr.end {
                break k;
            }
        }
        cur = cur.parent()?;
    };
    let name = String::from_utf8_lossy(key.scalar_bytes()).into_owned();
    // Anchor through the pair's value side so `path_from_root` works.
    let ptr = NodeRef::new(value_anchor(key)).path_from_root();
    let tokens = ptr.tokens();
    if tokens.len() < 3
        || tokens[0].as_ref() != "components"
        || !RENAMEABLE_SECTIONS.contains(&tokens[1].as_ref())
        || tokens[2].as_ref() != name.as_str()
    {
        return None;
    }
    Some(KeySite {
        range: key.byte_range(),
        section: tokens[1].to_string(),
        name,
    })
}

/// Validates a proposed new component name: non-empty, no `/`, `~`, or `#`,
/// no whitespace (the first three would corrupt RFC 6901 pointer tails).
fn validate_name(new_name: &str) -> Result<(), String> {
    if new_name.is_empty() {
        Err("new name must not be empty".to_owned())
    } else if new_name.chars().any(char::is_whitespace) {
        Err("new name must not contain whitespace".to_owned())
    } else if new_name.contains('/') || new_name.contains('~') || new_name.contains('#') {
        Err("new name must not contain '/', '~', or '#'".to_owned())
    } else {
        Ok(())
    }
}

/// Computes the workspace-wide rename edit for the component key under
/// `offset` in the home document.
///
/// The result spans every loaded workspace document: the declaration key
/// plus each `$ref` edge resolving to `#/components/<section>/<Old>` —
/// local edges in the home document and external edges from other
/// documents pointing into it. Only the pointer tail is rewritten; the
/// rest of each raw ref string (file part, quoting context) is preserved
/// verbatim by splitting at the last `#`.
///
/// Ranges assume each open buffer matches its workspace-loaded copy for
/// ref locations; the declaration edit is computed against the live
/// buffer.
///
/// # Errors
/// When the position is not on a renameable component key or the new name
/// is invalid (see the name validation rules). The caller surfaces these as JSON-RPC
/// `invalid_params` errors rather than silently doing nothing.
pub fn rename(
    ws: &Workspace,
    home: &OpenDoc,
    new_name: &str,
    offset: usize,
) -> Result<WorkspaceEdit, String> {
    validate_name(new_name)?;
    let site = key_site(&home.low, offset)
        .ok_or_else(|| "no renameable component key at this position".to_owned())?;
    {
        // Reject renames that would collide with an existing sibling.
        let section_ptr = suspect_low::Pointer::from_tokens(vec![
            "components".into(),
            site.section.clone().into(),
        ]);
        if home
            .low
            .root()
            .pointer(&section_ptr)
            .and_then(|section| section.get(new_name))
            .is_some()
        {
            return Err(format!(
                "`components/{}` already contains an entry named `{new_name}`",
                site.section
            ));
        }
    }
    let home_uri = home.low.uri().clone();
    let new_tail = Pointer::from_tokens(vec![
        "components".into(),
        site.section.clone().into(),
        new_name.into(),
    ])
    .to_path();

    // Byte-range edits per document URI, converted to LSP form below.
    let mut per_doc: HashMap<suspect_source::Uri, Vec<(std::ops::Range<usize>, String)>> =
        HashMap::new();
    let mut home_seen = false;
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        let is_home = uri == home_uri;
        home_seen |= is_home;
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for edge in handle.edges().iter() {
            let pointer = match &edge.parsed {
                ParsedRef::Local(p) if is_home => Some(p),
                ParsedRef::External {
                    uri: target,
                    pointer,
                } if target == &home_uri => Some(pointer),
                _ => None,
            };
            let Some(p) = pointer else { continue };
            // Exact match OR deeper path into the component (`.../A` vs
            // `.../A/properties/x`): only the name segment is rewritten so
            // deep refs stay intact.
            let renamed = match p.tokens().get(2).map(|t| t.as_ref()) {
                // deep ref: keep everything except the name segment
                Some(n) if n == site.name && p.tokens().len() > 3 => {
                    let mut toks: Vec<Box<str>> = p.tokens().to_vec();
                    toks[2] = new_name.into();
                    Pointer::from_tokens(toks).to_path()
                }
                Some(n) if n == site.name => new_tail.clone(),
                _ => continue,
            };
            // The fragment starts after the last `#`; everything before it
            // (empty for local refs) must be preserved exactly.
            let Some(hash) = edge.raw.rfind('#') else {
                continue;
            };
            edits.push((
                edge.at.clone(),
                format!("{}#{}", &edge.raw[..hash], renamed),
            ));
        }
        if is_home {
            edits.push((site.range.clone(), new_name.to_owned()));
        }
        if !edits.is_empty() {
            per_doc.insert(uri, edits);
        }
    }
    if !home_seen {
        // Unsaved/never-loaded home buffer: still rewrite its declaration.
        per_doc.insert(home_uri, vec![(site.range, new_name.to_owned())]);
    }

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for (uri, mut edits) in per_doc {
        let Some(url) = Url::parse(uri.as_str()).ok() else {
            continue;
        };
        // LSP clients apply TextEdits back-to-front; sort descending.
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        let inner = if uri == *home.low.uri() {
            home.low.inner()
        } else {
            match ws.get(&uri) {
                Some(h) => h.doc().inner(),
                None => continue,
            }
        };
        let lsp_edits = edits
            .into_iter()
            .map(|(r, text)| TextEdit {
                range: lsp_range(inner.bytes(), inner.line_index(), r),
                new_text: text,
            })
            .collect();
        changes.insert(url, lsp_edits);
    }
    Ok(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ref::WorkspaceBuilder;
    use suspect_source::Uri;

    const BASE: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: 'schemas.yaml#/components/schemas/Pet'
"#;

    const SCHEMAS: &str = r#"
components:
  schemas:
    Pet:
      type: object
    PetOwner:
      type: object
      properties:
        pet:
          $ref: '#/components/schemas/Pet'
"#;

    fn setup(dir: &std::path::Path) -> (Arc<Workspace>, OpenDoc) {
        std::fs::write(dir.join("base.yaml"), BASE).unwrap();
        std::fs::write(dir.join("schemas.yaml"), SCHEMAS).unwrap();
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("base.yaml").unwrap();
        let schemas_path = dir.join("schemas.yaml");
        let uri = Uri::from_path(&schemas_path).unwrap();
        let home = OpenDoc::parse(uri, SCHEMAS.to_owned());
        (Arc::new(ws), home)
    }

    use std::sync::Arc;

    fn offset_of(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap() + needle.len() / 2
    }

    #[test]
    fn prepare_rename_accepts_component_key() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-accept");
        std::fs::create_dir_all(&dir).unwrap();
        let (_ws, home) = setup(&dir);
        let off = offset_of(SCHEMAS, "    Pet:");
        let ph = prepare_rename(&home, off).expect("renameable");
        assert_eq!(ph.placeholder, "Pet");
        // Range starts at the key token itself (after the four-space indent).
        assert_eq!(ph.range.start, SCHEMAS.find("    Pet:").unwrap() + 4);
        assert_eq!(ph.range.len(), "Pet".len());
    }

    #[test]
    fn prepare_rename_rejects_non_component_positions() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-reject");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base.yaml"), BASE).unwrap();
        let base_path = dir.join("base.yaml");
        let uri = Uri::from_path(&base_path).unwrap();
        let doc = OpenDoc::parse(uri, BASE.to_owned());
        // Path key under `paths`: not renameable in v1.
        assert!(prepare_rename(&doc, offset_of(BASE, "/pets")).is_none());
        // Key inside a schema body (pointer too deep) would also be None;
        // here: the `get` method key and the `$ref` value.
        assert!(prepare_rename(&doc, offset_of(BASE, "get:")).is_none());
        assert!(
            prepare_rename(
                &doc,
                offset_of(BASE, "schemas.yaml#/components/schemas/Pet")
            )
            .is_none()
        );
    }

    #[test]
    fn prepare_rename_rejects_body_keys_in_home_doc() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-body");
        std::fs::create_dir_all(&dir).unwrap();
        let (_ws, home) = setup(&dir);
        // `type:` inside the Pet body sits at depth 4 — not a component name.
        assert!(prepare_rename(&home, offset_of(SCHEMAS, "type: object")).is_none());
        // Section key itself (`schemas`) has only two pointer tokens.
        assert!(prepare_rename(&home, offset_of(SCHEMAS, "schemas:")).is_none());
    }

    #[test]
    fn rename_rewrites_refs_across_two_files() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-two-files");
        std::fs::create_dir_all(&dir).unwrap();
        let (ws, home) = setup(&dir);
        let off = offset_of(SCHEMAS, "    Pet:");
        let edit = rename(&ws, &home, "Cat", off).expect("rename ok");
        let changes = edit.changes.expect("changes present");

        let schemas_url = Url::parse(home.low.uri().as_str()).unwrap();
        let base_path = dir.join("base.yaml");
        let base_url = Url::from_file_path(&base_path).unwrap();

        let schema_edits = &changes[&schemas_url];
        // Declaration key + the local ref inside PetOwner.properties.pet.
        assert_eq!(schema_edits.len(), 2, "{schema_edits:?}");
        assert!(schema_edits.iter().any(|e| e.new_text == "Cat"));
        assert!(
            schema_edits
                .iter()
                .any(|e| e.new_text == "#/components/schemas/Cat"),
            "{schema_edits:?}"
        );

        let base_edits = &changes[&base_url];
        assert_eq!(base_edits.len(), 1, "{base_edits:?}");
        assert_eq!(
            base_edits[0].new_text, "schemas.yaml#/components/schemas/Cat",
            "external ref rewritten, file prefix preserved"
        );
    }

    #[test]
    fn rename_rejects_illegal_names() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-illegal");
        std::fs::create_dir_all(&dir).unwrap();
        let (ws, home) = setup(&dir);
        let off = offset_of(SCHEMAS, "    Pet:");
        for bad in ["", "a/b", "a~b", "a#b", "a b", "\ta"] {
            let err = rename(&ws, &home, bad, off).expect_err("must reject");
            assert!(!err.is_empty());
        }
    }

    #[test]
    fn rename_rewrites_deep_refs() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-deep");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.yaml"),
            "components:\n  schemas:\n    A:\n      properties:\n        x:\n          $ref: '#/components/schemas/A/properties/x'\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("user.yaml"),
            "allOf:\n  - $ref: 'base.yaml#/components/schemas/A/properties/x'\n  - $ref: 'base.yaml#/components/schemas/A'\n",
        )
        .unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("base.yaml").unwrap();
        let home = OpenDoc::parse(
            Uri::from_path(&dir.join("base.yaml")).unwrap(),
            std::fs::read_to_string(dir.join("base.yaml")).unwrap(),
        );
        let off = offset_of(home.text.as_str(), "A:").saturating_sub(1);
        let edit = rename(&ws, &home, "Zed", off).expect("rename succeeds");
        let s = serde_json::to_string(&edit.changes).unwrap();
        assert!(s.contains("Zed/properties/x"), "deep ref tail kept: {s}");
        assert!(
            s.contains("#/components/schemas/Zed"),
            "plain ref rewritten: {s}"
        );
    }

    #[test]
    fn rename_rejects_duplicate_sibling() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-dup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("dup.yaml"),
            "components:\n  schemas:\n    A:\n      type: object\n    B:\n      type: object\n",
        )
        .unwrap();
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        let home = OpenDoc::parse(
            Uri::from_path(&dir.join("dup.yaml")).unwrap(),
            std::fs::read_to_string(dir.join("dup.yaml")).unwrap(),
        );
        let off = offset_of(home.text.as_str(), "A:").saturating_sub(1);
        let err = rename(&ws, &home, "B", off).unwrap_err();
        assert!(err.contains("already contains"), "{err}");
    }

    #[test]
    fn rename_rejects_non_key_position() {
        let dir = std::env::temp_dir().join("suspect-lsp-rename-nonkey");
        std::fs::create_dir_all(&dir).unwrap();
        let (ws, home) = setup(&dir);
        let err = rename(&ws, &home, "Cat", offset_of(SCHEMAS, "type: object"))
            .expect_err("position not on component key");
        assert!(err.contains("no renameable component key"), "{err}");
    }
}
