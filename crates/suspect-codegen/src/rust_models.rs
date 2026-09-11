//! Canonical Contract → immutable Rust model/type/documentation plan.
//!
//! Native model planning consumes the canonical source-addressed contract.
//! It emits neutral model candidates, not schema codecs or an HTTP client.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, ContractSeverity, SchemaDialect, SchemaId};

use crate::OutFile;
use crate::schema_view;

// The generated dependency-free runtime is also used for exact source-number
// classification. Consumers exercise this same code through generated packages.
mod applicators;
pub(crate) mod resources;
#[allow(dead_code)]
pub(crate) mod runtime;

/// Whether a finding prevents emission, release, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// This source shape has no implemented faithful representation.
    Error,
    /// A source-aware codec must still enforce the contract before release.
    CodecObligation,
    /// Source annotation retained without inventing validation behavior.
    Annotation,
}

/// A source-linked model planning finding.
#[derive(Debug, Clone)]
pub struct ModelDiagnostic {
    /// Original source address.
    pub source: SchemaId,
    /// Original byte range, when available.
    pub at: Range<usize>,
    /// Stable finding identifier.
    pub code: &'static str,
    /// Effect on emission and release claims.
    pub kind: DiagnosticKind,
    /// Human-readable requirement or limitation.
    pub message: String,
}

/// Why a declaration exists for its source schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepresentationRole {
    /// The complete source model, including null when allowed.
    Model,
    /// Its non-null values, allowing optional presence without nested null.
    NonNullValue,
}

/// One immutable public declaration and documentation identity.
#[derive(Debug, Clone)]
pub struct ModelSymbol {
    source: SchemaId,
    name: String,
    role: RepresentationRole,
    pub(crate) code: String,
    pub(crate) description: String,
    pub(crate) source_json: String,
}
impl ModelSymbol {
    /// Canonical source identity shared with documentation.
    #[must_use]
    pub fn source(&self) -> &SchemaId {
        &self.source
    }
    /// Allocated, idiomatic Rust name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Complete model or its non-null representation.
    #[must_use]
    pub fn role(&self) -> RepresentationRole {
        self.role
    }
    /// Planned source artifact path.
    #[must_use]
    pub fn file(&self) -> &str {
        "rust/src/models.rs"
    }
}

/// An immutable symbol/type/docs snapshot. Rendering does no source inference.
#[derive(Debug)]
pub struct ModelPlan {
    symbols: Vec<ModelSymbol>,
    diagnostics: Vec<ModelDiagnostic>,
    openapi_version: String,
    pub(crate) declarations: BTreeMap<Key, Decl>,
}
impl ModelPlan {
    /// Every selected root, reference target and promoted inline declaration.
    #[must_use]
    pub fn symbols(&self) -> &[ModelSymbol] {
        &self.symbols
    }
    /// Original-source findings, including mandatory codec obligations.
    #[must_use]
    pub fn diagnostics(&self) -> &[ModelDiagnostic] {
        &self.diagnostics
    }
    /// Unsupported lowering blocks all artifact emission.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::Error)
    }
    /// Whether this plan can be promoted as a complete SDK.
    #[must_use]
    pub fn release_ready(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|d| d.kind == DiagnosticKind::Annotation)
    }
    /// Render a dependency-free Rust package and docs from this same plan.
    ///
    /// # Errors
    /// Returns the findings if unsupported representations prevent emission.
    /// Codec obligations permit reviewable artifacts but always block release.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        if self.has_errors() {
            return Err(self.diagnostics.clone());
        }
        let mut code = String::from(
            "//! Experimental neutral models. Schema codecs and HTTP are not implemented.\n\n",
        );
        let mut docs = format!(
            "# Rust contract models\n\nOpenAPI {}. Experimental neutral model candidates; schema codecs and HTTP are not implemented. Every codec obligation blocks SDK release.\n\nThe generated package uses Rust 2024, requires Rust 1.88 or later, and has no external dependencies. Run `cargo test` and `cargo doc --no-deps`.\n\nRequired nullable fields use `Nullable<T>`; optional non-null fields use `Option<T>`; optional nullable fields use `Presence<T>`. No default is inferred from an example or annotation. Constructors fill only source-defined singleton values. Neutral models retain readOnly/writeOnly annotations without applying request/response projection.\n\n`parse_json`, `parse_json_bytes`, and `stringify_json` provide a bounded exact JSON representation layer. Parsed numbers retain their original tokens, including negative zero and arbitrary exponents. Duplicate decoded object keys, malformed UTF-8 byte input, malformed escapes, and unpaired escaped UTF-16 surrogates are rejected. Rust strings represent Unicode scalar values, so this is not a claim that lone surrogate code points can be represented. `JsonLimits` bounds bytes, nesting, output, and explicit scanning/traversal work; it is not complete CPU or allocator accounting. Object output follows deterministic `BTreeMap` key order.\n\nNumber parsing checks only JSON numeric grammar and integer integrality. It scans and stores O(token length), without exponent expansion or floating point. Checked i128/u128 conversions return None out of range. Numeric equality and arithmetic are not implemented. JSON arrays and extra-field objects preserve the same exact-number representation.\n\nThe JSON helpers do not validate schemas, convert models, select union branches, or apply defaults. Native types preserve representable structure. They do not prove scalar constraints, object exactness, union exclusivity, defaults, read/write applicability or schema validation. No Serde derives are emitted; serialization through an unrelated library is not an implemented model codec. Unsupported shapes block artifacts instead of falling back to a dynamic type.\n\n",
            prose(&self.openapi_version)
        );
        for symbol in &self.symbols {
            let source = source_text(&symbol.source);
            code.push_str(&format!("/// Description: {}\n///\n/// Source: {}\n/// Role: {:?}. Source constraints remain codec obligations.\n", prose(&symbol.description), prose(&source), symbol.role));
            code.push_str(&symbol.code);
            code.push_str("\n\n");
            docs.push_str(&format!("## `{}`\n\n[Code](src/models.rs). Role: `{:?}`.\n\n{}\n\nSource: `{}`\n\n```rust,ignore\n{}\n```\n\nOriginal schema:\n\n```json\n{}\n```\n\n", symbol.name, symbol.role, prose(&symbol.description), prose(&source), symbol.code, symbol.source_json));
        }
        docs.push_str("## Findings and codec obligations\n\n");
        for diagnostic in &self.diagnostics {
            docs.push_str(&format!(
                "- `{:?}` / `{}` at `{}`: {}\n",
                diagnostic.kind,
                diagnostic.code,
                prose(&source_text(&diagnostic.source)),
                prose(&diagnostic.message)
            ));
        }
        Ok(vec![
            OutFile { path: "rust/Cargo.toml".into(), content: "[package]\nname = \"generated-models\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.88\"\npublish = false\n\n[workspace]\n".into() },
            OutFile { path: "rust/src/lib.rs".into(), content: "//! Experimental OpenAPI models plus an exact JSON representation runtime.\n//!\n//! The JSON helpers preserve representation and enforce resource limits. They do\n//! not validate source schemas or encode/decode the generated model types.\n#![forbid(unsafe_code)]\npub mod models;\nmod support;\nmod json;\npub use support::{ExtraFieldError, JsonInteger, JsonNonNullValue, JsonNumber, JsonValue, Never, Nullable, NumberError, Presence};\npub use json::{JsonError, JsonErrorKind, JsonLimits, parse_json, parse_json_bytes, stringify_json};\n".into() },
            OutFile { path: "rust/src/models.rs".into(), content: code },
            OutFile { path: "rust/src/support.rs".into(), content: include_str!("rust_models/runtime.rs").into() },
            OutFile { path: "rust/src/json.rs".into(), content: include_str!("rust_codecs/json_runtime.rs").into() },
            OutFile { path: "rust/README.md".into(), content: docs },
        ])
    }
}

pub(crate) type Key = (SchemaId, RepresentationRole);

/// Plan selected schema closures directly from their canonical source identities.
/// Unknown roots and unsupported shapes are ordinary source-linked errors.
#[must_use]
pub fn plan_models(contract: &Contract, roots: &[SchemaId]) -> ModelPlan {
    plan_models_with_profile(contract, roots, false, false)
}

/// Plan native carriers for the additive v2 scoped-applicator profile.
/// Conditional requirements and pattern/unevaluated constraints remain bound
/// codec obligations; they are never flattened into unconditional fields.
#[must_use]
pub fn plan_models_v2(contract: &Contract, roots: &[SchemaId]) -> ModelPlan {
    plan_models_with_profile(contract, roots, true, false)
}

/// Plan resource-aware carriers without selecting a dynamic fallback as a
/// static Rust type. Base closures retain their established v2/v1 declarations.
#[must_use]
pub fn plan_models_v3(contract: &Contract, roots: &[SchemaId]) -> ModelPlan {
    if resources::required(contract, roots) {
        plan_models_with_profile(contract, roots, true, true)
    } else {
        plan_models_v2(contract, roots)
    }
}

fn plan_models_with_profile(
    contract: &Contract,
    roots: &[SchemaId],
    applicators: bool,
    resources: bool,
) -> ModelPlan {
    let reachable = schema_view::closure(contract, roots);
    let transparent: BTreeMap<_, _> = reachable
        .iter()
        .filter_map(|id| transparent_all_of(contract, id).map(|carrier| (id.clone(), carrier)))
        .collect();
    let referenced: BTreeSet<_> = reachable
        .iter()
        .filter_map(|id| contract.schema(id))
        .flat_map(|schema| schema.references())
        .filter_map(|reference| reference.target.clone())
        .collect();
    let overlays: BTreeSet<_> = transparent
        .keys()
        .flat_map(|id| {
            let count = contract
                .schema(id)
                .and_then(|s| s.raw().get("allOf"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (1..count)
                .map(|index| id.child("allOf").child(&index.to_string()))
                .collect::<Vec<_>>()
        })
        .filter(|id| !roots.contains(id) && !referenced.contains(id))
        .collect();
    let mut planner = Planner {
        contract,
        names: BTreeMap::new(),
        nullable: BTreeMap::new(),
        diagnostics: Vec::new(),
        transparent,
        applicators,
        resources,
        dynamic: if resources {
            self::resources::dynamic_dependents(contract, &reachable)
        } else {
            BTreeSet::new()
        },
    };
    for root in roots {
        if contract.schema(root).is_none() {
            planner.report(
                root,
                "unknown-model-root",
                DiagnosticKind::Error,
                "selected root is not an indexed schema",
            );
        }
        planner.report(root, "model-codec-unimplemented", DiagnosticKind::CodecObligation, "this root has no source-aware encoder/decoder; JSON validity, exact numeric tokens, constraints, absence/null, object keys and union exclusivity must be validated before release");
    }
    for diagnostic in contract.diagnostics() {
        if schema_view::diagnostic_applies(contract, &reachable, diagnostic) {
            planner.diagnostics.push(ModelDiagnostic {
                source: diagnostic.source.clone(),
                at: diagnostic.at.clone(),
                code: diagnostic.code,
                kind: match diagnostic.severity {
                    ContractSeverity::Error => DiagnosticKind::Error,
                    ContractSeverity::Warning => DiagnosticKind::Annotation,
                },
                message: diagnostic.message.clone(),
            });
        }
    }
    for problem in schema_view::problems(contract, &reachable) {
        planner.report(
            &problem.source,
            problem.code,
            DiagnosticKind::Error,
            problem.message,
        );
    }
    let mut declared: BTreeSet<_> = roots
        .iter()
        .filter(|id| contract.schema(id).is_some())
        .cloned()
        .collect();
    for id in &reachable {
        let schema = contract.schema(id).expect("reachable schema");
        if !overlays.contains(id) && schema_view::raw(schema).as_object().is_some_and(promote) {
            declared.insert(id.clone());
        }
        for reference in schema.references() {
            if let Some(target) = &reference.target {
                declared.insert(target.clone());
            }
        }
        let nullability = if resources {
            self::resources::null_allowed(contract, id, planner.dynamic.contains(id))
        } else if applicators {
            applicators::null_allowed(contract, id)
        } else {
            schema_view::null_allowed(contract, id)
        };
        let nullable = nullability.unwrap_or_else(|problem| {
            planner.report(
                &problem.source,
                problem.code,
                DiagnosticKind::Error,
                problem.message,
            );
            false
        });
        planner.nullable.insert(id.clone(), nullable);
        if !overlays.contains(id) {
            planner.check_shape(id);
        }
    }
    planner.names = allocate_names(contract, &declared, &planner.nullable);
    let mut declarations = BTreeMap::new();
    for id in &declared {
        let key = planner.core_key(id);
        declarations.insert(key.clone(), planner.lower_decl(id));
        if planner.is_nullable(id) {
            declarations.insert(
                (id.clone(), RepresentationRole::Model),
                Decl::Alias(Type::Nullable(Box::new(Type::Named(key)))),
            );
        }
    }
    reject_alias_cycles(&declarations, &mut planner);
    box_layout_cycles(&mut declarations);
    let mut symbols = Vec::new();
    for (key, declaration) in &declarations {
        let raw = contract.schema(&key.0).expect("declared schema").raw();
        let name = planner.names[key].clone();
        symbols.push(ModelSymbol {
            source: key.0.clone(),
            role: key.1,
            code: declaration.render(&name, &planner.names),
            name,
            description: schema_view::description(
                contract.schema(&key.0).expect("declared schema"),
            )
            .into(),
            source_json: serde_json::to_string_pretty(raw).expect("schema JSON"),
        });
    }
    ModelPlan {
        symbols,
        diagnostics: planner.diagnostics,
        openapi_version: contract.openapi_version().into(),
        declarations,
    }
}

struct Planner<'a> {
    contract: &'a Contract,
    names: BTreeMap<Key, String>,
    nullable: BTreeMap<SchemaId, bool>,
    diagnostics: Vec<ModelDiagnostic>,
    transparent: BTreeMap<SchemaId, SchemaId>,
    applicators: bool,
    resources: bool,
    dynamic: BTreeSet<SchemaId>,
}

#[derive(Debug, Clone)]
pub(crate) enum Type {
    Primitive(&'static str),
    Named(Key),
    Nullable(Box<Type>),
    Optional(Box<Type>),
    Presence(Box<Type>),
    Vec(Box<Type>),
    Map(Box<Type>),
    Boxed(Box<Type>),
}
impl Type {
    pub(crate) fn render(&self, names: &BTreeMap<Key, String>) -> String {
        match self {
            Self::Primitive(value) => (*value).into(),
            Self::Named(key) => names[key].clone(),
            Self::Nullable(inner) => format!("crate::Nullable<{}>", inner.render(names)),
            Self::Optional(inner) => format!("std::option::Option<{}>", inner.render(names)),
            Self::Presence(inner) => format!("crate::Presence<{}>", inner.render(names)),
            Self::Vec(inner) => format!("std::vec::Vec<{}>", inner.render(names)),
            Self::Map(inner) => format!(
                "std::collections::BTreeMap<std::string::String, {}>",
                inner.render(names)
            ),
            Self::Boxed(inner) => format!("std::boxed::Box<{}>", inner.render(names)),
        }
    }
    fn references(&self, guarded: bool, result: &mut BTreeSet<Key>) {
        match self {
            Self::Named(key) => {
                result.insert(key.clone());
            }
            Self::Nullable(inner) | Self::Optional(inner) | Self::Presence(inner) => {
                inner.references(guarded, result)
            }
            Self::Vec(inner) | Self::Map(inner) | Self::Boxed(inner) if !guarded => {
                inner.references(false, result)
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub(crate) enum Decl {
    Alias(Type),
    Struct {
        fields: Vec<Field>,
        extras: Option<(String, Type)>,
    },
    Enum(Vec<Variant>),
    Literals(Vec<Literal>),
}
#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) wire: String,
    pub(crate) source: SchemaId,
    description: String,
    pub(crate) ty: Type,
    pub(crate) init: Option<Init>,
}
#[derive(Debug)]
pub(crate) enum Init {
    Absent,
    Optional,
    Literal(Key, String),
}
#[derive(Debug)]
pub(crate) struct Variant {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) source: SchemaId,
}
#[derive(Debug)]
pub(crate) struct Literal {
    pub(crate) name: String,
    pub(crate) json: String,
    pub(crate) value: Value,
}

impl Decl {
    fn types(&self) -> Vec<&Type> {
        match self {
            Self::Alias(ty) => vec![ty],
            Self::Struct { fields, .. } => fields.iter().map(|f| &f.ty).collect(),
            Self::Enum(variants) => variants.iter().map(|v| &v.ty).collect(),
            Self::Literals(_) => vec![],
        }
    }
    fn render(&self, name: &str, names: &BTreeMap<Key, String>) -> String {
        match self {
            Self::Alias(ty) => format!("pub type {name} = {};", ty.render(names)),
            Self::Literals(values) => {
                let mut code =
                    format!("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum {name} {{\n");
                for value in values {
                    code.push_str(&format!(
                        "    /// Exact JSON literal: {}\n    {},\n",
                        prose(&value.json),
                        value.name
                    ));
                }
                code.push_str("}\n");
                code.push_str(&format!("impl {name} {{\n    /// The source-defined JSON literal; this accessor is not a schema codec.\n    #[must_use]\n    pub fn wire_json(&self) -> &'static str {{\n        match *self {{\n"));
                for value in values {
                    code.push_str(&format!(
                        "            Self::{} => {},\n",
                        value.name,
                        quote(&value.json)
                    ));
                }
                code.push_str("        }\n    }\n}");
                code
            }
            Self::Enum(variants) => {
                let mut code = format!("#[derive(Debug, Clone)]\npub enum {name} {{\n");
                for variant in variants {
                    code.push_str(&format!("    /// Declared member: {}. Use its source-aware codec to enforce membership and exclusivity.\n    {}({}),\n", prose(&source_text(&variant.source)), variant.name, variant.ty.render(names)));
                }
                code.push('}');
                code
            }
            Self::Struct { fields, extras } => {
                let mut code = format!("#[derive(Debug, Clone)]\npub struct {name} {{\n");
                for field in fields {
                    code.push_str(&format!(
                        "    /// Description: {} Wire name: {}. Source: {}\n    pub {}: {},\n",
                        prose(&field.description),
                        prose(&quote(&field.wire)),
                        prose(&source_text(&field.source)),
                        field.name,
                        field.ty.render(names)
                    ));
                }
                if let Some((extras, ty)) = extras {
                    code.push_str(&format!("    pub(crate) {extras}: std::collections::BTreeMap<std::string::String, {}>,\n", ty.render(names)));
                }
                // Closed structs also have a private field: consumers cannot bypass
                // source-derived singleton initialization using a struct literal.
                if extras.is_none() {
                    code.push_str("    pub(crate) _construction: (),\n");
                }
                code.push_str("}\n");
                let args = fields
                    .iter()
                    .filter(|f| f.init.is_none())
                    .map(|f| format!("{}: {}", f.name, f.ty.render(names)))
                    .collect::<Vec<_>>()
                    .join(", ");
                code.push_str(&format!("impl {name} {{\n    /// Construct required fields; optional fields start absent and singleton values follow the source.\n    #[must_use]\n    pub fn new({args}) -> Self {{\n        Self {{\n"));
                for field in fields {
                    let init = match &field.init {
                        None => field.name.clone(),
                        Some(Init::Absent) => "crate::Presence::Absent".into(),
                        Some(Init::Optional) => "std::option::Option::None".into(),
                        Some(Init::Literal(key, variant)) => format!("{}::{variant}", names[key]),
                    };
                    if init == field.name {
                        code.push_str(&format!("            {init},\n"));
                    } else {
                        code.push_str(&format!("            {}: {init},\n", field.name));
                    }
                }
                if let Some((extras, _)) = extras {
                    code.push_str(&format!(
                        "            {extras}: std::collections::BTreeMap::new(),\n"
                    ));
                } else {
                    code.push_str("            _construction: (),\n");
                }
                code.push_str("        }\n    }\n");
                if let Some((extras, ty)) = extras {
                    let ty = ty.render(names);
                    code.push_str(&format!("    /// Insert an undeclared wire property. Declared names cannot be shadowed.\n    ///\n    /// # Errors\n    /// Returns the conflicting key when it is declared by this model.\n    pub fn insert_extra(&mut self, key: std::string::String, value: {ty}) -> std::result::Result<std::option::Option<{ty}>, crate::ExtraFieldError> {{\n"));
                    if !fields.is_empty() {
                        let keys = fields
                            .iter()
                            .map(|f| quote(&f.wire))
                            .collect::<Vec<_>>()
                            .join(" | ");
                        code.push_str(&format!("        if matches!(key.as_str(), {keys}) {{\n            return std::result::Result::Err(crate::ExtraFieldError(key));\n        }}\n"));
                    }
                    code.push_str(&format!("        std::result::Result::Ok(self.{extras}.insert(key, value))\n    }}\n    /// Iterate undeclared wire fields without exposing unchecked mutation.\n    pub fn extra_fields(&self) -> impl std::iter::Iterator<Item = (&str, &{ty})> {{\n        self.{extras}.iter().map(|(key, value)| (key.as_str(), value))\n    }}\n"));
                }
                code.push('}');
                if args.is_empty() {
                    code.push_str(&format!(
                        "\nimpl std::default::Default for {name} {{\n    fn default() -> Self {{\n        Self::new()\n    }}\n}}"
                    ));
                }
                code
            }
        }
    }
}

impl Planner<'_> {
    fn report(&mut self, id: &SchemaId, code: &'static str, kind: DiagnosticKind, message: &str) {
        self.diagnostics.push(ModelDiagnostic {
            source: id.clone(),
            at: self.contract.source_span(id).unwrap_or(0..0),
            code,
            kind,
            message: message.into(),
        });
    }
    fn is_nullable(&self, id: &SchemaId) -> bool {
        self.nullable.get(id).copied().unwrap_or(false)
    }
    fn core_key(&self, id: &SchemaId) -> Key {
        (
            id.clone(),
            if self.is_nullable(id) {
                RepresentationRole::NonNullValue
            } else {
                RepresentationRole::Model
            },
        )
    }
    fn full_type(&mut self, id: &SchemaId) -> Type {
        if self
            .names
            .contains_key(&(id.clone(), RepresentationRole::Model))
        {
            return Type::Named((id.clone(), RepresentationRole::Model));
        }
        let core = self.core_type(id);
        if self.is_nullable(id) {
            Type::Nullable(Box::new(core))
        } else {
            core
        }
    }
    fn core_type(&mut self, id: &SchemaId) -> Type {
        let key = self.core_key(id);
        if self.names.contains_key(&key) {
            return Type::Named(key);
        }
        match self.lower_decl(id) {
            Decl::Alias(ty) => ty,
            _ => {
                self.report(
                    id,
                    "unplanned-declaration",
                    DiagnosticKind::Error,
                    "this inline representation requires a planned declaration",
                );
                Type::Primitive("crate::Never")
            }
        }
    }
    fn check_shape(&mut self, id: &SchemaId) {
        let schema = self.contract.schema(id).expect("reachable schema");
        let value = schema_view::raw(schema);
        let Some(raw) = value.as_object() else {
            return;
        };
        if raw.get("const").is_some_and(Value::is_number) && raw.contains_key("enum") {
            self.report(id, "numeric-literal-intersection", DiagnosticKind::Error, "numeric const/enum intersections require exact mathematical equality, not JSON token equality; that lowering is not implemented");
        }
        for keyword in [
            "allOf",
            "not",
            "if",
            "then",
            "else",
            "dependentSchemas",
            "dependentRequired",
            "patternProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "prefixItems",
            "contains",
            "propertyNames",
            "$dynamicRef",
        ] {
            if keyword == "allOf" && self.transparent.contains_key(id) {
                continue;
            }
            if raw.contains_key(keyword) {
                if self.resources && keyword == "$dynamicRef" {
                    self.report(&id.child(keyword),"v3-dynamic-carrier",DiagnosticKind::CodecObligation,"dynamic references retain an exact JSON carrier; runtime resource scope selects the binding rather than a generation-time fallback");
                    continue;
                }
                if self.applicators && keyword != "$dynamicRef" {
                    self.report(&id.child(keyword), "v2-codec-obligation", DiagnosticKind::CodecObligation,
                        &format!("{keyword} is retained as a source-bound v2 assertion; the native carrier does not replace its scoped validation"));
                    continue;
                }
                self.report(
                    &id.child(keyword),
                    "unsupported-rust-representation",
                    DiagnosticKind::Error,
                    &format!("{keyword} lowering is not implemented for Rust models"),
                );
            }
        }
        if raw.contains_key("$ref") {
            let assertions = raw
                .keys()
                .filter(|key| {
                    !annotation(key) && key.as_str() != "$ref" && key.as_str() != "nullable"
                })
                .collect::<Vec<_>>();
            if !assertions.is_empty() && !self.applicators {
                self.report(
                    id,
                    "ref-sibling-representation",
                    DiagnosticKind::Error,
                    "reference assertion siblings require an intersection representation",
                );
            }
        }
        let union = raw.contains_key("oneOf") || raw.contains_key("anyOf");
        if !self.applicators
            && ((raw.contains_key("oneOf") && raw.contains_key("anyOf"))
                || (union
                    && [
                        "type",
                        "enum",
                        "const",
                        "properties",
                        "additionalProperties",
                        "items",
                    ]
                    .iter()
                    .any(|key| raw.contains_key(*key))))
        {
            self.report(
                id,
                "union-intersection-representation",
                DiagnosticKind::Error,
                "union assertion siblings require an intersection representation",
            );
        }
        if raw.get("type").is_none()
            && !self.applicators
            && !self.transparent.contains_key(id)
            && !raw.contains_key("$ref")
            && !union
            && !raw.contains_key("enum")
            && !raw.contains_key("const")
            && raw.keys().any(|key| !annotation(key))
        {
            self.report(id, "untyped-constraints-representation", DiagnosticKind::Error, "untyped constraints do not imply an object or scalar type; conditional domain lowering is not implemented");
        }
        for keyword in [
            "default",
            "examples",
            "example",
            "readOnly",
            "writeOnly",
            "deprecated",
            "discriminator",
            "format",
        ] {
            if raw.contains_key(keyword) {
                self.report(&id.child(keyword), "retained-annotation", DiagnosticKind::Annotation, &format!("{keyword} is retained in source documentation; neutral model planning does not invent validation or directional policy"));
            }
        }
        if matches!(schema.dialect(), SchemaDialect::OpenApi30)
            && raw.get("nullable") == Some(&Value::Bool(true))
            && !raw.contains_key("type")
        {
            self.report(&id.child("nullable"), "nullable-without-type", DiagnosticKind::Annotation, "OpenAPI 3.0 nullable only changes a type explicitly declared on the same Schema Object");
        }
    }
    fn lower_decl(&mut self, id: &SchemaId) -> Decl {
        let Some(schema) = self.contract.schema(id) else {
            return Decl::Alias(Type::Primitive("crate::Never"));
        };
        let value = schema_view::raw(schema);
        match &*value {
            Value::Bool(false) => return Decl::Alias(Type::Primitive("crate::Never")),
            Value::Bool(true) => return Decl::Alias(Type::Primitive("crate::JsonNonNullValue")),
            _ => {}
        }
        let Some(raw) = value.as_object() else {
            self.report(
                id,
                "invalid-schema",
                DiagnosticKind::Error,
                "schema is neither an object nor Boolean",
            );
            return Decl::Alias(Type::Primitive("crate::Never"));
        };
        if self.resources
            && (raw.contains_key("$dynamicRef")
                || self.dynamic.contains(id)
                    && (raw.contains_key("anyOf") || raw.contains_key("oneOf")))
        {
            // Converter-side union trials have no caller resource stack. A
            // context-sensitive value must be preserved until whole-root v3
            // validation, not narrowed by standalone fallback/branch checks.
            return Decl::Alias(Type::Primitive("crate::JsonNonNullValue"));
        }
        if let Some(carrier) = self.transparent.get(id).cloned() {
            let strengthened: BTreeSet<_> = raw
                .get("allOf")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .skip(1)
                .flat_map(|member| {
                    member
                        .get("required")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            if !strengthened.is_empty() {
                let mut target = carrier.clone();
                while let Some(next) = self
                    .contract
                    .schema(&target)
                    .and_then(|schema| schema.references().iter().find(|r| r.keyword == "$ref"))
                    .and_then(|r| r.target.clone())
                {
                    target = next;
                }
                let mut object = self
                    .contract
                    .schema(&target)
                    .expect("proved carrier")
                    .raw()
                    .as_object()
                    .expect("proved object")
                    .clone();
                let mut required: BTreeSet<_> = object
                    .get("required")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                required.extend(strengthened);
                object.insert(
                    "required".into(),
                    Value::Array(required.into_iter().map(Value::String).collect()),
                );
                return self.object_decl(&target, &object);
            }
            return Decl::Alias(self.core_type(&carrier));
        }
        if raw.contains_key("$ref") {
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone());
            return Decl::Alias(match target {
                Some(target) => self.core_type(&target),
                None => {
                    self.report(
                        &id.child("$ref"),
                        "unresolved-model-reference",
                        DiagnosticKind::Error,
                        "reference has no canonical target",
                    );
                    Type::Primitive("crate::Never")
                }
            });
        }
        if let Some(literals) = scalar_literals(raw) {
            if literals.iter().any(|v| v.is_object() || v.is_array()) {
                if self.applicators {
                    // The codec retains the complete literal constraint. A
                    // structural JSON carrier preserves every admitted value.
                    return Decl::Alias(Type::Primitive("crate::JsonNonNullValue"));
                }
                self.report(
                    id,
                    "compound-literal-representation",
                    DiagnosticKind::Error,
                    "object and array enum/const representations are not implemented",
                );
            }
            return Decl::Literals(literal_variants(
                literals
                    .into_iter()
                    .filter(|v| !v.is_null() && accepts_type(raw, v))
                    .collect(),
            ));
        }
        if let Some((keyword, members)) = ["oneOf", "anyOf"]
            .into_iter()
            .find_map(|k| raw.get(k).and_then(Value::as_array).map(|a| (k, a)))
        {
            let mut used = BTreeSet::new();
            let mut variants = Vec::new();
            for (index, member) in members.iter().enumerate() {
                let child = id.child(keyword).child(&index.to_string());
                let branch =
                    schema_view::raw(self.contract.schema(&child).expect("indexed branch"));
                if only_null(&branch) || *branch == Value::Bool(false) {
                    continue;
                }
                let label = branch_label(raw, member, index);
                variants.push(Variant {
                    name: allocate_variant(&pascal(&label), &mut used),
                    ty: self.core_type(&child),
                    source: child,
                });
            }
            return Decl::Enum(variants);
        }
        let mut types = schema_types(raw);
        types.retain(|t| t != "null");
        if types.contains(&"number".into()) {
            types.retain(|t| t != "integer");
        }
        if types.len() > 1 {
            if self.applicators {
                return Decl::Alias(Type::Primitive("crate::JsonNonNullValue"));
            }
            self.report(
                &id.child("type"),
                "type-union-representation",
                DiagnosticKind::Error,
                "multiple non-null type alternatives need an implemented Rust union representation",
            );
            return Decl::Alias(Type::Primitive("crate::Never"));
        }
        let ty = match types.first().map(String::as_str) {
            Some("object") => return self.object_decl(id, raw),
            Some("array") => Type::Vec(Box::new(
                if self.applicators && raw.contains_key("prefixItems") {
                    // items constrains only the tail; applying its native type to
                    // every positional element would exclude valid prefix values.
                    Type::Primitive("crate::JsonValue")
                } else if raw.contains_key("items") {
                    self.full_type(&id.child("items"))
                } else {
                    Type::Primitive("crate::JsonValue")
                },
            )),
            Some("string") => Type::Primitive("std::string::String"),
            Some("boolean") => Type::Primitive("bool"),
            Some("integer") => {
                Type::Primitive(native_integer_for(schema).unwrap_or("crate::JsonInteger"))
            }
            Some("number") => Type::Primitive("crate::JsonNumber"),
            None if raw.contains_key("type") => Type::Primitive("crate::Never"),
            None => Type::Primitive("crate::JsonNonNullValue"),
            _ => {
                self.report(
                    &id.child("type"),
                    "invalid-model-type",
                    DiagnosticKind::Error,
                    "unsupported or invalid schema type",
                );
                Type::Primitive("crate::Never")
            }
        };
        Decl::Alias(ty)
    }
    fn object_decl(&mut self, id: &SchemaId, raw: &Map<String, Value>) -> Decl {
        let properties = raw.get("properties").and_then(Value::as_object);
        let patterned = self.applicators
            && raw
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|p| !p.is_empty());
        let required: BTreeSet<_> = raw
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for name in &required {
            if !properties.is_some_and(|p| p.contains_key(*name)) && !self.applicators {
                self.report(
                    &id.child("required"),
                    "undeclared-required-field",
                    DiagnosticKind::Error,
                    "required undeclared keys need a dedicated construction representation",
                );
            }
        }
        if properties.is_none_or(Map::is_empty) && patterned {
            return Decl::Alias(Type::Map(Box::new(Type::Primitive("crate::JsonValue"))));
        }
        if properties.is_none_or(Map::is_empty)
            && raw
                .get("additionalProperties")
                .is_some_and(Value::is_object)
        {
            return Decl::Alias(Type::Map(Box::new(
                self.full_type(&id.child("additionalProperties")),
            )));
        }
        let mut fields = Vec::new();
        let mut used = BTreeSet::from(["_construction".to_owned()]);
        for (wire, _) in properties.into_iter().flatten() {
            let source = id.child("properties").child(wire);
            let description = self
                .contract
                .schema(&source)
                .map(schema_view::description)
                .unwrap_or("")
                .to_owned();
            let core = self.core_type(&source);
            let nullable = self.is_nullable(&source);
            let (ty, init) = if required.contains(wire.as_str()) {
                let init = self.singleton(&source);
                (
                    if nullable {
                        Type::Nullable(Box::new(core))
                    } else {
                        core
                    },
                    init,
                )
            } else if nullable {
                (Type::Presence(Box::new(core)), Some(Init::Absent))
            } else {
                (Type::Optional(Box::new(core)), Some(Init::Optional))
            };
            fields.push(Field {
                name: allocate_local(&snake(wire), &mut used),
                wire: wire.clone(),
                source,
                description,
                ty,
                init,
            });
        }
        let extras = (patterned || raw.get("additionalProperties") != Some(&Value::Bool(false)))
            .then(|| {
                let ty = if !patterned
                    && raw
                        .get("additionalProperties")
                        .is_some_and(Value::is_object)
                {
                    self.full_type(&id.child("additionalProperties"))
                } else {
                    Type::Primitive("crate::JsonValue")
                };
                (allocate_local("_extra_fields", &mut used), ty)
            });
        Decl::Struct { fields, extras }
    }
    fn singleton(&self, id: &SchemaId) -> Option<Init> {
        let mut source = id.clone();
        let mut visited = BTreeSet::new();
        while visited.insert(source.clone()) {
            if self.is_nullable(&source) {
                return None;
            }
            let schema = self.contract.schema(&source)?;
            if let Some(target) = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone())
            {
                source = target;
                continue;
            }
            let raw = schema.raw().as_object()?;
            let values = scalar_literals(raw)?
                .into_iter()
                .filter(|v| !v.is_null() && accepts_type(raw, v))
                .collect::<Vec<_>>();
            let variants = literal_variants(values);
            if variants.len() != 1 {
                return None;
            }
            return Some(Init::Literal(
                self.core_key(&source),
                variants[0].name.clone(),
            ));
        }
        None
    }
}

fn promote(raw: &Map<String, Value>) -> bool {
    schema_types(raw).iter().any(|t| t == "object")
        || ["oneOf", "anyOf", "allOf", "enum", "const"]
            .iter()
            .any(|key| raw.contains_key(*key))
}
fn schema_types(raw: &Map<String, Value>) -> Vec<String> {
    match raw.get("type") {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => vec![],
    }
}
fn annotation(key: &str) -> bool {
    key.starts_with("x-")
        || matches!(
            key,
            "$id"
                | "$schema"
                | "$anchor"
                | "$defs"
                | "definitions"
                | "$comment"
                | "title"
                | "description"
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
fn scalar_literals(raw: &Map<String, Value>) -> Option<Vec<&Value>> {
    if let Some(value) = raw.get("const") {
        if raw
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.contains(value))
        {
            return Some(vec![]);
        }
        return Some(vec![value]);
    }
    raw.get("enum")
        .and_then(Value::as_array)
        .map(|values| values.iter().collect())
}
fn accepts_type(raw: &Map<String, Value>, value: &Value) -> bool {
    let types = schema_types(raw);
    types.is_empty()
        || types.iter().any(|t| match t.as_str() {
            "null" => value.is_null(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            "number" => value.is_number(),
            "integer" => value
                .as_number()
                .is_some_and(|n| n.to_string().parse::<runtime::JsonInteger>().is_ok()),
            _ => false,
        })
}
// A reference carrier may retain redundant overlays or strengthen presence of
// already named object fields. Other intersections require a different proof.
fn transparent_all_of(contract: &Contract, id: &SchemaId) -> Option<SchemaId> {
    let value = schema_view::raw(contract.schema(id)?);
    let raw = value.as_object()?;
    if raw.keys().any(|key| key != "allOf" && !annotation(key)) {
        return None;
    }
    let members = raw.get("allOf")?.as_array()?;
    let carrier = id.child("allOf").child("0");
    members.first()?;
    let value = schema_view::raw(contract.schema(&carrier)?);
    let first = value.as_object()?;
    if !first.contains_key("$ref") || first.keys().any(|key| key != "$ref" && !annotation(key)) {
        return None;
    }
    let mut target = contract
        .schema(&carrier)?
        .references()
        .iter()
        .find(|r| r.keyword == "$ref")?
        .target
        .clone()?;
    let mut seen = BTreeSet::new();
    let base = loop {
        if !seen.insert(target.clone()) {
            return None;
        }
        let schema = contract.schema(&target)?;
        if let Some(next) = schema
            .references()
            .iter()
            .find(|r| r.keyword == "$ref")
            .and_then(|r| r.target.as_ref())
        {
            target = next.clone();
        } else {
            break schema.raw().as_object()?;
        }
    };
    for member in &members[1..] {
        let overlay = member.as_object()?;
        for (key, value) in overlay {
            if annotation(key) {
                continue;
            }
            match key.as_str() {
                "properties" if value.as_object().is_some_and(Map::is_empty) => {}
                "type" if base.get("type") == Some(value) => {}
                "required"
                    if schema_types(base).iter().any(|ty| ty == "object")
                        && value.as_array().is_some_and(|required| {
                            required.iter().all(|key| {
                                key.as_str().is_some_and(|key| {
                                    base.get("properties")
                                        .and_then(Value::as_object)
                                        .is_some_and(|properties| properties.contains_key(key))
                                })
                            })
                        }) => {}
                _ => return None,
            }
        }
    }
    Some(carrier)
}

fn only_null(value: &Value) -> bool {
    value.get("type").is_some_and(|t| {
        t == "null"
            || t.as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(|t| t == "null"))
    }) || value.get("const") == Some(&Value::Null)
        || value
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_null))
}
fn literal_variants(values: Vec<&Value>) -> Vec<Literal> {
    let mut seen = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        let json = serde_json::to_string(value).expect("literal JSON");
        if !seen.insert(json.clone()) {
            continue;
        }
        let label = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Value{json}"));
        result.push(Literal {
            name: allocate_variant(&pascal(&label), &mut used),
            json,
            value: value.clone(),
        });
    }
    result
}
fn branch_label(raw: &Map<String, Value>, member: &Value, index: usize) -> String {
    if let Some(reference) = member.get("$ref").and_then(Value::as_str) {
        if let Some(mapping) = raw
            .get("discriminator")
            .and_then(|d| d.get("mapping"))
            .and_then(Value::as_object)
            && let Some((name, _)) = mapping
                .iter()
                .find(|(_, value)| value.as_str() == Some(reference))
        {
            return name.clone();
        }
        return reference
            .rsplit('/')
            .next()
            .unwrap_or("Variant")
            .replace("~1", "/")
            .replace("~0", "~");
    }
    format!("Variant{}", index + 1)
}
fn native_integer_for(schema: suspect_ir::contract::Schema<'_>) -> Option<&'static str> {
    let Some((lower, upper)) = schema_view::integer_interval(schema) else {
        // Preserve the existing exact u128-capable inclusive proof too.
        return native_integer(schema.raw().as_object()?);
    };
    if lower >= 0 {
        return [
            (u8::MAX as i128, "u8"),
            (u16::MAX as i128, "u16"),
            (u32::MAX as i128, "u32"),
            (u64::MAX as i128, "u64"),
            (i128::MAX, "u128"),
        ]
        .into_iter()
        .find(|(max, _)| upper <= *max)
        .map(|(_, name)| name);
    }
    [
        (i8::MIN as i128, i8::MAX as i128, "i8"),
        (i16::MIN as i128, i16::MAX as i128, "i16"),
        (i32::MIN as i128, i32::MAX as i128, "i32"),
        (i64::MIN as i128, i64::MAX as i128, "i64"),
        (i128::MIN, i128::MAX, "i128"),
    ]
    .into_iter()
    .find(|(min, max, _)| lower >= *min && upper <= *max)
    .map(|(_, _, name)| name)
}

fn native_integer(raw: &Map<String, Value>) -> Option<&'static str> {
    let lower = raw
        .get("minimum")?
        .as_number()?
        .to_string()
        .parse::<runtime::JsonInteger>()
        .ok()?;
    let upper = raw
        .get("maximum")?
        .as_number()?
        .to_string()
        .parse::<runtime::JsonInteger>()
        .ok()?;
    if let (Some(lower), Some(upper)) = (lower.to_u128(), upper.to_u128()) {
        if lower > upper {
            return None;
        }
        return [
            ((u8::MAX as u128), "u8"),
            ((u16::MAX as u128), "u16"),
            ((u32::MAX as u128), "u32"),
            ((u64::MAX as u128), "u64"),
            (u128::MAX, "u128"),
        ]
        .into_iter()
        .find(|(bound, _)| upper <= *bound)
        .map(|(_, ty)| ty);
    }
    let (lower, upper) = (lower.to_i128()?, upper.to_i128()?);
    if lower > upper {
        return None;
    }
    [
        (i8::MIN as i128, i8::MAX as i128, "i8"),
        (i16::MIN as i128, i16::MAX as i128, "i16"),
        (i32::MIN as i128, i32::MAX as i128, "i32"),
        (i64::MIN as i128, i64::MAX as i128, "i64"),
        (i128::MIN, i128::MAX, "i128"),
    ]
    .into_iter()
    .find(|(min, max, _)| lower >= *min && upper <= *max)
    .map(|(_, _, ty)| ty)
}

fn reject_alias_cycles(declarations: &BTreeMap<Key, Decl>, planner: &mut Planner<'_>) {
    fn cycle(
        key: &Key,
        declarations: &BTreeMap<Key, Decl>,
        active: &mut BTreeSet<Key>,
        done: &mut BTreeSet<Key>,
    ) -> bool {
        if done.contains(key) {
            return false;
        }
        let Some(Decl::Alias(ty)) = declarations.get(key) else {
            return false;
        };
        if !active.insert(key.clone()) {
            return true;
        }
        let mut refs = BTreeSet::new();
        ty.references(false, &mut refs);
        let found = refs
            .iter()
            .any(|target| cycle(target, declarations, active, done));
        active.remove(key);
        done.insert(key.clone());
        found
    }
    let mut done = BTreeSet::new();
    for key in declarations.keys() {
        if cycle(key, declarations, &mut BTreeSet::new(), &mut done) {
            planner.report(&key.0, "recursive-type-alias", DiagnosticKind::Error, "an alias-only cycle has no finite Rust declaration; a source object or union must guard recursion");
        }
    }
}

// Collapse aliases while examining layout edges. Vec and maps already guard
// their elements. Only deterministic DFS feedback edges receive Box wrappers.
fn box_layout_cycles(declarations: &mut BTreeMap<Key, Decl>) {
    fn concrete(
        ty: &Type,
        declarations: &BTreeMap<Key, Decl>,
        active: &mut BTreeSet<Key>,
        result: &mut BTreeSet<Key>,
    ) {
        match ty {
            Type::Named(key) if active.insert(key.clone()) => {
                if let Some(Decl::Alias(inner)) = declarations.get(key) {
                    concrete(inner, declarations, active, result);
                } else {
                    result.insert(key.clone());
                }
                active.remove(key);
            }
            Type::Nullable(inner) | Type::Optional(inner) | Type::Presence(inner) => {
                concrete(inner, declarations, active, result)
            }
            _ => {}
        }
    }
    let graph: BTreeMap<Key, BTreeSet<Key>> = declarations
        .iter()
        .filter(|(_, decl)| !matches!(decl, Decl::Alias(_)))
        .map(|(key, decl)| {
            let mut targets = BTreeSet::new();
            for ty in decl.types() {
                concrete(ty, declarations, &mut BTreeSet::new(), &mut targets);
            }
            (key.clone(), targets)
        })
        .collect();
    fn visit(
        key: &Key,
        graph: &BTreeMap<Key, BTreeSet<Key>>,
        active: &mut BTreeSet<Key>,
        done: &mut BTreeSet<Key>,
        feedback: &mut BTreeSet<(Key, Key)>,
    ) {
        if done.contains(key) {
            return;
        }
        active.insert(key.clone());
        for target in graph.get(key).into_iter().flatten() {
            if active.contains(target) {
                feedback.insert((key.clone(), target.clone()));
            } else {
                visit(target, graph, active, done, feedback);
            }
        }
        active.remove(key);
        done.insert(key.clone());
    }
    let mut feedback = BTreeSet::new();
    let mut done = BTreeSet::new();
    for key in graph.keys() {
        visit(key, &graph, &mut BTreeSet::new(), &mut done, &mut feedback);
    }
    let resolutions: BTreeMap<Key, BTreeSet<Key>> = declarations
        .keys()
        .map(|key| {
            let mut targets = BTreeSet::new();
            concrete(
                &Type::Named(key.clone()),
                declarations,
                &mut BTreeSet::new(),
                &mut targets,
            );
            (key.clone(), targets)
        })
        .collect();
    fn apply(
        owner: &Key,
        ty: &mut Type,
        resolutions: &BTreeMap<Key, BTreeSet<Key>>,
        feedback: &BTreeSet<(Key, Key)>,
    ) {
        match ty {
            Type::Named(key)
                if resolutions
                    .get(key)
                    .into_iter()
                    .flatten()
                    .any(|target| feedback.contains(&(owner.clone(), target.clone()))) =>
            {
                *ty = Type::Boxed(Box::new(ty.clone()));
            }
            Type::Nullable(inner) | Type::Optional(inner) | Type::Presence(inner) => {
                apply(owner, inner, resolutions, feedback)
            }
            _ => {}
        }
    }
    for (key, decl) in declarations {
        match decl {
            Decl::Struct { fields, .. } => {
                for field in fields {
                    apply(key, &mut field.ty, &resolutions, &feedback);
                }
            }
            Decl::Enum(variants) => {
                for variant in variants {
                    apply(key, &mut variant.ty, &resolutions, &feedback);
                }
            }
            _ => {}
        }
    }
}

fn allocate_names(
    contract: &Contract,
    declared: &BTreeSet<SchemaId>,
    nullable: &BTreeMap<SchemaId, bool>,
) -> BTreeMap<Key, String> {
    let hints = crate::model_naming::Hints::new(contract);
    let mut proposed: BTreeMap<String, Vec<Key>> = BTreeMap::new();
    for id in declared {
        let name = hints
            .get(id)
            .map_or_else(|| source_name(id), |name| pascal(&name));
        proposed
            .entry(name.clone())
            .or_default()
            .push((id.clone(), RepresentationRole::Model));
        if nullable.get(id) == Some(&true) {
            proposed
                .entry(format!("{name}Value"))
                .or_default()
                .push((id.clone(), RepresentationRole::NonNullValue));
        }
    }
    let mut used: BTreeSet<_> = proposed.keys().cloned().collect();
    let mut names = BTreeMap::new();
    for (base, keys) in proposed {
        if keys.len() == 1 {
            names.insert(keys[0].clone(), base);
            continue;
        }
        for key in keys {
            let hash = format!("{}#{:?}", source_text(&key.0), key.1)
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
                    (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                });
            let candidate = format!("{base}H{hash:016x}");
            names.insert(key, allocate_local(&candidate, &mut used));
        }
    }
    names
}
fn source_name(id: &SchemaId) -> String {
    let tokens = id
        .pointer()
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let start = if tokens.first().is_some_and(|t| t == "components")
        && tokens.get(1).is_some_and(|t| t == "schemas")
    {
        2
    } else {
        0
    };
    let mut parts = Vec::new();
    for token in &tokens[start..] {
        if matches!(token.as_str(), "properties" | "$defs" | "definitions") {
            continue;
        }
        parts.push(match token.as_str() {
            "items" => "Item".into(),
            "oneOf" | "anyOf" => "Variant".into(),
            _ => token.clone(),
        });
    }
    pascal(&parts.join("_"))
}
fn words(value: &str) -> Vec<String> {
    let chars: Vec<_> = value.chars().collect();
    let mut result = Vec::new();
    let mut word = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
            if !ch.is_ascii() {
                result.push(format!("u{:x}", u32::from(ch)));
            }
            continue;
        }
        let prev = index.checked_sub(1).and_then(|i| chars.get(i));
        let next = chars.get(index + 1);
        if ch.is_ascii_uppercase()
            && !word.is_empty()
            && (prev.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                || (prev.is_some_and(char::is_ascii_uppercase)
                    && next.is_some_and(char::is_ascii_lowercase)))
        {
            result.push(std::mem::take(&mut word));
        }
        word.push(ch.to_ascii_lowercase());
    }
    if !word.is_empty() {
        result.push(word);
    }
    result
}
pub(crate) fn pascal(value: &str) -> String {
    let mut name = words(value)
        .into_iter()
        .map(|word| format!("{}{}", word[..1].to_ascii_uppercase(), &word[1..]))
        .collect::<String>();
    if name.is_empty() {
        name = "Value".into();
    }
    if name.starts_with(|ch: char| ch.is_ascii_digit()) {
        name.insert_str(0, "Value");
    }
    if name == "Self" {
        name.push_str("Model");
    }
    name
}
pub(crate) fn snake(value: &str) -> String {
    let mut name = words(value).join("_");
    if name.is_empty() {
        name = "field".into();
    }
    if name.starts_with(|ch: char| ch.is_ascii_digit()) {
        name.insert(0, '_');
    }
    if matches!(
        name.as_str(),
        "as" | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "union"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
            | "abstract"
    ) {
        name.push('_');
    }
    name
}
fn allocate_local(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}
fn allocate_variant(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}Variant{suffix}");
        suffix += 1;
    }
    name
}
fn quote(value: &str) -> String {
    format!("{value:?}")
}
fn source_text(id: &SchemaId) -> String {
    format!("{}#{}", id.document(), id.pointer())
}
/// Escape untrusted prose while making valid HTTP URLs explicit rustdoc links.
pub(crate) fn prose(mut value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    while !value.is_empty() {
        let http = value
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"));
        let https = value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"));
        if http || https {
            let end = value
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, '<' | '>' | '`' | '"')
                })
                .unwrap_or(value.len());
            let mut candidate = value[..end].trim_end_matches(['.', ',', ';']);
            let mut parens = candidate
                .bytes()
                .filter(|byte| *byte == b')')
                .count()
                .saturating_sub(candidate.bytes().filter(|byte| *byte == b'(').count());
            let mut brackets = candidate
                .bytes()
                .filter(|byte| *byte == b']')
                .count()
                .saturating_sub(candidate.bytes().filter(|byte| *byte == b'[').count());
            while let Some(last) = candidate.as_bytes().last() {
                match last {
                    b')' if parens > 0 => parens -= 1,
                    b']' if brackets > 0 => brackets -= 1,
                    _ => break,
                }
                candidate = &candidate[..candidate.len() - 1];
            }
            if url::Url::parse(candidate).is_ok_and(|url| url.has_host()) {
                escaped.push('<');
                escaped.push_str(candidate);
                escaped.push('>');
                value = &value[candidate.len()..];
                continue;
            }
            // Consume a malformed URL-shaped token once, rather than retrying
            // every nested scheme prefix and turning hostile prose quadratic.
            // The token contains no backticks or whitespace, so code is inert.
            escaped.push('`');
            escaped.push_str(&value[..end]);
            escaped.push('`');
            value = &value[end..];
            continue;
        }
        let character = value.chars().next().expect("remaining prose is nonempty");
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '`' => escaped.push_str("&#96;"),
            '[' => escaped.push_str("&#91;"),
            ']' => escaped.push_str("&#93;"),
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => escaped.push(' '),
            _ => escaped.push(character),
        }
        value = &value[character.len_utf8()..];
    }
    escaped
}
