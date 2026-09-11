//! Native type/codec descriptors lowered from the checked finite program.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::{Contract, ResponseStatus, SchemaId};
use suspect_schema::{
    OwnedOutcome, OwnedProgram, OwnedSchema, ProgramInstruction as I, ProgramType,
};

use super::{HttpDiagnostic, diag};

/// One model member's independently retained wire and native names.
#[derive(Debug, Clone)]
pub struct DartField {
    pub name: String,
    pub wire_name: String,
    pub required: bool,
    /// Program node for the field; absent only for unconstrained undeclared keys.
    pub target: Option<usize>,
    /// Required literal exposed as a read-only native getter.
    pub fixed: Option<Value>,
}

/// Native representation, with finite program indices for child model links.
#[derive(Debug, Clone)]
pub enum Shape {
    Any,
    /// Dynamic binding depends on the entered parent resources. Native fields
    /// carry exact JSON and are checked by the complete enclosing codec.
    Contextual,
    Never,
    Null,
    Boolean,
    String,
    Number,
    Integer,
    Alias(usize),
    Array(Option<usize>),
    Object {
        fields: Vec<DartField>,
        extras: Extras,
    },
    Enum(Vec<(String, String)>),
    Union {
        targets: Vec<usize>,
        direct: bool,
        variants: Vec<String>,
    },
    /// A validated, exact JSON value wrapper for non-native intersections,
    /// tuple domains and conditional/untyped constraints in the owned subset.
    Checked,
}

#[derive(Debug, Clone, Copy)]
pub enum Extras {
    Any,
    Closed,
    Typed(usize),
    /// Pattern/scoped constraints need complete-object validation. Values stay
    /// exact JSON; neither a pattern nor additionalProperties is a global type.
    Checked,
}

/// Public native identity for one reachable canonical schema.
#[derive(Debug, Clone)]
pub struct DartModel {
    pub source: SchemaId,
    pub name: String,
    pub codec_name: String,
    pub nullable: bool,
    pub description: String,
    pub deprecated: bool,
    pub index: usize,
    pub shape: Shape,
    pub parents: Vec<String>,
}

impl DartModel {
    /// Allocated object members; empty for other representations.
    #[must_use]
    pub fn members(&self) -> &[DartField] {
        match &self.shape {
            Shape::Object { fields, .. } => fields,
            _ => &[],
        }
    }
}

/// Finite native model/codec graph. Every descriptor has a validation root.
#[derive(Debug)]
pub struct ModelPlan {
    pub(crate) models: Vec<DartModel>,
    by_source: BTreeMap<SchemaId, usize>,
    scoped: bool,
}

impl ModelPlan {
    #[must_use]
    pub fn symbols(&self) -> &[DartModel] {
        &self.models
    }
    #[must_use]
    pub fn model(&self, source: &SchemaId) -> Option<&DartModel> {
        self.by_source.get(source).map(|&index| &self.models[index])
    }
    #[must_use]
    pub fn native_type(&self, source: &SchemaId) -> Option<String> {
        self.by_source.get(source).map(|&index| self.ty(index))
    }
    /// Whether this signature uses Dart null rather than a checked JsonNull
    /// carrier. V2 aliases retain the target's exact-JSON representation.
    #[must_use]
    pub fn uses_native_null(&self, index: usize) -> bool {
        let m = &self.models[index];
        m.nullable
            && !matches!(
                m.shape,
                Shape::Any | Shape::Contextual | Shape::Null | Shape::Never | Shape::Checked
            )
            && !(self.scoped
                && matches!(m.shape, Shape::Alias(_))
                && matches!(
                    self.models[self.concrete(index)].shape,
                    Shape::Any | Shape::Contextual | Shape::Null | Shape::Never | Shape::Checked
                ))
    }
    #[must_use]
    pub fn ty(&self, index: usize) -> String {
        let m = &self.models[index];
        let base = match &m.shape {
            Shape::Any | Shape::Contextual => "JsonValue".into(),
            Shape::Never => "Never".into(),
            Shape::Null => "Null".into(),
            Shape::Boolean => "bool".into(),
            Shape::String => "String".into(),
            Shape::Number => "JsonNumber".into(),
            Shape::Integer => "JsonInteger".into(),
            Shape::Array(target) => format!("List<{}>", self.optional_ty(*target)),
            Shape::Alias(target) => self.ty(*target).trim_end_matches('?').to_owned(),
            Shape::Object { .. } | Shape::Enum(_) | Shape::Union { .. } | Shape::Checked => {
                m.name.clone()
            }
        };
        if self.uses_native_null(index) {
            format!("{base}?")
        } else {
            base
        }
    }
    pub(super) fn optional_ty(&self, index: Option<usize>) -> String {
        index.map_or_else(|| "JsonValue".into(), |i| self.ty(i))
    }
    pub(super) fn used_names(&self) -> BTreeSet<String> {
        let mut result = reserved();
        for model in &self.models {
            result.insert(model.name.clone());
            result.insert(model.codec_name.clone());
            if let Shape::Union { variants, .. } = &model.shape {
                result.extend(variants.iter().cloned());
            }
        }
        result
    }
    pub(super) fn concrete(&self, mut index: usize) -> usize {
        while let Shape::Alias(target) = self.models[index].shape {
            index = target;
        }
        index
    }
}

pub(super) fn plan(
    contract: &Contract,
    compiled: &OwnedSchema,
    program: &OwnedProgram,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let sources: BTreeMap<_, _> = compiled
        .roots()
        .map(|id| {
            (
                (id.document().to_string(), id.pointer().to_owned()),
                id.clone(),
            )
        })
        .collect();
    let mut names = reserved();
    let mut models = Vec::new();
    let mut errors = Vec::new();
    for (index, node) in program.nodes.iter().enumerate() {
        let source = sources[&(node.source.document.clone(), node.source.pointer.clone())].clone();
        let name = allocate(&source_name(contract, &source), &mut names);
        let codec_name = allocate(&format!("{}Codec", member(&name)), &mut names);
        let nullable = match compiled.validate(&source, &Value::Null) {
            OwnedOutcome::Valid => true,
            OwnedOutcome::Invalid(_) => false,
            OwnedOutcome::EvaluationFailure(finding) => {
                errors.push(diag(
                    contract,
                    finding.source,
                    "dart-nullability-evaluation",
                    format!("nullability proof is incomplete: {}", finding.message),
                ));
                false
            }
        };
        let raw = contract.source(&source);
        let mut representation = shape(program, index);
        if let Shape::Object { fields, .. } = &mut representation {
            for field in fields {
                if let (Some(target), Some(value)) = (field.target, &field.fixed) {
                    let node = &program.nodes[target];
                    let field_source =
                        &sources[&(node.source.document.clone(), node.source.pointer.clone())];
                    // Project a constant getter only when the complete field
                    // schema accepts the literal. Contradictory schemas retain
                    // compilable native fields and runtime validation obligations.
                    if !matches!(compiled.validate(field_source, value), OwnedOutcome::Valid) {
                        field.fixed = None;
                    }
                }
            }
        }
        models.push(DartModel {
            source,
            name,
            codec_name,
            nullable,
            index,
            description: raw
                .and_then(|raw| raw.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            deprecated: raw
                .and_then(|raw| raw.get("deprecated"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            shape: representation,
            parents: Vec::new(),
        });
    }
    for model in &models {
        // A native typedef/list cycle with no object or union guard has no
        // finite declaration in Dart. Source refs retain their exact identity.
        if alias_cycle(&models, model.index, &mut BTreeSet::new()) {
            errors.push(diag(
                contract,
                model.source.clone(),
                "dart-unguarded-alias-cycle",
                "recursive aliases/arrays require an object or union declaration guard",
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut result = ModelPlan {
        by_source: models.iter().map(|m| (m.source.clone(), m.index)).collect(),
        models,
        scoped: program.version != OwnedProgram::V1_VERSION,
    };
    // Tagged alternatives become a sealed interface implemented by the actual
    // branch classes. The proof uses required + const compiled checks, not
    // discriminator hints or conventional field/operation names.
    for index in 0..result.models.len() {
        let Shape::Union { targets, .. } = &result.models[index].shape else {
            continue;
        };
        let targets = targets.clone();
        let direct = tagged(&result, program, &targets);
        let parent = result.models[index].name.clone();
        let mut variants = Vec::new();
        if direct {
            for target in &targets {
                let target = result.concrete(*target);
                if !result.models[target].parents.contains(&parent) {
                    result.models[target].parents.push(parent.clone());
                }
            }
        } else {
            for (ordinal, _) in targets.iter().enumerate() {
                variants.push(allocate(
                    &format!("{parent}Variant{}", ordinal + 1),
                    &mut names,
                ));
            }
        }
        result.models[index].shape = Shape::Union {
            targets,
            direct,
            variants,
        };
    }
    Ok(result)
}

fn shape(program: &OwnedProgram, index: usize) -> Shape {
    let checks = &program.nodes[index].checks;
    if checks
        .iter()
        .any(|c| matches!(c.instruction, I::DynamicRef { .. }))
    {
        return Shape::Contextual;
    }
    if checks
        .iter()
        .any(|c| matches!(c.instruction, I::Always { value: false }))
    {
        return Shape::Never;
    }
    if checks.is_empty()
        || checks
            .iter()
            .all(|c| matches!(c.instruction, I::Always { value: true }))
    {
        return Shape::Any;
    }
    let refs: Vec<_> = checks
        .iter()
        .filter_map(|c| {
            if let I::Ref { target } = c.instruction {
                Some(target)
            } else {
                None
            }
        })
        .collect();
    if let [target] = refs.as_slice() {
        return Shape::Alias(*target);
    }
    if checks.iter().any(|c| {
        matches!(
            c.instruction,
            I::AllOf { .. } | I::Not { .. } | I::PrefixItems { .. }
        )
    }) {
        return Shape::Checked;
    }
    let unions: Vec<_> = checks
        .iter()
        .filter_map(|c| match &c.instruction {
            I::OneOf { targets } | I::AnyOf { targets } => Some(targets),
            _ => None,
        })
        .collect();
    if let [targets] = unions.as_slice() {
        return Shape::Union {
            targets: (*targets).clone(),
            direct: false,
            variants: Vec::new(),
        };
    }
    if !unions.is_empty() {
        return Shape::Checked;
    }
    if let Some(values) = checks.iter().find_map(|c| {
        if let I::Enum { values } = &c.instruction {
            Some(values)
        } else {
            None
        }
    }) {
        if values.iter().all(|v| v.is_string() || v.is_null())
            && values.iter().any(Value::is_string)
        {
            let mut used = BTreeSet::from([
                "index".into(),
                "name".into(),
                "values".into(),
                "wireValue".into(),
                "toString".into(),
                "hashCode".into(),
                "runtimeType".into(),
            ]);
            return Shape::Enum(
                values
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|wire| (wire.into(), allocate(&member(wire), &mut used)))
                    .collect(),
            );
        }
        return Shape::Checked;
    }
    let types = checks.iter().find_map(|c| {
        if let I::Type { types } = &c.instruction {
            Some(types)
        } else {
            None
        }
    });
    let types = types.map(|types| {
        types
            .iter()
            .copied()
            .filter(|t| {
                *t != ProgramType::Null
                    && (*t != ProgramType::Integer || !types.contains(&ProgramType::Number))
            })
            .collect::<Vec<_>>()
    });
    let ty = match types.as_deref() {
        Some([]) => return Shape::Null,
        Some([ty]) => *ty,
        Some(_) => return Shape::Checked,
        None => {
            return checks
                .iter()
                .find_map(|c| {
                    if let I::Const { value } = &c.instruction {
                        Some(match value {
                            Value::Null => Shape::Null,
                            Value::Bool(_) => Shape::Boolean,
                            Value::String(_) => Shape::String,
                            Value::Number(_) => Shape::Number,
                            _ => Shape::Checked,
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or(Shape::Checked);
        }
    };
    match ty {
        ProgramType::Null => Shape::Null,
        ProgramType::Boolean => Shape::Boolean,
        ProgramType::String => Shape::String,
        ProgramType::Number => Shape::Number,
        ProgramType::Integer => Shape::Integer,
        ProgramType::Array => Shape::Array(checks.iter().find_map(|c| {
            if let I::Items { target, start: 0 } = c.instruction {
                Some(target)
            } else {
                None
            }
        })),
        ProgramType::Object => {
            let properties = checks.iter().find_map(|c| {
                if let I::Properties { properties } = &c.instruction {
                    Some(properties)
                } else {
                    None
                }
            });
            let required = checks
                .iter()
                .filter_map(|c| {
                    if let I::Required { names } = &c.instruction {
                        Some(names)
                    } else {
                        None
                    }
                })
                .flatten()
                .collect::<BTreeSet<_>>();
            let extra_target = checks.iter().find_map(|c| {
                if let I::AdditionalProperties { target, .. } = c.instruction {
                    Some(target)
                } else {
                    None
                }
            });
            let scoped_extras = checks.iter().any(|c| {
                matches!(
                    c.instruction,
                    I::PatternProperties { .. }
                        | I::AdditionalPropertiesWithPatterns { .. }
                        | I::UnevaluatedProperties { .. }
                )
            });
            let extras = match extra_target {
                Some(target)
                    if program.nodes[target]
                        .checks
                        .iter()
                        .any(|c| matches!(c.instruction, I::Always { value: false })) =>
                {
                    Extras::Closed
                }
                Some(target) => Extras::Typed(target),
                None if scoped_extras => Extras::Checked,
                None => Extras::Any,
            };
            let mut used = BTreeSet::from([
                "extraFields".into(),
                "runtimeType".into(),
                "hashCode".into(),
                "toString".into(),
                "noSuchMethod".into(),
            ]);
            let mut fields: Vec<_> = properties
                .into_iter()
                .flatten()
                .map(|property| {
                    let fixed = if required.contains(&property.name) {
                        program.nodes[property.target].checks.iter().find_map(|c| {
                            if let I::Const {
                                value: value @ (Value::String(_) | Value::Bool(_) | Value::Null),
                            } = &c.instruction
                            {
                                Some(value.clone())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };
                    DartField {
                        name: allocate(&member(&property.name), &mut used),
                        wire_name: property.name.clone(),
                        required: required.contains(&property.name),
                        target: Some(property.target),
                        fixed,
                    }
                })
                .collect();
            for name in required {
                if !fields.iter().any(|f| &f.wire_name == name) {
                    fields.push(DartField {
                        name: allocate(&member(name), &mut used),
                        wire_name: name.clone(),
                        required: true,
                        target: extra_target,
                        fixed: None,
                    });
                }
            }
            fields.sort_by(|a, b| a.wire_name.cmp(&b.wire_name));
            Shape::Object { fields, extras }
        }
    }
}

fn alias_cycle(models: &[DartModel], index: usize, active: &mut BTreeSet<usize>) -> bool {
    if !active.insert(index) {
        return true;
    }
    let result = match models[index].shape {
        Shape::Alias(target) | Shape::Array(Some(target)) => alias_cycle(models, target, active),
        _ => false,
    };
    active.remove(&index);
    result
}

fn tagged(plan: &ModelPlan, program: &OwnedProgram, targets: &[usize]) -> bool {
    if targets.is_empty() {
        return false;
    }
    let mut seen = BTreeSet::new();
    let mut possible: Option<BTreeMap<String, BTreeSet<String>>> = None;
    for target in targets {
        let index = plan.concrete(*target);
        if !seen.insert(index) || plan.models[*target].nullable {
            return false;
        }
        let Shape::Object { fields, .. } = &plan.models[index].shape else {
            return false;
        };
        let tags = fields
            .iter()
            .filter(|f| f.required)
            .filter_map(|f| {
                let target = f.target?;
                program.nodes[target].checks.iter().find_map(|c| {
                    if let I::Const {
                        value: Value::String(value),
                    } = &c.instruction
                    {
                        Some((f.wire_name.clone(), value.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect::<BTreeMap<_, _>>();
        if let Some(possible) = &mut possible {
            possible.retain(|key, values| {
                tags.get(key)
                    .is_some_and(|value| values.insert(value.clone()))
            });
        } else {
            possible = Some(
                tags.into_iter()
                    .map(|(key, value)| (key, BTreeSet::from([value])))
                    .collect(),
            );
        }
    }
    possible.is_some_and(|tags| !tags.is_empty())
}

fn source_name(contract: &Contract, id: &SchemaId) -> String {
    let pointer = id.pointer();
    let segments = pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let mut names = Vec::new();
    let mut start = 0;
    let mut sole_success = None;
    if segments.first().is_some_and(|s| s == "components")
        && segments.get(1).is_some_and(|s| s == "schemas")
    {
        names.push(exported(&segments[2]));
        start = 3;
    } else if let Some(operation) = contract
        .operations()
        .filter(|op| pointer.starts_with(&format!("{}/", op.source().pointer())))
        .max_by_key(|op| op.source().pointer().len())
    {
        names.push(exported(operation.operation_id().unwrap_or("Operation")));
        start = operation.source().pointer().split('/').count() - 1;
        let successes = operation
            .responses()
            .iter()
            .filter_map(|response| {
                if let Some(ResponseStatus::Exact(status)) = response.status() {
                    (200..300).contains(&status).then_some(status)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if let [status] = successes.as_slice() {
            sole_success = Some(status.to_string());
        }
    }
    let mut i = start;
    while i < segments.len() {
        match segments[i].as_str() {
            "properties" | "content" | "schema" => {}
            "application/json" => {}
            "requestBody" => names.push("Body".into()),
            "responses" if i + 1 < segments.len() => {
                names.push("Response".into());
                i += 1;
                if sole_success.as_ref() != Some(&segments[i]) {
                    names.push(segments[i].clone());
                }
            }
            "parameters" => names.push("Parameter".into()),
            "additionalProperties" => names.push("Extra".into()),
            "items" => names.push("Item".into()),
            other => names.push(exported(other)),
        }
        i += 1;
    }
    let name = names.join("");
    if name.is_empty() {
        "Model".into()
    } else {
        name
    }
}

pub(super) fn keyword(name: &str) -> bool {
    [
        "abstract",
        "as",
        "assert",
        "async",
        "await",
        "base",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "covariant",
        "default",
        "deferred",
        "do",
        "dynamic",
        "else",
        "enum",
        "export",
        "extends",
        "extension",
        "external",
        "factory",
        "false",
        "final",
        "finally",
        "for",
        "Function",
        "get",
        "hide",
        "if",
        "implements",
        "import",
        "in",
        "interface",
        "is",
        "late",
        "library",
        "mixin",
        "new",
        "null",
        "of",
        "on",
        "operator",
        "part",
        "required",
        "rethrow",
        "return",
        "sealed",
        "set",
        "show",
        "static",
        "super",
        "switch",
        "sync",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typedef",
        "var",
        "void",
        "when",
        "while",
        "with",
        "yield",
    ]
    .contains(&name)
}
pub(super) fn exported(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if out.is_empty() && c.is_ascii_digit() {
                out.push('N');
            }
            out.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        out.push_str("Value");
    }
    out
}
pub(super) fn member(name: &str) -> String {
    let mut value = exported(name);
    value.replace_range(..1, &value[..1].to_ascii_lowercase());
    if keyword(&value) {
        value.push_str("Value");
    }
    value
}
pub(super) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut result = base.to_owned();
    let mut suffix = 2;
    while !used.insert(result.clone()) {
        result = format!("{base}{suffix}");
        suffix += 1;
    }
    result
}
fn reserved() -> BTreeSet<String> {
    [
        "NoBody",
        "BasicCredentials",
        "AuthorizationCredential",
        "CredentialProvider",
        "CredentialInfo",
        "CredentialRequest",
        "ServerInfo",
        "ServerSelection",
        "ServerVariable",
        "LinkMetadata",
        "Deprecated",
        "Enum",
        "Pattern",
        "Record",
        "Symbol",
        "Type",
        "Client",
        "Credentials",
        "Presence",
        "Absent",
        "Present",
        "JsonValue",
        "JsonNull",
        "JsonBoolean",
        "JsonString",
        "JsonArray",
        "JsonObject",
        "JsonNumber",
        "JsonInteger",
        "JsonLimits",
        "JsonException",
        "SchemaSource",
        "ValidationFinding",
        "ValidationStatus",
        "ValidationResult",
        "ModelCodec",
        "CodecException",
        "CodecFailureKind",
        "CancellationToken",
        "HttpTransport",
        "TransportRequest",
        "TransportResponse",
        "RawResponse",
        "SdkResponse",
        "SdkException",
        "ApiException",
        "CancelledException",
        "TimeoutException",
        "TransportException",
        "UnexpectedResponseException",
        "InvalidResponseException",
        "MediaTypeException",
        "ResourceLimitException",
        "ConfigurationException",
        "ClientClosedException",
        "String",
        "Object",
        "List",
        "Map",
        "Set",
        "Future",
        "Stream",
        "Duration",
        "Uri",
        "BigInt",
        "Never",
        "Null",
        "Uint8List",
        "HashSet",
        "Completer",
        "Timer",
        "Comparable",
        "Iterable",
        "Iterator",
        "MapEntry",
        "StringBuffer",
        "RegExp",
        "ArgumentError",
        "StateError",
        "Error",
        "Exception",
        "FormatException",
        "StackTrace",
        "Function",
        "StreamSubscription",
        "StreamController",
        "FutureOr",
        "BytesBuilder",
        "DateTime",
        "Stopwatch",
        "IoTransport",
        "HttpClient",
        "HttpClientRequest",
        "HttpClientResponse",
        "HttpHeaders",
        "Socket",
        "ConnectionTask",
        "InternetAddress",
        "SecurityContext",
        "parseJson",
        "parseJsonBytes",
        "writeJson",
        "validateJson",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
