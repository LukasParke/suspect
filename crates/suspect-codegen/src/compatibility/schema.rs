//! Sufficient inclusion proofs, intentionally not a general schema diff oracle.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, ContractSeverity, Schema, SchemaDialect, SchemaId};

use super::{Direction, Impact, Location, SchemaDelta};

pub(super) type Correspondence = BTreeMap<SchemaId, BTreeSet<SchemaId>>;

pub(super) struct Assessment {
    pub changed: bool,
    pub impact: Impact,
    pub reasoning: Vec<String>,
    pub deltas: Vec<SchemaDelta>,
}

pub(super) fn assess(
    old: &Contract,
    old_id: &SchemaId,
    new: &Contract,
    new_id: &SchemaId,
    direction: Direction,
    correspondence: &mut Correspondence,
) -> Assessment {
    let mut deltas = Vec::new();
    let mut budget = Budget::new();
    let traversal = differences(
        old,
        old_id,
        new,
        new_id,
        correspondence,
        &mut deltas,
        &mut budget,
    );
    let equal = equivalent(old, old_id, new, new_id, &mut budget);
    if equal == Ok(true) && traversal.is_ok() {
        return Assessment {
            changed: false,
            impact: Impact::Compatible,
            reasoning: vec![],
            deltas,
        };
    }
    let mut reasons = Vec::new();
    if let Err(reason) = traversal {
        reasons.push(reason);
    }
    if let Err(reason) = equal {
        reasons.push(reason);
    }
    if !reasons.is_empty() {
        return Assessment {
            changed: true,
            impact: Impact::Unknown,
            reasoning: reasons,
            deltas,
        };
    }
    let (from, from_id, to, to_id, rule) = match direction {
        Direction::Request => (
            old,
            old_id,
            new,
            new_id,
            "old request values are included in the new input domain",
        ),
        Direction::Response => (
            new,
            new_id,
            old,
            old_id,
            "new response values are included in the old output domain",
        ),
    };
    let proof = subset(
        from,
        from_id,
        to,
        to_id,
        &mut budget,
        &mut BTreeSet::new(),
        0,
    );
    let (impact, reasoning) = match proof {
        Proof::Yes => (Impact::Compatible, vec![format!("Proved by sufficient structural inclusion rules: {rule}.")]),
        Proof::No(reason) => (Impact::PotentiallyBreaking, vec![format!("Could not prove that {rule}: {reason}"),
            "No satisfiability or counterexample proof is claimed; other assertions may make the apparent narrowing redundant.".into()]),
        Proof::Unknown(reason) => (Impact::Unknown, vec![format!("Could not establish whether {rule}: {reason}")]),
    };
    Assessment {
        changed: true,
        impact,
        reasoning,
        deltas,
    }
}

struct Budget {
    remaining: usize,
}
impl Budget {
    fn new() -> Self {
        Self { remaining: 32_768 }
    }
    fn tick(&mut self) -> Result<(), String> {
        if self.remaining == 0 {
            return Err("schema comparison work budget exhausted; compatibility is unknown".into());
        }
        self.remaining -= 1;
        Ok(())
    }
}

/// Schema annotations only: never strip identically named properties or keys
/// inside enum/const/example instance values.
fn annotation(key: &str) -> bool {
    matches!(
        key,
        "title"
            | "description"
            | "$comment"
            | "example"
            | "examples"
            | "default"
            | "deprecated"
            | "externalDocs"
    )
}

fn source_metadata(key: &str) -> bool {
    // References below are compared by canonical target edges. Dynamic scope is
    // rejected before these address-only fields can be ignored.
    matches!(key, "$id" | "$anchor" | "$schema" | "$defs" | "definitions")
}

fn checked<'a>(contract: &'a Contract, id: &SchemaId) -> Result<Schema<'a>, String> {
    let schema = contract
        .schema(id)
        .ok_or_else(|| format!("schema is not indexed: {}#{}", id.document(), id.pointer()))?;
    if !matches!(schema.raw(), Value::Bool(_) | Value::Object(_)) {
        return Err("schema is neither an object nor a Boolean".into());
    }
    if let SchemaDialect::Uri(uri) = schema.dialect()
        && !matches!(
            uri.trim_end_matches('#'),
            "https://spec.openapis.org/oas/3.1/dialect/base"
                | "https://json-schema.org/draft/2020-12/schema"
        )
    {
        return Err(format!("no compatibility proof profile for dialect {uri}"));
    }
    if !schema.ignores_ref_siblings()
        && (schema.raw().get("$dynamicRef").is_some()
            || schema.raw().get("$dynamicAnchor").is_some())
    {
        return Err(
            "dynamic reference scope is not part of the static compatibility proof profile".into(),
        );
    }
    sanity(schema)?;
    if let Some(diagnostic) = contract.diagnostics().iter().find(|d| {
        d.severity == ContractSeverity::Error
            && contract.schema_diagnostic_applies(std::slice::from_ref(id), d)
    }) {
        return Err(format!(
            "contract diagnostic {}: {}",
            diagnostic.code, diagnostic.message
        ));
    }
    if schema
        .references()
        .iter()
        .any(|reference| reference.target.is_none())
    {
        return Err(
            "unresolved schema reference; equal reference text is not an equivalence proof".into(),
        );
    }
    Ok(schema)
}

/// Contract views preserve malformed keyword values too. Equality of malformed
/// source is not a proof, even when no native target was requested.
fn sanity(schema: Schema<'_>) -> Result<(), String> {
    let Some(raw) = schema.raw().as_object() else {
        return Ok(());
    };
    if schema.ignores_ref_siblings() {
        return if raw.get("$ref").is_some_and(Value::is_string) {
            Ok(())
        } else {
            Err("a schema reference must be a URI-reference string".into())
        };
    }
    let schema_value = |value: &Value| matches!(value, Value::Bool(_) | Value::Object(_));
    let strings = |value: &Value| {
        value.as_array().is_some_and(|values| {
            values.iter().all(Value::is_string)
                && values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == values.len()
        })
    };
    for (key, value) in raw {
        let valid = match key.as_str() {
            "type" => {
                types(raw, schema.dialect()).is_some() && (!value.is_array() || strings(value))
            }
            "enum" => value.is_array(),
            "required" => strings(value),
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                value
                    .as_object()
                    .is_some_and(|values| values.values().all(schema_value))
            }
            "additionalProperties"
            | "unevaluatedProperties"
            | "unevaluatedItems"
            | "items"
            | "contains"
            | "propertyNames"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contentSchema" => schema_value(value),
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => value
                .as_array()
                .is_some_and(|values| !values.is_empty() && values.iter().all(schema_value)),
            "minimum" | "maximum" => value.is_number(),
            "exclusiveMinimum" | "exclusiveMaximum" => {
                if matches!(schema.dialect(), SchemaDialect::OpenApi30) {
                    value.is_boolean()
                } else {
                    value.is_number()
                }
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" | "minContains" | "maxContains" => value.as_u64().is_some(),
            "uniqueItems" | "readOnly" | "writeOnly" | "deprecated" | "nullable" => {
                value.is_boolean()
            }
            "$ref" | "$id" | "$anchor" | "$schema" | "pattern" | "format" => value.is_string(),
            "dependentRequired" => value
                .as_object()
                .is_some_and(|values| values.values().all(strings)),
            "$vocabulary" => {
                return Err(
                    "vocabulary declarations require a vocabulary-aware equivalence proof".into(),
                );
            }
            "multipleOf" => value.as_number().is_some_and(|number| {
                let text = number.to_string();
                !text.starts_with('-')
                    && text
                        .split(['e', 'E'])
                        .next()
                        .is_some_and(|mantissa| mantissa.bytes().any(|b| matches!(b, b'1'..=b'9')))
            }),
            _ => true,
        };
        if !valid {
            return Err(format!(
                "`{key}` is malformed or outside the supported keyword-value proof profile"
            ));
        }
    }
    Ok(())
}

fn children(schema: Schema<'_>) -> BTreeMap<String, SchemaId> {
    if schema.ignores_ref_siblings() {
        return BTreeMap::new();
    }
    schema
        .children()
        .iter()
        .filter_map(|id| {
            let relative = id.pointer().strip_prefix(schema.id().pointer())?;
            if relative.starts_with("/$defs/") || relative.starts_with("/definitions/") {
                return None;
            }
            Some((relative.to_owned(), id.clone()))
        })
        .collect()
}

fn local(schema: Schema<'_>) -> Value {
    if schema.ignores_ref_siblings() {
        return serde_json::json!({"$ref":"<resolved-static-reference>"});
    }
    let mut value = schema.raw().clone();
    for relative in children(schema).keys() {
        if let Some(child) = value.pointer_mut(relative) {
            *child = Value::Null;
        }
    }
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| !annotation(key) && !source_metadata(key));
        if object.contains_key("$ref") {
            object.insert(
                "$ref".into(),
                Value::String("<resolved-static-reference>".into()),
            );
        }
        for key in ["required", "enum", "type"] {
            if let Some(Value::Array(values)) = object.get_mut(key) {
                values
                    .sort_by_cached_key(|value| serde_json::to_string(value).expect("schema JSON"));
            }
        }
    }
    value
}

/// Bisimilar static graphs have equal assertions without expanding recursion.
/// This rule also follows external targets when `$ref` text has not changed.
fn equivalent(
    a: &Contract,
    aid: &SchemaId,
    b: &Contract,
    bid: &SchemaId,
    budget: &mut Budget,
) -> Result<bool, String> {
    let mut pending = vec![(aid.clone(), bid.clone())];
    let mut seen = BTreeSet::new();
    let mut equal = true;
    while let Some((aid, bid)) = pending.pop() {
        budget.tick()?;
        if !seen.insert((aid.clone(), bid.clone())) {
            continue;
        }
        let left = checked(a, &aid)?;
        let right = checked(b, &bid)?;
        if left.dialect() != right.dialect() {
            return Err("schema dialect changed; cross-dialect inclusion is not proved".into());
        }
        equal &= local(left) == local(right);
        let lc = children(left);
        let rc = children(right);
        equal &= lc.keys().eq(rc.keys());
        for (key, child) in &lc {
            if let Some(other) = rc.get(key) {
                pending.push((child.clone(), other.clone()));
            }
        }
        let lr: BTreeMap<_, _> = left
            .references()
            .iter()
            .map(|r| (&r.keyword, &r.target))
            .collect();
        let rr: BTreeMap<_, _> = right
            .references()
            .iter()
            .map(|r| (&r.keyword, &r.target))
            .collect();
        equal &= lr.keys().eq(rr.keys());
        for (keyword, target) in lr {
            if let (Some(target), Some(Some(other))) = (target, rr.get(keyword)) {
                pending.push((target.clone(), other.clone()));
            }
        }
    }
    Ok(equal)
}

#[derive(Debug)]
enum Proof {
    Yes,
    No(String),
    Unknown(String),
}

impl Proof {
    fn all(proofs: impl IntoIterator<Item = Proof>) -> Self {
        let mut no = None;
        for proof in proofs {
            match proof {
                Self::Unknown(reason) => return Self::Unknown(reason),
                Self::No(reason) => {
                    no.get_or_insert(reason);
                }
                Self::Yes => {}
            }
        }
        no.map_or(Self::Yes, Self::No)
    }
}

fn pure_reference(schema: Schema<'_>) -> Option<SchemaId> {
    let object = schema.raw().as_object()?;
    if !object.contains_key("$ref") {
        return None;
    }
    if !schema.ignores_ref_siblings()
        && object
            .keys()
            .any(|key| key != "$ref" && !annotation(key) && !source_metadata(key))
    {
        return None;
    }
    schema
        .references()
        .iter()
        .find(|r| r.keyword == "$ref")
        .and_then(|r| r.target.clone())
}

fn subset(
    a: &Contract,
    aid: &SchemaId,
    b: &Contract,
    bid: &SchemaId,
    budget: &mut Budget,
    active: &mut BTreeSet<(SchemaId, SchemaId)>,
    depth: usize,
) -> Proof {
    if depth >= 128 {
        return Proof::Unknown("schema inclusion depth budget exhausted".into());
    }
    match equivalent(a, aid, b, bid, budget) {
        Ok(true) => return Proof::Yes,
        Err(reason) => return Proof::Unknown(reason),
        _ => {}
    }
    let key = (aid.clone(), bid.clone());
    if !active.insert(key.clone()) {
        return Proof::Unknown(
            "changed recursive schema requires a recursive inclusion proof".into(),
        );
    }
    let result = subset_inner(a, aid, b, bid, budget, active, depth);
    active.remove(&key);
    result
}

fn subset_inner(
    a: &Contract,
    aid: &SchemaId,
    b: &Contract,
    bid: &SchemaId,
    budget: &mut Budget,
    active: &mut BTreeSet<(SchemaId, SchemaId)>,
    depth: usize,
) -> Proof {
    let (left, right) = match (checked(a, aid), checked(b, bid)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(reason), _) | (_, Err(reason)) => return Proof::Unknown(reason),
    };
    if left.raw() == &Value::Bool(false) || right.raw() == &Value::Bool(true) {
        return Proof::Yes;
    }
    match (pure_reference(left), pure_reference(right)) {
        // Align two reference-only wrappers before comparing their targets. A
        // mixed-dialect closure need not compare one target with the other
        // side's inert wrapper; corresponding target dialects are still checked.
        (Some(left), Some(right)) => {
            return subset(a, &left, b, &right, budget, active, depth + 1);
        }
        (Some(target), None) => {
            return subset(a, &target, b, bid, budget, active, depth + 1);
        }
        (None, Some(target)) => {
            return subset(a, aid, b, &target, budget, active, depth + 1);
        }
        (None, None) => {}
    }
    if right.raw() == &Value::Bool(false) {
        return Proof::No("the destination schema rejects every value".into());
    }
    let empty = Map::new();
    let l = left.raw().as_object().unwrap_or(&empty);
    let r = right.raw().as_object().unwrap_or(&empty);
    let known = |key: &str| {
        annotation(key)
            || source_metadata(key)
            || matches!(
                key,
                "type"
                    | "nullable"
                    | "enum"
                    | "const"
                    | "required"
                    | "properties"
                    | "additionalProperties"
                    | "items"
                    | "minimum"
                    | "maximum"
                    | "exclusiveMinimum"
                    | "exclusiveMaximum"
                    | "minLength"
                    | "maxLength"
                    | "minItems"
                    | "maxItems"
                    | "uniqueItems"
                    | "minProperties"
                    | "maxProperties"
            )
    };
    if let Some(key) = l.keys().chain(r.keys()).find(|key| !known(key)) {
        return Proof::Unknown(format!(
            "changed schema contains `{key}`; composition, pattern, extension and annotation-dependent inclusion is not proved"
        ));
    }
    if matches!(left.dialect(), SchemaDialect::OpenApi30)
        && [l, r]
            .iter()
            .any(|o| o.contains_key("exclusiveMinimum") || o.contains_key("exclusiveMaximum"))
    {
        return Proof::Unknown(
            "OpenAPI 3.0 Boolean exclusive bounds need a separate inclusion rule".into(),
        );
    }
    let mut proofs = Vec::new();
    match (types(l, left.dialect()), types(r, right.dialect())) {
        (Some(left), Some(right)) => {
            if !left
                .iter()
                .all(|kind| right.contains(kind) || (kind == "integer" && right.contains("number")))
            {
                proofs.push(Proof::No("the type/null domain is narrowed".into()));
            }
        }
        _ => return Proof::Unknown("invalid or unrecognized type/nullable declaration".into()),
    }
    // const AND enum must each be implied. Treating the destination const as
    // its whole domain would incorrectly prove inclusion into a contradiction.
    let source_literals = literals(l);
    for restriction in [
        r.get("const").map(|value| vec![value]),
        r.get("enum")
            .and_then(Value::as_array)
            .map(|values| values.iter().collect()),
    ]
    .into_iter()
    .flatten()
    {
        match &source_literals {
            Some(values) if values.iter().all(|value| restriction.contains(value)) => {},
            Some(_) => proofs.push(Proof::No("enum/const inclusion was not proved using exact JSON values (numeric spellings are not approximated)".into())),
            None => proofs.push(Proof::No("the destination adds an enum/const restriction".into())),
        }
    }
    for (key, minimum) in [
        ("minimum", true),
        ("maximum", false),
        ("exclusiveMinimum", true),
        ("exclusiveMaximum", false),
        ("minLength", true),
        ("maxLength", false),
        ("minItems", true),
        ("maxItems", false),
        ("minProperties", true),
        ("maxProperties", false),
    ] {
        if let Some(new) = r.get(key) {
            let Some(old) = l.get(key) else {
                proofs.push(Proof::No(format!("the destination adds `{key}`")));
                continue;
            };
            if old == new {
                continue;
            }
            match (exact_integer(old), exact_integer(new)) {
                (Some(old), Some(new)) if if minimum { old >= new } else { old <= new } => {},
                (Some(_), Some(_)) => proofs.push(Proof::No(format!("the destination tightens `{key}`"))),
                _ => proofs.push(Proof::Unknown(format!("`{key}` needs exact decimal/big-integer implication; no floating-point comparison is used"))),
            }
        }
    }
    if r.get("uniqueItems") == Some(&Value::Bool(true))
        && l.get("uniqueItems") != Some(&Value::Bool(true))
    {
        proofs.push(Proof::No(
            "the destination requires unique array items".into(),
        ));
    }
    match (required(l), required(r)) {
        (Some(left), Some(right)) if !right.is_subset(&left) => proofs.push(Proof::No(
            "the destination requires additional object properties".into(),
        )),
        (None, _) | (_, None) => {
            return Proof::Unknown("invalid required-property collection".into());
        }
        _ => {}
    }
    let lp = l.get("properties").and_then(Value::as_object);
    let rp = r.get("properties").and_then(Value::as_object);
    let property_keys: BTreeSet<_> = lp
        .into_iter()
        .flat_map(Map::keys)
        .chain(rp.into_iter().flat_map(Map::keys))
        .collect();
    for key in property_keys {
        let left = if lp.is_some_and(|p| p.contains_key(key)) {
            Slot::At(aid.child("properties").child(key))
        } else {
            extra(l, aid)
        };
        let right = if rp.is_some_and(|p| p.contains_key(key)) {
            Slot::At(bid.child("properties").child(key))
        } else {
            extra(r, bid)
        };
        proofs.push(subset_slot(a, left, b, right, budget, active, depth + 1));
    }
    proofs.push(subset_slot(
        a,
        extra(l, aid),
        b,
        extra(r, bid),
        budget,
        active,
        depth + 1,
    ));
    let items = |raw: &Map<String, Value>, id: &SchemaId| {
        if raw.contains_key("items") {
            Slot::At(id.child("items"))
        } else {
            Slot::Any
        }
    };
    proofs.push(subset_slot(
        a,
        items(l, aid),
        b,
        items(r, bid),
        budget,
        active,
        depth + 1,
    ));
    Proof::all(proofs)
}

enum Slot {
    Any,
    Never,
    At(SchemaId),
}
fn extra(raw: &Map<String, Value>, id: &SchemaId) -> Slot {
    match raw.get("additionalProperties") {
        None | Some(Value::Bool(true)) => Slot::Any,
        Some(Value::Bool(false)) => Slot::Never,
        _ => Slot::At(id.child("additionalProperties")),
    }
}
fn subset_slot(
    a: &Contract,
    left: Slot,
    b: &Contract,
    right: Slot,
    budget: &mut Budget,
    active: &mut BTreeSet<(SchemaId, SchemaId)>,
    depth: usize,
) -> Proof {
    match (left, right) {
        (Slot::Never, _) | (_, Slot::Any) => Proof::Yes,
        (Slot::At(left), Slot::At(right)) => subset(a, &left, b, &right, budget, active, depth),
        (Slot::At(left), Slot::Never)
            if a.schema(&left)
                .is_some_and(|s| s.raw() == &Value::Bool(false)) =>
        {
            Proof::Yes
        }
        (Slot::Any, Slot::At(right))
            if b.schema(&right).is_some_and(|s| {
                s.raw() == &Value::Bool(true) || s.raw().as_object().is_some_and(Map::is_empty)
            }) =>
        {
            Proof::Yes
        }
        _ => Proof::No("a property/item domain or additional-properties policy is narrowed".into()),
    }
}

fn exact_integer(value: &Value) -> Option<i128> {
    // This deliberately declines decimals/exponents and out-of-range integers.
    // serde_json f64 accessors would lose information in canonical contracts.
    value.as_number()?.to_string().parse().ok()
}

fn types(raw: &Map<String, Value>, dialect: &SchemaDialect) -> Option<BTreeSet<String>> {
    let all = [
        "null", "boolean", "string", "integer", "number", "array", "object",
    ];
    let mut types: BTreeSet<String> = match raw.get("type") {
        None => all.iter().map(|kind| (*kind).into()).collect(),
        Some(Value::String(kind)) => BTreeSet::from([kind.clone()]),
        Some(Value::Array(values)) if !matches!(dialect, SchemaDialect::OpenApi30) => values
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<_>>()?,
        _ => return None,
    };
    if types.is_empty() || types.iter().any(|kind| !all.contains(&kind.as_str())) {
        return None;
    }
    if matches!(dialect, SchemaDialect::OpenApi30) && raw.contains_key("type") {
        match raw.get("nullable") {
            Some(Value::Bool(true)) => {
                types.insert("null".into());
            }
            None | Some(Value::Bool(false)) => {}
            _ => return None,
        }
    }
    Some(types)
}

fn required(raw: &Map<String, Value>) -> Option<BTreeSet<&str>> {
    match raw.get("required") {
        None => Some(BTreeSet::new()),
        Some(Value::Array(values)) => values.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

fn literals(raw: &Map<String, Value>) -> Option<Vec<&Value>> {
    // Using const alone is a safe superset of const AND enum. Empty intersection
    // and mathematically equal numeric spellings are not inferred here.
    raw.get("const").map(|value| vec![value]).or_else(|| {
        raw.get("enum")
            .and_then(Value::as_array)
            .map(|v| v.iter().collect())
    })
}

fn differences(
    old: &Contract,
    old_id: &SchemaId,
    new: &Contract,
    new_id: &SchemaId,
    correspondence: &mut Correspondence,
    deltas: &mut Vec<SchemaDelta>,
    budget: &mut Budget,
) -> Result<(), String> {
    let mut pending = vec![(old_id.clone(), new_id.clone())];
    let mut seen = BTreeSet::new();
    while let Some((old_id, new_id)) = pending.pop() {
        budget.tick()?;
        if !seen.insert((old_id.clone(), new_id.clone())) {
            continue;
        }
        correspondence
            .entry(old_id.clone())
            .or_default()
            .insert(new_id.clone());
        let (Some(left), Some(right)) = (old.schema(&old_id), new.schema(&new_id)) else {
            continue;
        };
        let (ll, rr) = (local(left), local(right));
        if left.dialect() != right.dialect() {
            deltas.push(SchemaDelta {
                keyword: "$schema".into(),
                source_before: Some(Location::at(old, &old_id)),
                source_after: Some(Location::at(new, &new_id)),
                before: Some(Value::String(format!("{:?}", left.dialect()))),
                after: Some(Value::String(format!("{:?}", right.dialect()))),
            });
        }
        match (ll.as_object(), rr.as_object()) {
            (Some(l), Some(r)) => {
                let keys: BTreeSet<_> = l.keys().chain(r.keys()).collect();
                for key in keys {
                    if l.get(key) != r.get(key) {
                        deltas.push(SchemaDelta {
                            keyword: key.clone(),
                            source_before: left
                                .raw()
                                .get(key)
                                .map(|_| Location::at(old, &old_id.child(key))),
                            source_after: right
                                .raw()
                                .get(key)
                                .map(|_| Location::at(new, &new_id.child(key))),
                            before: left.raw().get(key).cloned(),
                            after: right.raw().get(key).cloned(),
                        });
                    }
                }
            }
            _ if ll != rr => deltas.push(SchemaDelta {
                keyword: "schema".into(),
                source_before: Some(Location::at(old, &old_id)),
                source_after: Some(Location::at(new, &new_id)),
                before: Some(left.raw().clone()),
                after: Some(right.raw().clone()),
            }),
            _ => {}
        }
        let lc = children(left);
        let rc = children(right);
        for (key, child) in lc {
            if let Some(other) = rc.get(&key) {
                pending.push((child, other.clone()));
            }
        }
        for reference in left.references() {
            if let (Some(target), Some(other)) = (
                &reference.target,
                right
                    .references()
                    .iter()
                    .find(|r| r.keyword == reference.keyword)
                    .and_then(|r| r.target.as_ref()),
            ) {
                pending.push((target.clone(), other.clone()));
            }
        }
    }
    Ok(())
}
