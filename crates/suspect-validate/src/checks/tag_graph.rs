//! Tag-graph validation: `parent` references must resolve to declared
//! tags, and the hierarchy must be acyclic (3.2).

use rustc_hash::FxHashMap;
use suspect_oas::OpenApi;

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-tag-parent-unknown` (Error) and `oas-tag-parent-cycle` (Error):
/// 3.2 tag hierarchies must reference declared tags and form a forest.
pub(crate) fn check_tag_graph(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let Some(tags) = api.root().get("tags") else {
        return;
    };
    let mut names: FxHashMap<String, (usize, NodeSpan)> = FxHashMap::default();
    let mut parents: Vec<(String, Option<String>, NodeSpan)> = Vec::new();
    for tag in tags.items() {
        let Some(name) = tag.get("name").and_then(|n| n.as_str()).map(String::from) else {
            continue;
        };
        let span = tag.get("name").unwrap_or(tag).byte_range();
        let parent = tag.get("parent").and_then(|n| n.as_str()).map(String::from);
        if let Some(parent) = &parent {
            parents.push((name.clone(), Some(parent.clone()), span.clone()));
        }
        names.entry(name).or_insert((parents.len(), span));
    }
    // Rebuild spans per name (entry order can interleave with parents).
    let mut spans: FxHashMap<String, std::ops::Range<usize>> = FxHashMap::default();
    for tag in tags.items() {
        if let (Some(name), Some(node)) = (
            tag.get("name").and_then(|n| n.as_str()).map(String::from),
            tag.get("name"),
        ) {
            spans.insert(name, node.byte_range());
        }
    }
    for (name, parent, span) in &parents {
        let Some(parent_name) = parent else {
            continue;
        };
        if !names.contains_key(parent_name) {
            out.push(diag(
                api,
                "oas-tag-parent-unknown",
                Severity::Error,
                spans.get(name).cloned().unwrap_or_else(|| span.clone()),
                format!(
                    "tag `{name}` declares parent `{parent_name}`, which is not a declared tag"
                ),
            ));
        }
    }
    // Cycle detection: walk each tag's parent chain with a visited set.
    for (name, parent, _) in &parents {
        let mut seen: Vec<&str> = vec![name];
        let mut cursor = parent.as_deref();
        while let Some(current) = cursor {
            if seen.contains(&current) {
                let chain: Vec<&str> = seen
                    .iter()
                    .copied()
                    .chain(std::iter::once(current))
                    .collect();
                out.push(diag(
                    api,
                    "oas-tag-parent-cycle",
                    Severity::Error,
                    spans.get(name).cloned().unwrap_or(0..0),
                    format!(
                        "tag `{name}` participates in a parent cycle: {}",
                        chain.join(" -> ")
                    ),
                ));
                break;
            }
            seen.push(current);
            cursor = parents
                .iter()
                .find(|(n, p, _)| n == current && p.is_some())
                .and_then(|(_, p, _)| p.as_deref());
            if cursor == Some(name.as_str()) {
                break;
            }
        }
    }
}

type NodeSpan = std::ops::Range<usize>;
