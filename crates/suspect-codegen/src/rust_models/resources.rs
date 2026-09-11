//! Resource-sensitive native carrier planning over canonical physical SchemaIds.
use crate::schema_view;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{AnchorKind, Contract, SchemaId};

/// Keep ordinary closures on the existing profile without attempting a second
/// schema interpretation or adding dynamic candidate bindings as body inputs.
pub(crate) fn required(contract: &Contract, roots: &[SchemaId]) -> bool {
    schema_view::closure(contract, roots).iter().any(|id| {
        let Some(schema) = contract.schema(id) else {
            return false;
        };
        let raw = schema_view::raw(schema);
        raw.get("$id").is_some()
            || raw.get("$dynamicRef").is_some()
            || raw.get("$dynamicAnchor").is_some()
            || contract.resource_scope(id).is_some_and(|scope| {
                scope.base_source().is_some()
                    || contract.resource(scope.resource()).is_some_and(|resource| {
                        resource.anchors().iter().any(|anchor| {
                            // Selecting below an id-less dynamic-anchor
                            // declaration retains that lexical admission
                            // requirement without evaluating its parent.
                            anchor.kind() == AnchorKind::Dynamic
                                && anchor.target().document() == id.document()
                                && id
                                    .pointer()
                                    .strip_prefix(anchor.target().pointer())
                                    .is_some_and(|tail| tail.is_empty() || tail.starts_with('/'))
                        })
                    })
            })
    })
}

/// A conservative reverse closure: only native representation decisions depend
/// on this set. Runtime targets still come from the checked DynamicRef opcode.
pub(super) fn dynamic_dependents(
    contract: &Contract,
    reachable: &[SchemaId],
) -> BTreeSet<SchemaId> {
    let available: BTreeSet<_> = reachable.iter().cloned().collect();
    let mut reverse: BTreeMap<SchemaId, Vec<SchemaId>> = BTreeMap::new();
    let mut pending = Vec::new();
    for id in reachable {
        let Some(schema) = contract.schema(id) else {
            continue;
        };
        if !schema.ignores_ref_siblings() {
            if schema.raw().get("$dynamicRef").is_some() {
                pending.push(id.clone());
            }
            for child in schema.children() {
                if available.contains(child) {
                    reverse.entry(child.clone()).or_default().push(id.clone());
                }
            }
            for candidate in contract
                .dynamic_reference(id)
                .into_iter()
                .flat_map(|r| r.candidates())
            {
                if available.contains(candidate.target()) {
                    reverse
                        .entry(candidate.target().clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
        for target in schema.references().iter().filter_map(|r| r.target.as_ref()) {
            if available.contains(target) {
                reverse.entry(target.clone()).or_default().push(id.clone());
            }
        }
    }
    let mut affected = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if affected.insert(id.clone()) {
            pending.extend(reverse.get(&id).into_iter().flatten().cloned());
        }
    }
    affected
}

pub(super) fn null_allowed(
    contract: &Contract,
    id: &SchemaId,
    dynamic: bool,
) -> Result<bool, schema_view::Problem> {
    if !dynamic {
        return super::applicators::null_allowed(contract, id);
    }
    let schema = contract.schema(id).ok_or_else(|| schema_view::Problem {
        source: id.clone(),
        code: "unknown-model-root",
        message: "resource carrier requires an indexed schema",
    })?;
    let raw = schema_view::raw(schema);
    // Local assertions can prove non-null independently of every dynamic
    // binding. Otherwise retain null in the exact JSON carrier; no fallback
    // assumption may exclude a value admitted by an entered outer resource.
    Ok(schema_view::accepts_literal(schema, &Value::Null)
        && !raw.get("const").is_some_and(|v| !v.is_null())
        && !raw
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.iter().any(Value::is_null)))
}
