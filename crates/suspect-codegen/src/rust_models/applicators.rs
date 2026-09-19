//! Rust carrier nullability proof for v2. This is a finite representation proof,
//! not a replacement for the source-bound owned/native schema evaluators.
use crate::schema_view;
use serde_json::Value;
use std::collections::BTreeSet;
use suspect_ir::contract::{Contract, SchemaId};

pub(super) fn null_allowed(
    contract: &Contract,
    root: &SchemaId,
    policy: schema_view::DialectPolicy,
) -> Result<bool, schema_view::Problem> {
    fn visit(
        contract: &Contract,
        id: &SchemaId,
        active: &mut BTreeSet<SchemaId>,
        work: &mut usize,
        policy: schema_view::DialectPolicy,
    ) -> Option<bool> {
        *work = work.checked_sub(1)?;
        if active.len() >= 256 || !active.insert(id.clone()) {
            return None;
        }
        let result = (|| {
            let schema = contract.schema(id)?;
            let value = schema_view::raw(schema);
            if let Some(value) = value.as_bool() {
                return Some(value);
            }
            let raw = value.as_object()?;
            if raw.contains_key("$dynamicRef") || raw.contains_key("$recursiveRef") {
                return None;
            }
            // A local type/literal rejection is a sufficient non-null proof,
            // independent of later branch/resource behavior. It never implies
            // that any other instance will validate against the whole schema.
            if !schema_view::accepts_literal(schema, &Value::Null, policy)
                || raw.get("const").is_some_and(|v| !v.is_null())
                || raw
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_some_and(|v| !v.iter().any(Value::is_null))
            {
                return Some(false);
            }
            let mut accepts = true;
            for reference in schema.references() {
                accepts &= visit(contract, reference.target.as_ref()?, active, work, policy)?;
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                if let Some(values) = raw.get(keyword).and_then(Value::as_array) {
                    let mut matched = 0;
                    for index in 0..values.len() {
                        matched += usize::from(visit(
                            contract,
                            &id.child(keyword).child(&index.to_string()),
                            active,
                            work,
                            policy,
                        )?);
                    }
                    accepts &= match keyword {
                        "allOf" => matched == values.len(),
                        "anyOf" => matched != 0,
                        _ => matched == 1,
                    };
                }
            }
            if raw.contains_key("not") {
                accepts &= !visit(contract, &id.child("not"), active, work, policy)?;
            }
            if raw.contains_key("if") && (raw.contains_key("then") || raw.contains_key("else")) {
                let selected = if visit(contract, &id.child("if"), active, work, policy)? {
                    "then"
                } else {
                    "else"
                };
                if raw.contains_key(selected) {
                    accepts &= visit(contract, &id.child(selected), active, work, policy)?;
                }
            }
            // All remaining modern applicators apply only to objects/arrays;
            // propertyNames applies to strings created from object member names.
            Some(accepts)
        })();
        active.remove(id);
        result
    }
    visit(contract,root,&mut BTreeSet::new(),&mut 100_000,policy).ok_or_else(||schema_view::Problem{
        source:root.clone(),code:"native-nullability-analysis",message:"v2 carrier nullability proof is incomplete within the finite static-reference analysis profile",
    })
}
