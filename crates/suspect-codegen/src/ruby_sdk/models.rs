//! Native shapes lowered only from checked portable instructions. Source access
//! here is for names/prose/provenance, never assertion or reference discovery.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest, Sha256};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedProgram, ProgramInstruction as Op, ProgramType};

use super::{HttpDiagnostic, diagnostic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScalarKind {
    Null,
    Boolean,
    Integer,
    Number,
    String,
}

/// Extra members are closed, unconstrained JSON, or converted through a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraFields {
    Closed,
    Json,
    Typed(usize),
    /// Exact JSON extra values selected by source applicators. Pattern overlaps
    /// and unevaluated membership are validated by the complete scoped program;
    /// they are not flattened into one additional-properties type.
    Scoped {
        patterns: Vec<PatternExtra>,
        additional: Option<usize>,
        unevaluated: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternExtra {
    pub source: SchemaId,
    pub pattern: String,
    pub schema_index: usize,
}

/// A named native member with a distinct wire name and absence policy.
#[derive(Debug, Clone)]
pub struct ModelField {
    pub wire_name: String,
    pub name: String,
    pub schema_index: usize,
    pub required: bool,
    pub source: SchemaId,
    pub description: String,
    /// A required source literal can be supplied by the keyword constructor.
    /// This is not a JSON Schema `default`, and optional fields remain UNSET.
    pub literal: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum ModelShape {
    /// Only genuinely unconstrained source schemas use the JSON value domain.
    Json,
    /// Exact JSON-domain storage refined by nonstructural assertions (e.g. not).
    /// The portable program enforces every assertion at both codec boundaries.
    RefinedJson,
    /// Exact source literals, including heterogeneous and structural values.
    Literal(Vec<Value>),
    Never,
    Scalar(Vec<ScalarKind>),
    Alias(usize),
    /// Context-sensitive JSON carrier. Candidates are dependencies, not a
    /// generation-time selection of the dynamic fallback or override.
    Dynamic {
        initial_target: usize,
        initial_resource: usize,
        anchor: Option<String>,
        candidates: Vec<usize>,
    },
    Object {
        fields: Vec<ModelField>,
        extras: ExtraFields,
        nullable: bool,
    },
    Array {
        items: Option<usize>,
        prefix: Vec<usize>,
        nullable: bool,
    },
    /// Native branch values; the entire source union is always revalidated.
    Union {
        branches: Vec<usize>,
        exclusive: bool,
    },
}

impl ModelShape {
    fn edges(&self) -> Vec<usize> {
        match self {
            Self::Alias(target) => vec![*target],
            // Dynamic targets remain in the validation closure. JSON storage
            // does not require standalone native carriers for inert candidates.
            Self::Dynamic { .. } => Vec::new(),
            Self::Object { fields, extras, .. } => {
                let mut edges = fields.iter().map(|f| f.schema_index).collect::<Vec<_>>();
                match extras {
                    ExtraFields::Typed(i) => edges.push(*i),
                    ExtraFields::Scoped {
                        patterns,
                        additional,
                        unevaluated,
                    } => {
                        edges.extend(patterns.iter().map(|p| p.schema_index));
                        edges.extend(additional);
                        edges.extend(unevaluated);
                    }
                    _ => {}
                }
                edges
            }
            Self::Array { items, prefix, .. } => prefix.iter().copied().chain(*items).collect(),
            Self::Union { branches, .. } => branches.clone(),
            Self::Json | Self::RefinedJson | Self::Literal(_) | Self::Never | Self::Scalar(_) => {
                Vec::new()
            }
        }
    }
}

/// One codec symbol. Object shapes additionally emit a native Models class.
#[derive(Debug, Clone)]
pub struct ModelSymbol {
    pub source: SchemaId,
    pub schema_index: usize,
    pub name: String,
    /// Allocated RBS type alias under the generated Types module.
    pub type_name: String,
    /// An in-place validation cycle cannot produce a completed valid value.
    /// RBS uses bottom; codecs retain distinct evaluation-failure semantics.
    pub signature_uninhabited: bool,
    pub description: String,
    pub shape: ModelShape,
}

#[derive(Debug)]
pub struct ModelPlan {
    symbols: BTreeMap<usize, ModelSymbol>,
}

impl ModelPlan {
    pub fn symbols(&self) -> impl Iterator<Item = &ModelSymbol> {
        self.symbols.values()
    }
    #[must_use]
    pub fn symbol(&self, schema_index: usize) -> Option<&ModelSymbol> {
        self.symbols.get(&schema_index)
    }
    #[must_use]
    pub fn source_symbol(&self, source: &SchemaId) -> Option<&ModelSymbol> {
        self.symbols
            .values()
            .find(|symbol| &symbol.source == source)
    }
    /// Resolve transparent reference/refinement carriers to their native shape.
    #[must_use]
    pub fn carrier(&self, mut index: usize) -> &ModelSymbol {
        // Alias cycles were rejected during planning.
        loop {
            let symbol = &self.symbols[&index];
            match symbol.shape {
                ModelShape::Alias(target) => index = target,
                _ => return symbol,
            }
        }
    }
}

pub(super) fn plan(
    contract: &Contract,
    program: &OwnedProgram,
    roots: &[usize],
    hints: BTreeMap<usize, String>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let sources: BTreeMap<_, _> = contract
        .schemas()
        .map(|schema| {
            (
                (
                    schema.id().document().to_string(),
                    schema.id().pointer().to_owned(),
                ),
                schema.id().clone(),
            )
        })
        .collect();
    let ids: Vec<_> = program
        .nodes
        .iter()
        .map(|node| sources[&(node.source.document.clone(), node.source.pointer.clone())].clone())
        .collect();
    let mut shapes = BTreeMap::new();
    let mut errors = Vec::new();
    let mut pending = roots.iter().copied().collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    while let Some(index) = pending.pop_first() {
        if !visited.insert(index) {
            continue;
        }
        match shape(contract, program, &ids, index) {
            Ok(shape) => {
                pending.extend(shape.edges());
                shapes.insert(index, shape);
            }
            Err(message) => errors.push(diagnostic(
                contract,
                ids[index].clone(),
                "ruby-model-representation",
                message,
            )),
        }
    }
    for &index in shapes.keys() {
        let mut at = index;
        let mut seen = BTreeSet::new();
        while let Some(ModelShape::Alias(target)) = shapes.get(&at) {
            if !seen.insert(at) {
                errors.push(diagnostic(
                    contract,
                    ids[index].clone(),
                    "ruby-model-nonproductive-cycle",
                    "reference-only native model cycles have no productive value representation",
                ));
                break;
            }
            at = *target;
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut names = reserved_constants();
    let mut type_names = reserved_members();
    type_names.extend(
        [
            "bool",
            "bot",
            "top",
            "untyped",
            "void",
            "instance",
            "singleton",
            "type",
            "interface",
            "out",
            "unchecked",
            "json_value",
            "headers",
        ]
        .map(str::to_owned),
    );
    let symbols = shapes
        .into_iter()
        .map(|(index, shape)| {
            let id = &ids[index];
            let base = source_name(index, &ids, &hints);
            let name = allocate(&base, &mut names);
            let type_name = allocate(&member(&name), &mut type_names);
            let description = contract
                .source(id)
                .and_then(|raw| raw.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            (
                index,
                ModelSymbol {
                    source: id.clone(),
                    schema_index: index,
                    name,
                    type_name,
                    signature_uninhabited: program.version != OwnedProgram::V3_VERSION
                        && in_place_cycle(program, index),
                    description,
                    shape,
                },
            )
        })
        .collect();
    Ok(ModelPlan { symbols })
}

fn in_place_cycle(program: &OwnedProgram, root: usize) -> bool {
    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut pending = vec![(root, false)];
    while let Some((index, leaving)) = pending.pop() {
        if leaving {
            active.remove(&index);
            complete.insert(index);
            continue;
        }
        if complete.contains(&index) {
            continue;
        }
        if !active.insert(index) {
            return true;
        }
        pending.push((index, true));
        for check in &program.nodes[index].checks {
            match &check.instruction {
                Op::Ref { target } | Op::Not { target } => pending.push((*target, false)),
                Op::AllOf { targets } | Op::AnyOf { targets } | Op::OneOf { targets } => {
                    pending.extend(targets.iter().map(|target| (*target, false)))
                }
                _ => {}
            }
        }
    }
    false
}

fn shape(
    contract: &Contract,
    program: &OwnedProgram,
    ids: &[SchemaId],
    index: usize,
) -> Result<ModelShape, String> {
    let checks = &program.nodes[index].checks;
    let scoped = program.version != OwnedProgram::V1_VERSION;
    if let Some((initial_target, initial_resource, anchor)) =
        checks.iter().find_map(|c| match &c.instruction {
            Op::DynamicRef {
                target,
                initial_resource,
                anchor,
            } => Some((*target, *initial_resource, anchor.clone())),
            _ => None,
        })
    {
        let candidates = program
            .resource_context
            .as_ref()
            .into_iter()
            .flat_map(|context| &context.resources)
            .flat_map(|r| &r.dynamic_anchors)
            .filter(|(name, _, _)| anchor.as_ref() == Some(name))
            .map(|(_, _, target)| *target)
            .collect();
        return Ok(ModelShape::Dynamic {
            initial_target,
            initial_resource,
            anchor,
            candidates,
        });
    }
    if checks
        .iter()
        .any(|c| matches!(c.instruction, Op::Always { value: false }))
    {
        return Ok(ModelShape::Never);
    }
    if checks
        .iter()
        .all(|c| matches!(c.instruction, Op::Always { value: true }))
    {
        return Ok(ModelShape::Json);
    }
    let mut types = None;
    let mut references = Vec::new();
    let mut unions = Vec::new();
    let mut intersections = Vec::new();
    let mut structural = false;
    for check in checks {
        match &check.instruction {
            Op::Type { types: declared } => types = Some(declared.clone()),
            Op::Ref { target } => references.push(*target),
            Op::OneOf { targets } => unions.push((targets.clone(), true)),
            Op::AnyOf { targets } => unions.push((targets.clone(), false)),
            Op::AllOf { targets } => intersections.extend(targets),
            Op::Properties { .. }
            | Op::AdditionalProperties { .. }
            | Op::Items { .. }
            | Op::PrefixItems { .. } => structural = true,
            _ => {}
        }
    }
    let combinations =
        usize::from(!references.is_empty()) + unions.len() + usize::from(!intersections.is_empty());
    if combinations > 1 || (combinations > 0 && structural) {
        if scoped {
            return Ok(ModelShape::RefinedJson);
        }
        return Err("combined structural carriers need a proved intersection representation; Ruby emission is blocked rather than discarding named fields or branch semantics".into());
    }
    if let Some(&target) = references.first() {
        return Ok(ModelShape::Alias(target));
    }
    if let Some((branches, exclusive)) = unions.pop() {
        // Context-free materialization trials could otherwise select a different
        // branch than the rooted resource-aware validator. Preserve exact JSON.
        if program.version == OwnedProgram::V3_VERSION {
            return Ok(ModelShape::RefinedJson);
        }
        return Ok(ModelShape::Union {
            branches,
            exclusive,
        });
    }
    if !intersections.is_empty() {
        let mut carriers = BTreeSet::new();
        for target in intersections {
            if let Some(carrier) = structural_carrier(program, target, &mut BTreeSet::new())? {
                carriers.insert(carrier);
            }
        }
        if carriers.len() > 1 {
            if scoped {
                return Ok(ModelShape::RefinedJson);
            }
            return Err("allOf has multiple distinct native carriers; merged object/intersection models are not implemented".into());
        }
        if let Some(target) = carriers.pop_first() {
            return Ok(ModelShape::Alias(target));
        }
    }
    let literals = checks.iter().find_map(|check| match &check.instruction {
        Op::Const { value } => Some(vec![value.clone()]),
        Op::Enum { values } => Some(values.clone()),
        _ => None,
    });
    // Structural declarations retain their field/collection carriers. A literal
    // alone is naturally represented by exact Ruby values, not invented fields.
    if !structural && let Some(values) = literals {
        return Ok(ModelShape::Literal(values));
    }
    let Some(types) = types.or_else(|| literal_types(program, index)) else {
        if structural {
            if scoped {
                return Ok(ModelShape::RefinedJson);
            }
            return Err(
                "structural assertions without an explicit kind need a native kind-dispatch model"
                    .into(),
            );
        }
        return Ok(ModelShape::RefinedJson);
    };
    let nullable = types.contains(&ProgramType::Null);
    let mut nonnull: Vec<_> = types
        .iter()
        .filter(|t| **t != ProgramType::Null)
        .copied()
        .collect();
    if nonnull.contains(&ProgramType::Number) {
        nonnull.retain(|t| *t != ProgramType::Integer);
    }
    if nonnull == [ProgramType::Object] {
        let properties = checks
            .iter()
            .find_map(|check| match &check.instruction {
                Op::Properties { properties } => Some(properties.as_slice()),
                _ => None,
            })
            .unwrap_or(&[]);
        let required: BTreeSet<_> = checks
            .iter()
            .filter_map(|c| match &c.instruction {
                Op::Required { names } => Some(names),
                _ => None,
            })
            .flatten()
            .cloned()
            .collect();
        if required
            .iter()
            .any(|name| !properties.iter().any(|property| &property.name == name))
        {
            if scoped {
                return Ok(ModelShape::RefinedJson);
            }
            return Err("required undeclared properties need an explicit native construction field; this representation is not implemented".into());
        }
        let mut members = reserved_members();
        let fields = properties
            .iter()
            .map(|property| {
                let source = ids[property.target].clone();
                let required = required.contains(&property.name);
                ModelField {
                    wire_name: property.name.clone(),
                    name: allocate(&member(&property.name), &mut members),
                    schema_index: property.target,
                    required,
                    description: contract
                        .source(&source)
                        .and_then(|raw| raw.get("description"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    source,
                    literal: required
                        .then(|| singleton_literal(program, property.target, &mut BTreeSet::new()))
                        .flatten(),
                }
            })
            .collect();
        let patterns = checks
            .iter()
            .find_map(|check| match &check.instruction {
                Op::PatternProperties { patterns } => Some(
                    patterns
                        .iter()
                        .map(|(pattern, _, target)| PatternExtra {
                            source: ids[*target].clone(),
                            pattern: pattern.clone(),
                            schema_index: *target,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let unevaluated = checks.iter().find_map(|check| match check.instruction {
            Op::UnevaluatedProperties { target } => Some(target),
            _ => None,
        });
        let scoped_extras =
            (!patterns.is_empty() || unevaluated.is_some()).then(|| ExtraFields::Scoped {
                patterns,
                additional: checks.iter().find_map(|check| match check.instruction {
                    Op::AdditionalProperties { target, .. }
                    | Op::AdditionalPropertiesWithPatterns { target, .. } => Some(target),
                    _ => None,
                }),
                unevaluated,
            });
        let extras =
            scoped_extras.unwrap_or_else(|| {
                checks
                    .iter()
                    .find_map(|check| match &check.instruction {
                        Op::AdditionalProperties { target, .. } => Some(
                            if program.nodes[*target].checks.iter().any(|check| {
                                matches!(check.instruction, Op::Always { value: false })
                            }) {
                                ExtraFields::Closed
                            } else {
                                ExtraFields::Typed(*target)
                            },
                        ),
                        _ => None,
                    })
                    .unwrap_or(ExtraFields::Json)
            });
        return Ok(ModelShape::Object {
            fields,
            extras,
            nullable,
        });
    }
    if nonnull == [ProgramType::Array] {
        let items = checks.iter().find_map(|check| match check.instruction {
            Op::Items { target, .. } => Some(target),
            _ => None,
        });
        let prefix = checks
            .iter()
            .find_map(|check| match &check.instruction {
                Op::PrefixItems { targets } => Some(targets.clone()),
                _ => None,
            })
            .unwrap_or_default();
        return Ok(ModelShape::Array {
            items,
            prefix,
            nullable,
        });
    }
    if types
        .iter()
        .any(|t| matches!(t, ProgramType::Array | ProgramType::Object))
    {
        if scoped {
            return Ok(ModelShape::RefinedJson);
        }
        return Err("type arrays combining structural and non-null scalar kinds need a native kind-dispatch model; that layer is not implemented".into());
    }
    Ok(ModelShape::Scalar(
        types
            .into_iter()
            .map(|ty| match ty {
                ProgramType::Null => ScalarKind::Null,
                ProgramType::Boolean => ScalarKind::Boolean,
                ProgramType::Integer => ScalarKind::Integer,
                ProgramType::Number => ScalarKind::Number,
                ProgramType::String => ScalarKind::String,
                _ => unreachable!("structural types checked"),
            })
            .collect(),
    ))
}

/// A refinement-only branch supplies no public storage. Every such branch is
/// still executed by the original allOf instruction at both codec boundaries.
fn structural_carrier(
    program: &OwnedProgram,
    index: usize,
    seen: &mut BTreeSet<usize>,
) -> Result<Option<usize>, String> {
    if !seen.insert(index) {
        return Err("nonproductive recursive composition has no native carrier".into());
    }
    let checks = &program.nodes[index].checks;
    let result = if checks.iter().any(|c| {
        matches!(
            c.instruction,
            Op::Type { .. }
                | Op::Properties { .. }
                | Op::AdditionalProperties { .. }
                | Op::Items { .. }
                | Op::PrefixItems { .. }
                | Op::AnyOf { .. }
                | Op::OneOf { .. }
        )
    }) {
        Some(index)
    } else {
        let mut carriers = BTreeSet::new();
        for check in checks {
            match &check.instruction {
                Op::Ref { target } => {
                    if let Some(carrier) = structural_carrier(program, *target, seen)? {
                        carriers.insert(carrier);
                    }
                }
                Op::AllOf { targets } => {
                    for target in targets {
                        if let Some(carrier) = structural_carrier(program, *target, seen)? {
                            carriers.insert(carrier);
                        }
                    }
                }
                _ => {}
            }
        }
        if carriers.len() > 1 {
            return Err("allOf has multiple distinct native carriers".into());
        }
        carriers.pop_first()
    };
    seen.remove(&index);
    Ok(result)
}

fn literal_types(program: &OwnedProgram, index: usize) -> Option<Vec<ProgramType>> {
    let values = program.nodes[index]
        .checks
        .iter()
        .find_map(|c| match &c.instruction {
            Op::Const { value } => Some(vec![value]),
            Op::Enum { values } => Some(values.iter().collect()),
            _ => None,
        })?;
    let mut types = Vec::new();
    for value in values {
        let ty = match value {
            Value::Null => ProgramType::Null,
            Value::Bool(_) => ProgramType::Boolean,
            Value::String(_) => ProgramType::String,
            Value::Number(_) => ProgramType::Number,
            _ => return None,
        };
        if !types.contains(&ty) {
            types.push(ty);
        }
    }
    Some(types)
}

fn singleton_literal(
    program: &OwnedProgram,
    index: usize,
    seen: &mut BTreeSet<usize>,
) -> Option<Value> {
    if !seen.insert(index) {
        return None;
    }
    program.nodes[index]
        .checks
        .iter()
        .find_map(|c| match &c.instruction {
            Op::Const { value } if !value.is_array() && !value.is_object() => Some(value.clone()),
            Op::Enum { values }
                if values.len() == 1 && !values[0].is_array() && !values[0].is_object() =>
            {
                Some(values[0].clone())
            }
            Op::Ref { target } => singleton_literal(program, *target, seen),
            _ => None,
        })
}

fn source_name(index: usize, ids: &[SchemaId], hints: &BTreeMap<usize, String>) -> String {
    let id = &ids[index];
    let (base, suffix) = if let Some(name) = hints.get(&index) {
        return name.clone();
    } else if let Some((parent, name)) = hints
        .iter()
        .filter(|(parent, _)| {
            ids[**parent].document() == id.document()
                && id
                    .pointer()
                    .starts_with(&format!("{}/", ids[**parent].pointer()))
        })
        .max_by_key(|(parent, _)| ids[**parent].pointer().len())
    {
        (
            name.clone(),
            id.pointer()[ids[*parent].pointer().len()..].to_owned(),
        )
    } else if let Some(suffix) = id.pointer().strip_prefix("/components/schemas/") {
        (String::new(), suffix.to_owned())
    } else {
        ("Schema".into(), id.pointer().to_owned())
    };
    let parts = suffix
        .split('/')
        .filter(|p| !p.is_empty() && !["properties", "schema"].contains(p))
        .map(|p| constant(&p.replace("~1", "/").replace("~0", "~")))
        .collect::<String>();
    constant(&(base + &parts))
}

pub(super) fn constant(value: &str) -> String {
    let mut name = String::new();
    let mut upper = true;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else if !c.is_ascii() {
            name.push_str(&format!("U{:X}", c as u32));
            upper = true;
        } else {
            upper = true;
        }
    }
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        name = format!("Schema{name}");
    }
    if name.len() > 100 {
        name = format!("{}{:x}", &name[..64], Sha256::digest(value.as_bytes()));
    }
    name
}

pub(super) fn member(value: &str) -> String {
    let chars: Vec<_> = value.chars().collect();
    let mut name = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase()
                && i > 0
                && (chars[i - 1].is_ascii_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars[i - 1].is_ascii_uppercase()
                        && chars.get(i + 1).is_some_and(char::is_ascii_lowercase))
                && !name.ends_with('_')
            {
                name.push('_');
            }
            name.push(c.to_ascii_lowercase());
        } else if !c.is_ascii() {
            name.push_str(&format!("_u{:x}_", c as u32));
        } else if !name.ends_with('_') {
            name.push('_');
        }
    }
    name = name.trim_matches('_').to_owned();
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name = format!("field_{name}");
    }
    if reserved_members().contains(&name) {
        name.push_str("_value");
    }
    if name.len() > 100 {
        name = format!("{}_{:x}", &name[..64], Sha256::digest(value.as_bytes()));
    }
    name
}

pub(super) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}

pub(super) fn reserved_members() -> BTreeSet<String> {
    [
        "alias",
        "and",
        "begin",
        "break",
        "case",
        "class",
        "def",
        "defined",
        "do",
        "else",
        "elsif",
        "end",
        "ensure",
        "false",
        "for",
        "if",
        "in",
        "module",
        "next",
        "nil",
        "not",
        "or",
        "redo",
        "rescue",
        "retry",
        "return",
        "self",
        "super",
        "then",
        "true",
        "undef",
        "unless",
        "until",
        "when",
        "while",
        "yield",
        "BEGIN",
        "END",
        "__FILE__",
        "__LINE__",
        "__ENCODING__",
        "initialize",
        "allocate",
        "new",
        "object_id",
        "send",
        "public_send",
        "__send__",
        "__id__",
        "method",
        "methods",
        "hash",
        "display",
        "clone",
        "dup",
        "freeze",
        "inspect",
        "to_s",
        "to_h",
        "to_json",
        "instance_eval",
        "instance_exec",
        "instance_variable_get",
        "instance_variable_set",
        "respond_to_missing",
        "extend",
        "tap",
        "then",
        "extra_fields",
        "schema_index",
        "validate",
        "validate_model",
        "codec",
        "method_missing",
        "auth",
        "server_url",
        "transport",
        "closed",
        "prepare_request",
        "call_operation",
        "consume_response",
        "response_context",
        "attach",
        "credentials",
        "open",
        "close",
        "raise",
        "fail",
        "lambda",
        "proc",
        "loop",
        "catch",
        "throw",
        "sleep",
        "format",
        "sprintf",
        "abort",
        "exit",
        "system",
        "exec",
        "fork",
        "p",
        "puts",
        "print",
        "printf",
        "warn",
        "require",
        "require_relative",
        "load",
        "eval",
        "autoload",
        "binding",
        "caller",
        "caller_locations",
        "block_given",
        "respond_to",
        "instance_variables",
        "instance_variable_defined",
        "instance_of",
        "is_a",
        "kind_of",
        "itself",
        "equal",
        "eql",
        "define_singleton_method",
        "singleton_class",
        "singleton_methods",
        "protected_methods",
        "private_methods",
        "public_methods",
        "initialize_copy",
        "initialize_dup",
        "initialize_clone",
        "frozen",
        "remove_instance_variable",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(super) fn reserved_constants() -> BTreeSet<String> {
    [
        "WireModel",
        "Bytes",
        "Part",
        "BytePart",
        "ItemStream",
        "NoContent",
        "NO_CONTENT",
        "Link",
        "BasicCredential",
        "AuthorizationCredential",
        "CredentialContext",
        "Quickstart",
        "Enumerator",
        "Queue",
        "SizedQueue",
        "OAuthFlow",
        "Object",
        "Class",
        "Module",
        "Kernel",
        "BasicObject",
        "Exception",
        "StandardError",
        "String",
        "Array",
        "Hash",
        "Integer",
        "Float",
        "Numeric",
        "NilClass",
        "TrueClass",
        "FalseClass",
        "Time",
        "File",
        "IO",
        "Thread",
        "Mutex",
        "JSON",
        "Net",
        "URI",
        "OpenSSL",
        "Set",
        "Struct",
        "Data",
        "Client",
        "Codecs",
        "Models",
        "Types",
        "Codec",
        "Model",
        "Internal",
        "Json",
        "JsonNumber",
        "Unset",
        "UNSET",
        "SdkError",
        "ApiError",
        "ApiResponse",
        "RequestError",
        "ResponseError",
        "TransportError",
        "ValidationError",
        "EvaluationFailure",
        "JsonError",
        "CodecError",
        "TimeoutError",
        "CancelledError",
        "CancellationToken",
        "ExchangeContext",
        "ResourceLimitError",
        "NetHTTPTransport",
        "PreparedRequest",
        "WireResponse",
        "Timeout",
        "Process",
        "Encoding",
        "ArgumentError",
        "RangeError",
        "VERSION",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
