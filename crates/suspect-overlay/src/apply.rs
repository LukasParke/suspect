use suspect_low::{LowDoc, Pointer};
use suspect_source::Source;

use crate::error::OverlayError;
use crate::model::OverlayDoc;
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

/// Applies an overlay's actions, in order, to a target document root.
///
/// Per Overlay 1.0 §4.4.3: `update` objects merge recursively into selected
/// objects (new keys append) and are appended as entries to selected arrays;
/// `remove` deletes selected nodes from their parent. Actions chain — each
/// sees the result of the previous one, so queries evaluate against the
/// current tree state (re-materialized per action).
///
/// # Errors
/// [`OverlayError::TargetNotContainer`] when an action selects a scalar;
/// [`OverlayError::InvalidAction`] when an action lacks a compiled target or
/// its `update` is neither object nor array.
pub fn apply(
    overlay: &OverlayDoc<'_>,
    target_root: suspect_low::NodeRef<'_>,
) -> Result<Applied, OverlayError> {
    let mut tree = Value::from_node(target_root);
    let mut applied_actions = 0usize;
    let mut unmatched = Vec::new();

    for (index, action) in overlay.actions().iter().enumerate() {
        let path = match &action.parsed {
            Some(p) => p,
            None => {
                return Err(OverlayError::InvalidAction {
                    index,
                    reason: "target did not compile".into(),
                });
            }
        };

        // Evaluate against the current state: materialize the owned tree to
        // YAML and parse it, giving NodeRefs whose pointers map 1:1 onto the
        // owned tree's structure (key order and value shapes are preserved).
        let scratch_yaml = tree.to_yaml();
        let scratch = LowDoc::parse(
            "mem://overlay-target.yaml".into(),
            Source::from_vec(scratch_yaml.into_bytes()),
        );
        let matches: Vec<Pointer> = path
            .query(scratch.root())
            .iter()
            .map(|node| node.path_from_root())
            .collect();

        if matches.is_empty() {
            unmatched.push(action.target.to_owned());
            continue;
        }
        applied_actions += 1;

        if action.remove {
            // deepest paths first so nested removals don't shift siblings
            let mut ordered = matches.clone();
            ordered.sort_by_key(|p| std::cmp::Reverse(p.tokens().len()));
            for ptr in ordered {
                remove_at(&mut tree, &ptr);
            }
        } else {
            let update_node = action.update.ok_or(OverlayError::InvalidAction {
                index,
                reason: "missing `update`".into(),
            })?;
            let update = Value::from_node(update_node);
            for ptr in &matches {
                let node = resolve_mut(&mut tree, ptr.tokens()).ok_or_else(|| {
                    OverlayError::TargetNotContainer {
                        index,
                        path: action.target.to_owned(),
                    }
                })?;
                match node {
                    Value::Object(_) => node.merge(&update),
                    // spec §4.4.3: "If the target selects an array, the value
                    // of this field MUST be an entry to append to the array"
                    Value::Array(items) => match &update {
                        Value::Object(_) | Value::Array(_) => items.push(update.clone()),
                        _ => {
                            return Err(OverlayError::TargetNotContainer {
                                index,
                                path: action.target.to_owned(),
                            });
                        }
                    },
                    _ => {
                        return Err(OverlayError::TargetNotContainer {
                            index,
                            path: action.target.to_owned(),
                        });
                    }
                }
            }
        }
    }

    Ok(Applied {
        output: tree,
        applied_actions,
        unmatched_targets: unmatched,
    })
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
