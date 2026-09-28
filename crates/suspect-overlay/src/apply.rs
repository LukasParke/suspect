use suspect_low::{LowDoc, Pointer};
use suspect_source::Source;

use crate::error::OverlayError;
use crate::model::{ActionView, OverlayDoc};
use crate::value::Value;

/// Result of applying an overlay to a target document.
#[derive(Debug, Clone)]
pub struct Applied {
    /// The transformed document tree.
    pub output: Value,
    /// Number of actions that matched at least one target node.
    pub applied_actions: usize,
    /// Targets (raw expressions) that matched nothing — legal per spec, but
    /// tooling wants to know.
    pub unmatched_targets: Vec<String>,
}

/// Per-action record produced by [`explain`].
#[derive(Debug, Clone, PartialEq)]
pub struct ActionExplanation {
    /// Zero-based position in `actions`.
    pub index: usize,
    /// `update`, `copy`, or `remove`.
    pub kind: &'static str,
    /// The raw target expression.
    pub target: String,
    /// How many nodes the target selected in the tree state at this point
    /// in the sequence.
    pub matches: usize,
    /// The first match's JSON before the action (compact, truncated).
    pub before: Option<String>,
    /// The first match's JSON after the action (compact, truncated).
    pub after: Option<String>,
}

/// Applies an overlay's actions, in order, to a target document root.
///
/// Per Overlay 1.1 §4.4.3: `update` objects merge recursively into selected
/// objects (new keys append, arrays concatenate, incompatible combinations
/// error), array values append to (or concatenate with) selected arrays,
/// primitive updates replace selected primitives; `copy` merges a node
/// selected from the current document state into each target; `remove`
/// deletes selected nodes from their parent. Actions chain — each sees the
/// result of the previous one (queries evaluate against the current tree
/// state, re-materialized per action).
///
/// # Errors
/// [`OverlayError::TargetNotContainer`] when an update/copy combination is
/// incompatible; [`OverlayError::MergeConflict`] when a recursive merge hits
/// an incompatible property combination; [`OverlayError::InvalidAction`] /
/// [`OverlayError::CopySourceUnresolved`] for malformed or unresolvable
/// actions.
pub fn apply(
    overlay: &OverlayDoc<'_>,
    target_root: suspect_low::NodeRef<'_>,
) -> Result<Applied, OverlayError> {
    let mut tree = Value::from_node(target_root);
    let mut applied_actions = 0usize;
    let mut unmatched = Vec::new();

    for (index, action) in overlay.actions().iter().enumerate() {
        if apply_action(&mut tree, action, index)? {
            applied_actions += 1;
        } else {
            unmatched.push(action.target.to_owned());
        }
    }

    Ok(Applied {
        output: tree,
        applied_actions,
        unmatched_targets: unmatched,
    })
}

/// Explains what each action would do, without persisting any result:
/// evaluates the sequence step by step and records, per action, the match
/// count plus the first match's before/after JSON.
///
/// # Errors
/// Same failures as [`apply`].
pub fn explain(
    overlay: &OverlayDoc<'_>,
    target_root: suspect_low::NodeRef<'_>,
) -> Result<Vec<ActionExplanation>, OverlayError> {
    let mut tree = Value::from_node(target_root);
    let mut out = Vec::new();

    for (index, action) in overlay.actions().iter().enumerate() {
        let kind = if action.remove {
            "remove"
        } else if action.copy.is_some() {
            "copy"
        } else {
            "update"
        };
        let mut explanation = ActionExplanation {
            index,
            kind,
            target: action.target.to_owned(),
            matches: 0,
            before: None,
            after: None,
        };

        let path = match &action.parsed {
            Some(p) => p,
            None => {
                return Err(OverlayError::InvalidAction {
                    index,
                    reason: "target did not compile".into(),
                });
            }
        };
        let scratch = scratch_doc(&tree);
        let matches: Vec<Pointer> = path
            .query(scratch.root())
            .iter()
            .map(|node| node.path_from_root())
            .collect();
        explanation.matches = matches.len();
        if let Some(first) = matches.first() {
            explanation.before = Some(truncate_json(&snapshot(&tree, first)));
        }

        if apply_action(&mut tree, action, index)? && explanation.matches > 0 && kind != "remove" {
            let first = matches.first().expect("matches checked above");
            explanation.after = Some(truncate_json(&snapshot(&tree, first)));
        }
        out.push(explanation);
    }
    Ok(out)
}

/// Applies one action to the tree; `Ok(false)` = target matched nothing.
fn apply_action(
    tree: &mut Value,
    action: &ActionView<'_>,
    index: usize,
) -> Result<bool, OverlayError> {
    let path = match &action.parsed {
        Some(p) => p,
        None => {
            return Err(OverlayError::InvalidAction {
                index,
                reason: "target did not compile".into(),
            });
        }
    };

    let scratch = scratch_doc(tree);
    let matches: Vec<Pointer> = path
        .query(scratch.root())
        .iter()
        .map(|node| node.path_from_root())
        .collect();
    if matches.is_empty() {
        return Ok(false);
    }

    if action.remove {
        // deepest paths first so nested removals don't shift siblings
        let mut ordered = matches.clone();
        ordered.sort_by_key(|p| std::cmp::Reverse(p.tokens().len()));
        for ptr in ordered {
            remove_at(tree, &ptr);
        }
        return Ok(true);
    }

    if let Some(copy_expr) = &action.copy {
        // Overlay 1.1 §4.5.6: `copy` selects a single node from the
        // document being transformed (current state) and merges it into
        // every target.
        let parsed_copy = suspect_jsonpath::Path::parse(copy_expr)?;
        let scratch = scratch_doc(tree);
        let sources = parsed_copy.query(scratch.root());
        let Some(source_node) = sources.first() else {
            return Err(OverlayError::CopySourceUnresolved {
                index,
                source: (*copy_expr).to_owned(),
            });
        };
        let source_ptr = source_node.path_from_root();
        let source_value = snapshot(tree, &source_ptr);
        for ptr in &matches {
            let node = resolve_mut(tree, ptr.tokens()).ok_or(OverlayError::TargetNotContainer {
                index,
                path: action.target.to_owned(),
            })?;
            merge_checked(node, &source_value, index, action.target)?;
        }
        return Ok(true);
    }

    let update_node = action.update.ok_or(OverlayError::InvalidAction {
        index,
        reason: "missing `update`".into(),
    })?;
    let update = Value::from_node(update_node);
    for ptr in &matches {
        let node = resolve_mut(tree, ptr.tokens()).ok_or(OverlayError::TargetNotContainer {
            index,
            path: action.target.to_owned(),
        })?;
        match node {
            Value::Object(_) => merge_checked(node, &update, index, action.target)?,
            // Overlay 1.1 §4.4.3: an array update concatenates; an object or
            // primitive update appends.
            Value::Array(items) => match &update {
                Value::Array(updates) => items.extend(updates.clone()),
                Value::Object(_) | Value::Null | Value::Bool(_) => {
                    items.push(update.clone());
                }
                Value::Int(_) | Value::Float(_) | Value::Str(_) => {
                    items.push(update.clone());
                }
            },
            // Overlay 1.1: primitive targets are replaced by a primitive
            // update value.
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) => {
                match &update {
                    Value::Object(_) | Value::Array(_) => {
                        return Err(OverlayError::MergeConflict {
                            index,
                            path: action.target.to_owned(),
                            detail: "object or array update on a primitive target".into(),
                        });
                    }
                    _ => *node = update.clone(),
                }
            }
        }
    }
    Ok(true)
}

/// Recursive merge per Overlay 1.1 §4.4.3: target-only properties are left
/// unchanged, update-only properties are inserted, primitives replace
/// primitives, arrays concatenate, objects merge recursively, and every
/// other combination is an error.
fn merge_checked(
    target: &mut Value,
    update: &Value,
    index: usize,
    target_expr: &str,
) -> Result<(), OverlayError> {
    match (target, update) {
        (Value::Object(entries), Value::Object(updates)) => {
            for (uk, uv) in updates {
                match entries.iter_mut().find(|(k, _)| k.as_ref() == uk.as_ref()) {
                    Some((_, existing)) => {
                        merge_checked(existing, uv, index, target_expr)?;
                    }
                    None => entries.push((uk.clone(), uv.clone())),
                }
            }
            Ok(())
        }
        (Value::Array(items), Value::Array(updates)) => {
            items.extend(updates.clone());
            Ok(())
        }
        (
            slot @ (Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_)),
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_),
        ) => {
            *slot = update.clone();
            Ok(())
        }
        _ => Err(OverlayError::MergeConflict {
            index,
            path: target_expr.to_owned(),
            detail: "incompatible property combination in recursive merge".into(),
        }),
    }
}

/// Materializes the owned tree into a scratch document for querying.
fn scratch_doc(tree: &Value) -> LowDoc {
    let yaml = tree.to_yaml();
    LowDoc::parse(
        "mem://overlay-target.yaml".into(),
        Source::from_vec(yaml.into_bytes()),
    )
}

/// The owned value at `ptr`, via the tree's YAML form (round-trips through
/// the scratch document to keep key order and value shapes).
fn snapshot(tree: &Value, ptr: &Pointer) -> Value {
    let scratch = scratch_doc(tree);
    scratch
        .root()
        .pointer(ptr)
        .map_or(Value::Null, Value::from_node)
}

/// Compact JSON, truncated for explanation output.
fn truncate_json(value: &Value) -> String {
    let json = value.to_json();
    const CAP: usize = 480;
    if json.chars().count() <= CAP {
        json
    } else {
        let cut: String = json.chars().take(CAP).collect();
        format!("{cut}…")
    }
}

fn resolve_mut<'t>(tree: &'t mut Value, tokens: &[Box<str>]) -> Option<&'t mut Value> {
    let mut cur = tree;
    for token in tokens {
        cur = match cur {
            Value::Object(entries) => {
                &mut entries
                    .iter_mut()
                    .find(|(k, _)| k.as_ref() == token.as_ref())?
                    .1
            }
            Value::Array(items) => {
                let idx: usize = token.parse().ok()?;
                items.get_mut(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

fn remove_at(tree: &mut Value, ptr: &Pointer) {
    let Some(parent_ptr) = ptr.parent() else {
        return;
    };
    let Some(last) = ptr.tokens().last() else {
        return;
    };
    let Some(parent) = resolve_mut(tree, parent_ptr.tokens()) else {
        return;
    };
    match parent {
        Value::Object(entries) => entries.retain(|(k, _)| k.as_ref() != last.as_ref()),
        Value::Array(items) => {
            if let Ok(idx) = last.parse::<usize>()
                && idx < items.len()
            {
                items.remove(idx);
            }
        }
        _ => {}
    }
}
