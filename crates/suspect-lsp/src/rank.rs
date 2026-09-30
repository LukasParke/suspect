//! Schema-aware completion: proposals ranked by what the contract offers.
//!
//! A completion list is advice, not a keyword dump. When the cursor is in
//! a `$ref`, the useful proposals are the components this workspace
//! actually uses, near to the position, and type-compatible with where the
//! reference lands. When it is in a constrained value position, the useful
//! proposals are the enum members and examples the schema itself
//! declares. This module computes that ordering; the completion machinery
//! supplies the items.
//!
//! Ranking is derived from the shared semantic model and the reference
//! index, so it improves as the contract gets richer rather than needing
//! its own heuristics.

use std::collections::BTreeMap;

use suspect_low::{LowDoc, Pointer};

use crate::meaning::Index;

/// How strongly a candidate is suggested at a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Rank {
    /// A value the position's own schema permits exactly (an enum member,
    /// a declared example, a default).
    Exact,
    /// A component of the kind the position expects, used elsewhere in the
    /// workspace.
    Likely,
    /// A component of the right kind, unused.
    Plausible,
    /// Everything else that is merely valid here.
    Valid,
}

impl Rank {
    /// A human label, for a completion item's detail text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "the schema's own value",
            Self::Likely => "used in this workspace",
            Self::Plausible => "same component kind",
            Self::Valid => "valid here",
        }
    }
}

/// A ranked completion proposal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Proposal {
    /// The text to insert.
    pub insert: String,
    /// How strongly it is suggested.
    pub rank: Rank,
    /// A short explanation, shown next to the item.
    pub detail: String,
    /// How often the workspace already uses this candidate; the tie-break
    /// inside a rank.
    pub weight: usize,
}

/// The schema's own permitted values at a position: enum members, `const`,
/// `default`, and `examples`.
#[must_use]
pub fn schema_values(low: &LowDoc, offset: usize) -> Vec<String> {
    let model = crate::meaning::Model::new(low);
    let Some(schema) = model.enclosing_schema(offset) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(node) = schema.get("enum") {
        for item in node.items() {
            if let Some(text) = item.as_str() {
                out.push(text.to_owned());
            } else {
                out.push(suspect_overlay::Value::from_node(item).to_json());
            }
        }
    }
    if let Some(node) = schema.get("examples") {
        if node.kind() == suspect_low::ValueKind::Array {
            for item in node.items() {
                if let Some(text) = item.as_str() {
                    out.push(text.to_owned());
                }
            }
        } else if let Some(text) = node.as_str() {
            out.push(text.to_owned());
        }
    }
    if let Some(node) = schema.get("const")
        && let Some(text) = node.as_str()
    {
        out.push(text.to_owned());
    }
    if let Some(node) = schema.get("default") {
        if let Some(text) = node.as_str() {
            out.push(text.to_owned());
        } else {
            out.push(suspect_overlay::Value::from_node(node).to_json());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The pointer of the object that declares `properties` for a position, if
/// the position is inside a property (`…/properties/<name>`) or is the
/// `properties` mapping itself.
#[must_use]
pub fn declaring_schema_pointer(pointer: &suspect_low::Pointer) -> Option<suspect_low::Pointer> {
    let mut parent = pointer.parent()?;
    // Either the cursor is on the `properties` key itself, or on a property
    // inside it; both name the declaring object one level up.
    if parent
        .tokens()
        .last()
        .is_some_and(|token| token.as_ref() == "properties")
    {
        return parent.parent();
    }
    // …/properties/<name>/<sub>: strip the schema tail until `properties`.
    loop {
        let last = parent.tokens().last()?.to_string();
        if last == "properties" {
            return parent.parent();
        }
        if !matches!(
            last.as_str(),
            "type"
                | "format"
                | "items"
                | "description"
                | "enum"
                | "default"
                | "minimum"
                | "maximum"
                | "minLength"
                | "maxLength"
                | "pattern"
                | "example"
                | "nullable"
                | "deprecated"
                | "title"
        ) {
            return None;
        }
        parent = parent.parent()?;
    }
}

/// The logical pointer of a cursor position, for ranking against it.
#[must_use]
pub fn position_at(low: &LowDoc, offset: usize) -> Pointer {
    crate::meaning::Model::new(low)
        .at(offset)
        .map_or_else(Pointer::root, |meaning| meaning.pointer)
}

/// Whether a position must hold a schema, so a `$ref` there should only
/// offer schema components.
#[must_use]
pub fn expects_schema(low: &LowDoc, offset: usize) -> bool {
    crate::meaning::Model::new(low)
        .at(offset)
        .is_some_and(|m| m.in_schema || m.pointer.to_path().contains("/schema"))
}

/// A sort prefix for a rank, so ranked proposals appear in order.
#[must_use]
pub fn rank_level(rank: Rank) -> u8 {
    match rank {
        Rank::Exact => 0,
        Rank::Likely => 1,
        Rank::Plausible => 2,
        Rank::Valid => 3,
    }
}

/// Ranks `$ref` targets for a position.
///
/// A component is `Likely` when the workspace already references it — the
/// candidates a maintainer has actually used — and `Plausible` otherwise.
/// `expected` is the component section the position can accept, so a
/// reference where a schema belongs never suggests a security scheme.
#[must_use]
pub fn rank_refs(
    index: &Index,
    current_uri: &str,
    position: &Pointer,
    expected: &[&str],
    all: Vec<String>,
) -> Vec<Proposal> {
    let used: BTreeMap<&str, usize> = {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for reference in index.references() {
            *counts.entry(reference.text.as_str()).or_default() += 1;
        }
        counts
    };
    // Pointer distance: a component defined near the cursor is usually the
    // one being reached for.
    let home = position.to_path();

    let mut proposals: Vec<Proposal> = all
        .into_iter()
        .filter_map(|insert| {
            let section = insert
                .rsplit('/')
                .nth(1)
                .map(str::to_owned)
                .unwrap_or_default();
            if !expected.is_empty() && !expected.contains(&section.as_str()) {
                return None;
            }
            let is_local = !insert.contains('#') || insert.starts_with("#/");
            let kind_ok = is_local || insert.contains(current_uri);
            let count = used.get(insert.as_str()).copied().unwrap_or(0);
            let rank = if count > 0 {
                Rank::Likely
            } else if kind_ok {
                Rank::Plausible
            } else {
                Rank::Valid
            };
            let detail = if count > 0 {
                format!("referenced {count} time(s) already — {}", rank.label())
            } else {
                format!("{section} — {}", rank.label())
            };
            Some(Proposal {
                insert,
                rank,
                detail,
                // How often the workspace already reaches for it.
                weight: count,
            })
        })
        .collect();

    proposals.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| b.weight.cmp(&a.weight))
            .then_with(|| {
                pointer_distance(&home, &a.insert).cmp(&pointer_distance(&home, &b.insert))
            })
            .then_with(|| a.insert.cmp(&b.insert))
    });
    proposals
}

/// How many components two pointers share: a small number means the
/// candidates sit near each other in the document.
fn pointer_distance(from: &str, to: &str) -> usize {
    let to = to.trim_start_matches('#').trim_start_matches('/');
    let from: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    from.iter().zip(&to).take_while(|(a, b)| a == b).count()
}

/// Sibling property names, ranked by whether the enclosing schema requires
/// them: a required sibling the author has not written yet is the single
/// most useful completion in a schema.
#[must_use]
pub fn rank_siblings(low: &LowDoc, offset: usize, already: &[String]) -> Vec<Proposal> {
    let model = crate::meaning::Model::new(low);
    let Some(meaning) = model.at(offset) else {
        return Vec::new();
    };
    // The cursor is usually inside a property's own schema, or on the key
    // of a property. The siblings live in the object that *declares*
    // `properties`, which the pointer names directly.
    let Some(declaring) = declaring_schema_pointer(&meaning.pointer) else {
        return Vec::new();
    };
    let Some(schema) = low.root().pointer(&declaring) else {
        return Vec::new();
    };
    let Some(properties) = schema.get("properties") else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .map(|node| {
            node.items()
                .iter()
                .filter_map(|item| item.as_str())
                .collect()
        })
        .unwrap_or_default();
    let mut proposals: Vec<Proposal> = properties
        .entries()
        .into_iter()
        .filter(|entry| !already.iter().any(|name| name == entry.key))
        .map(|entry| {
            let is_required = required.contains(&entry.key);
            let kind = entry
                .value
                .and_then(|value| value.get("type"))
                .and_then(|node| node.as_str())
                .unwrap_or("");
            Proposal {
                insert: entry.key.to_owned(),
                rank: if is_required {
                    Rank::Exact
                } else {
                    Rank::Valid
                },
                weight: usize::from(is_required),
                detail: if is_required {
                    format!(
                        "required{}",
                        if kind.is_empty() {
                            String::new()
                        } else {
                            format!(", {kind}")
                        }
                    )
                } else if kind.is_empty() {
                    "optional property".to_owned()
                } else {
                    format!("optional, {kind}")
                },
            }
        })
        .collect();
    proposals.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.insert.cmp(&b.insert)));
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::Source;

    fn doc(text: &str) -> LowDoc {
        LowDoc::parse(
            "mem://rank.yaml".into(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    const SPEC: &str = r#"
openapi: 3.1.0
info: {title: Rank, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - name: status
          in: query
          schema:
            type: string
            enum: [available, pending, sold]
            default: available
      responses:
        '200':
          description: ok
components:
  schemas:
    Pet:
      type: object
      required: [id]
      properties:
        id: {type: string}
        tag: {type: string}
        age: {type: integer}
    Error: {type: object}
  securitySchemes:
    apiKey: {type: apiKey, in: header, name: X-Key}
"#;

    #[test]
    fn schema_values_come_from_the_enclosing_schema() {
        let low = doc(SPEC);
        let offset = low
            .inner()
            .bytes()
            .windows(6)
            .position(|w| w == b"pendin")
            .unwrap()
            + 2;
        let values = schema_values(&low, offset);
        assert_eq!(
            values,
            vec![
                "available".to_owned(),
                "pending".to_owned(),
                "sold".to_owned()
            ],
            "the enum is what the position permits"
        );
    }

    #[test]
    fn a_required_sibling_outranks_an_optional_one() {
        let low = doc(SPEC);
        let offset = low
            .inner()
            .bytes()
            .windows(9)
            .position(|w| w == b"tag: {typ")
            .unwrap()
            + 2;

        // Nothing written yet: the required sibling leads.
        let proposals = rank_siblings(&low, offset, &[]);
        assert_eq!(proposals[0].insert, "id");
        assert_eq!(
            proposals[0].rank,
            Rank::Exact,
            "a required sibling is the most useful completion"
        );
        assert!(
            proposals[0].detail.contains("required"),
            "{}",
            proposals[0].detail
        );
        let others: Vec<&str> = proposals[1..].iter().map(|p| p.insert.as_str()).collect();
        assert!(others.contains(&"tag") && others.contains(&"age"));
        assert!(
            proposals[1..].iter().all(|p| p.rank == Rank::Valid),
            "the rest are ordinary optional properties"
        );

        // `id` already written: it is not offered again, and the remaining
        // optional properties tie-break by name.
        let remaining = rank_siblings(&low, offset, &["id".to_owned()]);
        let names: Vec<&str> = remaining.iter().map(|p| p.insert.as_str()).collect();
        assert_eq!(names, vec!["age", "tag"], "equal ranks order by name");
    }

    #[test]
    fn a_used_component_outranks_an_unused_one() {
        let low = doc(SPEC);
        let mut index = Index::default();
        index.index_document(low.uri().as_str(), &low);
        // Make one component already referenced twice.
        let extra = r#"
openapi: 3.1.0
info: {title: Extra, version: '1'}
paths: {}
components:
  schemas:
    A: {$ref: '#/components/schemas/Pet'}
    B: {$ref: '#/components/schemas/Pet'}
    C: {$ref: '#/components/schemas/Error'}
"#;
        let other = doc(extra);
        index.index_document(other.uri().as_str(), &other);

        let position = Pointer::parse("/paths/~1pets/get").expect("pointer");
        let all = vec![
            "#/components/schemas/Pet".to_owned(),
            "#/components/schemas/Error".to_owned(),
            "#/components/schemas/A".to_owned(),
        ];
        let ranked = rank_refs(&index, low.uri().as_str(), &position, &["schemas"], all);
        assert_eq!(ranked[0].insert, "#/components/schemas/Pet");
        assert_eq!(ranked[0].rank, Rank::Likely);
        assert!(ranked[0].detail.contains("referenced"), "{:?}", ranked[0]);
        // A schema position never suggests a security scheme.
        assert!(
            ranked.iter().all(|p| p.insert.contains("schemas")),
            "{ranked:?}"
        );
    }

    #[test]
    fn ref_ranking_filters_to_the_expected_section() {
        let low = doc(SPEC);
        let index = Index::default();
        let position = Pointer::parse("/paths").expect("pointer");
        let all = vec![
            "#/components/schemas/Pet".to_owned(),
            "#/components/securitySchemes/apiKey".to_owned(),
        ];
        let ranked = rank_refs(&index, low.uri().as_str(), &position, &["schemas"], all);
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].insert.contains("schemas"));
    }
}
