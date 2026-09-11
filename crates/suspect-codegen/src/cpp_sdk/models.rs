//! Finite native representation planning. No validation semantics live here.

use super::{HttpDiagnostic, diagnostic};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::OwnedProgram;

/// Original schema identity, native type/codec names and representation.
#[derive(Debug, Clone)]
pub struct ModelSymbol {
    pub source: SchemaId,
    pub name: String,
    pub cpp_type: String,
    pub codec_name: String,
    pub(crate) index: usize,
    pub shape: Shape,
    pub nullable: bool,
    pub definition: String,
    /// Actual constructor, including argument order and source-backed tag defaults.
    pub constructor: Option<Constructor>,
}

/// The fixed native codec member names used by the C++ emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecMethods {
    pub decode: &'static str,
    pub encode: &'static str,
    pub to_json: &'static str,
}

/// A codec's actual allocated owner and full public value domain.
#[derive(Debug, Clone, Copy)]
pub struct CodecDescriptor<'a> {
    pub owner: &'a str,
    pub value_alias: &'static str,
    pub cpp_type: &'a str,
    pub error_type: &'static str,
    pub methods: CodecMethods,
}

impl ModelSymbol {
    /// Scalars, arrays and references are public aliases. Nullable named shapes
    /// additionally own a separate non-null struct or enum declaration.
    #[must_use]
    pub fn has_definition(&self) -> bool {
        matches!(
            self.shape,
            Shape::Enum(_) | Shape::Object { .. } | Shape::Union { .. }
        )
    }

    #[must_use]
    pub fn codec(&self) -> CodecDescriptor<'_> {
        CodecDescriptor {
            owner: &self.codec_name,
            value_alias: "Value",
            cpp_type: &self.cpp_type,
            error_type: "CodecError",
            methods: CodecMethods {
                decode: "decode",
                encode: "encode",
                to_json: "to_json",
            },
        }
    }
}

/// One emitted required-member constructor. Names are native C++ symbols.
#[derive(Debug, Clone)]
pub struct Constructor {
    pub name: String,
    pub parameters: Vec<ConstructorParameter>,
}
#[derive(Debug, Clone)]
pub struct ConstructorParameter {
    pub name: String,
    pub member_name: String,
    pub source: SchemaId,
    pub cpp_type: String,
}
/// A source-proved singleton string tag, never an invented business default.
#[derive(Debug, Clone)]
pub struct TagInitializer {
    pub type_name: String,
    pub case_name: String,
    pub wire_value: String,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub wire: String,
    pub name: String,
    pub schema: SchemaId,
    pub required: bool,
    pub cpp_type: String,
    pub initializer: Option<TagInitializer>,
}

#[derive(Debug, Clone)]
pub enum Extras {
    Closed,
    Any,
    Typed(SchemaId),
    /// Names may match several pattern schemas or the additional policy.
    /// Values stay exact JSON until the whole-object codec checks every rule.
    Patterned,
}

#[derive(Debug, Clone)]
pub enum Shape {
    Json,
    /// A checked JSON carrier for constraints with no faithful static layout.
    ValidatedJson,
    Never,
    Null,
    Boolean,
    Number,
    Integer,
    String,
    Enum(Vec<(String, String)>),
    Object {
        fields: Vec<Field>,
        extras: Extras,
    },
    Array(Option<SchemaId>),
    Ref {
        target: SchemaId,
        boxed: bool,
    },
    Union {
        branches: Vec<SchemaId>,
        exactly_one: bool,
    },
}

/// Immutable source-to-native symbol table and definition order.
#[derive(Debug)]
pub struct ModelPlan {
    pub(crate) symbols: BTreeMap<SchemaId, ModelSymbol>,
    pub(crate) order: Vec<SchemaId>,
}
impl ModelPlan {
    pub fn symbols(&self) -> impl Iterator<Item = &ModelSymbol> {
        self.symbols.values()
    }
    #[must_use]
    pub fn symbol(&self, source: &SchemaId) -> Option<&ModelSymbol> {
        self.symbols.get(source)
    }
    pub(crate) fn get(&self, source: &SchemaId) -> &ModelSymbol {
        &self.symbols[source]
    }
}

pub(crate) fn plan(
    contract: &Contract,
    roots: &[SchemaId],
    wire_seeds: &BTreeMap<SchemaId, String>,
    reserved: &BTreeSet<String>,
    program: &OwnedProgram,
    namespace: &str,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let closure = crate::schema_view::closure(contract, roots);
    let mut seeds = BTreeMap::new();
    for id in &closure {
        let parts: Vec<_> = id.pointer().split('/').collect();
        if parts.len() >= 4 && parts[1] == "components" && parts[2] == "schemas" {
            let mut base = id.clone();
            // Find the actual indexed component address without parsing pointers.
            if let Some(component) = contract.schemas().find(|s| {
                s.id().document() == id.document()
                    && s.id().pointer() == format!("/components/schemas/{}", parts[3])
            }) {
                base = component.id().clone();
            }
            seeds.insert(base, pascal(&unescape(parts[3])));
        }
    }
    seeds.extend(
        wire_seeds
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    let mut names = reserved_names();
    names.extend(reserved.iter().cloned());
    let mut symbols = BTreeMap::new();
    let mut errors = Vec::new();
    for id in &closure {
        let name = allocate(&symbol_base(id, &seeds), &mut names);
        let definition = allocate(&format!("{name}Value"), &mut names);
        let codec_name = allocate(&format!("{name}Codec"), &mut names);
        let Some(schema) = contract.schema(id) else {
            continue;
        };
        let raw = crate::schema_view::raw(schema);
        let scoped = program.version == OwnedProgram::V2_VERSION
            || program.version == OwnedProgram::V3_VERSION;
        let shaped = shape(contract, id, &raw, scoped).or_else(|error| {
            if scoped {
                Ok((Shape::ValidatedJson, false))
            } else {
                Err(error)
            }
        });
        match shaped {
            Ok((shape, nullable)) => {
                // A union arm tested in isolation would lose its caller's
                // resource stack. Keep the whole value checked in v3 instead
                // of guessing a dynamic binding during layout selection.
                let (shape, nullable) = if program.version == OwnedProgram::V3_VERSION
                    && matches!(shape, Shape::Union { .. })
                {
                    (Shape::ValidatedJson, false)
                } else {
                    (shape, nullable)
                };
                let index = program
                    .roots
                    .iter()
                    .find(|r| {
                        r.source.document == id.document().to_string()
                            && r.source.pointer == id.pointer()
                    })
                    .expect("all reachable schemas selected")
                    .target;
                symbols.insert(
                    id.clone(),
                    ModelSymbol {
                        source: id.clone(),
                        name: name.clone(),
                        cpp_type: String::new(),
                        codec_name,
                        index,
                        shape,
                        nullable,
                        definition: if nullable { definition } else { name },
                        constructor: None,
                    },
                );
            }
            Err(message) => errors.push(diagnostic(
                contract,
                id.clone(),
                "cpp-model-unsupported",
                message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Only reference edges on a cycle need owning indirection. Value containers
    // and ordinary acyclic object/variant fields remain native value members.
    for id in &closure {
        let boxed = if let Shape::Ref { target, .. } = &symbols[id].shape {
            reaches(target, id, &symbols, &mut BTreeSet::new())
        } else {
            false
        };
        if let Shape::Ref { boxed: slot, .. } = &mut symbols.get_mut(id).unwrap().shape {
            *slot = boxed;
        }
    }
    for id in &closure {
        match type_of(id, &symbols, &mut BTreeSet::new(), namespace) {
            Ok(ty) => symbols.get_mut(id).unwrap().cpp_type = ty,
            Err(message) => errors.push(diagnostic(
                contract,
                id.clone(),
                "cpp-recursive-alias-unsupported",
                message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Store the actual signatures consumed by both rendering and compatibility.
    for id in &closure {
        let mut updated = symbols[id].clone();
        if let Shape::Object { fields, .. } = &mut updated.shape {
            let mut parameters = Vec::new();
            for field in fields {
                let child = &symbols[&field.schema];
                field.cpp_type = if field.required {
                    child.cpp_type.clone()
                } else {
                    format!("Presence<{}>", child.cpp_type)
                };
                if field.required {
                    field.initializer =
                        singleton_tag(&field.schema, &symbols, &mut BTreeSet::new());
                    if field.initializer.is_none() {
                        parameters.push(ConstructorParameter {
                            name: format!("arg{}", parameters.len()),
                            member_name: field.name.clone(),
                            source: field.schema.clone(),
                            cpp_type: child.cpp_type.clone(),
                        });
                    }
                }
            }
            updated.constructor = Some(Constructor {
                name: updated.definition.clone(),
                parameters,
            });
        }
        symbols.insert(id.clone(), updated);
    }
    let mut order = Vec::new();
    let mut active = BTreeSet::new();
    let mut done = BTreeSet::new();
    for id in &closure {
        if !definition_order(id, &symbols, &mut active, &mut done, &mut order) {
            errors.push(diagnostic(contract, id.clone(), "cpp-recursive-layout-unsupported", "no finite by-value layout; a productive object/union must guard each recursive reference"));
        }
    }
    if errors.is_empty() {
        Ok(ModelPlan { symbols, order })
    } else {
        Err(errors)
    }
}

fn shape(
    contract: &Contract,
    id: &SchemaId,
    raw: &Value,
    scoped: bool,
) -> Result<(Shape, bool), String> {
    if let Some(truth) = raw.as_bool() {
        return Ok((if truth { Shape::Json } else { Shape::Never }, false));
    }
    let object = raw
        .as_object()
        .ok_or("schema is not an object or boolean")?;
    let has = |key| object.contains_key(key);
    if has("$dynamicRef") {
        return Ok((Shape::ValidatedJson, false));
    }
    if has("$ref") {
        if [
            "type",
            "properties",
            "required",
            "items",
            "prefixItems",
            "additionalProperties",
            "allOf",
            "anyOf",
            "oneOf",
        ]
        .iter()
        .any(|key| has(*key))
        {
            return Err("shape-changing $ref siblings need a native intersection view".into());
        }
        let reference = contract
            .schema(id)
            .unwrap()
            .references()
            .iter()
            .find(|r| r.keyword == "$ref")
            .and_then(|r| r.target.clone())
            .ok_or("unresolved reference")?;
        return Ok((
            Shape::Ref {
                target: reference,
                boxed: false,
            },
            false,
        ));
    }
    if has("allOf") && !(scoped && raw["type"] == "object") {
        return Err("native allOf intersections are incomplete; emission is blocked".into());
    }
    if has("prefixItems") {
        return Err("native positional tuple models are incomplete; emission is blocked".into());
    }
    if has("oneOf") || has("anyOf") {
        if has("oneOf") && has("anyOf")
            || ["type", "properties", "items", "additionalProperties"]
                .iter()
                .any(|key| has(*key))
        {
            return Err(
                "union with additional native shape constraints needs an intersection view".into(),
            );
        }
        let key = if has("oneOf") { "oneOf" } else { "anyOf" };
        let branches = raw[key].as_array().ok_or("union must be an array")?;
        if branches.is_empty() {
            return Err("union has no alternatives".into());
        }
        return Ok((
            Shape::Union {
                branches: (0..branches.len())
                    .map(|n| id.child(key).child(&n.to_string()))
                    .collect(),
                exactly_one: key == "oneOf",
            },
            false,
        ));
    }
    let mut types: Vec<&str> = match raw.get("type") {
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).collect(),
        None => Vec::new(),
        _ => return Err("invalid type declaration".into()),
    };
    let literals: Vec<&Value> = if let Some(values) = raw.get("enum").and_then(Value::as_array) {
        values.iter().collect()
    } else {
        raw.get("const").into_iter().collect()
    };
    if types.is_empty() && !literals.is_empty() {
        for literal in &literals {
            let ty = match literal {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            if !types.contains(&ty) {
                types.push(ty);
            }
        }
    }
    let old_nullable = contract
        .schema(id)
        .is_some_and(|s| matches!(s.dialect(), suspect_ir::contract::SchemaDialect::OpenApi30))
        && raw.get("nullable") == Some(&Value::Bool(true))
        && raw.get("type").is_some_and(Value::is_string);
    let nullable = types.contains(&"null") && types.len() > 1 || old_nullable;
    if nullable {
        types.retain(|ty| *ty != "null");
    }
    if types.len() > 1 {
        return Err("multi-type/heterogeneous literal unions need explicit oneOf/anyOf native alternatives in this profile".into());
    }
    let shape = match types.first().copied() {
        Some("null") => Shape::Null,
        Some("boolean") => Shape::Boolean,
        Some("number") => Shape::Number,
        Some("integer") => Shape::Integer,
        Some("string") => {
            if !literals.is_empty() && literals.iter().all(|v| v.is_string() || v.is_null()) {
                let mut used = BTreeSet::new();
                let mut values = Vec::new();
                for value in literals.iter().filter_map(|v| v.as_str()) {
                    if !values.iter().any(|(wire, _)| wire == value) {
                        values.push((value.to_owned(), allocate(&pascal(value), &mut used)));
                    }
                }
                if values.is_empty() {
                    Shape::Never
                } else {
                    Shape::Enum(values)
                }
            } else {
                Shape::String
            }
        }
        Some("array") => Shape::Array(if has("items") {
            Some(id.child("items"))
        } else {
            None
        }),
        Some("object") => {
            let mut used = BTreeSet::from(["extra".into()]);
            let required: BTreeSet<_> = raw
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let fields: Vec<_> = raw
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(wire, _)| Field {
                    wire: wire.clone(),
                    name: allocate(&snake(wire), &mut used),
                    schema: id.child("properties").child(wire),
                    required: required.contains(wire.as_str()),
                    cpp_type: String::new(),
                    initializer: None,
                })
                .collect();
            if !scoped
                && required
                    .iter()
                    .any(|key| !fields.iter().any(|f| &f.wire == key))
            {
                return Err("required names outside properties need named typed members; emission is blocked".into());
            }
            let extras = if scoped
                && raw
                    .get("patternProperties")
                    .and_then(Value::as_object)
                    .is_some_and(|v| !v.is_empty())
            {
                Extras::Patterned
            } else {
                match raw.get("additionalProperties") {
                    Some(Value::Bool(false)) => Extras::Closed,
                    None | Some(Value::Bool(true)) => Extras::Any,
                    Some(Value::Object(_)) => Extras::Typed(id.child("additionalProperties")),
                    _ => return Err("invalid additionalProperties".into()),
                }
            };
            Shape::Object { fields, extras }
        }
        None => {
            if ["properties", "additionalProperties", "items", "required"]
                .iter()
                .any(|key| has(*key))
            {
                return Err("applicators without a declared native type accept other JSON kinds; a faithful conditional native view is incomplete".into());
            }
            // The schema genuinely admits the full JSON domain, perhaps with
            // validation-only constraints such as `not`; it is not a fallback.
            if scoped {
                Shape::ValidatedJson
            } else {
                Shape::Json
            }
        }
        _ => return Err("unrecognized native type".into()),
    };
    Ok((shape, nullable))
}

fn singleton_tag(
    id: &SchemaId,
    symbols: &BTreeMap<SchemaId, ModelSymbol>,
    seen: &mut BTreeSet<SchemaId>,
) -> Option<TagInitializer> {
    if !seen.insert(id.clone()) {
        return None;
    }
    let symbol = &symbols[id];
    if symbol.nullable {
        return None;
    }
    match &symbol.shape {
        Shape::Enum(values) if values.len() == 1 => Some(TagInitializer {
            type_name: symbol.definition.clone(),
            case_name: values[0].1.clone(),
            wire_value: values[0].0.clone(),
        }),
        Shape::Ref {
            target,
            boxed: false,
        } => singleton_tag(target, symbols, seen),
        _ => None,
    }
}

fn dependencies(shape: &Shape) -> Vec<&SchemaId> {
    match shape {
        Shape::Ref { target, .. } => vec![target],
        Shape::Array(item) => item.iter().collect(),
        Shape::Object { fields, extras } => fields
            .iter()
            .map(|f| &f.schema)
            .chain(match extras {
                Extras::Typed(id) => Some(id),
                _ => None,
            })
            .collect(),
        Shape::Union { branches, .. } => branches.iter().collect(),
        _ => Vec::new(),
    }
}
fn reaches(
    from: &SchemaId,
    wanted: &SchemaId,
    symbols: &BTreeMap<SchemaId, ModelSymbol>,
    seen: &mut BTreeSet<SchemaId>,
) -> bool {
    from == wanted
        || seen.insert(from.clone())
            && dependencies(&symbols[from].shape)
                .iter()
                .any(|child| reaches(child, wanted, symbols, seen))
}
fn type_of(
    id: &SchemaId,
    symbols: &BTreeMap<SchemaId, ModelSymbol>,
    seen: &mut BTreeSet<SchemaId>,
    namespace: &str,
) -> Result<String, String> {
    if !seen.insert(id.clone()) {
        return Err(
            "recursive aliases without a native object/variant guard are not representable".into(),
        );
    }
    let symbol = &symbols[id];
    let base = match &symbol.shape {
        Shape::Json | Shape::ValidatedJson => "JsonValue".into(),
        Shape::Never => "Never".into(),
        Shape::Null => "Null".into(),
        Shape::Boolean => "bool".into(),
        Shape::Number => "JsonNumber".into(),
        Shape::Integer => "JsonInteger".into(),
        Shape::String => "std::string".into(),
        Shape::Enum(_) | Shape::Object { .. } | Shape::Union { .. } => {
            format!("::{namespace}::{}", symbol.definition)
        }
        Shape::Array(item) => format!(
            "std::vector<{}>",
            match item {
                Some(id) => type_of(id, symbols, seen, namespace)?,
                None => "JsonValue".into(),
            }
        ),
        Shape::Ref { target, boxed } => {
            let target = type_of(target, symbols, seen, namespace)?;
            if *boxed {
                format!("Box<{target}>")
            } else {
                target
            }
        }
    };
    seen.remove(id);
    Ok(if symbol.nullable {
        format!("Nullable<{base}>")
    } else {
        base
    })
}
fn definition_order(
    id: &SchemaId,
    symbols: &BTreeMap<SchemaId, ModelSymbol>,
    active: &mut BTreeSet<SchemaId>,
    done: &mut BTreeSet<SchemaId>,
    order: &mut Vec<SchemaId>,
) -> bool {
    if done.contains(id) {
        return true;
    }
    if !active.insert(id.clone()) {
        return false;
    }
    if !matches!(symbols[id].shape, Shape::Ref { boxed: true, .. }) {
        for dep in dependencies(&symbols[id].shape) {
            if !definition_order(dep, symbols, active, done, order) {
                return false;
            }
        }
    }
    active.remove(id);
    done.insert(id.clone());
    order.push(id.clone());
    true
}
fn symbol_base(id: &SchemaId, seeds: &BTreeMap<SchemaId, String>) -> String {
    if let Some((base, name)) = seeds
        .iter()
        .filter(|(base, _)| {
            base.document() == id.document()
                && (id.pointer() == base.pointer()
                    || id.pointer().starts_with(&format!("{}/", base.pointer())))
        })
        .max_by_key(|(base, _)| base.pointer().len())
    {
        let mut name = name.clone();
        for token in id.pointer()[base.pointer().len()..]
            .split('/')
            .filter(|p| !p.is_empty() && *p != "properties")
        {
            name.push_str(&match token {
                "items" => "Item".into(),
                "additionalProperties" => "Extra".into(),
                "oneOf" | "anyOf" => "Branch".into(),
                _ => pascal(&unescape(token)),
            });
        }
        name
    } else {
        format!("Schema{}", pascal(id.pointer()))
    }
}
fn unescape(s: &str) -> String {
    s.replace("~1", "/").replace("~0", "~")
}
pub(crate) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let base = compact(base);
    let mut candidate = base.clone();
    let mut suffix = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}{suffix}");
        suffix += 1;
    }
    candidate
}
// Keep public names and native documentation filenames usable without losing
// identity. The readable role prefix/suffix remain; the full source is retained
// separately in every descriptor. Collision allocation still follows hashing.
fn compact(value: &str) -> String {
    if value.len() <= 80 {
        return value.into();
    }
    let hash = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!(
        "{}_h{}_{}",
        value[..48].trim_end_matches('_'),
        &hash[..12],
        value[value.len() - 16..].trim_start_matches('_')
    )
}
pub(crate) fn pascal(name: &str) -> String {
    let mut result = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else if c.is_ascii() {
            upper = true;
        } else {
            result.push_str(&format!("U{:X}", c as u32));
            upper = true;
        }
    }
    if result.is_empty() {
        result.push_str("Value");
    }
    if result.as_bytes()[0].is_ascii_digit() {
        result.insert(0, 'N');
    }
    if KEYWORDS.contains(&result.as_str()) {
        result.push_str("Value");
    }
    compact(&result)
}
pub(crate) fn snake(name: &str) -> String {
    let mut result = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && !result.is_empty() && !result.ends_with('_') {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else if c.is_ascii() {
            if !result.is_empty() && !result.ends_with('_') {
                result.push('_');
            }
        } else {
            if !result.is_empty() && !result.ends_with('_') {
                result.push('_');
            }
            result.push_str(&format!("u{:x}_", c as u32));
        }
    }
    let mut result = result.trim_matches('_').to_owned();
    if result.is_empty() {
        result = "value".into();
    }
    if result.as_bytes()[0].is_ascii_digit() {
        result.insert_str(0, "n_");
    }
    if KEYWORDS.contains(&result.as_str()) {
        result.push('_');
    }
    compact(&result)
}
pub(crate) fn valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !s.contains("__")
        && !KEYWORDS.contains(&s)
}
pub(crate) fn reserved_names() -> BTreeSet<String> {
    [
        "Client",
        "ClientOptions",
        "CallOptions",
        "Credentials",
        "CurlTransport",
        "CurlOptions",
        "Transport",
        "TransportOptions",
        "HttpRequest",
        "HttpResponse",
        "Headers",
        "ResponseMetadata",
        "SdkError",
        "TransportError",
        "CodecError",
        "JsonValue",
        "JsonNumber",
        "JsonInteger",
        "JsonLimits",
        "Result",
        "Box",
        "Never",
        "Null",
        "Unit",
        "Presence",
        "Nullable",
        "Cancellation",
        "Bytes",
        "BasicCredentials",
        "Authorization",
        "CredentialRequest",
        "CredentialProvider",
        "HttpExchange",
        "ResponseBody",
        "ItemStream",
        "StreamFraming",
        "LinkMetadata",
        "ServerChoice",
        "Source",
        "detail",
        "schema",
        "models",
        "std",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
const KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "atomic_cancel",
    "atomic_commit",
    "atomic_noexcept",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "compl",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "co_await",
    "co_return",
    "co_yield",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "reflexpr",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "synchronized",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
];
