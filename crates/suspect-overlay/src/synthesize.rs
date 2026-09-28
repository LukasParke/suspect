//! Overlay synthesis: derive an Overlay 1.1 document from the semantic
//! difference between two documents.
//!
//! The result is a minimal, reviewable action list that transforms `old`
//! into `new`: removals first (deepest, exact targets), then value
//! insertions/updates merged per parent object. Sequencing removals before
//! insertions makes renames expressible as remove + add.

use crate::value::Value;
use suspect_low::NodeRef;

/// One synthesized action before serialization.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    /// Remove the node at the target expression.
    Remove(String),
    /// Merge `patch` into the object at the target expression.
    Update(String, Value),
}

/// Builds an Overlay 1.1 document (as an owned [`Value`], ready for YAML
/// emission) whose actions transform `old` into `new`.
#[must_use]
pub fn synthesize_overlay(old_root: NodeRef<'_>, new_root: NodeRef<'_>, title: &str) -> Value {
    let old = Value::from_node(old_root);
    let new = Value::from_node(new_root);
    let mut pending: Vec<Pending> = Vec::new();
    diff_node(&[], &old, &new, &mut pending);

    let mut actions: Vec<Value> = Vec::new();
    for p in pending {
        actions.push(match p {
            Pending::Remove(target) => Value::Object(vec![
                ("target".into(), Value::Str(target.into_boxed_str())),
                ("remove".into(), Value::Bool(true)),
            ]),
            Pending::Update(target, patch) => Value::Object(vec![
                ("target".into(), Value::Str(target.into_boxed_str())),
                ("update".into(), patch),
            ]),
        });
    }

    Value::Object(vec![
        ("overlay".into(), Value::Str("1.1.0".into())),
        (
            "info".into(),
            Value::Object(vec![
                (
                    "title".into(),
                    Value::Str(title.to_owned().into_boxed_str()),
                ),
                ("version".into(), Value::Str("1.0.0".into())),
            ]),
        ),
        ("actions".into(), Value::Array(actions)),
    ])
}

fn diff_node(prefix: &[String], old: &Value, new: &Value, out: &mut Vec<Pending>) {
    #[allow(clippy::needless_pass_by_ref_mut)] // recursive helper accumulates via `out`
    match (old, new) {
        (Value::Object(old_entries), Value::Object(new_entries)) => {
            // Removals first so subsequent insertions cannot collide with
            // stale keys (this is what makes renames work).
            for (k, ov) in old_entries {
                let child = child_prefix(prefix, k);
                match new.get(k) {
                    None => out.push(Pending::Remove(pointer_to_jsonpath(&child))),
                    Some(nv) => diff_node(&child, ov, nv, out),
                }
            }
            // Added keys merge into one update per parent.
            let mut added = Value::Object(Vec::new());
            for (k, nv) in new_entries {
                if old.get(k).is_none()
                    && let Value::Object(entries) = &mut added
                {
                    entries.push((k.clone(), nv.clone()));
                }
            }
            if let Value::Object(entries) = &added
                && !entries.is_empty()
            {
                out.push(Pending::Update(pointer_to_jsonpath(prefix), added));
            }
        }
        (Value::Array(old_items), Value::Array(new_items)) if old_items != new_items => {
            // Array replacement in two spec-legal steps: clear the old
            // elements, then concatenate the new ones onto the emptied
            // array (Overlay 1.1 concatenates array updates).
            if !old_items.is_empty() {
                let mut star = prefix.to_vec();
                star.push("*".to_owned());
                out.push(Pending::Remove(pointer_to_jsonpath(&star)));
            }
            if !new_items.is_empty() {
                out.push(Pending::Update(
                    pointer_to_jsonpath(prefix),
                    Value::Array(new_items.clone()),
                ));
            }
        }
        (a, b) if a != b => {
            // Scalar change or type change: replace through the parent.
            // The remove clears any old object/array shape; the parent
            // update re-inserts the new value.
            let (parent, key) = split_last(prefix);
            out.push(Pending::Remove(pointer_to_jsonpath(prefix)));
            if !parent.is_empty() {
                let mut patch = Value::Object(Vec::new());
                if let Value::Object(entries) = &mut patch {
                    entries.push((key.to_owned().into_boxed_str(), b.clone()));
                }
                out.push(Pending::Update(pointer_to_jsonpath(&parent), patch));
            } else {
                // Top-level replacement of a non-object root.
                out.push(Pending::Update("$".to_owned(), b.clone()));
            }
        }
        _ => {}
    }
}

fn child_prefix(prefix: &[String], key: &str) -> Vec<String> {
    let mut child = prefix.to_vec();
    child.push(key.to_owned());
    child
}

/// Splits the last token off a pointer path; empty `prefix` yields an empty
/// parent.
fn split_last(prefix: &[String]) -> (Vec<String>, &str) {
    match prefix.split_last() {
        Some((last, head)) => (head.to_vec(), last),
        None => (Vec::new(), ""),
    }
}

/// Renders a pointer token path as an RFC 9535 query: simple names use dot
/// notation, everything else bracket-quoted with RFC escapes.
#[must_use]
pub fn pointer_to_jsonpath(prefix: &[String]) -> String {
    if prefix.is_empty() {
        return "$".to_owned();
    }
    let mut out = String::from("$");
    for token in prefix {
        let simple = token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '-')
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$');
        if simple {
            out.push('.');
            out.push_str(token);
        } else {
            out.push_str("['");
            for c in token.chars() {
                match c {
                    '\'' => out.push_str("\\'"),
                    '\\' => out.push_str("\\\\"),
                    c => out.push(c),
                }
            }
            out.push('\'');
            out.push(']');
        }
    }
    out
}
