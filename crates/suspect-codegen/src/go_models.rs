//! Canonical Contract → immutable Go model/type/documentation plan.
//!
//! Native model planning consumes the canonical source-addressed contract.
//! It emits neutral model candidates, not schema codecs or an HTTP client.
//! Scalar constraints, literal membership and union exclusivity remain codec
//! obligations; every selected root stays codec-blocked and `release_ready`
//! remains false until actual source-aware Go codecs exist.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use suspect_ir::contract::{Contract, ContractSeverity, SchemaDialect, SchemaId};

use crate::OutFile;
use crate::rust_models::{DiagnosticKind, ModelDiagnostic, RepresentationRole, pascal};
use crate::schema_view;

pub(crate) type Key = (SchemaId, RepresentationRole);

/// One immutable public Go declaration and documentation identity.
#[derive(Debug, Clone)]
pub struct GoSymbol {
    source: SchemaId,
    name: String,
    role: RepresentationRole,
    code: String,
    description: String,
    source_json: String,
}
impl GoSymbol {
    /// Canonical source identity shared with documentation.
    #[must_use]
    pub fn source(&self) -> &SchemaId {
        &self.source
    }
    /// Allocated, idiomatic Go name.
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
        "go/models.go"
    }
}

/// One declared struct field's planned wire identity.
#[derive(Debug, Clone)]
pub struct GoFieldWire {
    /// Owning allocated model name.
    pub model: String,
    /// Allocated Go field name.
    pub field: String,
    /// Exact wire (property) name from the source.
    pub wire: String,
    /// Whether the source declares the property required.
    pub required: bool,
    /// Whether null is a distinct representable state.
    pub nullable: bool,
    /// Original source address of the property schema.
    pub source: SchemaId,
}

/// An immutable Go symbol/type/docs snapshot. Rendering does no source inference.
#[derive(Debug)]
pub struct ModelPlan {
    symbols: Vec<GoSymbol>,
    diagnostics: Vec<ModelDiagnostic>,
    openapi_version: String,
    api_title: String,
    fields: Vec<GoFieldWire>,
    /// Typed lowering output retained for source-aware codec planning;
    /// codecs never re-parse emitted Go source.
    pub(crate) descriptors: GoDescriptors,
}

/// Typed declaration table and allocated names, `pub(crate)` because only
/// codec planning consumes them.
#[derive(Debug)]
pub(crate) struct GoDescriptors {
    pub names: BTreeMap<Key, String>,
    pub declarations: BTreeMap<Key, GoDecl>,
}
impl ModelPlan {
    /// Every selected root, reference target and promoted inline declaration.
    #[must_use]
    pub fn symbols(&self) -> &[GoSymbol] {
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
    /// Whether this plan can be promoted as a complete SDK. Model-only plans
    /// always retain codec obligations, so this stays false.
    #[must_use]
    pub fn release_ready(&self) -> bool {
        false
    }
    /// Planned wire identities of every declared struct field.
    #[must_use]
    pub fn fields(&self) -> &[GoFieldWire] {
        &self.fields
    }
    /// Declared packaging identity of the generated module.
    #[must_use]
    pub fn module(&self) -> &'static str {
        "example.com/generated-models"
    }
    /// Typed declaration table for codec planning.
    #[must_use]
    pub(crate) fn descriptors(&self) -> &GoDescriptors {
        &self.descriptors
    }
    /// Render a dependency-free Go package and docs from this same plan.
    ///
    /// # Errors
    /// Returns the findings if unsupported representations prevent emission.
    /// Codec obligations permit reviewable artifacts but always block release.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        render_plan(self)
    }
}

/// Plan selected schema closures directly from their canonical source identities.
/// Unknown roots and unsupported shapes are ordinary source-linked errors.
pub(crate) fn requires_resources(contract: &Contract, closure: &[SchemaId]) -> bool {
    closure.iter().any(|id| {
        let Some(schema) = contract.schema(id) else {
            return false;
        };
        let raw = schema_view::raw(schema);
        raw.get("$id").is_some()
            || raw.get("$dynamicRef").is_some()
            || raw.get("$dynamicAnchor").is_some()
            || contract
                .resource_scope(id)
                .is_some_and(|scope| scope.base_source().is_some())
    })
}

/// Plan native representations and retain their codec obligations.
#[must_use]
pub fn plan_models(contract: &Contract, roots: &[SchemaId]) -> ModelPlan {
    plan_models_with_policy(contract, roots, schema_view::DialectPolicy::default())
}

/// The same plan under explicit versioned dialect interpretation choices.
#[must_use]
pub fn plan_models_with_policy(
    contract: &Contract,
    roots: &[SchemaId],
    policy: schema_view::DialectPolicy,
) -> ModelPlan {
    let reachable = schema_view::closure(contract, roots);
    let resources = requires_resources(contract, &reachable);
    let scoped = resources
        || schema_view::has_intersections(contract, &reachable)
        || reachable
            .iter()
            .filter_map(|id| contract.schema(id))
            .any(|schema| {
                let raw = schema_view::raw(schema);
                [
                    "if",
                    "dependentRequired",
                    "dependentSchemas",
                    "contains",
                    "patternProperties",
                    "propertyNames",
                    "unevaluatedProperties",
                    "unevaluatedItems",
                ]
                .iter()
                .any(|keyword| raw.get(*keyword).is_some())
                    || raw.get("const").is_some_and(serde_json::Value::is_number)
                    || raw
                        .get("enum")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|values| values.iter().any(serde_json::Value::is_number))
            });
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
        scoped,
        resources,
        policy,
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
        planner.report(root, "model-codec-unimplemented", DiagnosticKind::CodecObligation, "this root has no source-aware encoder/decoder; JSON validity, exact numeric tokens, constraints, absence/null, object keys and exclusivity must be validated before release");
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
        let nullable =
            schema_view::null_allowed(contract, id, planner.policy).unwrap_or_else(|problem| {
                if scoped {
                    // Conditions can constrain null dynamically. Keep a distinct
                    // nullable state unless a local type/literal already excludes
                    // it; the complete source codec decides whether it is valid.
                    return scoped_nullability(
                        contract,
                        id,
                        &mut BTreeSet::new(),
                        &mut 100_000,
                        planner.policy,
                    )
                    .unwrap_or(true);
                }
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
    let mut declarations: BTreeMap<Key, GoDecl> = BTreeMap::new();
    for id in &declared {
        let key = planner.core_key(id);
        declarations.insert(key.clone(), planner.lower_decl(id));
        if planner.is_nullable(id) {
            declarations.insert(
                (id.clone(), RepresentationRole::Model),
                GoDecl::Alias(GoType::Nullable(Box::new(GoType::Named(key)))),
            );
        }
    }
    reject_alias_cycles(&declarations, &mut planner);
    pointer_layout_cycles(&mut declarations);
    allocate_auxiliary_names(&mut declarations, &planner.names);
    let mut symbols = Vec::new();
    let mut fields = Vec::new();
    for (key, declaration) in &declarations {
        let raw = contract.schema(&key.0).expect("declared schema").raw();
        let name = planner.names[key].clone();
        let description =
            schema_view::description(contract.schema(&key.0).expect("declared schema"));
        let mut header = format!(
            "// Role: {}. Source: {}. Scalar constraints remain codec\n// obligations.\n",
            role_label(key.1),
            go_prose(&source_text(&key.0)),
        );
        if !description.is_empty() {
            header = format!("// Description: {}\n{header}", go_prose(description));
        }
        let mut wires: Vec<String> = Vec::new();
        if let GoDecl::Struct {
            fields: declared_fields,
            extras,
        } = declaration
        {
            for field in declared_fields {
                let required = if field.init.is_none() {
                    "required"
                } else {
                    "optional"
                };
                let null = if planner.is_nullable(&field.source) {
                    ", null allowed"
                } else {
                    ""
                };
                wires.push(format!(
                    "{} -> {} ({}{})",
                    field.name,
                    serde_json::to_string(&field.wire).expect("wire name JSON"),
                    required,
                    null
                ));
                if !field.description.is_empty() {
                    wires.push(format!(
                        "{} description: {}",
                        field.name,
                        go_prose(&field.description)
                    ));
                }
                fields.push(GoFieldWire {
                    model: name.clone(),
                    field: field.name.clone(),
                    wire: field.wire.clone(),
                    required: field.init.is_none(),
                    nullable: planner.is_nullable(&field.source),
                    source: field.source.clone(),
                });
            }
            if let Some(extras) = extras {
                wires.push(format!(
                    "extra -> undeclared wire properties typed map[string]{}",
                    extras.render(&planner.names)
                ));
            }
        }
        symbols.push(GoSymbol {
            source: key.0.clone(),
            role: key.1,
            code: declaration.render(&name, &planner.names, &header, &wires, false),
            name,
            description: description.to_owned(),
            source_json: serde_json::to_string_pretty(raw).expect("schema JSON"),
        });
    }
    let api_title = contract
        .document(contract.entry())
        .and_then(|doc| doc.get("info").and_then(|info| info.get("title")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    ModelPlan {
        symbols,
        diagnostics: planner.diagnostics,
        openapi_version: contract.openapi_version().into(),
        api_title,
        fields,
        descriptors: GoDescriptors {
            names: planner.names,
            declarations,
        },
    }
}

struct Planner<'a> {
    contract: &'a Contract,
    names: BTreeMap<Key, String>,
    nullable: BTreeMap<SchemaId, bool>,
    diagnostics: Vec<ModelDiagnostic>,
    transparent: BTreeMap<SchemaId, SchemaId>,
    scoped: bool,
    resources: bool,
    policy: schema_view::DialectPolicy,
}

#[derive(Debug, Clone)]
pub(crate) enum GoType {
    Primitive(&'static str),
    Named(Key),
    Nullable(Box<GoType>),
    Optional(Box<GoType>),
    Presence(Box<GoType>),
    Slice(Box<GoType>),
    Map(Box<GoType>),
    Pointer(Box<GoType>),
}
impl GoType {
    pub(crate) fn render(&self, names: &BTreeMap<Key, String>) -> String {
        match self {
            Self::Primitive(value) => (*value).into(),
            Self::Named(key) => names[key].clone(),
            Self::Nullable(inner) => format!("Nullable[{}]", inner.render(names)),
            Self::Optional(inner) => format!("Optional[{}]", inner.render(names)),
            Self::Presence(inner) => format!("Presence[{}]", inner.render(names)),
            Self::Slice(inner) => format!("[]{}", inner.render(names)),
            Self::Map(inner) => format!("map[string]{}", inner.render(names)),
            Self::Pointer(inner) => format!("*{}", inner.render(names)),
        }
    }
    fn references(&self, guarded: bool, result: &mut BTreeSet<Key>) {
        match self {
            Self::Named(key) => {
                result.insert(key.clone());
            }
            Self::Nullable(inner) | Self::Optional(inner) | Self::Presence(inner) => {
                inner.references(guarded, result);
            }
            Self::Slice(inner) | Self::Map(inner) | Self::Pointer(inner) if !guarded => {
                inner.references(false, result);
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub(crate) enum GoDecl {
    Alias(GoType),
    Struct {
        fields: Vec<GoField>,
        extras: Option<GoType>,
    },
    Literals {
        underlying: &'static str,
        values: Vec<GoLiteral>,
    },
    Union(Vec<GoVariant>),
}
#[derive(Debug)]
pub(crate) struct GoVariant {
    pub name: String,
    pub ty: GoType,
    pub source: SchemaId,
}
#[derive(Debug)]
pub(crate) struct GoField {
    pub(crate) name: String,
    pub(crate) param: String,
    pub(crate) wire: String,
    pub(crate) source: SchemaId,
    pub(crate) description: String,
    pub(crate) ty: GoType,
    pub(crate) init: Option<Init>,
}
#[derive(Debug)]
pub(crate) enum Init {
    Absent,
    Optional,
}
#[derive(Debug)]
pub(crate) struct GoLiteral {
    pub(crate) name: String,
    pub(crate) token: String,
}

impl GoDecl {
    pub(crate) fn types(&self) -> Vec<&GoType> {
        match self {
            Self::Alias(ty) => vec![ty],
            Self::Struct { fields, .. } => fields.iter().map(|f| &f.ty).collect(),
            Self::Literals { .. } => vec![],
            Self::Union(variants) => variants.iter().map(|variant| &variant.ty).collect(),
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
    fn full_type(&mut self, id: &SchemaId) -> GoType {
        if self
            .names
            .contains_key(&(id.clone(), RepresentationRole::Model))
        {
            return GoType::Named((id.clone(), RepresentationRole::Model));
        }
        let core = self.core_type(id);
        if self.is_nullable(id) {
            GoType::Nullable(Box::new(core))
        } else {
            core
        }
    }
    fn core_type(&mut self, id: &SchemaId) -> GoType {
        let key = self.core_key(id);
        if self.names.contains_key(&key) {
            return GoType::Named(key);
        }
        match self.lower_decl(id) {
            GoDecl::Alias(ty) => ty,
            _ => {
                self.report(
                    id,
                    "unplanned-declaration",
                    DiagnosticKind::Error,
                    "this inline representation requires a planned declaration",
                );
                GoType::Primitive("Value")
            }
        }
    }
}

impl Planner<'_> {
    fn check_shape(&mut self, id: &SchemaId) {
        let schema = self.contract.schema(id).expect("reachable schema");
        if schema.raw() == &Value::Bool(true) {
            self.report(id, "go-value-domain-admits-null", DiagnosticKind::Annotation, "boolean-true schemas lower to the runtime Value domain, which includes null; rejecting null remains a codec obligation");
        }
        let value = schema_view::raw(schema);
        let Some(raw) = value.as_object() else {
            return;
        };
        if !self.scoped
            && raw.get("const").is_some_and(Value::is_number)
            && raw.contains_key("enum")
        {
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
            if self.scoped && (keyword != "$dynamicRef" || self.resources) {
                continue;
            }
            if raw.contains_key(keyword) {
                self.report(
                    &id.child(keyword),
                    "unsupported-go-representation",
                    DiagnosticKind::Error,
                    &format!("{keyword} lowering is not implemented for Go models"),
                );
            }
        }
        if raw.contains_key("$ref") {
            let assertions = raw
                .keys()
                .filter(|key| {
                    !annotation(key) && key.as_str() != "$ref" && key.as_str() != "nullable"
                })
                .count();
            if assertions > 0 && !self.scoped {
                self.report(
                    id,
                    "ref-sibling-representation",
                    DiagnosticKind::Error,
                    "reference assertion siblings require an intersection representation",
                );
            }
        }
        if !self.scoped
            && (raw.contains_key("oneOf") || raw.contains_key("anyOf"))
            && [
                "type",
                "enum",
                "const",
                "properties",
                "additionalProperties",
                "items",
            ]
            .iter()
            .any(|key| raw.contains_key(*key))
        {
            self.report(
                id,
                "union-intersection-representation",
                DiagnosticKind::Error,
                "union assertion siblings require an intersection representation",
            );
        }
        let mut types = schema_types(raw);
        types.retain(|t| t != "null");
        if types.contains(&"number".into()) {
            types.retain(|t| t != "integer");
        }
        if types.len() > 1 && !self.scoped {
            self.report(
                &id.child("type"),
                "type-union-representation",
                DiagnosticKind::Error,
                "multiple non-null type alternatives have no faithful Go representation; unions are not invented",
            );
        }
        if !self.scoped
            && raw.get("type").is_none()
            && !self.transparent.contains_key(id)
            && !raw.contains_key("$ref")
            && !raw.contains_key("oneOf")
            && !raw.contains_key("anyOf")
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
}

impl Planner<'_> {
    fn lower_decl(&mut self, id: &SchemaId) -> GoDecl {
        let Some(schema) = self.contract.schema(id) else {
            return GoDecl::Alias(GoType::Primitive("Value"));
        };
        let value = schema_view::raw(schema);
        match &*value {
            Value::Bool(false) => {
                if self.scoped {
                    return GoDecl::Alias(GoType::Primitive("Value"));
                }
                self.report(
                    id,
                    "uninhabited-go-representation",
                    DiagnosticKind::Error,
                    "boolean-false and empty schemas are uninhabited; Go has no uninhabited type",
                );
                return GoDecl::Alias(GoType::Primitive("Value"));
            }
            Value::Bool(true) => return GoDecl::Alias(GoType::Primitive("Value")),
            _ => {}
        }
        let Some(raw) = value.as_object() else {
            self.report(
                id,
                "invalid-schema",
                DiagnosticKind::Error,
                "schema is neither an object nor Boolean",
            );
            return GoDecl::Alias(GoType::Primitive("Value"));
        };
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
            return GoDecl::Alias(self.core_type(&carrier));
        }
        if self.resources && raw.contains_key("$dynamicRef") {
            // The initial target is not a generation-time type selection.
            // Preserve local named object fields where their type is explicit;
            // otherwise a checked JSON carrier can represent every binding.
            if schema_types(raw) == vec!["object".to_owned()] {
                return self.object_decl(id, raw);
            }
            return GoDecl::Alias(GoType::Primitive("Value"));
        }
        if raw.contains_key("$ref") {
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone());
            return GoDecl::Alias(match target {
                Some(target) => self.core_type(&target),
                None => {
                    self.report(
                        &id.child("$ref"),
                        "unresolved-model-reference",
                        DiagnosticKind::Error,
                        "reference has no canonical target",
                    );
                    GoType::Primitive("Value")
                }
            });
        }
        if self.scoped && schema_types(raw) == vec!["object".to_owned()] {
            return self.object_decl(id, raw);
        }
        if let Some(literals) = scalar_literals(raw) {
            if self.scoped
                && (literals.iter().all(|v| v.is_null())
                    || literals
                        .iter()
                        .any(|v| !v.is_null() && !v.is_string() && !v.is_boolean())
                    || literals.iter().any(|v| v.is_string())
                        && literals.iter().any(|v| v.is_boolean())
                    || raw.get("const").is_some_and(Value::is_number))
            {
                return GoDecl::Alias(GoType::Primitive("Value"));
            }
            return self.literals_decl(id, raw, literals);
        }
        if raw.contains_key("oneOf") || raw.contains_key("anyOf") {
            if self.scoped {
                // Same-object named fields remain ergonomic. Other composition
                // domains use checked JSON values instead of choosing a branch
                // whose conversion could erase another branch's properties.
                if schema_types(raw) == vec!["object".to_owned()] {
                    return self.object_decl(id, raw);
                }
                return GoDecl::Alias(GoType::Primitive("Value"));
            }
            let keyword = if raw.contains_key("oneOf") {
                "oneOf"
            } else {
                "anyOf"
            };
            let mut names = BTreeSet::new();
            let variants = raw[keyword]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, member)| {
                    let source = id.child(keyword).child(&index.to_string());
                    let value =
                        schema_view::raw(self.contract.schema(&source).expect("indexed branch"));
                    if *value == Value::Bool(false)
                        || value.get("type") == Some(&Value::String("null".into()))
                    {
                        return None;
                    }
                    let label = member
                        .get("$ref")
                        .and_then(Value::as_str)
                        .and_then(|reference| reference.rsplit('/').next())
                        .map(pascal)
                        .unwrap_or_else(|| format!("Variant{}", index + 1));
                    Some(GoVariant {
                        name: allocate_local(&label, &mut names),
                        ty: self.full_type(&source),
                        source,
                    })
                })
                .collect();
            return GoDecl::Union(variants);
        }
        let mut types = schema_types(raw);
        types.retain(|t| t != "null");
        if types.contains(&"number".into()) {
            types.retain(|t| t != "integer");
        }
        if types.len() > 1 {
            return GoDecl::Alias(GoType::Primitive("Value"));
        }
        let ty = match types.first().map(String::as_str) {
            Some("object") => return self.object_decl(id, raw),
            Some("array") => GoType::Slice(Box::new(
                if self.scoped && raw.contains_key("prefixItems") {
                    GoType::Primitive("Value")
                } else if raw.contains_key("items") {
                    self.full_type(&id.child("items"))
                } else {
                    GoType::Primitive("Value")
                },
            )),
            Some("string") => GoType::Primitive("string"),
            Some("boolean") => GoType::Primitive("bool"),
            Some("integer") => GoType::Primitive("Integer"),
            Some("number") => GoType::Primitive("Number"),
            None if raw.contains_key("type") => {
                if self.scoped && schema_types(raw) == vec!["null".to_owned()] {
                    return GoDecl::Alias(GoType::Primitive("Value"));
                }
                self.report(
                    &id.child("type"),
                    "invalid-model-type",
                    DiagnosticKind::Error,
                    "unsupported or invalid schema type",
                );
                GoType::Primitive("Value")
            }
            None => GoType::Primitive("Value"),
            _ => {
                self.report(
                    &id.child("type"),
                    "invalid-model-type",
                    DiagnosticKind::Error,
                    "unsupported or invalid schema type",
                );
                GoType::Primitive("Value")
            }
        };
        GoDecl::Alias(ty)
    }
}

impl Planner<'_> {
    fn object_decl(&mut self, id: &SchemaId, raw: &Map<String, Value>) -> GoDecl {
        let properties = raw.get("properties").and_then(Value::as_object);
        let required: BTreeSet<_> = raw
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for name in &required {
            if !self.scoped && !properties.is_some_and(|p| p.contains_key(*name)) {
                self.report(
                    &id.child("required"),
                    "undeclared-required-field",
                    DiagnosticKind::Error,
                    "required undeclared keys need a dedicated construction representation",
                );
            }
        }
        let patterned = self.scoped
            && raw
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|patterns| !patterns.is_empty());
        if patterned && properties.is_none_or(Map::is_empty) {
            return GoDecl::Alias(GoType::Map(Box::new(GoType::Primitive("Value"))));
        }
        if !patterned
            && properties.is_none_or(Map::is_empty)
            && raw
                .get("additionalProperties")
                .is_some_and(Value::is_object)
        {
            return GoDecl::Alias(GoType::Map(Box::new(
                self.full_type(&id.child("additionalProperties")),
            )));
        }
        let mut fields = Vec::new();
        let mut used = ["SetExtra", "Extra", "MarshalJSON", "UnmarshalJSON"]
            .into_iter()
            .map(str::to_owned)
            .collect();
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
                (
                    if nullable {
                        GoType::Nullable(Box::new(core))
                    } else {
                        core
                    },
                    None,
                )
            } else if nullable {
                (GoType::Presence(Box::new(core)), Some(Init::Absent))
            } else {
                (GoType::Optional(Box::new(core)), Some(Init::Optional))
            };
            fields.push(GoField {
                name: allocate_local(&pascal(wire), &mut used),
                param: String::new(),
                wire: wire.clone(),
                source: source.clone(),
                description,
                ty,
                init,
            });
        }
        let extras = (patterned || raw.get("additionalProperties") != Some(&Value::Bool(false)))
            .then(|| {
                if patterned {
                    GoType::Primitive("Value")
                } else if raw
                    .get("additionalProperties")
                    .is_some_and(Value::is_object)
                {
                    self.full_type(&id.child("additionalProperties"))
                } else {
                    GoType::Primitive("Value")
                }
            });
        assign_params(&mut fields);
        GoDecl::Struct { fields, extras }
    }
    fn literals_decl(
        &mut self,
        id: &SchemaId,
        raw: &Map<String, Value>,
        literals: Vec<&Value>,
    ) -> GoDecl {
        let mut underlying = "";
        let mut seen = BTreeSet::new();
        let mut used: BTreeSet<String> = BTreeSet::new();
        let mut values = Vec::new();
        for value in literals {
            if value.is_null() {
                continue;
            }
            if !accepts_literal_kind(raw, value) {
                continue;
            }
            let kind = if value.is_string() {
                "string"
            } else if value.is_boolean() {
                "bool"
            } else {
                self.report(id, "go-literal-kind-unsupported", DiagnosticKind::Error, "numeric, object and array enum/const members require exact literal-equality lowering that is not implemented for Go models");
                return GoDecl::Alias(GoType::Primitive("Value"));
            };
            if underlying.is_empty() {
                underlying = kind;
            } else if underlying != kind {
                self.report(
                    id,
                    "go-literal-kind-mixed",
                    DiagnosticKind::Error,
                    "mixed string and boolean literal kinds have no single underlying Go type",
                );
                return GoDecl::Alias(GoType::Primitive("Value"));
            }
            let token = serde_json::to_string(value).expect("literal JSON");
            if seen.insert(token.clone()) {
                let label = value.as_str().unwrap_or(if *value == Value::Bool(true) {
                    "true"
                } else {
                    "false"
                });
                values.push(GoLiteral {
                    name: allocate_local(&pascal(label), &mut used),
                    token,
                });
            }
        }
        if underlying.is_empty() {
            self.report(
                id,
                "uninhabited-go-representation",
                DiagnosticKind::Error,
                "this literal schema has no remaining non-null member; Go has no uninhabited type",
            );
            return GoDecl::Alias(GoType::Primitive("Value"));
        }
        GoDecl::Literals {
            underlying: if underlying == "string" {
                "string"
            } else {
                "bool"
            },
            values,
        }
    }
}

// A representation-only proof for the null instance. Object/array/string/number
// applicators do not apply to it; actual validation always uses the full checked
// program. Incomplete recursion stays a nullable carrier, never an invented
// non-null guarantee. Shared schema/IR/provenance remain untouched.
fn scoped_nullability(
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
        if !schema_view::accepts_literal(schema, &Value::Null, policy) {
            return Some(false);
        }
        if raw.contains_key("$dynamicRef") || raw.contains_key("$recursiveRef") {
            return None;
        }
        let mut accepts = true;
        if let Some(value) = raw.get("const") {
            accepts &= value.is_null();
        }
        if let Some(values) = raw.get("enum").and_then(Value::as_array) {
            accepts &= values.iter().any(Value::is_null);
        }
        for reference in schema.references() {
            accepts &=
                scoped_nullability(contract, reference.target.as_ref()?, active, work, policy)?;
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(values) = raw.get(keyword).and_then(Value::as_array) {
                let mut count = 0;
                for i in 0..values.len() {
                    count += usize::from(scoped_nullability(
                        contract,
                        &id.child(keyword).child(&i.to_string()),
                        active,
                        work,
                        policy,
                    )?);
                }
                accepts &= match keyword {
                    "allOf" => count == values.len(),
                    "anyOf" => count > 0,
                    _ => count == 1,
                };
            }
        }
        if raw.contains_key("not") {
            accepts &= !scoped_nullability(contract, &id.child("not"), active, work, policy)?;
        }
        if raw.contains_key("if") {
            let selected = if scoped_nullability(contract, &id.child("if"), active, work, policy)? {
                "then"
            } else {
                "else"
            };
            if raw.contains_key(selected) {
                accepts &= scoped_nullability(contract, &id.child(selected), active, work, policy)?;
            }
        }
        Some(accepts)
    })();
    active.remove(id);
    result
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
fn accepts_literal_kind(raw: &Map<String, Value>, value: &Value) -> bool {
    let types = schema_types(raw);
    types.is_empty()
        || types.iter().any(|t| match t.as_str() {
            "null" => value.is_null(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
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

// Package-level identifiers of the copied exact-JSON runtime plus the wrapper
// declarations emitted in models.go. Generated model names never shadow them.
const RESERVED: &[&str] = &[
    "Codecs",
    "Codec",
    "CodecError",
    "Client",
    "ClientOptions",
    "Credentials",
    "Doer",
    "APIResponse",
    "SDKError",
    "HTTPSource",
    "ValidationError",
    "ValidationFinding",
    "ValidationSource",
    "Validate",
    "NewClient",
    "Value",
    "Number",
    "Integer",
    "JSONError",
    "JSONErrorKind",
    "JSONLimits",
    "JSONSyntax",
    "JSONDuplicateName",
    "JSONBadUnicode",
    "JSONLimit",
    "JSONCycle",
    "JSONType",
    "Parse",
    "ParseNumber",
    "ParseInteger",
    "Encode",
    "DefaultLimits",
    "Nullable",
    "Optional",
    "Presence",
    "NullableNull",
    "NullableValue",
    "OptionalAbsent",
    "OptionalSome",
    "PresenceMissing",
    "PresenceNull",
    "PresenceSome",
    "New",
    "String",
    "Limits",
];

// Literal constants and union wrappers share the package namespace with model
// declarations and constructors. Keep their allocated suffix in the retained
// descriptor so every renderer, codec and compatibility adapter uses it.
fn allocate_auxiliary_names(
    declarations: &mut BTreeMap<Key, GoDecl>,
    names: &BTreeMap<Key, String>,
) {
    let mut used = RESERVED
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    used.extend(names.values().cloned());
    for (key, declaration) in declarations.iter() {
        if matches!(declaration, GoDecl::Struct { .. }) {
            used.insert(format!("New{}", names[key]));
        }
    }
    for (key, declaration) in declarations {
        let model = &names[key];
        let mut allocate = |suffix: &mut String, constructor: bool| {
            let base = format!("{model}{suffix}");
            let mut full = base.clone();
            let mut serial = 2;
            while used.contains(&full) || (constructor && used.contains(&format!("New{full}"))) {
                full = format!("{base}{serial}");
                serial += 1;
            }
            used.insert(full.clone());
            if constructor {
                used.insert(format!("New{full}"));
            }
            *suffix = full[model.len()..].to_owned();
        };
        match declaration {
            GoDecl::Literals { values, .. } => {
                for value in values {
                    allocate(&mut value.name, false);
                }
            }
            GoDecl::Union(variants) => {
                for variant in variants {
                    allocate(&mut variant.name, true);
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
    let mut used: BTreeSet<_> = RESERVED.iter().map(|name| (*name).to_owned()).collect();
    used.extend(proposed.keys().cloned());
    let mut names = BTreeMap::new();
    for (base, keys) in proposed {
        if keys.len() == 1
            && !RESERVED.contains(&base.as_str())
            && !used.contains(&format!("New{base}"))
        {
            used.insert(format!("New{base}"));
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
            let mut name = candidate.clone();
            let mut suffix = 2;
            while used.contains(&name) || used.contains(&format!("New{name}")) {
                name = format!("{candidate}{suffix}");
                suffix += 1;
            }
            used.insert(name.clone());
            used.insert(format!("New{name}"));
            names.insert(key, name);
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
            "oneOf" | "anyOf" | "allOf" => "Variant".into(),
            _ => token.clone(),
        });
    }
    pascal(&parts.join("_"))
}
fn allocate_local(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}{suffix}");
        suffix += 1;
    }
    name
}
fn source_text(id: &SchemaId) -> String {
    format!("{}#{}", id.document(), id.pointer())
}

// Constructor parameters carry the lowercase field name; Go keywords get an
// underscore and collisions get deterministic numeric suffixes.
fn assign_params(fields: &mut [GoField]) {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for field in fields {
        let mut param = field.name[..1].to_ascii_lowercase() + &field.name[1..];
        if matches!(
            param.as_str(),
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ) {
            param.push('_');
        }
        field.param = allocate_local(&param, &mut used);
    }
}

fn reject_alias_cycles(declarations: &BTreeMap<Key, GoDecl>, planner: &mut Planner<'_>) {
    fn cycle(
        key: &Key,
        declarations: &BTreeMap<Key, GoDecl>,
        active: &mut BTreeSet<Key>,
        done: &mut BTreeSet<Key>,
    ) -> bool {
        if done.contains(key) {
            return false;
        }
        let Some(GoDecl::Alias(ty)) = declarations.get(key) else {
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
            planner.report(&key.0, "recursive-type-alias", DiagnosticKind::Error, "an alias-only cycle has no finite Go declaration; a source object must guard recursion");
        }
    }
}

// Collapse aliases and wrapper instantiations while examining value layout
// edges. Slices and maps already guard their elements. Only deterministic DFS
// feedback edges receive pointer indirection.
fn pointer_layout_cycles(declarations: &mut BTreeMap<Key, GoDecl>) {
    fn concrete(
        ty: &GoType,
        declarations: &BTreeMap<Key, GoDecl>,
        active: &mut BTreeSet<Key>,
        result: &mut BTreeSet<Key>,
    ) {
        match ty {
            GoType::Named(key) if active.insert(key.clone()) => {
                if let Some(GoDecl::Alias(inner)) = declarations.get(key) {
                    concrete(inner, declarations, active, result);
                } else {
                    result.insert(key.clone());
                }
                active.remove(key);
            }
            GoType::Nullable(inner) | GoType::Optional(inner) | GoType::Presence(inner) => {
                concrete(inner, declarations, active, result);
            }
            _ => {}
        }
    }
    let graph: BTreeMap<Key, BTreeSet<Key>> = declarations
        .iter()
        .filter(|(_, decl)| !matches!(decl, GoDecl::Alias(_)))
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
                &GoType::Named(key.clone()),
                declarations,
                &mut BTreeSet::new(),
                &mut targets,
            );
            (key.clone(), targets)
        })
        .collect();
    fn apply(
        owner: &Key,
        ty: &mut GoType,
        resolutions: &BTreeMap<Key, BTreeSet<Key>>,
        feedback: &BTreeSet<(Key, Key)>,
    ) {
        match ty {
            GoType::Named(key)
                if resolutions
                    .get(key)
                    .into_iter()
                    .flatten()
                    .any(|target| feedback.contains(&(owner.clone(), target.clone()))) =>
            {
                *ty = GoType::Pointer(Box::new(ty.clone()));
            }
            GoType::Nullable(inner) | GoType::Optional(inner) | GoType::Presence(inner) => {
                apply(owner, inner, resolutions, feedback);
            }
            _ => {}
        }
    }
    for (key, decl) in declarations {
        if let GoDecl::Struct { fields, .. } = decl {
            for field in fields.iter_mut() {
                apply(key, &mut field.ty, &resolutions, &feedback);
            }
        }
    }
}

impl GoDecl {
    /// Renders one declaration; `header` carries description/source docs and
    /// `wires` carries the struct wire-map documentation lines.
    fn render(
        &self,
        name: &str,
        names: &BTreeMap<Key, String>,
        header: &str,
        wires: &[String],
        codecs: bool,
    ) -> String {
        match self {
            Self::Alias(ty) => format!("{header}type {name} = {}\n\n", ty.render(names)),
            Self::Literals { underlying, values } => {
                let mut code = format!("{header}type {name} {underlying}\n\nconst (\n");
                let full: Vec<String> = values
                    .iter()
                    .map(|value| format!("{name}{}", value.name))
                    .collect();
                let width = full.iter().map(String::len).max().unwrap_or(0);
                for (constant, value) in full.iter().zip(values) {
                    code.push_str(&format!("\t{constant:<width$} {name} = {}\n", value.token));
                }
                code.push_str(")\n\n");
                code.push_str(&guards(name, codecs));
                code
            }
            Self::Struct { fields, extras } => {
                let mut code = String::from(header);
                if !wires.is_empty() {
                    code.push_str("//\n// Wire map:\n");
                    for wire in wires {
                        code.push_str(&format!("//\t{wire}\n"));
                    }
                }
                code.push_str(&format!("type {name} struct {{\n"));
                let mut entries: Vec<(String, String)> = fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.render(names)))
                    .collect();
                if let Some(extras) = extras {
                    entries.push((
                        "extra".into(),
                        format!("map[string]{}", extras.render(names)),
                    ));
                }
                let width = entries.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
                for (key, ty) in &entries {
                    code.push_str(&format!("\t{key:<width$} {ty}\n"));
                }
                code.push_str("}\n\n");
                let args = fields
                    .iter()
                    .filter(|field| field.init.is_none())
                    .map(|field| format!("{} {}", field.param, field.ty.render(names)))
                    .collect::<Vec<_>>()
                    .join(", ");
                code.push_str(&format!(
                    "// New{name} constructs {name} with its required fields. Optional fields\n// start absent or null; no default is inferred from source examples.\nfunc New{name}({args}) {name} {{\n\treturn {name}{{\n"
                ));
                let required: Vec<&GoField> =
                    fields.iter().filter(|field| field.init.is_none()).collect();
                let key_width = required
                    .iter()
                    .map(|field| field.name.len())
                    .max()
                    .unwrap_or(0);
                for field in &required {
                    let pad = " ".repeat(key_width - field.name.len() + 1);
                    code.push_str(&format!("\t\t{}:{pad}{},\n", field.name, field.param));
                }
                code.push_str("\t}\n}\n\n");
                if let Some(extras) = extras {
                    let ty = extras.render(names);
                    code.push_str(&format!(
                        "// SetExtra records an undeclared wire property. Declared wire\n// names cannot be shadowed by extra fields.\nfunc (x *{name}) SetExtra(key string, value {ty}) error {{\n\tswitch key {{\n"
                    ));
                    let declared: Vec<String> = fields
                        .iter()
                        .map(|field| {
                            serde_json::to_string(&field.wire).expect("wire name JSON string")
                        })
                        .collect();
                    if !declared.is_empty() {
                        code.push_str(&format!(
                            "\tcase {}:\n\t\treturn fmt.Errorf(\"declared wire name %q cannot be an extra field\", key)\n",
                            declared.join(", ")
                        ));
                    }
                    code.push_str(&format!(
                        "\t}}\n\tif x.extra == nil {{\n\t\tx.extra = make(map[string]{ty})\n\t}}\n\tx.extra[key] = value\n\treturn nil\n}}\n\n// Extra returns a shallow copy of the undeclared property map. Adding,\n// replacing or deleting returned entries does not change the stored map.\n// Nested maps, slices and pointers retain ordinary Go aliasing semantics;\n// source codecs revalidate their current values on every encode.\nfunc (x *{name}) Extra() map[string]{ty} {{\n\tif len(x.extra) == 0 {{\n\t\treturn nil\n\t}}\n\tout := make(map[string]{ty}, len(x.extra))\n\tfor k, v := range x.extra {{\n\t\tout[k] = v\n\t}}\n\treturn out\n}}\n\n"
                    ));
                }
                code.push_str(&guards(name, codecs));
                code
            }
            Self::Union(variants) => {
                let mut code = format!("{header}type {name} interface {{ is{name}() }}\n");
                for variant in variants {
                    let variant_name = format!("{name}{}", variant.name);
                    let ty = variant.ty.render(names);
                    code.push_str(&format!("// {variant_name} preserves one source union alternative.\ntype {variant_name} struct {{ Value {ty} }}\nfunc ({variant_name}) is{name}() {{}}\nfunc New{variant_name}(value {ty}) {name} {{return {variant_name}{{Value:value}}}}\n"));
                    if codecs {
                        code.push_str(&format!("func(value {variant_name}) MarshalJSON()([]byte,error){{return Codecs.{name}.Encode(value)}}\n"));
                    } else {
                        code.push_str(&guards(&variant_name, false));
                    }
                }
                code
            }
        }
    }
}

fn guards(name: &str, codecs: bool) -> String {
    if codecs {
        return format!(
            "// MarshalJSON validates the mutable native model through its source codec.\nfunc(value {name}) MarshalJSON()([]byte,error){{return Codecs.{name}.Encode(value)}}\nfunc(value *{name}) UnmarshalJSON(data []byte)error{{next,err:=Codecs.{name}.Decode(data);if err==nil{{*value=next}};return err}}\n"
        );
    }
    format!(
        "// MarshalJSON and UnmarshalJSON fail explicitly: model serialization\n// needs source-aware codecs that are not implemented yet.\nfunc ({name}) MarshalJSON() ([]byte, error) {{\n\treturn nil, errModelsNotSerializable\n}}\n\nfunc ({name}) UnmarshalJSON([]byte) error {{\n\treturn errModelsNotSerializable\n}}\n"
    )
}

const MODELS_HEADER: &str = "// Code generated from canonical OpenAPI source identities. Neutral model\n// candidates only: schema codecs, HTTP transport and model serialization are\n// not implemented. Scalar constraints, literal membership, union exclusivity\n// and object-key bounds remain codec obligations recorded in\n// model-manifest.json. Wrapper zero values are absent or null, never a\n// constructed value.\n\npackage sdk\n";

const WRAPPERS: &str = r#"// errModelsNotSerializable guards against silent encoding/json behavior:
// generated wrappers and models fail explicitly instead of emitting wrong
// wire names or flattening presence wrappers.
var errModelsNotSerializable = errors.New("model serialization is not implemented; source-aware codecs are required before generated models can be encoded or decoded")

// Nullable distinguishes an explicit JSON null from a non-null value.
// The zero value is null, never a constructed value.
type Nullable[T any] struct {
	IsValue bool
	Value   T
}

// NullableNull is the explicit null state.
func NullableNull[T any]() Nullable[T] {
	return Nullable[T]{}
}

// NullableValue wraps a non-null value.
func NullableValue[T any](value T) Nullable[T] {
	return Nullable[T]{IsValue: true, Value: value}
}

func (Nullable[T]) MarshalJSON() ([]byte, error) {
	return nil, errModelsNotSerializable
}

func (Nullable[T]) UnmarshalJSON([]byte) error {
	return errModelsNotSerializable
}

// Optional distinguishes an absent property from a present one.
// The zero value is absent.
type Optional[T any] struct {
	IsSet bool
	Value T
}

// OptionalAbsent is the absent state.
func OptionalAbsent[T any]() Optional[T] {
	return Optional[T]{}
}

// OptionalSome wraps a present value.
func OptionalSome[T any](value T) Optional[T] {
	return Optional[T]{IsSet: true, Value: value}
}

func (Optional[T]) MarshalJSON() ([]byte, error) {
	return nil, errModelsNotSerializable
}

func (Optional[T]) UnmarshalJSON([]byte) error {
	return errModelsNotSerializable
}

// Presence distinguishes absent, null and value states.
// The zero value is absent.
type Presence[T any] struct {
	IsSet bool
	Null  bool
	Value T
}

// PresenceMissing is the absent state.
func PresenceMissing[T any]() Presence[T] {
	return Presence[T]{}
}

// PresenceNull is the explicit null state.
func PresenceNull[T any]() Presence[T] {
	return Presence[T]{IsSet: true, Null: true}
}

// PresenceSome wraps a present non-null value.
func PresenceSome[T any](value T) Presence[T] {
	return Presence[T]{IsSet: true, Value: value}
}

func (Presence[T]) MarshalJSON() ([]byte, error) {
	return nil, errModelsNotSerializable
}

func (Presence[T]) UnmarshalJSON([]byte) error {
	return errModelsNotSerializable
}
"#;

fn render_plan(plan: &ModelPlan) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
    render_mode(plan, false)
}
pub(crate) fn render_codecs(plan: &ModelPlan) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
    render_mode(plan, true)
}
fn render_mode(plan: &ModelPlan, codecs: bool) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
    if plan.has_errors() {
        return Err(plan.diagnostics().to_vec());
    }
    let header = if codecs {
        "// Code generated from canonical OpenAPI source identities.\n// Source codecs validate decoding and mutable encoding. Presence wrappers\n// preserve absent/null/value states; standalone wrappers need a source model.\n\npackage sdk\n"
    } else {
        MODELS_HEADER
    };
    let mut models = format!("{header}\n{WRAPPERS}\n");
    let mut native_sources = BTreeMap::new();
    for symbol in &plan.symbols {
        if codecs {
            let key = (symbol.source.clone(), symbol.role);
            let declaration = &plan.descriptors.declarations[&key];
            let header = format!(
                "// {}: {}\n// Source: {}#{}\n",
                symbol.name,
                go_prose(&symbol.description),
                go_prose(symbol.source.document().as_str()),
                go_prose(symbol.source.pointer())
            );
            native_sources.insert(
                symbol.name.clone(),
                declaration.render(&symbol.name, &plan.descriptors.names, &header, &[], true),
            );
        } else {
            native_sources.insert(symbol.name.clone(), symbol.code.clone());
        }
        models.push_str(&native_sources[&symbol.name]);
        models.push('\n');
    }
    let imports = if models.contains("fmt.Errorf") {
        "import (\n\t\"errors\"\n\t\"fmt\"\n)\n"
    } else {
        "import \"errors\"\n"
    };
    let models = models.replacen("package sdk\n", &format!("package sdk\n\n{imports}"), 1);
    let status = if codecs {
        "Source-bound codecs are available as `Codecs.<Model>`. Defined model/literal JSON adapters delegate to those codecs. Standalone presence wrappers still require a source-bound model codec; aliases retain their underlying native behavior."
    } else {
        "Every selected root retains a missing-codec obligation. Model/literal JSON adapters refuse serialization until source-bound codecs are included. Use the canonical codec plan for validated wire conversion."
    };
    let mut readme = format!(
        "# Go contract models\n\nOpenAPI {}. Source-addressed model views for module `example.com/generated-models` (package `sdk`, Go 1.23 or later, no external dependencies). The module identity is packaging configuration only and is deliberately separate from the API title and version: {} / {}. {status} Complete SDK release acceptance remains separate.\n\nWrapper zero values are absent or null, never a constructed value: `Nullable[T]` zero is null, `Optional[T]` zero is absent, `Presence[T]` zero is absent.\n\nExact wire names live in `model-manifest.json` and the generated doc comments. Required fields become constructor arguments; optional fields start absent and schema defaults are not applied.\n\n## Models\n\n",
        md_prose(&plan.openapi_version),
        md_prose(&plan.api_title),
        md_prose(&plan.openapi_version),
    );
    for symbol in &plan.symbols {
        readme.push_str(&format!(
            "## `{}`\n\n{}\n\nRole: `{}`.\n\nSource: `{}`\n\n```go\n{}\n```\n\nOriginal schema:\n\n```json\n{}\n```\n\n",
            symbol.name,
            md_prose(&symbol.description),
            role_label(symbol.role),
            md_prose(&source_text(&symbol.source)),
            native_sources[&symbol.name],
            symbol.source_json,
        ));
    }
    readme.push_str("## Findings and codec obligations\n\n");
    for diagnostic in plan
        .diagnostics()
        .iter()
        .filter(|diagnostic| !codecs || diagnostic.kind != DiagnosticKind::CodecObligation)
    {
        readme.push_str(&format!(
            "- `{}` / `{}` at `{}`: {}\n",
            kind_label(diagnostic.kind),
            diagnostic.code,
            md_prose(&source_text(&diagnostic.source)),
            md_prose(&diagnostic.message),
        ));
    }
    let manifest = json!({
        "module": plan.module(),
        "package": "sdk",
        "go": "1.23.0",
        "openapiVersion": &plan.openapi_version,
        "apiTitle": &plan.api_title,
        "releaseReady": plan.release_ready(),
        "sourceCodecs": codecs,
        "symbols": plan.symbols.iter().map(|symbol| json!({
            "name": &symbol.name,
            "role": role_label(symbol.role),
            "source": source_text(&symbol.source),
            "file": symbol.file(),
        })).collect::<Vec<_>>(),
        "fields": plan.fields.iter().map(|field| json!({
            "model": &field.model,
            "field": &field.field,
            "wire": &field.wire,
            "required": field.required,
            "nullable": field.nullable,
            "source": source_text(&field.source),
        })).collect::<Vec<_>>(),
        "diagnostics": plan.diagnostics.iter().filter(|diagnostic|!codecs||diagnostic.kind!=DiagnosticKind::CodecObligation).map(|diagnostic| json!({
            "code": diagnostic.code,
            "kind": kind_label(diagnostic.kind),
            "source": source_text(&diagnostic.source),
            "message": &diagnostic.message,
        })).collect::<Vec<_>>(),
    });
    Ok(vec![
        OutFile {
            path: "go/go.mod".into(),
            content: "module example.com/generated-models\n\ngo 1.23.0\n".into(),
        },
        OutFile {
            path: "go/json.go".into(),
            content: crate::go_json::runtime_source().into(),
        },
        OutFile {
            path: "go/models.go".into(),
            content: models,
        },
        OutFile {
            path: "go/README.md".into(),
            content: readme,
        },
        OutFile {
            path: "go/model-manifest.json".into(),
            content: serde_json::to_string_pretty(&manifest).expect("manifest JSON"),
        },
    ])
}

/// Collapses line structure for inert plain-text Go comments.
fn go_prose(value: &str) -> String {
    value.replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ")
}

/// Escapes untrusted prose for inert README markdown.
fn md_prose(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '`' => escaped.push_str("&#96;"),
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => escaped.push(' '),
            other => escaped.push(other),
        }
    }
    escaped
}

fn role_label(role: RepresentationRole) -> &'static str {
    match role {
        RepresentationRole::Model => "model",
        RepresentationRole::NonNullValue => "non_null_value",
    }
}

fn kind_label(kind: DiagnosticKind) -> &'static str {
    match kind {
        DiagnosticKind::Error => "error",
        DiagnosticKind::CodecObligation => "codec_obligation",
        DiagnosticKind::Annotation => "annotation",
    }
}
