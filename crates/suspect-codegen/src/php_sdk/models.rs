//! Native representation planning. Validation semantics stay in OwnedCompiler.

use crate::http_contract::{HttpDiagnostic, Operation};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{Contract, SchemaId};

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub wire: String,
    pub source: SchemaId,
    pub required: bool,
    pub description: String,
    /// Required single-valued string tags are initialized by the constructor.
    pub initializer: Option<Initializer>,
}

#[derive(Debug, Clone)]
pub enum Initializer {
    EnumCase {
        type_name: String,
        case_name: String,
        wire_value: String,
    },
}

/// Actual allocated codec methods; compatibility adapters never infer these from PHP text.
#[derive(Debug, Clone)]
pub struct CodecSymbols {
    pub decode: String,
    pub encode: String,
    pub from_value: String,
    pub to_value: String,
}

#[derive(Debug, Clone)]
pub struct Constructor {
    pub method: String,
    /// Parameter/member names in actual PHP constructor order.
    pub parameters: Vec<String>,
    pub extra_parameter: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Shape {
    /// The actual arbitrary JSON domain (including conditionally typed assertions).
    Json,
    Null,
    Boolean,
    Number,
    String,
    Enum {
        cases: Vec<(String, String)>,
    },
    Object {
        fields: Vec<Field>,
        extras: Extras,
    },
    Array {
        item: Option<SchemaId>,
    },
    Ref(SchemaId),
    Union(Vec<SchemaId>),
    /// Native union from a JSON Schema `type` array.
    Types(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum Extras {
    Closed,
    Json,
    Typed(SchemaId),
    /// Whole-object validation dispatches every matching pattern. Values stay lossless.
    Patterned(Vec<(String, SchemaId)>),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub source: SchemaId,
    /// Stable PHP symbol / codec suffix.
    pub name: String,
    pub shape: Shape,
    pub nullable: bool,
    pub description: String,
    pub constructor: Option<Constructor>,
    pub codecs: CodecSymbols,
}

#[derive(Debug)]
pub struct ModelPlan {
    pub nodes: BTreeMap<SchemaId, Node>,
}

impl ModelPlan {
    /// Render a native PHP type or a PHPStan/Psalm-compatible PHPDoc type.
    pub fn type_name(&self, id: &SchemaId, phpdoc: bool) -> String {
        self.ty(id, phpdoc, &mut BTreeSet::new(), &mut BTreeMap::new())
    }

    fn ty(
        &self,
        id: &SchemaId,
        phpdoc: bool,
        active: &mut BTreeSet<SchemaId>,
        memo: &mut BTreeMap<SchemaId, String>,
    ) -> String {
        if let Some(name) = memo.get(id) {
            return name.clone();
        }
        let node = &self.nodes[id];
        let mut result = match &node.shape {
            Shape::Object { .. } | Shape::Enum { .. } => node.name.clone(),
            Shape::Json => "JsonValue".into(),
            Shape::Null => "null".into(),
            Shape::Boolean => "bool".into(),
            Shape::Number => "JsonNumber".into(),
            Shape::String => "string".into(),
            Shape::Array { item } => {
                if phpdoc {
                    format!(
                        "list<{}>",
                        item.as_ref()
                            .map(|i| self.ty(i, phpdoc, active, memo))
                            .unwrap_or_else(|| "JsonValue".into())
                    )
                } else {
                    "array".into()
                }
            }
            Shape::Types(types) => union(types.iter().map(|t| type_keyword(t, phpdoc)).collect()),
            Shape::Ref(target) => {
                assert!(
                    active.insert(id.clone()),
                    "reference cycles rejected during planning"
                );
                let ty = self.ty(target, phpdoc, active, memo);
                active.remove(id);
                ty
            }
            Shape::Union(branches) => {
                assert!(
                    active.insert(id.clone()),
                    "unproductive union cycles rejected during planning"
                );
                let ty = union(
                    branches
                        .iter()
                        .map(|b| self.ty(b, phpdoc, active, memo))
                        .collect(),
                );
                active.remove(id);
                ty
            }
        };
        if node.nullable {
            // A null inside list<A|null|B> does not make the list itself nullable.
            result = union(vec![result, "null".into()]);
        }
        memo.insert(id.clone(), result.clone());
        result
    }
}

pub(crate) fn union(types: Vec<String>) -> String {
    // Native unions never contain generics. PHPDoc lists may contain nested
    // unions, so split only at angle-bracket depth zero.
    let mut unique = BTreeSet::new();
    for ty in types {
        let mut depth = 0usize;
        let mut start = 0;
        for (at, c) in ty.char_indices() {
            if c == '<' {
                depth += 1;
            }
            if c == '>' {
                depth = depth.saturating_sub(1);
            }
            if c == '|' && depth == 0 {
                unique.insert(ty[start..at].to_owned());
                start = at + 1;
            }
        }
        unique.insert(ty[start..].to_owned());
    }
    unique.into_iter().collect::<Vec<_>>().join("|")
}

fn type_keyword(kind: &str, phpdoc: bool) -> String {
    match kind {
        "string" => "string",
        "boolean" => "bool",
        "integer" | "number" => "JsonNumber",
        "null" => "null",
        "array" if phpdoc => "list<JsonValue>",
        "array" => "array",
        // Type-only objects retain their distinct JSON object representation.
        "object" => "JsonValue",
        _ => unreachable!("OwnedCompiler checked type keyword"),
    }
    .into()
}

#[cfg(not(feature = "http-protocol"))]
pub(crate) fn plan(
    contract: &Contract,
    reachable: &[SchemaId],
    operations: &[Operation],
    used: &mut BTreeSet<String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    plan_inner(
        contract,
        reachable,
        operations,
        &BTreeMap::new(),
        used,
        false,
        false,
    )
}

pub(crate) fn plan_named(
    contract: &Contract,
    reachable: &[SchemaId],
    hints: &BTreeMap<SchemaId, String>,
    used: &mut BTreeSet<String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    plan_inner(contract, reachable, &[], hints, used, false, false)
}

pub(crate) fn plan_scoped(
    contract: &Contract,
    reachable: &[SchemaId],
    hints: &BTreeMap<SchemaId, String>,
    used: &mut BTreeSet<String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    plan_inner(contract, reachable, &[], hints, used, true, false)
}

pub(crate) fn plan_resources(
    contract: &Contract,
    reachable: &[SchemaId],
    hints: &BTreeMap<SchemaId, String>,
    used: &mut BTreeSet<String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    plan_inner(contract, reachable, &[], hints, used, true, true)
}

fn plan_inner(
    contract: &Contract,
    reachable: &[SchemaId],
    operations: &[Operation],
    hints: &BTreeMap<SchemaId, String>,
    used: &mut BTreeSet<String>,
    scoped: bool,
    resources: bool,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let mut nodes = BTreeMap::new();
    for id in reachable {
        let schema = contract.schema(id).expect("checked closure");
        let raw = schema.raw();
        let hinted = if id.pointer().starts_with("/components/schemas/") {
            None
        } else {
            hints
                .iter()
                .filter(|(root, _)| {
                    root.document() == id.document()
                        && (id == *root
                            || id
                                .pointer()
                                .strip_prefix(root.pointer())
                                .is_some_and(|p| p.starts_with('/')))
                })
                .max_by_key(|(root, _)| root.pointer().len())
                .map(|(root, name)| {
                    format!("{name}{}", path_name(&id.pointer()[root.pointer().len()..]))
                })
        };
        let name = allocate(
            &hinted.unwrap_or_else(|| schema_name(contract, id, operations)),
            used,
        );
        let types: Vec<String> = match raw.get("type") {
            Some(Value::String(t)) => vec![t.clone()],
            Some(Value::Array(ts)) => ts
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let nullable = types.iter().any(|t| t == "null") && types.len() > 1;
        let nonnull: Vec<_> = types
            .iter()
            .filter(|t| t.as_str() != "null")
            .cloned()
            .collect();
        let literals = raw
            .get("enum")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| raw.get("const").map(|v| vec![v.clone()]));
        let string_literals = literals.as_ref().is_some_and(|values| {
            values.iter().any(Value::is_string)
                && values.iter().all(|v| v.is_string() || v.is_null())
        });
        // Branch trials cannot select a carrier outside the caller's dynamic context.
        let dynamic_union = resources && (raw.get("oneOf").is_some() || raw.get("anyOf").is_some());
        let complex_intersection = dynamic_union
            || scoped
                && (raw.get("$ref").is_some()
                    && [
                        "properties",
                        "additionalProperties",
                        "items",
                        "prefixItems",
                        "allOf",
                        "oneOf",
                        "anyOf",
                    ]
                    .iter()
                    .any(|k| raw.get(k).is_some())
                    || (raw.get("oneOf").is_some() || raw.get("anyOf").is_some())
                        && ["properties", "items", "prefixItems", "allOf"]
                            .iter()
                            .any(|k| raw.get(k).is_some())
                    || raw.get("oneOf").is_some() && raw.get("anyOf").is_some()
                    || nonnull.len() > 1
                        && ["properties", "items", "prefixItems", "additionalProperties"]
                            .iter()
                            .any(|k| raw.get(k).is_some()));
        let shape = if complex_intersection {
            Shape::Json
        } else if raw.get("$ref").is_some() {
            for keyword in [
                "properties",
                "additionalProperties",
                "items",
                "prefixItems",
                "allOf",
                "oneOf",
                "anyOf",
            ] {
                if raw.get(keyword).is_some() {
                    errors.push(super::diagnostic(
                        contract,
                        id.child(keyword),
                        "php-ref-shape-sibling",
                        "structural reference siblings require a separate native intersection plan",
                    ));
                }
            }
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone())
                .expect("OwnedCompiler checked reference");
            Shape::Ref(target)
        } else if let Some(branches) = raw
            .get("oneOf")
            .or_else(|| raw.get("anyOf"))
            .and_then(Value::as_array)
        {
            if raw.get("oneOf").is_some() && raw.get("anyOf").is_some() {
                errors.push(super::diagnostic(
                    contract,
                    id.clone(),
                    "php-composition-intersection",
                    "simultaneous oneOf and anyOf need an explicit native intersection",
                ));
            }
            for keyword in ["properties", "items", "prefixItems", "allOf"] {
                if raw.get(keyword).is_some() {
                    errors.push(super::diagnostic(
                        contract,
                        id.child(keyword),
                        "php-union-shape-sibling",
                        "structural union siblings require a native intersection",
                    ));
                }
            }
            let keyword = if raw.get("oneOf").is_some() {
                "oneOf"
            } else {
                "anyOf"
            };
            Shape::Union(
                (0..branches.len())
                    .map(|i| id.child(keyword).child(&i.to_string()))
                    .collect(),
            )
        } else if let Some(branches) = raw.get("allOf").and_then(Value::as_array) {
            if branches.len() == 1 && raw.get("properties").is_none() && raw.get("items").is_none()
            {
                Shape::Ref(id.child("allOf").child("0"))
            } else if scoped {
                Shape::Json
            } else {
                errors.push(super::diagnostic(contract, id.child("allOf"), "php-allof-native-unsupported", "multi-carrier allOf needs a proved native intersection; no JSON fallback is emitted"));
                Shape::Json
            }
        } else if string_literals && (types.is_empty() || nonnull == ["string"]) {
            let mut cases = Vec::new();
            let mut seen = BTreeSet::new();
            let mut case_names = BTreeSet::from([
                "cases".into(),
                "from".into(),
                "tryfrom".into(),
                "name".into(),
                "value".into(),
            ]);
            for value in literals.as_ref().expect("string literal set") {
                if let Some(text) = value.as_str()
                    && seen.insert(text.to_owned())
                {
                    cases.push((
                        allocate(
                            &pascal(if text.is_empty() { "Empty" } else { text }),
                            &mut case_names,
                        ),
                        text.to_owned(),
                    ));
                }
            }
            Shape::Enum { cases }
        } else if scoped
            && types.is_empty()
            && literals
                .as_ref()
                .is_some_and(|values| values.iter().any(|v| v.is_object() || v.is_array()))
        {
            Shape::Json
        } else if types.is_empty()
            && let Some(values) = &literals
        {
            let mut kinds = BTreeSet::new();
            for value in values {
                let kind = match value {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Object(_) | Value::Array(_) => {
                        errors.push(super::diagnostic(contract, id.clone(), "php-structural-literal-unsupported", "structural literals need an explicit native object/array carrier; no opaque fallback is emitted"));
                        continue;
                    }
                };
                kinds.insert(kind.to_owned());
            }
            let kinds: Vec<_> = kinds.into_iter().collect();
            match kinds.as_slice() {
                [kind] if kind == "null" => Shape::Null,
                [kind] if kind == "boolean" => Shape::Boolean,
                [kind] if kind == "number" => Shape::Number,
                [kind] if kind == "string" => Shape::String,
                _ => Shape::Types(kinds),
            }
        } else if nonnull.len() > 1 {
            if ["properties", "items", "prefixItems", "additionalProperties"]
                .iter()
                .any(|k| raw.get(k).is_some())
            {
                errors.push(super::diagnostic(
                    contract,
                    id.child("type"),
                    "php-type-union-shape",
                    "multi-kind unions with object/array shape assertions require branch schemas",
                ));
            }
            Shape::Types(types.clone())
        } else {
            match nonnull.first().map(String::as_str) {
                Some("string") => Shape::String,
                Some("boolean") => Shape::Boolean,
                Some("number" | "integer") => Shape::Number,
                Some("object") => {
                    let mut names = BTreeSet::from(["extra".into(), "this".into()]);
                    let required: BTreeSet<_> = raw
                        .get("required")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    let properties = raw.get("properties").and_then(Value::as_object);
                    for key in &required {
                        if !scoped && !properties.is_some_and(|p| p.contains_key(*key)) {
                            errors.push(super::diagnostic(
                                contract,
                                id.child("required"),
                                "php-required-undeclared",
                                "required keys must be declared properties for native construction",
                            ));
                        }
                    }
                    let fields = properties
                        .into_iter()
                        .flatten()
                        .map(|(wire, value)| Field {
                            name: allocate(&member(wire), &mut names),
                            wire: wire.clone(),
                            source: id.child("properties").child(wire),
                            required: required.contains(wire.as_str()),
                            description: value
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .into(),
                            initializer: None,
                        })
                        .collect();
                    let extras = if scoped
                        && raw
                            .get("patternProperties")
                            .and_then(Value::as_object)
                            .is_some_and(|patterns| !patterns.is_empty())
                    {
                        Extras::Patterned(
                            raw["patternProperties"]
                                .as_object()
                                .unwrap()
                                .keys()
                                .map(|name| {
                                    (name.clone(), id.child("patternProperties").child(name))
                                })
                                .collect(),
                        )
                    } else {
                        match raw.get("additionalProperties") {
                            Some(Value::Bool(false)) => Extras::Closed,
                            Some(Value::Object(_)) => {
                                Extras::Typed(id.child("additionalProperties"))
                            }
                            _ => Extras::Json,
                        }
                    };
                    Shape::Object { fields, extras }
                }
                Some("array") => {
                    if raw.get("prefixItems").is_some() && !scoped {
                        errors.push(super::diagnostic(
                            contract,
                            id.child("prefixItems"),
                            "php-tuple-native-unsupported",
                            "tuple arrays require a native positional representation",
                        ));
                    }
                    Shape::Array {
                        item: if scoped && raw.get("prefixItems").is_some() {
                            None
                        } else {
                            raw.get("items").map(|_| id.child("items"))
                        },
                    }
                }
                None if types == ["null"]
                    || literals.as_ref().is_some_and(|l| l == &[Value::Null]) =>
                {
                    Shape::Null
                }
                None => {
                    if !scoped
                        && ["properties", "additionalProperties", "items", "prefixItems"]
                            .iter()
                            .any(|k| raw.get(k).is_some())
                    {
                        errors.push(super::diagnostic(contract, id.clone(), "php-conditional-shape-unsupported", "structural assertions without an explicit native object/array type require a conditional carrier"));
                    }
                    Shape::Json
                }
                _ => unreachable!("checked schema types"),
            }
        };
        let nullable = nullable
            || matches!(&shape, Shape::Enum { .. })
                && types.is_empty()
                && literals.as_ref().is_some_and(|l| l.contains(&Value::Null));
        let codecs = CodecSymbols {
            decode: format!("decode{name}"),
            encode: format!("encode{name}"),
            from_value: format!("from{name}"),
            to_value: format!("to{name}"),
        };
        nodes.insert(
            id.clone(),
            Node {
                source: id.clone(),
                name,
                shape,
                nullable,
                description: raw
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                constructor: None,
                codecs,
            },
        );
    }
    let mut plan = ModelPlan { nodes };
    // Alias/union/list cycles without a nominal object cannot be expressed as
    // finite PHP type declarations. Productive recursive object fields are fine.
    fn height(
        plan: &ModelPlan,
        id: &SchemaId,
        active: &mut BTreeSet<SchemaId>,
        memo: &mut BTreeMap<SchemaId, usize>,
    ) -> Option<usize> {
        if let Some(height) = memo.get(id) {
            return Some(*height);
        }
        if active.len() >= 128 || !active.insert(id.clone()) {
            return None;
        }
        let child = match &plan.nodes[id].shape {
            Shape::Ref(target) | Shape::Array { item: Some(target) } => {
                height(plan, target, active, memo)?
            }
            Shape::Union(branches) => {
                let mut deepest = 0;
                for branch in branches {
                    deepest = deepest.max(height(plan, branch, active, memo)?);
                }
                deepest
            }
            _ => 0,
        };
        active.remove(id);
        if child >= 128 {
            return None;
        }
        memo.insert(id.clone(), child + 1);
        Some(child + 1)
    }
    let mut heights = BTreeMap::new();
    for id in reachable {
        if height(&plan, id, &mut BTreeSet::new(), &mut heights).is_none() {
            if scoped {
                plan.nodes.get_mut(id).unwrap().shape = Shape::Json;
                continue;
            }
            errors.push(super::diagnostic(
                contract,
                id.clone(),
                "php-unproductive-type-cycle",
                "aliases/unions/arrays require a finite nominal type graph of depth at most 128",
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    fn initializer(
        plan: &ModelPlan,
        id: &SchemaId,
        seen: &mut BTreeSet<SchemaId>,
    ) -> Option<Initializer> {
        if !seen.insert(id.clone()) {
            return None;
        }
        let node = &plan.nodes[id];
        if node.nullable {
            return None;
        }
        match &node.shape {
            Shape::Enum { cases } if cases.len() == 1 => Some(Initializer::EnumCase {
                type_name: node.name.clone(),
                case_name: cases[0].0.clone(),
                wire_value: cases[0].1.clone(),
            }),
            Shape::Ref(target) => initializer(plan, target, seen),
            _ => None,
        }
    }
    let initializers: BTreeMap<_, _> = reachable
        .iter()
        .filter_map(|id| {
            initializer(&plan, id, &mut BTreeSet::new()).map(|value| (id.clone(), value))
        })
        .collect();
    for node in plan.nodes.values_mut() {
        if let Shape::Object { fields, extras } = &mut node.shape {
            for field in fields.iter_mut().filter(|f| f.required) {
                field.initializer = initializers.get(&field.source).cloned();
            }
            let mut args: Vec<_> = fields.iter().filter(|f| f.initializer.is_none()).collect();
            args.sort_by_key(|f| !f.required);
            node.constructor = Some(Constructor {
                method: "__construct".into(),
                parameters: args.iter().map(|f| f.name.clone()).collect(),
                extra_parameter: (!matches!(extras, Extras::Closed)).then(|| "extra".into()),
            });
        }
    }
    if errors.is_empty() {
        Ok(plan)
    } else {
        Err(errors)
    }
}

fn schema_name(contract: &Contract, id: &SchemaId, operations: &[Operation]) -> String {
    let pointer = id.pointer();
    if let Some(tail) = pointer.strip_prefix("/components/schemas/") {
        let (name, rest) = tail.split_once('/').unwrap_or((tail, ""));
        return format!(
            "{}{}",
            pascal(&name.replace("~1", "/").replace("~0", "~")),
            path_name(rest)
        );
    }
    for op in operations {
        if id.document() == op.source.document()
            && let Some(tail) = pointer.strip_prefix(&format!("{}/", op.source.pointer()))
        {
            let stem = pascal(&op.operation_id);
            if let Some(tail) = tail.strip_prefix("requestBody/content/application~1json/schema") {
                return format!("{stem}Body{}", path_name(tail));
            }
            if let Some(tail) = tail.strip_prefix("responses/")
                && let Some((status, rest)) = tail.split_once("/content/application~1json/schema")
            {
                return format!("{stem}Response{status}{}", path_name(rest));
            }
            if let Some(p) = op.parameters.iter().find(|p| p.schema == *id) {
                return format!("{stem}{}", pascal(&p.wire_name));
            }
            return format!("{stem}{}", path_name(tail));
        }
    }
    let _ = contract;
    let name = path_name(pointer);
    if name.is_empty() {
        "ValueModel".into()
    } else {
        name
    }
}

fn path_name(path: &str) -> String {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let mut name = String::new();
    while let Some(part) = parts.next() {
        match part {
            "properties" | "$defs" | "definitions" => {
                // A property literally named `items` or `properties` is a name,
                // not another structural segment to strip from its public symbol.
                if let Some(part) = parts.next() {
                    name.push_str(&pascal(&part.replace("~1", "/").replace("~0", "~")));
                }
            }
            "schema" | "content" => {}
            "items" => name.push_str("Item"),
            "oneOf" | "anyOf" | "allOf" => {
                name.push_str("Variant");
                if let Some(index) = parts.next() {
                    name.push_str(
                        &index
                            .parse::<usize>()
                            .map(|i| (i + 1).to_string())
                            .unwrap_or_else(|_| pascal(index)),
                    );
                }
            }
            part => name.push_str(&pascal(&part.replace("~1", "/").replace("~0", "~"))),
        }
    }
    name
}

pub(crate) fn reserved_symbols() -> BTreeSet<String> {
    [
        "Client",
        "ClientInterface",
        "Credentials",
        "ClientOptions",
        "RequestOptions",
        "Transport",
        "CurlTransport",
        "HttpRequest",
        "HttpResponse",
        "ResponseCapture",
        "ApiError",
        "SdkError",
        "JsonNumber",
        "DecimalMath",
        "JsonValue",
        "JsonKind",
        "JsonLimits",
        "JsonParser",
        "JsonWriter",
        "JsonError",
        "Absent",
        "Codecs",
        "CodecContext",
        "Validator",
        "ValidationSession",
        "ValidationError",
        "ValidationProgram",
        "ValidationInstruction",
        "ValidationNode",
        "ValidationLiteral",
        "ValidationAnnotations",
        "ValidationEvaluation",
        "ScopedValidation",
        "PatternProperty",
        "ValidationResource",
        "ValidationResources",
        "ResourceValidation",
        "PatternProgram",
        "PatternState",
        "RuntimeConfig",
        "CancellationToken",
        "CallContext",
        "CallControl",
        "Wire",
        "Model",
        "Protocol",
        "Bytes",
        "NoBody",
        "BasicCredential",
        "ApiKeyCredential",
        "AuthorizationCredential",
        "CredentialRequest",
        "WireValue",
        "PayloadValue",
        "PartValue",
        "Link",
        "BodyReader",
        "StreamTransport",
        "StreamResponse",
        "CurlStreamState",
        "CurlBody",
        "ItemStream",
        "ProtocolData",
        "Parts",
    ]
    .iter()
    .map(|s| s.to_ascii_lowercase())
    .collect()
}

pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !reserved(&value.to_ascii_lowercase())
}

fn reserved(value: &str) -> bool {
    [
        "abstract",
        "and",
        "array",
        "as",
        "bool",
        "break",
        "callable",
        "case",
        "catch",
        "class",
        "clone",
        "const",
        "continue",
        "declare",
        "default",
        "die",
        "do",
        "echo",
        "else",
        "elseif",
        "empty",
        "enddeclare",
        "endfor",
        "endforeach",
        "endif",
        "endswitch",
        "endwhile",
        "enum",
        "eval",
        "exit",
        "extends",
        "false",
        "final",
        "finally",
        "float",
        "fn",
        "for",
        "foreach",
        "function",
        "global",
        "goto",
        "if",
        "implements",
        "include",
        "include_once",
        "instanceof",
        "insteadof",
        "int",
        "interface",
        "isset",
        "iterable",
        "list",
        "match",
        "mixed",
        "namespace",
        "never",
        "new",
        "null",
        "object",
        "or",
        "parent",
        "print",
        "private",
        "protected",
        "public",
        "readonly",
        "require",
        "require_once",
        "resource",
        "return",
        "self",
        "static",
        "string",
        "switch",
        "throw",
        "trait",
        "true",
        "try",
        "unset",
        "use",
        "var",
        "void",
        "while",
        "xor",
        "yield",
        "this",
        "__halt_compiler",
        "__class__",
        "__dir__",
        "__file__",
        "__function__",
        "__line__",
        "__method__",
        "__namespace__",
        "__trait__",
    ]
    .contains(&value)
}

pub(crate) fn pascal(value: &str) -> String {
    let mut name = String::new();
    let mut upper = true;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else {
            upper = true;
        }
    }
    if name.is_empty() {
        name = "Value".into();
    }
    if name.as_bytes()[0].is_ascii_digit() {
        name.insert_str(0, "Value");
    }
    if reserved(&name.to_ascii_lowercase()) {
        name.push_str("Value");
    }
    // Keep paths/classnames portable without discarding source identity.
    if name.len() > 100 {
        use sha2::{Digest, Sha256};
        name = format!("{}{:x}", &name[..64], Sha256::digest(value.as_bytes()))[..100].into();
    }
    name
}

pub(crate) fn member(value: &str) -> String {
    let mut name = pascal(value);
    name[..1].make_ascii_lowercase();
    name
}

pub(crate) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.to_ascii_lowercase()) {
        name = format!("{base}{suffix}");
        suffix += 1;
    }
    name
}
