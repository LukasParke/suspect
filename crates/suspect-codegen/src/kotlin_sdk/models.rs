//! Kotlin-first native shape allocation. Assertions remain in the owned program.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::OwnedProgram;

use super::{HttpDiagnostic, diagnostic};

/// One source schema, its native name and public codec property.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub source: SchemaId,
    pub name: String,
    pub codec_name: String,
    pub kotlin_type: String,
    /// Actual native constructor, where this schema declares a data/value class.
    pub constructor: Option<String>,
    pub(crate) index: usize,
    pub shape: Shape,
    pub nullable: bool,
}

/// Allocated property and primary-constructor participation.
#[derive(Debug, Clone)]
pub struct Field {
    pub wire_name: String,
    pub name: String,
    pub schema: SchemaId,
    pub required: bool,
    /// Includes Presence for source-optional members.
    pub kotlin_type: String,
    /// A required single string tag is codec-owned and omitted from constructors.
    pub constant: Option<String>,
}

/// Retained native representations, without reparsing emitted Kotlin.
#[derive(Debug, Clone)]
pub enum Shape {
    Any,
    /// Schema-bound carrier for shapes whose complete constraints cannot be
    /// projected into static Kotlin fields. Both codec directions remain checked.
    CheckedJson,
    Never,
    Null,
    Boolean,
    String,
    Number,
    Alias(SchemaId),
    Array(Option<SchemaId>),
    Object {
        fields: Vec<Field>,
        additional: Additional,
    },
    StringEnum(Vec<(String, String)>),
    Union {
        variants: Vec<(String, SchemaId)>,
        exclusive: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Additional {
    Closed,
    Any,
    /// Keys/values admitted by scoped pattern/unevaluated rules. Stored as exact
    /// JSON, not projected through additionalProperties alone.
    Scoped,
    Typed(SchemaId),
}

/// All reachable shapes; aliases retain distinct source-schema codecs.
#[derive(Debug)]
pub struct ModelPlan {
    symbols: Vec<Symbol>,
    by_id: BTreeMap<SchemaId, usize>,
}

impl ModelPlan {
    #[must_use]
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    #[must_use]
    pub fn symbol(&self, id: &SchemaId) -> Option<&Symbol> {
        self.by_id.get(id).map(|&index| &self.symbols[index])
    }

    pub(crate) fn get(&self, id: &SchemaId) -> &Symbol {
        self.symbol(id).expect("admitted Kotlin schema")
    }

    pub fn target(&self, id: &SchemaId) -> &Symbol {
        let mut symbol = self.get(id);
        while let Shape::Alias(target) = &symbol.shape {
            symbol = self.get(target);
        }
        symbol
    }
}

pub(crate) fn plan(
    contract: &Contract,
    roots: &[SchemaId],
    program: &OwnedProgram,
    mut used: BTreeSet<String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let scoped = program.version != OwnedProgram::V1_VERSION;
    if scoped {
        used.extend(
            ["ScopedProgram", "ScopedMarks", "ScopedResult"]
                .into_iter()
                .map(str::to_owned),
        );
    }
    if program.version == OwnedProgram::V3_VERSION {
        used.extend(
            ["IndexedResources", "ResourceScope", "ResourceCycle"]
                .into_iter()
                .map(str::to_owned),
        );
    }
    let mut errors = Vec::new();
    let mut by_id = BTreeMap::new();
    let mut symbols = Vec::new();
    let mut codec_names = BTreeSet::new();
    for id in roots {
        let raw = contract.schema(id).expect("owned root").raw();
        for key in ["readOnly", "writeOnly"] {
            if raw.get(key) == Some(&Value::Bool(true)) {
                errors.push(diagnostic(contract, id.child(key), "kotlin-direction-unsupported", "active readOnly/writeOnly requires directional native views, not a neutral model"));
            }
        }
        let hint = if program.version == OwnedProgram::V3_VERSION
            && raw.get("$dynamicRef").is_some()
            && id.pointer().ends_with("/items")
        {
            format!("{}Item", schema_name(contract, id))
        } else {
            schema_name(contract, id)
        };
        let name = allocate(&hint, &mut used);
        let codec_name = allocate(&member_name(&name), &mut codec_names);
        let projected = shape(contract, id);
        let projected =
            if program.version == OwnedProgram::V3_VERSION && raw.get("$dynamicRef").is_some() {
                Ok((Shape::CheckedJson, false))
            } else if scoped {
                match projected {
                    Ok((Shape::Object { fields, additional }, nullable)) => {
                        let patterns = raw
                            .get("patternProperties")
                            .and_then(Value::as_object)
                            .is_some_and(|p| !p.is_empty());
                        Ok((
                            Shape::Object {
                                fields,
                                additional: if patterns {
                                    Additional::Scoped
                                } else {
                                    additional
                                },
                            },
                            nullable,
                        ))
                    }
                    Err(_) => Ok((Shape::CheckedJson, false)),
                    other => other,
                }
            } else {
                projected
            };
        match projected {
            Ok((shape, nullable)) => {
                let index = program
                    .roots
                    .iter()
                    .find(|root| {
                        root.source.document == id.document().as_str()
                            && root.source.pointer == id.pointer()
                    })
                    .expect("selected codec root")
                    .target;
                by_id.insert(id.clone(), symbols.len());
                let constructor = matches!(&shape, Shape::Object { .. } | Shape::CheckedJson)
                    .then(|| name.clone());
                symbols.push(Symbol {
                    source: id.clone(),
                    name,
                    codec_name,
                    kotlin_type: String::new(),
                    constructor,
                    index,
                    shape,
                    nullable,
                });
            }
            Err(message) => errors.push(diagnostic(
                contract,
                id.clone(),
                "kotlin-model-unsupported",
                message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut result = ModelPlan { symbols, by_id };
    for index in 0..result.symbols.len() {
        match native_type(&result, &result.symbols[index].source, &mut BTreeSet::new()) {
            Ok(ty) => result.symbols[index].kotlin_type = ty,
            Err(message) => errors.push(diagnostic(
                contract,
                result.symbols[index].source.clone(),
                "kotlin-recursive-alias",
                message,
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for index in 0..result.symbols.len() {
        let mut shape = result.symbols[index].shape.clone();
        if let Shape::Object { fields, .. } = &mut shape {
            for field in fields {
                let child = result.get(&field.schema);
                field.kotlin_type = if field.required {
                    child.kotlin_type.clone()
                } else {
                    format!("Presence<{}>", child.kotlin_type)
                };
                let target = result.target(&field.schema);
                if field.required
                    && !child.kotlin_type.ends_with('?')
                    && let Shape::StringEnum(cases) = &target.shape
                    && cases.len() == 1
                {
                    field.constant = Some(format!("{}.{}", target.name, cases[0].0));
                }
            }
        }
        result.symbols[index].shape = shape;
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}

fn native_type(
    plan: &ModelPlan,
    id: &SchemaId,
    seen: &mut BTreeSet<SchemaId>,
) -> Result<String, &'static str> {
    if !seen.insert(id.clone()) {
        return Err("an alias/array cycle needs an object or sealed union guard");
    }
    let s = plan.get(id);
    let base = match &s.shape {
        Shape::Any => "JsonValue".into(),
        Shape::Never => "Nothing".into(),
        Shape::Null => "Nothing?".into(),
        Shape::Boolean => "Boolean".into(),
        Shape::String => "String".into(),
        Shape::Number => "JsonNumber".into(),
        Shape::Alias(target) => native_type(plan, target, seen)?,
        Shape::Array(None) => "List<JsonValue>".into(),
        Shape::Array(Some(item)) => format!("List<{}>", native_type(plan, item, seen)?),
        Shape::Object { .. } | Shape::StringEnum(_) | Shape::Union { .. } | Shape::CheckedJson => {
            s.name.clone()
        }
    };
    seen.remove(id);
    Ok(if s.nullable && !base.ends_with('?') {
        format!("{base}?")
    } else {
        base
    })
}

fn shape(contract: &Contract, id: &SchemaId) -> Result<(Shape, bool), String> {
    let schema = contract.schema(id).expect("owned schema");
    let raw = schema.raw();
    if let Some(value) = raw.as_bool() {
        return Ok((if value { Shape::Any } else { Shape::Never }, false));
    }
    let raw = raw.as_object().expect("owned schema object");
    // Do not flatten intersections or silently discard sibling fields.
    for key in ["allOf", "prefixItems"] {
        if raw.contains_key(key) {
            return Err(format!(
                "{key} has no faithful Kotlin native representation in this profile"
            ));
        }
    }
    if raw.contains_key("$ref") {
        if [
            "properties",
            "additionalProperties",
            "items",
            "type",
            "oneOf",
            "anyOf",
        ]
        .iter()
        .any(|key| raw.contains_key(*key))
        {
            return Err("structural $ref siblings require native intersection planning".into());
        }
        let target = schema
            .references()
            .iter()
            .find(|r| r.keyword == "$ref")
            .and_then(|r| r.target.clone())
            .ok_or("unresolved reference")?;
        return Ok((Shape::Alias(target), false));
    }
    if let Some((key, variants)) = ["oneOf", "anyOf"]
        .into_iter()
        .find_map(|key| raw.get(key).and_then(Value::as_array).map(|v| (key, v)))
    {
        if ["properties", "items", "type", "additionalProperties"]
            .iter()
            .any(|key| raw.contains_key(*key))
            || (raw.contains_key("oneOf") && raw.contains_key("anyOf"))
        {
            return Err("union with structural siblings needs native intersection planning".into());
        }
        if variants.len() > 64 {
            return Err("native unions admit at most 64 alternatives".into());
        }
        let mut used = BTreeSet::new();
        let variants = variants
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let child = id.child(key).child(&i.to_string());
                let direct = contract
                    .schema(&child)
                    .and_then(|s| s.references().first())
                    .and_then(|r| r.target.as_ref());
                let label = direct
                    .map(|s| schema_name(contract, s))
                    .unwrap_or_else(|| format!("Variant{}", i + 1));
                (allocate(&format!("As{label}"), &mut used), child)
            })
            .collect();
        return Ok((
            Shape::Union {
                variants,
                exclusive: key == "oneOf",
            },
            false,
        ));
    }
    let types: Vec<&str> = match raw.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(types)) => types.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let nullable = types.contains(&"null") && types.len() > 1;
    let nonnull: Vec<_> = types.iter().copied().filter(|t| *t != "null").collect();
    if nonnull.len() > 1 {
        return Err(
            "multi-type unions other than T|null require explicit oneOf/anyOf native branches"
                .into(),
        );
    }
    let ty = nonnull.first().copied().or_else(|| types.first().copied());
    let string_values = raw
        .get("enum")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| raw.get("const").map(|v| vec![v.clone()]));
    if let Some(values) = string_values.as_ref()
        && values.iter().any(Value::is_string)
        && values.iter().all(|v| v.is_string() || v.is_null())
        && matches!(ty, None | Some("string"))
    {
        if values.len() > 256 {
            return Err("native string enums admit at most 256 values".into());
        }
        let mut used = BTreeSet::new();
        let nullable = values.iter().any(Value::is_null) && (nullable || ty.is_none());
        let values = values
            .iter()
            .filter(|v| v.is_string())
            .map(|v| {
                let wire = v.as_str().unwrap();
                let case = allocate(&enum_name(wire), &mut used);
                (case, wire.to_owned())
            })
            .collect();
        return Ok((Shape::StringEnum(values), nullable));
    }
    let result = match ty {
        Some("null") => Shape::Null,
        Some("boolean") => Shape::Boolean,
        Some("string") => Shape::String,
        Some("integer" | "number") => Shape::Number,
        Some("array") => Shape::Array(raw.get("items").map(|_| id.child("items"))),
        Some("object") => {
            let properties = raw.get("properties").and_then(Value::as_object);
            let required: BTreeSet<_> = raw
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if required
                .iter()
                .any(|name| properties.is_none_or(|properties| !properties.contains_key(*name)))
            {
                return Err("required members without a properties schema need an explicit native member plan".into());
            }
            if properties.is_some_and(|p| p.len() > 100) {
                return Err(
                    "Kotlin data classes admit at most 100 declared fields in this profile".into(),
                );
            }
            let mut used = BTreeSet::from(["additionalProperties".into(), "codec".into()]);
            let fields = properties
                .into_iter()
                .flatten()
                .map(|(wire, _)| Field {
                    wire_name: wire.clone(),
                    name: allocate(&member_name(wire), &mut used),
                    schema: id.child("properties").child(wire),
                    required: required.contains(wire.as_str()),
                    kotlin_type: String::new(),
                    constant: None,
                })
                .collect();
            let additional = match raw.get("additionalProperties") {
                Some(Value::Bool(false)) => Additional::Closed,
                None | Some(Value::Bool(true)) => Additional::Any,
                _ => Additional::Typed(id.child("additionalProperties")),
            };
            Shape::Object { fields, additional }
        }
        None if raw.keys().all(|key| annotation(key)) => Shape::Any,
        None => {
            return Err(
                "assertions without a supported explicit native type cannot become erased JSON"
                    .into(),
            );
        }
        Some(other) => return Err(format!("unsupported Kotlin type {other}")),
    };
    // Applicators do not imply type. Reject shapes that would drop declared
    // fields on instances of other admitted kinds.
    if raw.contains_key("properties") && ty != Some("object")
        || raw.contains_key("items") && ty != Some("array")
    {
        return Err("cross-kind applicators require an explicit native union".into());
    }
    Ok((result, nullable))
}

fn annotation(key: &str) -> bool {
    key.starts_with("x-")
        || matches!(
            key,
            "$schema"
                | "$comment"
                | "title"
                | "description"
                | "summary"
                | "example"
                | "examples"
                | "default"
                | "deprecated"
                | "readOnly"
                | "writeOnly"
                | "format"
                | "externalDocs"
                | "discriminator"
        )
}

fn schema_name(contract: &Contract, id: &SchemaId) -> String {
    let parts: Vec<_> = id
        .pointer()
        .split('/')
        .skip(1)
        .map(|p| p.replace("~1", "/").replace("~0", "~"))
        .collect();
    if parts.first().is_some_and(|s| s == "components")
        && parts.get(1).is_some_and(|s| s == "schemas")
    {
        return type_name(
            &parts[2..]
                .iter()
                .filter(|p| !matches!(p.as_str(), "properties" | "items" | "schema"))
                .cloned()
                .collect::<Vec<_>>()
                .join("_"),
        );
    }
    if let Some(op) = contract
        .operations()
        .filter(|op| {
            id.document() == op.source().document()
                && id
                    .pointer()
                    .starts_with(&format!("{}/", op.source().pointer()))
        })
        .max_by_key(|op| op.source().pointer().len())
    {
        let tail = &id.pointer()[op.source().pointer().len() + 1..];
        let mut tokens = vec![op.operation_id().unwrap_or("Operation").to_owned()];
        let tail: Vec<_> = tail.split('/').collect();
        if tail.first() == Some(&"requestBody") {
            tokens.push("Body".into());
        } else if tail.first() == Some(&"responses") {
            tokens.push(format!("Response{}", tail.get(1).unwrap_or(&"")));
        } else if tail.first() == Some(&"parameters") {
            tokens.push(format!("Parameter{}", tail.get(1).unwrap_or(&"")));
        }
        if let Some(at) = tail.iter().position(|p| *p == "schema") {
            tokens.extend(
                tail[at + 1..]
                    .iter()
                    .filter(|p| !matches!(**p, "properties" | "items"))
                    .map(|p| p.replace("~1", "/").replace("~0", "~")),
            );
        }
        return type_name(&tokens.join("_"));
    }
    type_name(&parts.join("_"))
}

pub(crate) fn type_name(input: &str) -> String {
    let mut result = String::new();
    let mut upper = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else if !c.is_ascii() {
            result.push_str(&format!("U{:X}", c as u32));
            upper = true;
        } else {
            upper = true;
        }
    }
    if result.is_empty() {
        result.push_str("Value");
    }
    if result.as_bytes()[0].is_ascii_digit() {
        result.insert(0, 'N');
    }
    if result.len() > 120 {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(input.as_bytes()));
        result.truncate(100); // Result is ASCII even for Unicode source names.
        result.push_str(&digest[..16]);
    }
    result
}

pub(crate) fn member_name(input: &str) -> String {
    let mut name = type_name(input);
    name[..1].make_ascii_lowercase();
    if keyword(&name)
        || matches!(
            name.as_str(),
            "toString"
                | "hashCode"
                | "equals"
                | "copy"
                | "getClass"
                | "notify"
                | "notifyAll"
                | "wait"
        )
    {
        name.push_str("Value");
    }
    name
}

fn enum_name(input: &str) -> String {
    let mut value = String::new();
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            value.push(c.to_ascii_uppercase());
        } else if c.is_ascii() {
            value.push('_');
        } else {
            value.push_str(&format!("U{:X}", c as u32));
        }
    }
    if value.is_empty() || value.bytes().all(|b| b == b'_') {
        value = "EMPTY".into();
    }
    if value.as_bytes()[0].is_ascii_digit() {
        value.insert(0, 'N');
    }
    if value.len() > 120 {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(input.as_bytes()));
        value.truncate(100);
        value.push_str(&digest[..16].to_ascii_uppercase());
    }
    value
}

pub(crate) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut value = base.to_owned();
    let mut suffix = 2;
    while !used.insert(value.clone()) {
        value = format!("{base}{suffix}");
        suffix += 1;
    }
    value
}

pub(crate) fn keyword(input: &str) -> bool {
    matches!(
        input,
        "as" | "break"
            | "class"
            | "continue"
            | "do"
            | "else"
            | "false"
            | "for"
            | "fun"
            | "if"
            | "in"
            | "interface"
            | "is"
            | "null"
            | "object"
            | "package"
            | "return"
            | "super"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typealias"
            | "typeof"
            | "val"
            | "var"
            | "when"
            | "while"
    )
}

pub(crate) fn reserved_types() -> BTreeSet<String> {
    [
        "Quickstart",
        "Dispatchers",
        "System",
        "Comparable",
        "Enum",
        "MutableList",
        "MutableMap",
        "String",
        "StringBuilder",
        "Boolean",
        "Int",
        "Long",
        "Double",
        "Float",
        "Char",
        "Character",
        "Byte",
        "List",
        "Map",
        "Set",
        "Nothing",
        "Any",
        "Unit",
        "Array",
        "ByteArray",
        "IntArray",
        "Pair",
        "Result",
        "Exception",
        "Throwable",
        "IllegalArgumentException",
        "IllegalStateException",
        "RuntimeException",
        "ArithmeticException",
        "AutoCloseable",
        "Client",
        "ClientOptions",
        "Credentials",
        "RequestOptions",
        "HttpRequest",
        "HttpResponse",
        "Transport",
        "TransportException",
        "JdkTransport",
        "JdkClient",
        "JdkRequest",
        "JdkResponse",
        "ResponseInfo",
        "SdkException",
        "ApiException",
        "FailureKind",
        "SourceLocation",
        "ValidationException",
        "ValidationFinding",
        "ModelCodec",
        "CodecLimits",
        "Codecs",
        "Presence",
        "JsonValue",
        "JsonNumber",
        "JsonNull",
        "JsonString",
        "JsonBoolean",
        "JsonArray",
        "JsonObject",
        "Json",
        "JsonException",
        "JsonErrorKind",
        "JsonLimits",
        "JsonBudget",
        "EvaluationException",
        "Validator",
        "SchemaValidation",
        "ValidationSession",
        "ModelBudget",
        "ValidationProgram",
        "GeneratedExamples",
        "URI",
        "Duration",
        "CompletionException",
        "CancellationException",
        "ExactDecimal",
        "BigInteger",
        "ByteBuffer",
        "ByteArrayOutputStream",
        "CodingErrorAction",
        "CompletableFuture",
        "CompletionStage",
        "Flow",
        "Proxy",
        "ProxySelector",
        "SocketAddress",
        "AtomicReference",
        "CoroutineContext",
        "CallContext",
        "WireUrl",
        "HeaderSnapshot",
        "JsonKt",
        "ValidationKt",
        "HttpKt",
        "CodecsKt",
        "ModelsKt",
        "ClientKt",
        "Charsets",
        "Regex",
        "LazyThreadSafetyMode",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
