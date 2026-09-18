//! Native value layout, independent of validation instruction emission.
use std::collections::{BTreeMap, BTreeSet};

use crate::schema_view;
use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedOutcome, OwnedProgram, OwnedSchema, ProgramInstruction};

use super::{HttpDiagnostic, allocate, diagnostic, exported, member};

#[derive(Debug, Clone)]
pub(crate) enum Type {
    Primitive(&'static str),
    Named(SchemaId),
    Array(Box<Type>),
    Nullable(Box<Type>),
    Indirect(Box<Type>),
}

impl Type {
    pub fn render(&self, plan: &ModelPlan) -> String {
        match self {
            Self::Primitive(name) => (*name).into(),
            Self::Named(id) => plan.names[id].clone(),
            Self::Array(inner) => format!("[{}]", inner.render(plan)),
            Self::Nullable(inner) => format!("Nullable<{}>", inner.render(plan)),
            Self::Indirect(inner) => format!("Indirect<{}>", inner.render(plan)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModelType {
    pub core: Type,
    /// Whether an explicit Nullable/Presence wrapper is used. JsonValue and
    /// JsonNull carry null in their own domains without this wrapper.
    pub nullable: bool,
}
impl ModelType {
    pub fn full(&self) -> Type {
        if self.nullable {
            Type::Nullable(Box::new(self.core.clone()))
        } else {
            self.core.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Field {
    pub source: SchemaId,
    pub name: String,
    pub wire: String,
    pub required: bool,
    pub model_type: ModelType,
    pub description: String,
}
impl Field {
    pub fn ty(&self, plan: &ModelPlan) -> String {
        if self.required {
            self.model_type.full().render(plan)
        } else {
            format!(
                "{}<{}>",
                if self.model_type.nullable {
                    "Presence"
                } else {
                    "OptionalField"
                },
                self.model_type.core.render(plan)
            )
        }
    }
    /// Native initializer default, distinct from source field requiredness.
    /// Defaults originate only in missing state or a single source string tag.
    pub fn initializer(&self, plan: &ModelPlan) -> Option<Initializer> {
        if !self.required {
            return Some(Initializer::Missing);
        }
        if !self.model_type.nullable
            && let Type::Named(id) = &self.model_type.core
            && let Some(Declaration::Literals(values)) = plan.declarations.get(id)
            && values.len() == 1
        {
            return Some(Initializer::StringLiteral {
                source: id.clone(),
                case_name: values[0].0.clone(),
                wire_value: values[0].1.clone(),
            });
        }
        None
    }
    pub fn default(&self, plan: &ModelPlan) -> Option<String> {
        self.initializer(plan).map(|init| match init {
            Initializer::Missing => ".missing".into(),
            Initializer::StringLiteral { case_name, .. } => format!(".{case_name}"),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Initializer {
    Missing,
    StringLiteral {
        source: SchemaId,
        case_name: String,
        wire_value: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Variant {
    pub name: String,
    pub source: SchemaId,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub(crate) enum Declaration {
    Object {
        fields: Vec<Field>,
        extras: Option<Type>,
    },
    /// Ordered pairs of (allocated native case, exact wire string literal).
    Literals(Vec<(String, String)>),
    Union(Vec<Variant>),
    /// Complete, exact wire value for a checked layout that does not have one
    /// unconditional native field/tuple declaration. Both codec directions
    /// validate the original source; no assertion or instance member is lost.
    Checked {
        value: Type,
    },
}
impl Declaration {
    /// Actual named constructor parameter order. Additional properties, when
    /// present on an object, follow these fields with an empty-map default.
    pub fn constructor_fields<'a>(&'a self, plan: &ModelPlan) -> Vec<&'a Field> {
        let Self::Object { fields, .. } = self else {
            return Vec::new();
        };
        let mut fields: Vec<_> = fields.iter().collect();
        fields.sort_by_key(|f| f.initializer(plan).is_some());
        fields
    }
}

#[derive(Debug)]
pub(crate) struct ModelPlan {
    pub names: BTreeMap<SchemaId, String>,
    pub types: BTreeMap<SchemaId, ModelType>,
    pub declarations: BTreeMap<SchemaId, Declaration>,
    pub codecs: BTreeMap<SchemaId, String>,
}
impl ModelPlan {
    pub fn declarations(&self) -> &BTreeMap<SchemaId, Declaration> {
        &self.declarations
    }
    pub fn names(&self) -> &BTreeMap<SchemaId, String> {
        &self.names
    }
    pub fn types(&self) -> &BTreeMap<SchemaId, ModelType> {
        &self.types
    }
    pub fn codec_symbols(&self) -> &BTreeMap<SchemaId, String> {
        &self.codecs
    }
    pub fn symbols(&self) -> BTreeMap<SchemaId, String> {
        self.types
            .iter()
            .map(|(id, ty)| (id.clone(), ty.full().render(self)))
            .collect()
    }
    pub fn reserved_names(&self) -> BTreeSet<String> {
        self.names
            .values()
            .cloned()
            .chain(RESERVED.iter().map(|s| (*s).into()))
            .collect()
    }
    pub fn ty(&self, id: &SchemaId) -> String {
        self.types[id].full().render(self)
    }
}

const RESERVED: &[&str] = &[
    "Client",
    "Credentials",
    "ClientOptions",
    "RequestOptions",
    "HTTPRequest",
    "HTTPResponse",
    "HTTPHeader",
    "HTTPTransport",
    "URLSessionTransport",
    "APIResponse",
    "SDKError",
    "TransportError",
    "JsonValue",
    "ExactValue",
    "JsonNumber",
    "JsonInteger",
    "JsonObject",
    "JsonNull",
    "JsonLimits",
    "JsonError",
    "JsonKey",
    "SourceLocation",
    "ValidationError",
    "ValidationSession",
    "ValidationProgram",
    "OptionalField",
    "Presence",
    "Nullable",
    "Indirect",
    "SourceCodable",
    "ModelCodec",
    "Codecs",
    "SDKJSONEncoder",
    "SDKJSONDecoder",
    "ModelContext",
    "Conversion",
    "Examples",
    "String",
    "Bool",
    "Int",
    "Int64",
    "UInt64",
    "Double",
    "Float",
    "Data",
    "URL",
    "URLRequest",
    "URLSession",
    "Error",
    "Never",
    "Optional",
    "Result",
    "Array",
    "Dictionary",
    "Set",
    "Task",
    "Duration",
    "Sendable",
    "Codable",
    "Encoder",
    "Decoder",
    "CodingKey",
    "Equatable",
    "Hashable",
    "Foundation",
    "FoundationNetworking",
    "SourceDecoder",
    "SourceEncoder",
    "BigSigned",
    "ExactDecimal",
    "DigitMath",
    "ValidationNode",
    "ValidationCheck",
    "ValidationInstruction",
    "ValidationResourceContext",
    "ValidationResource",
    "ValidationNodeScope",
    "PatternState",
    "PatternProgram",
    "JSONParser",
    "JSONWriter",
    "RejectedValue",
    "RejectedKeyed",
    "URLSessionTransfer",
    "HTTPBuild",
    "URLResponse",
    "HTTPURLResponse",
    "URLComponents",
    "URLSessionConfiguration",
    "URLSessionDataTask",
    "URLSessionTask",
    "URLSessionDataDelegate",
    "NSLock",
    "NSObject",
    "NSError",
    "TimeInterval",
    "CodingUserInfoKey",
    "KeyedEncodingContainer",
    "KeyedDecodingContainer",
    "SingleValueEncodingContainer",
    "SingleValueDecodingContainer",
    "UnkeyedEncodingContainer",
    "UnkeyedDecodingContainer",
    "KeyedEncodingContainerProtocol",
    "CustomStringConvertible",
    "ExpressibleByIntegerLiteral",
    "CheckedContinuation",
    "Unicode",
    "UInt8",
    "UInt16",
    "UInt32",
    "Int8",
    "Int16",
    "Int32",
    "UTF8",
    "CollectionOfOne",
    "Sequence",
    "CancellationError",
];

pub(super) fn plan(
    contract: &Contract,
    roots: &[SchemaId],
    validator: &OwnedSchema,
    program: &OwnedProgram,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let reachable = schema_view::closure(contract, roots);
    let hints = crate::model_naming::Hints::new(contract);
    let mut used: BTreeSet<String> = RESERVED.iter().map(|s| (*s).into()).collect();
    let names = reachable
        .iter()
        .map(|id| {
            let base = hints
                .get(id)
                .map_or_else(|| source_name(id), |name| exported(&name));
            (id.clone(), allocate(&base, &mut used))
        })
        .collect();
    let mut planner = Planner {
        contract,
        validator,
        applicators: program.version != OwnedProgram::V1_VERSION,
        contextual: context_dependent_sources(contract, program),
        plan: ModelPlan {
            names,
            types: BTreeMap::new(),
            declarations: BTreeMap::new(),
            codecs: BTreeMap::new(),
        },
        active: BTreeSet::new(),
        errors: Vec::new(),
    };
    for root in roots {
        planner.lower(root);
    }
    if !planner.errors.is_empty() {
        return Err(planner.errors);
    }
    // Every conversion has a source-bound codec, including inline scalar fields.
    let mut used = BTreeSet::new();
    for id in planner.plan.types.keys() {
        planner.plan.codecs.insert(
            id.clone(),
            allocate(&member(&planner.plan.names[id]), &mut used),
        );
    }
    box_layout_cycles(&mut planner.plan);
    Ok(planner.plan)
}

struct Planner<'a> {
    contract: &'a Contract,
    validator: &'a OwnedSchema,
    applicators: bool,
    contextual: BTreeSet<SchemaId>,
    plan: ModelPlan,
    active: BTreeSet<SchemaId>,
    errors: Vec<HttpDiagnostic>,
}

impl Planner<'_> {
    fn checked(&mut self, id: &SchemaId, value: Type, nullable: bool) -> ModelType {
        self.plan
            .declarations
            .insert(id.clone(), Declaration::Checked { value });
        ModelType {
            core: Type::Named(id.clone()),
            nullable,
        }
    }
    fn fail(&mut self, id: &SchemaId, message: impl Into<String>) -> ModelType {
        self.errors.push(diagnostic(
            self.contract,
            id.clone(),
            "swift-model-unsupported",
            message,
        ));
        // This placeholder is unreachable at emission: all errors block the plan.
        ModelType {
            core: Type::Primitive("Never"),
            nullable: false,
        }
    }
    fn lower(&mut self, id: &SchemaId) -> ModelType {
        if let Some(ty) = self.plan.types.get(id) {
            return ty.clone();
        }
        if !self.active.insert(id.clone()) {
            return self.fail(
                id,
                "an alias-only recursive schema has no finite Swift value representation",
            );
        }
        let result = self.lower_inner(id);
        self.plan.types.insert(id.clone(), result.clone());
        self.active.remove(id);
        result
    }
    fn lower_inner(&mut self, id: &SchemaId) -> ModelType {
        let Some(schema) = self.contract.schema(id) else {
            return self.fail(id, "schema is not indexed");
        };
        let raw_value = schema_view::raw(schema).into_owned();
        if self.contextual.contains(id) {
            // A dynamic binding can alter shape and nullability. The complete
            // owning value is validated once in its real resource context;
            // conversion must not trial a union or narrow a fallback in isolation.
            // Pure static aliases may reuse the same full-value native carrier,
            // while their own public source codec still starts at the alias node.
            if let Some(raw) = raw_value.as_object()
                && raw.contains_key("$ref")
                && raw.keys().all(|key| key == "$ref" || annotation(key))
                && let Some(target) = schema
                    .references()
                    .iter()
                    .find(|r| r.keyword == "$ref")
                    .and_then(|r| r.target.clone())
                && !self.active.contains(&target)
            {
                return self.lower(&target);
            }
            return self.checked(id, Type::Primitive("JsonValue"), false);
        }
        if raw_value == Value::Bool(false) {
            return self.fail(id, "boolean-false model values are uninhabited; this Swift model profile does not emit a constructible substitute");
        }
        let nullable = match self.validator.validate(id, &Value::Null) {
            OwnedOutcome::Valid => true,
            OwnedOutcome::Invalid(_) => false,
            OwnedOutcome::EvaluationFailure(f) => return self.fail(&f.source, f.message),
        };
        let Some(raw) = raw_value.as_object() else {
            return ModelType {
                core: Type::Primitive("JsonValue"),
                nullable: false,
            };
        };
        if raw.contains_key("$ref") {
            if raw.keys().any(|key| key != "$ref" && !annotation(key)) {
                if self.applicators {
                    return self.checked(id, Type::Primitive("JsonValue"), nullable);
                }
                return self.fail(id, "reference assertion siblings require an intersection layout; no fields are dropped");
            }
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone());
            return match target {
                Some(target) => self.lower(&target),
                None => self.fail(&id.child("$ref"), "reference has no canonical target"),
            };
        }
        for key in [
            "prefixItems",
            "patternProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "dependentSchemas",
            "propertyNames",
            "contains",
            "$dynamicRef",
        ] {
            if raw.contains_key(key) {
                if self.applicators && key != "$dynamicRef" {
                    continue;
                }
                return self.fail(
                    &id.child(key),
                    format!("{key} has no faithful Swift value layout in this profile"),
                );
            }
        }
        if raw.contains_key("allOf") {
            if let Some((target, required)) = transparent_all_of(self.contract, id) {
                if required.is_empty() {
                    return self.lower(&target);
                }
                let Some(base) = self
                    .contract
                    .schema(&target)
                    .and_then(|s| s.raw().as_object())
                else {
                    return self.fail(id, "intersection carrier is not an object");
                };
                let mut base = base.clone();
                let mut combined: BTreeSet<String> = base
                    .get("required")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                combined.extend(required);
                base.insert(
                    "required".into(),
                    Value::Array(combined.into_iter().map(Value::String).collect()),
                );
                return self.object(id, &target, &base, nullable);
            } else if !self.applicators {
                return self.fail(&id.child("allOf"), "allOf supports only a reference carrier with redundant type/empty-properties overlays or requiredness strengthening of existing object fields");
            }
        }
        if raw.contains_key("oneOf") || raw.contains_key("anyOf") {
            if raw.contains_key("oneOf") && raw.contains_key("anyOf")
                || [
                    "type",
                    "properties",
                    "additionalProperties",
                    "items",
                    "enum",
                    "const",
                ]
                .iter()
                .any(|key| raw.contains_key(*key))
            {
                if self.applicators {
                    return self.checked(id, Type::Primitive("JsonValue"), nullable);
                }
                return self.fail(
                    id,
                    "union assertion siblings require an intersection layout",
                );
            }
            let key = if raw.contains_key("oneOf") {
                "oneOf"
            } else {
                "anyOf"
            };
            let members = raw[key].as_array().expect("validated union");
            let ty = ModelType {
                core: Type::Named(id.clone()),
                nullable,
            };
            self.plan.types.insert(id.clone(), ty.clone());
            let mut variants = Vec::new();
            let mut used = BTreeSet::new();
            for index in 0..members.len() {
                let source = id.child(key).child(&index.to_string());
                let value =
                    schema_view::raw(self.contract.schema(&source).expect("indexed branch"));
                if *value == Value::Bool(false) || only_null(&value) {
                    continue;
                }
                let candidate = self.lower(&source);
                let label = self
                    .contract
                    .schema(&source)
                    .and_then(|s| s.references().first())
                    .and_then(|r| r.target.as_ref())
                    .map(source_name)
                    .unwrap_or_else(|| format!("Variant{}", index + 1));
                variants.push(Variant {
                    name: allocate(&member(&label), &mut used),
                    source,
                    ty: candidate.full(),
                });
            }
            if variants.is_empty() {
                return self.fail(id, "union has no constructible non-null alternatives");
            }
            // Do not expand or flatten multi-variant unions. Their exact branch
            // counts are checked by the portable program on decode AND encode.
            self.plan
                .declarations
                .insert(id.clone(), Declaration::Union(variants));
            return ty;
        }
        let types: Vec<&str> = match raw.get("type") {
            Some(Value::String(s)) => vec![s],
            Some(Value::Array(xs)) => xs.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let mut non_null: Vec<&str> = types.iter().copied().filter(|s| *s != "null").collect();
        if non_null.contains(&"number") {
            non_null.retain(|s| *s != "integer");
        }
        let literals: Vec<&Value> = if let Some(value) = raw.get("const") {
            vec![value]
        } else {
            raw.get("enum")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .collect()
        };
        if non_null.is_empty() && types.is_empty() && !literals.is_empty() {
            for value in literals.iter().filter(|v| !v.is_null()) {
                let kind =
                    match value {
                        Value::String(_) => "string",
                        Value::Bool(_) => "boolean",
                        Value::Number(_) => "number",
                        _ => return self.fail(
                            id,
                            "untyped object/array literal models require a dedicated exact layout",
                        ),
                    };
                if !non_null.contains(&kind) {
                    non_null.push(kind);
                }
            }
        }
        if non_null.len() > 1 {
            return self.fail(&id.child("type"), "multiple non-null scalar types require a declared union layout; no JsonValue fallback is emitted");
        }
        let kind = non_null.first().copied();
        if kind == Some("string") && !literals.is_empty() {
            let mut values = Vec::new();
            let mut used =
                BTreeSet::from(["rawValue".into(), "codec".into(), "schemaSource".into()]);
            for value in literals.iter().filter_map(|v| v.as_str()) {
                if !values
                    .iter()
                    .any(|(_, wire): &(String, String)| wire == value)
                {
                    values.push((allocate(&member(value), &mut used), value.to_owned()));
                }
            }
            if values.is_empty() {
                return self.fail(id, "string literal domain is uninhabited");
            }
            self.plan
                .declarations
                .insert(id.clone(), Declaration::Literals(values));
            return ModelType {
                core: Type::Named(id.clone()),
                nullable,
            };
        }
        let core = match kind {
            Some("object") => return self.object(id, id, raw, nullable),
            Some("array") => {
                if self.applicators && (raw.contains_key("prefixItems") || raw.get("items") == Some(&Value::Bool(false))) {
                    return self.checked(id, Type::Array(Box::new(Type::Primitive("JsonValue"))), nullable);
                }
                let item = if raw.contains_key("items") { self.lower(&id.child("items")).full() } else { Type::Primitive("JsonValue") };
                Type::Array(Box::new(item))
            }
            Some("string") => Type::Primitive("String"),
            Some("boolean") => Type::Primitive("Bool"),
            Some("integer") => Type::Primitive("JsonInteger"),
            Some("number") => Type::Primitive("JsonNumber"),
            None if only_null(&raw_value) => return ModelType { core: Type::Primitive("JsonNull"), nullable: false },
            None if raw.keys().all(|key| annotation(key)) => return ModelType { core: Type::Primitive("JsonValue"), nullable: false },
            None if self.applicators => return self.checked(id, Type::Primitive("JsonValue"), nullable),
            None => return self.fail(id, "untyped assertions do not imply an object or scalar type; conditional native layouts are unsupported"),
            _ => return self.fail(&id.child("type"), "unknown native type"),
        };
        // Applicable constraints may filter the chosen native domain; they are
        // never used to invent fields, coercions, or API-specific semantics.
        ModelType { core, nullable }
    }
    fn object(
        &mut self,
        id: &SchemaId,
        field_owner: &SchemaId,
        raw: &Map<String, Value>,
        nullable: bool,
    ) -> ModelType {
        let ty = ModelType {
            core: Type::Named(id.clone()),
            nullable,
        };
        self.plan.types.insert(id.clone(), ty.clone()); // guard recursive fields
        let required: BTreeSet<&str> = raw
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let properties = raw.get("properties").and_then(Value::as_object);
        for wire in &required {
            if !properties.is_some_and(|p| p.contains_key(*wire)) {
                if self.applicators {
                    continue;
                }
                self.fail(&id.child("required"), "required undeclared properties have no Swift constructor field in this profile");
            }
        }
        let mut used = BTreeSet::from([
            "additionalProperties".into(),
            "codec".into(),
            "schemaSource".into(),
            "encode".into(),
            "_encode".into(),
            "_decode".into(),
        ]);
        let mut fields = Vec::new();
        for (wire, _) in properties.into_iter().flatten() {
            let source = field_owner.child("properties").child(wire);
            let description = self
                .contract
                .schema(&source)
                .map(schema_view::description)
                .unwrap_or("")
                .to_owned();
            let field_type = self.lower(&source);
            fields.push(Field {
                source,
                name: allocate(&member(wire), &mut used),
                wire: wire.clone(),
                required: required.contains(wire.as_str()),
                model_type: field_type,
                description,
            });
        }
        let dynamic_patterns = self.applicators
            && raw
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|p| !p.is_empty());
        let extras = if dynamic_patterns {
            Some(Type::Primitive("JsonValue"))
        } else {
            match raw.get("additionalProperties") {
                Some(Value::Bool(false)) => None,
                Some(Value::Object(_)) => Some(
                    self.lower(&field_owner.child("additionalProperties"))
                        .full(),
                ),
                _ => Some(Type::Primitive("JsonValue")),
            }
        };
        self.plan
            .declarations
            .insert(id.clone(), Declaration::Object { fields, extras });
        ty
    }
}

fn only_null(value: &Value) -> bool {
    value.get("type").is_some_and(|v| {
        v == "null"
            || v.as_array()
                .is_some_and(|xs| xs.len() == 1 && xs[0] == "null")
    }) || value.get("const").is_some_and(Value::is_null)
        || value
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|xs| !xs.is_empty() && xs.iter().all(Value::is_null))
}

fn annotation(key: &str) -> bool {
    key.starts_with("x-")
        || matches!(
            key,
            "$id"
                | "$schema"
                | "$anchor"
                | "$dynamicAnchor"
                | "$defs"
                | "definitions"
                | "$comment"
                | "title"
                | "description"
                | "summary"
                | "default"
                | "example"
                | "examples"
                | "readOnly"
                | "writeOnly"
                | "deprecated"
                | "discriminator"
                | "xml"
                | "externalDocs"
                | "format"
                | "nullable"
        )
}

/// Compute value layouts requiring the active dynamic context from the admitted
/// instruction graph, including every parent which converts such a child. This
/// is a layout decision, never a compile-time selection of an anchor candidate.
fn context_dependent_sources(contract: &Contract, program: &OwnedProgram) -> BTreeSet<SchemaId> {
    if program.resource_context.is_none() {
        return BTreeSet::new();
    }
    let mut parents = vec![Vec::new(); program.nodes.len()];
    let mut contextual = BTreeSet::new();
    let mut pending = Vec::new();
    for (node, schema) in program.nodes.iter().enumerate() {
        for check in &schema.checks {
            use ProgramInstruction as I;
            let targets = match &check.instruction {
                I::DynamicRef { .. } => {
                    if contextual.insert(node) {
                        pending.push(node);
                    }
                    Vec::new()
                }
                I::Ref { target }
                | I::AdditionalProperties { target, .. }
                | I::AdditionalPropertiesWithPatterns { target, .. }
                | I::Items { target, .. }
                | I::Not { target }
                | I::Contains { target, .. }
                | I::PropertyNames { target }
                | I::UnevaluatedProperties { target }
                | I::UnevaluatedItems { target } => vec![*target],
                I::Properties { properties }
                | I::DependentSchemas {
                    dependencies: properties,
                } => properties.iter().map(|p| p.target).collect(),
                I::PrefixItems { targets }
                | I::AllOf { targets }
                | I::AnyOf { targets }
                | I::OneOf { targets } => targets.clone(),
                I::If {
                    condition,
                    then_target,
                    else_target,
                } => std::iter::once(*condition)
                    .chain(*then_target)
                    .chain(*else_target)
                    .collect(),
                I::PatternProperties { patterns } => patterns.iter().map(|p| p.2).collect(),
                I::Always { .. }
                | I::Type { .. }
                | I::Required { .. }
                | I::Bound { .. }
                | I::MultipleOf { .. }
                | I::Count { .. }
                | I::Enum { .. }
                | I::Const { .. }
                | I::UniqueItems
                | I::Pattern { .. }
                | I::DependentRequired { .. } => Vec::new(),
            };
            for target in targets {
                parents[target].push(node);
            }
        }
    }
    while let Some(node) = pending.pop() {
        for parent in &parents[node] {
            if contextual.insert(*parent) {
                pending.push(*parent);
            }
        }
    }
    let sources: BTreeSet<_> = contextual
        .into_iter()
        .map(|index| {
            let source = &program.nodes[index].source;
            (source.document.as_str(), source.pointer.as_str())
        })
        .collect();
    contract
        .schemas()
        .filter(|schema| {
            sources.contains(&(schema.id().document().as_str(), schema.id().pointer()))
        })
        .map(|schema| schema.id().clone())
        .collect()
}

fn transparent_all_of(contract: &Contract, id: &SchemaId) -> Option<(SchemaId, BTreeSet<String>)> {
    let value = schema_view::raw(contract.schema(id)?);
    let raw = value.as_object()?;
    if raw.keys().any(|key| key != "allOf" && !annotation(key)) {
        return None;
    }
    let members = raw.get("allOf")?.as_array()?;
    members.first()?;
    let first_value = schema_view::raw(contract.schema(&id.child("allOf").child("0"))?);
    let first = first_value.as_object()?;
    if !first.contains_key("$ref") || first.keys().any(|key| key != "$ref" && !annotation(key)) {
        return None;
    }
    let mut target = id.child("allOf").child("0");
    let mut seen = BTreeSet::new();
    let base = loop {
        if !seen.insert(target.clone()) {
            return None;
        }
        let schema = contract.schema(&target)?;
        let raw = schema.raw().as_object()?;
        if raw.contains_key("$ref") {
            if !schema_view::reference_only(schema)
                && raw.keys().any(|key| key != "$ref" && !annotation(key))
            {
                return None;
            }
            target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")?
                .target
                .clone()?;
        } else {
            break raw;
        }
    };
    let mut required = BTreeSet::new();
    for member in &members[1..] {
        for (key, value) in member.as_object()? {
            if annotation(key) {
                continue;
            }
            match key.as_str() {
                "type" if base.get("type") == Some(value) => {}
                "properties" if value.as_object().is_some_and(Map::is_empty) => {}
                "required" if base.get("type").is_some_and(|v| v == "object") => {
                    for name in value.as_array()? {
                        let name = name.as_str()?;
                        if !base.get("properties")?.as_object()?.contains_key(name) {
                            return None;
                        }
                        required.insert(name.into());
                    }
                }
                _ => return None,
            }
        }
    }
    Some((target, required))
}

fn source_name(id: &SchemaId) -> String {
    let tokens: Vec<String> = id
        .pointer()
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    let start = if tokens.first().is_some_and(|t| t == "components")
        && tokens.get(1).is_some_and(|t| t == "schemas")
    {
        2
    } else {
        0
    };
    let parts: Vec<&str> = tokens[start..]
        .iter()
        .filter_map(|s| match s.as_str() {
            "properties" | "$defs" | "definitions" => None,
            "items" => Some("Item"),
            "oneOf" | "anyOf" | "allOf" => Some("Variant"),
            s => Some(s),
        })
        .collect();
    let base = exported(&parts.join("_"));
    if base.len() <= 120 {
        return base;
    }
    let hash = base.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{}H{hash:016x}", &base[..96])
}

// Nullable/optional/union enums are indirect; arrays and extra-property maps
// already have finite value layout. Only required, non-null object feedback
// edges need a visible value-semantic Indirect wrapper.
fn box_layout_cycles(plan: &mut ModelPlan) {
    let graph: BTreeMap<SchemaId, BTreeSet<SchemaId>> = plan
        .declarations
        .iter()
        .filter_map(|(id, declaration)| {
            let Declaration::Object { fields, .. } = declaration else {
                return None;
            };
            Some((
                id.clone(),
                fields
                    .iter()
                    .filter(|f| f.required && !f.model_type.nullable)
                    .filter_map(|f| match &f.model_type.core {
                        Type::Named(target)
                            if matches!(
                                plan.declarations.get(target),
                                Some(Declaration::Object { .. })
                            ) =>
                        {
                            Some(target.clone())
                        }
                        _ => None,
                    })
                    .collect(),
            ))
        })
        .collect();
    fn visit(
        id: &SchemaId,
        graph: &BTreeMap<SchemaId, BTreeSet<SchemaId>>,
        active: &mut BTreeSet<SchemaId>,
        done: &mut BTreeSet<SchemaId>,
        feedback: &mut BTreeSet<(SchemaId, SchemaId)>,
    ) {
        if !done.insert(id.clone()) {
            return;
        }
        active.insert(id.clone());
        for target in graph.get(id).into_iter().flatten() {
            if active.contains(target) {
                feedback.insert((id.clone(), target.clone()));
            } else {
                visit(target, graph, active, done, feedback);
            }
        }
        active.remove(id);
    }
    let mut feedback = BTreeSet::new();
    let mut done = BTreeSet::new();
    for id in graph.keys() {
        visit(id, &graph, &mut BTreeSet::new(), &mut done, &mut feedback);
    }
    for (id, declaration) in &mut plan.declarations {
        if let Declaration::Object { fields, .. } = declaration {
            for field in fields {
                if field.required
                    && !field.model_type.nullable
                    && let Type::Named(target) = &field.model_type.core
                    && feedback.contains(&(id.clone(), target.clone()))
                {
                    field.model_type.core = Type::Indirect(Box::new(field.model_type.core.clone()));
                }
            }
        }
    }
}
