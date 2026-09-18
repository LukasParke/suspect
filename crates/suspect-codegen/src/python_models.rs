//! Canonical Contract → immutable Python model/dataclass plan.
//!
//! Bounded initial slice: primitive, nullable, simple object, array and
//! static-reference models render as source-addressed `kw_only=True`
//! dataclasses. Optional absence is a distinct `UNSET` sentinel, never
//! `None`; null stays a value state. Exact numbers refer to
//! `json_runtime.JsonNumber` (the audited template from
//! `crate::python_json::runtime_source`), never `float`. Schema defaults are
//! ignored and titles never drive naming. Runtime-checked JSON carriers retain
//! assertions that cannot be expressed by a native field/union type. Every selected root
//! retains a missing-codec obligation and `release_ready` stays false: this
//! is a model-only plan, not a client or a released SDK.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, ContractSeverity, SchemaDialect, SchemaId};

use crate::rust_models::{DiagnosticKind, ModelDiagnostic, RepresentationRole, pascal, snake};
use crate::schema_view;
use crate::{OutFile, python_json};

/// Python keywords and reserved names that cannot be parameter names.
/// A `kw_only=True` dataclass lowers every field to an `__init__` parameter,
/// so `self` and `cls` are excluded exactly like grammar keywords.
const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return",
    "self", "try", "while", "with", "yield",
];

/// One immutable public Python declaration and documentation identity.
#[derive(Debug, Clone)]
pub struct PythonSymbol {
    source: SchemaId,
    name: String,
    role: RepresentationRole,
    code: String,
    description: String,
    source_json: String,
}
impl PythonSymbol {
    /// Canonical source identity shared with documentation.
    #[must_use]
    pub fn source(&self) -> &SchemaId {
        &self.source
    }
    /// Allocated, idiomatic Python class name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Role of this declaration; this slice only plans whole models.
    #[must_use]
    pub fn role(&self) -> RepresentationRole {
        self.role
    }
    /// Planned source artifact path.
    #[must_use]
    pub fn file(&self) -> &'static str {
        "python/models.py"
    }
    /// Source description, uninterpreted.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Exact source schema JSON for review tooling.
    #[must_use]
    pub fn source_json(&self) -> &str {
        &self.source_json
    }
}

/// One declared dataclass field's planned wire identity.
#[derive(Debug, Clone)]
pub struct PythonFieldWire {
    /// Owning allocated model name.
    pub model: String,
    /// Allocated Python attribute name.
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

/// An immutable Python symbol/type/docs snapshot. Rendering does no source inference.
#[derive(Debug)]
pub struct ModelPlan {
    symbols: Vec<PythonSymbol>,
    diagnostics: Vec<ModelDiagnostic>,
    fields: Vec<PythonFieldWire>,
    openapi_version: String,
    /// Retained typed descriptors per schema identity. Codec planning
    /// consumes these directly; generated Python text is never re-parsed.
    pub(crate) declarations: BTreeMap<SchemaId, PyDecl>,
}
impl ModelPlan {
    /// Every selected root, reference target and promoted inline declaration.
    #[must_use]
    pub fn symbols(&self) -> &[PythonSymbol] {
        &self.symbols
    }
    /// Original-source findings, including mandatory codec obligations.
    #[must_use]
    pub fn diagnostics(&self) -> &[ModelDiagnostic] {
        &self.diagnostics
    }
    /// Planned wire identities of every declared dataclass field.
    #[must_use]
    pub fn fields(&self) -> &[PythonFieldWire] {
        &self.fields
    }
    /// Retained typed descriptors, keyed by canonical schema identity.
    #[must_use]
    pub(crate) fn declarations(&self) -> &BTreeMap<SchemaId, PyDecl> {
        &self.declarations
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
    /// Declared packaging identity of the generated module set.
    #[must_use]
    pub fn package(&self) -> &'static str {
        "generated_models"
    }
    /// OpenAPI version of the source contract.
    #[must_use]
    pub fn openapi_version(&self) -> &str {
        &self.openapi_version
    }
    /// Render a dependency-free standalone Python package and docs from this same plan.
    ///
    /// # Errors
    /// Returns the findings if unsupported representations prevent emission.
    /// Codec obligations permit reviewable artifacts but always block release.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        if self.has_errors() {
            return Err(self.diagnostics.clone());
        }
        Ok(render(self, false))
    }
}

/// Plan selected schema closures directly from their canonical source identities.
/// Unknown roots and unsupported shapes are ordinary source-linked errors.
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
    let mut planner = Planner {
        contract,
        resources: requires_resources(contract, &reachable),
        names: BTreeMap::new(),
        nullable: BTreeMap::new(),
        diagnostics: Vec::new(),
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
        planner.report(
            root,
            "python-model-codec-unimplemented",
            DiagnosticKind::CodecObligation,
            "this root has no source-aware Python encoder/decoder; JSON validity, exact numeric tokens, constraints, absence/null, object keys and exclusivity must be validated before release",
        );
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
        if schema_view::raw(schema).as_object().is_some_and(|raw| {
            raw.get("type").and_then(Value::as_str) == Some("object")
                || raw.contains_key("oneOf")
                || raw.contains_key("anyOf")
        }) {
            declared.insert(id.clone());
        }
        for reference in schema.references() {
            if let Some(target) = &reference.target {
                declared.insert(target.clone());
            }
        }
        let value =
            schema_view::null_allowed(contract, id, planner.policy).unwrap_or_else(|problem| {
            planner.report(
                &problem.source,
                "python-runtime-nullability",
                DiagnosticKind::Annotation,
                "conditional nullability is retained by the source codec; the native carrier conservatively includes every locally admitted value",
            );
            may_allow_null(contract, id, planner.policy)
        });
        planner.nullable.insert(id.clone(), value);
    }
    for id in &reachable {
        planner.check_shape(id);
    }
    planner.names = allocate_names(contract, &declared);
    let declarations: BTreeMap<SchemaId, PyDecl> = declared
        .iter()
        .map(|id| (id.clone(), planner.lower_decl(id)))
        .collect();
    reject_alias_cycles(&declarations, &mut planner);
    let mut symbols = Vec::new();
    let mut fields = Vec::new();
    for id in declaration_order(&declarations) {
        let declaration = &declarations[&id];
        let raw = contract.schema(&id).expect("declared schema").raw();
        let name = planner.names[&id].clone();
        if let PyDecl::Dataclass {
            fields: declared_fields,
            ..
        } = declaration
        {
            for field in declared_fields {
                fields.push(PythonFieldWire {
                    model: name.clone(),
                    field: field.name.clone(),
                    wire: field.wire.clone(),
                    required: field.required,
                    nullable: planner
                        .nullable
                        .get(&field.source)
                        .copied()
                        .unwrap_or(false),
                    source: field.source.clone(),
                });
            }
        }
        symbols.push(PythonSymbol {
            source: id.clone(),
            role: RepresentationRole::Model,
            code: declaration.render(&name, &planner.names, raw, &id),
            name,
            description: schema_view::description(contract.schema(&id).expect("declared schema"))
                .into(),
            source_json: serde_json::to_string_pretty(raw).expect("schema JSON"),
        });
    }
    ModelPlan {
        symbols,
        diagnostics: planner.diagnostics,
        fields,
        openapi_version: contract.openapi_version().into(),
        declarations,
    }
}

struct Planner<'a> {
    contract: &'a Contract,
    resources: bool,
    names: BTreeMap<SchemaId, String>,
    nullable: BTreeMap<SchemaId, bool>,
    diagnostics: Vec<ModelDiagnostic>,
    policy: schema_view::DialectPolicy,
}

/// Select the resource profile from indexed effective schema context. Ordinary
/// closures keep their established v1/v2 validation programs and native types.
pub(crate) fn requires_resources(contract: &Contract, closure: &[SchemaId]) -> bool {
    closure.iter().any(|id| {
        contract.schema(id).is_some_and(|schema| {
            !schema.ignores_ref_siblings()
                && (contract
                    .resource_scope(id)
                    .is_some_and(|scope| scope.base_source().is_some())
                    || ["$id", "$anchor", "$dynamicAnchor", "$dynamicRef"]
                        .iter()
                        .any(|keyword| schema.raw().get(keyword).is_some()))
        })
    })
}

// When shared nullability cannot decide a conditional, local type/literal
// restrictions and unconditional static references/allOf still exclude null.
// Other applicators remain runtime-checked. This bounded overapproximation never
// turns a possibly valid null into an unrepresentable native value.
fn may_allow_null(
    contract: &Contract,
    root: &SchemaId,
    policy: schema_view::DialectPolicy,
) -> bool {
    let mut pending = vec![root.clone()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if seen.len() >= 1024 {
            return true;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(schema) = contract.schema(&id) else {
            continue;
        };
        if !schema_view::accepts_literal(schema, &Value::Null, policy) {
            return false;
        }
        pending.extend(
            schema
                .references()
                .iter()
                .filter(|reference| reference.keyword == "$ref")
                .filter_map(|reference| reference.target.clone()),
        );
        if let Some(branches) = schema_view::raw(schema)
            .get("allOf")
            .and_then(Value::as_array)
        {
            pending.extend(
                (0..branches.len()).map(|index| id.child("allOf").child(&index.to_string())),
            );
        }
    }
    true
}

#[derive(Debug, Clone)]
pub(crate) enum PyType {
    Primitive(&'static str),
    JsonValue,
    Named(SchemaId),
    Nullable(Box<Self>),
    Optional(Box<Self>),
    List(Box<Self>),
    Map(Box<Self>),
    Union(Vec<(SchemaId, Self)>),
    Literal(Vec<Value>),
}
impl PyType {
    fn render(&self, names: &BTreeMap<SchemaId, String>) -> String {
        match self {
            // `types.NoneType` is the runtime class, but Python's static type
            // spelling for a null-only value is `None`, including containers.
            Self::Primitive("types.NoneType") => "None".into(),
            Self::Primitive(value) => (*value).into(),
            Self::JsonValue => "_json.JsonValue".into(),
            Self::Named(id) => names[id].clone(),
            Self::Nullable(inner) => format!("{} | None", inner.render(names)),
            Self::Optional(inner) => format!("{} | Unset", inner.render(names)),
            Self::List(inner) => format!("list[{}]", inner.render(names)),
            Self::Map(inner) => format!("dict[str, {}]", inner.render(names)),
            Self::Union(types) => {
                let types = types
                    .iter()
                    .map(|(_, ty)| ty.render(names))
                    .collect::<Vec<_>>();
                // An alias expression `None | None` is invalid at runtime.
                // typing.Union normalizes repeated null branches without
                // changing the validator's independent oneOf/anyOf checks.
                if types.iter().any(|ty| ty == "None") {
                    format!("typing.Union[{}]", types.join(", "))
                } else {
                    types.join(" | ")
                }
            }
            Self::Literal(values) => format!(
                "typing.Literal[{}]",
                values
                    .iter()
                    .map(python_literal)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    fn targets(&self, result: &mut BTreeSet<SchemaId>) {
        match self {
            Self::Named(id) => {
                result.insert(id.clone());
            }
            Self::Nullable(inner)
            | Self::Optional(inner)
            | Self::List(inner)
            | Self::Map(inner) => {
                inner.targets(result);
            }
            Self::Union(types) => {
                for (id, ty) in types {
                    result.insert(id.clone());
                    ty.targets(result);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub(crate) enum PyDecl {
    Alias(PyType),
    Dataclass {
        fields: Vec<PyField>,
        extras: Option<PyType>,
    },
}
#[derive(Debug)]
pub(crate) struct PyField {
    pub(crate) name: String,
    pub(crate) wire: String,
    pub(crate) ty: PyType,
    pub(crate) required: bool,
    pub(crate) source: SchemaId,
    pub(crate) fixed: Option<Value>,
}

impl PyDecl {
    /// Every distinct schema identity this declaration depends on, for
    /// branch-validation root collection.
    pub(crate) fn collect_validation_roots(&self, result: &mut Vec<SchemaId>) {
        match self {
            Self::Alias(ty) => {
                let mut targets = BTreeSet::new();
                ty.targets(&mut targets);
                result.extend(targets);
            }
            Self::Dataclass { fields, .. } => {
                for field in fields {
                    let mut targets = BTreeSet::new();
                    field.ty.targets(&mut targets);
                    result.extend(targets);
                    result.push(field.source.clone());
                }
            }
        }
    }
}

impl PyDecl {
    fn render(
        &self,
        name: &str,
        names: &BTreeMap<SchemaId, String>,
        raw: &Value,
        source: &SchemaId,
    ) -> String {
        match self {
            Self::Alias(PyType::Primitive("types.NoneType")) => format!(
                "if typing.TYPE_CHECKING:\n    {name}: typing.TypeAlias = None\nelse:\n    {name} = types.NoneType\n"
            ),
            Self::Alias(ty) => format!("{name} = {}\n", ty.render(names)),
            Self::Dataclass { fields, extras } => {
                let mut code = format!("@dataclasses.dataclass(kw_only=True)\nclass {name}:\n");
                let description = raw
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("No source description.");
                let description = format!(
                    "{description} Source: {}#{}.",
                    source.document(),
                    source.pointer()
                );
                code.push_str(&format!(
                    "    {}\n",
                    serde_json::to_string(&description).unwrap()
                ));
                for field in fields {
                    let ty = field.ty.render(names);
                    if let Some(value) = &field.fixed {
                        code.push_str(&format!(
                            "    {}: {ty} = dataclasses.field(init=False, default={})\n",
                            field.name,
                            python_literal(value)
                        ));
                    } else if field.required {
                        code.push_str(&format!("    {}: {ty}\n", field.name));
                    } else {
                        code.push_str(&format!("    {}: {ty} = UNSET\n", field.name));
                    }
                }
                if let Some(extra) = extras {
                    let extra_type = extra.render(names);
                    let wire_names = fields.iter().map(|f| f.wire.as_str()).collect::<Vec<_>>();
                    code.push_str(&format!("    _extra_fields: dict[str, {extra_type}] = dataclasses.field(default_factory=dict, init=False, repr=False)\n"));
                    code.push_str(&format!("    def set_extra(self, key: str, value: {extra_type}) -> None:\n        if type(key) is not str or key in {}:\n            raise ValueError('extra key must be an undeclared wire name')\n        self._extra_fields[key] = value\n    @property\n    def extra_fields(self) -> typing.Mapping[str, {extra_type}]:\n        return types.MappingProxyType(self._extra_fields)\n", serde_json::to_string(&wire_names).unwrap()));
                }
                code
            }
        }
    }
    fn alias_targets(&self) -> BTreeSet<SchemaId> {
        let mut targets = BTreeSet::new();
        if let Self::Alias(ty) = self {
            ty.targets(&mut targets);
        }
        targets
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
    fn lower_decl(&mut self, id: &SchemaId) -> PyDecl {
        let Some(schema) = self.contract.schema(id) else {
            return PyDecl::Alias(PyType::JsonValue);
        };
        match schema.raw() {
            Value::Bool(true) => return PyDecl::Alias(PyType::JsonValue),
            Value::Bool(false) => {
                return PyDecl::Alias(PyType::Primitive("typing.Never"));
            }
            _ => {}
        }
        let semantic = schema_view::raw(schema);
        let Some(raw) = semantic.as_object() else {
            self.report(
                id,
                "invalid-schema",
                DiagnosticKind::Error,
                "schema is neither an object nor Boolean",
            );
            return PyDecl::Alias(PyType::JsonValue);
        };
        if self.resources {
            // A nested dynamic reference is interpreted in its enclosing
            // resource stack. Convert one complete root carrier after that root
            // has been checked, without validating a child as a detached model.
            return PyDecl::Alias(if schema_types(raw) == ["null"] {
                PyType::Primitive("types.NoneType")
            } else {
                self.json_carrier(id, raw)
            });
        }
        for keyword in ["oneOf", "anyOf"] {
            if let Some(members) = raw.get(keyword).and_then(Value::as_array) {
                let types = (0..members.len())
                    .map(|index| {
                        let child = id.child(keyword).child(&index.to_string());
                        let ty = self.field_type(&child);
                        (child, ty)
                    })
                    .collect();
                return PyDecl::Alias(PyType::Union(types));
            }
        }
        let literals = raw
            .get("const")
            .map(|value| vec![value.clone()])
            .or_else(|| raw.get("enum").and_then(Value::as_array).cloned());
        if let Some(mut values) = literals {
            values.retain(|value| schema_view::accepts_literal(schema, value, self.policy));
            if values.is_empty() {
                return PyDecl::Alias(PyType::Primitive("typing.Never"));
            }
            if values.iter().all(|v| {
                v.is_null()
                    || v.is_string()
                    || v.is_boolean()
                    || v.as_number().is_some_and(|n| {
                        n.to_string()
                            .bytes()
                            .all(|b| b.is_ascii_digit() || b == b'-')
                    })
            }) {
                return PyDecl::Alias(PyType::Literal(values));
            }
        }
        if raw.contains_key("$ref") {
            if raw.keys().any(|key| !annotation(key) && key != "$ref") {
                // Assertion siblings remain attached to this source root. A
                // checked JSON carrier avoids discarding values while trying
                // to project a native class from only the reference target.
                return PyDecl::Alias(self.json_carrier(id, raw));
            }
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone());
            let ty = match target {
                Some(target) => PyType::Named(target),
                None => {
                    self.report(
                        &id.child("$ref"),
                        "unresolved-model-reference",
                        DiagnosticKind::Error,
                        "reference has no canonical target",
                    );
                    PyType::JsonValue
                }
            };
            return PyDecl::Alias(self.wrap_nullable(id, ty));
        }
        let mut types: Vec<String> = schema_types(raw);
        if types == ["null"] {
            return PyDecl::Alias(PyType::Primitive("types.NoneType"));
        }
        types.retain(|t| t != "null");
        if types.contains(&"number".to_owned()) {
            types.retain(|t| t != "integer");
        }
        if types.len() > 1 {
            self.report(
                &id.child("type"),
                "python-checked-json-carrier",
                DiagnosticKind::Annotation,
                "multiple native domains use the exact JSON carrier and source validation",
            );
            return PyDecl::Alias(PyType::JsonValue);
        }
        let ty = match types.first().map(String::as_str) {
            Some("object") => return self.object_decl(id, raw),
            Some("array") => PyType::List(Box::new(if raw.contains_key("prefixItems") {
                // Tuple/prefix and residual item assertions are checked by the
                // source program; a homogeneous items-only type would narrow
                // valid prefix values incorrectly.
                PyType::JsonValue
            } else if raw.contains_key("items") {
                self.field_type(&id.child("items"))
            } else {
                PyType::JsonValue
            })),
            Some("string") => PyType::Primitive("str"),
            Some("boolean") => PyType::Primitive("bool"),
            Some("integer") => PyType::Primitive("int"),
            Some("number") => PyType::Primitive("_json.JsonNumber"),
            None => PyType::JsonValue,
            _ => {
                self.report(
                    &id.child("type"),
                    "invalid-model-type",
                    DiagnosticKind::Error,
                    "unsupported or invalid schema type",
                );
                PyType::JsonValue
            }
        };
        PyDecl::Alias(self.wrap_nullable(id, ty))
    }
}

impl Planner<'_> {
    fn json_carrier(&self, id: &SchemaId, raw: &Map<String, Value>) -> PyType {
        let types = schema_types(raw);
        let domains = types
            .iter()
            .filter(|ty| ty.as_str() != "null")
            .map(String::as_str)
            .collect::<Vec<_>>();
        let ty = match domains.as_slice() {
            ["object"] => PyType::Map(Box::new(PyType::JsonValue)),
            ["array"] => PyType::List(Box::new(PyType::JsonValue)),
            ["string"] => PyType::Primitive("str"),
            ["boolean"] => PyType::Primitive("bool"),
            ["integer"] => PyType::Primitive("int"),
            ["number"] => PyType::Primitive("_json.JsonNumber"),
            _ => PyType::JsonValue,
        };
        self.wrap_nullable(id, ty)
    }
    fn check_shape(&mut self, id: &SchemaId) {
        let schema = self.contract.schema(id).expect("reachable schema");
        let value = schema_view::raw(schema);
        let Some(raw) = value.as_object() else {
            return;
        };
        if let Some(kind) = raw.get("type") {
            let types = schema_types(raw);
            let valid = match kind {
                Value::String(_) => types.len() == 1,
                Value::Array(values) => {
                    !values.is_empty()
                        && values.len() == types.len()
                        && types.iter().collect::<BTreeSet<_>>().len() == types.len()
                }
                _ => false,
            };
            if !valid
                || types.iter().any(|ty| {
                    !matches!(
                        ty.as_str(),
                        "object" | "array" | "string" | "boolean" | "integer" | "number" | "null"
                    )
                })
            {
                self.report(
                    &id.child("type"),
                    "invalid-model-type",
                    DiagnosticKind::Error,
                    "type must contain valid distinct schema type names",
                );
            }
            if types.iter().any(|ty| ty == "object")
                && self.nullable.get(id) == Some(&true)
                && raw
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|properties| !properties.is_empty())
            {
                self.report(
                    id,
                    "python-checked-json-carrier",
                    DiagnosticKind::Annotation,
                    "nullable object values use an exact checked dictionary carrier",
                );
            }
        }
        if let Some(required) = raw.get("required") {
            let valid = required.as_array().is_some_and(|values| {
                values.iter().all(Value::is_string)
                    && values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == values.len()
            });
            if !valid {
                self.report(
                    &id.child("required"),
                    "invalid-model-required",
                    DiagnosticKind::Error,
                    "required must be an array of unique strings",
                );
            }
        }
        for keyword in ["$dynamicRef", "$recursiveRef"] {
            if raw.contains_key(keyword)
                && !(keyword == "$dynamicRef"
                    && self.resources
                    && self.contract.dynamic_reference(id).is_some())
            {
                self.report(
                    &id.child(keyword),
                    "unsupported-python-representation",
                    DiagnosticKind::Error,
                    &format!(
                        "{keyword} lowering is not implemented for Python models in this slice"
                    ),
                );
            }
        }
        if raw.contains_key("$ref") {
            let siblings = raw
                .keys()
                .filter(|key| {
                    !annotation(key) && key.as_str() != "$ref" && key.as_str() != "nullable"
                })
                .count();
            if siblings > 0 {
                self.report(
                    id,
                    "ref-sibling-representation",
                    DiagnosticKind::Annotation,
                    "reference assertion siblings use a source-validated JSON carrier without flattening the target",
                );
            }
        }
        if raw.get("enum").is_some_and(|value| !value.is_array()) {
            self.report(
                &id.child("enum"),
                "invalid-model-enum",
                DiagnosticKind::Error,
                "enum must be an array of source literals",
            );
        }
        if raw.get("type").is_none()
            && !raw.contains_key("$ref")
            && !raw.contains_key("oneOf")
            && !raw.contains_key("anyOf")
            && !raw.contains_key("enum")
            && !raw.contains_key("const")
            && raw.keys().any(|key| !annotation(key))
        {
            self.report(
                id,
                "untyped-constraints-representation",
                DiagnosticKind::Annotation,
                "untyped assertions retain the exact JSON domain and are validated by the source codec; they do not imply an object or scalar type",
            );
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
            "title",
        ] {
            if raw.contains_key(keyword) {
                self.report(
                    &id.child(keyword),
                    "retained-annotation",
                    DiagnosticKind::Annotation,
                    &format!("{keyword} is retained in source documentation; neutral model planning ignores defaults and never names from titles"),
                );
            }
        }
        if matches!(schema.dialect(), SchemaDialect::OpenApi30)
            && raw.get("nullable") == Some(&Value::Bool(true))
            && !raw.contains_key("type")
        {
            self.report(
                &id.child("nullable"),
                "nullable-without-type",
                DiagnosticKind::Annotation,
                "OpenAPI 3.0 nullable only changes a type explicitly declared on the same Schema Object",
            );
        }
    }
    fn field_type(&mut self, id: &SchemaId) -> PyType {
        if self.names.contains_key(id) {
            let named = PyType::Named(id.clone());
            return self.wrap_nullable(id, named);
        }
        match self.lower_decl(id) {
            PyDecl::Alias(ty) => ty,
            PyDecl::Dataclass { .. } => {
                self.report(
                    id,
                    "unplanned-declaration",
                    DiagnosticKind::Error,
                    "this inline representation requires a planned declaration",
                );
                PyType::JsonValue
            }
        }
    }
    fn wrap_nullable(&self, id: &SchemaId, ty: PyType) -> PyType {
        if self.nullable.get(id).copied().unwrap_or(false) {
            PyType::Nullable(Box::new(ty))
        } else {
            ty
        }
    }
    fn object_decl(&mut self, id: &SchemaId, raw: &Map<String, Value>) -> PyDecl {
        let properties = raw.get("properties").and_then(Value::as_object);
        let patterned = raw
            .get("patternProperties")
            .and_then(Value::as_object)
            .is_some_and(|patterns| !patterns.is_empty());
        if self.nullable.get(id) == Some(&true)
            && properties.is_some_and(|properties| !properties.is_empty())
        {
            return PyDecl::Alias(PyType::Nullable(Box::new(PyType::Map(Box::new(
                PyType::JsonValue,
            )))));
        }
        if self.nullable.get(id) == Some(&true) && properties.is_none_or(Map::is_empty) {
            let inner = if patterned {
                PyType::JsonValue
            } else {
                match raw.get("additionalProperties") {
                    Some(Value::Bool(false)) => PyType::Primitive("typing.Never"),
                    Some(value) if value.is_object() => {
                        self.field_type(&id.child("additionalProperties"))
                    }
                    _ => PyType::JsonValue,
                }
            };
            return PyDecl::Alias(PyType::Nullable(Box::new(PyType::Map(Box::new(inner)))));
        }
        if raw.contains_key("properties") && properties.is_none() {
            self.report(
                &id.child("properties"),
                "invalid-model-properties",
                DiagnosticKind::Error,
                "properties must be an object of property schemas",
            );
        }
        let required: BTreeSet<&str> = raw
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for name in &required {
            if !properties.is_some_and(|p| p.contains_key(*name)) {
                self.report(
                    &id.child("required"),
                    "python-checked-json-carrier",
                    DiagnosticKind::Annotation,
                    "required undeclared names use the exact dictionary carrier with source validation",
                );
                return PyDecl::Alias(PyType::Map(Box::new(PyType::JsonValue)));
            }
        }
        let mut fields = Vec::new();
        let mut used = BTreeSet::from([
            "extra_fields".to_owned(),
            "_extra_fields".to_owned(),
            "set_extra".to_owned(),
        ]);
        for (wire, _) in properties.into_iter().flatten() {
            let source = id.child("properties").child(wire);
            let core = self.field_type(&source);
            let is_required = required.contains(wire.as_str());
            fields.push(PyField {
                name: allocate_local(&python_name(wire), &mut used),
                wire: wire.clone(),
                ty: if is_required {
                    core
                } else {
                    PyType::Optional(Box::new(core))
                },
                required: is_required,
                source,
                fixed: if is_required {
                    self.contract
                        .schema(&id.child("properties").child(wire))
                        .and_then(|schema| schema_view::raw(schema).get("const").cloned())
                        .filter(|value| !value.is_array() && !value.is_object())
                } else {
                    None
                },
            });
        }
        PyDecl::Dataclass {
            fields,
            // Patterns may admit values independently of additionalProperties.
            // Keep every undeclared value in the exact JSON carrier; the same
            // source program applies all matching patterns and residual rules.
            extras: if patterned {
                Some(PyType::JsonValue)
            } else {
                match raw.get("additionalProperties") {
                    Some(Value::Bool(false)) => None,
                    Some(value) if value.is_object() => {
                        Some(self.field_type(&id.child("additionalProperties")))
                    }
                    _ => Some(PyType::JsonValue),
                }
            },
        }
    }
}

fn python_literal(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number)
            if number
                .to_string()
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-') =>
        {
            number.to_string()
        }
        Value::Number(number) => format!("_json.JsonNumber({:?})", number.to_string()),
        Value::String(value) => serde_json::to_string(value).expect("Python string"),
        _ => unreachable!("compound literals rejected during model admission"),
    }
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

fn allocate_names(
    contract: &Contract,
    declared: &BTreeSet<SchemaId>,
) -> BTreeMap<SchemaId, String> {
    let hints = crate::model_naming::Hints::new(contract);
    let mut used = BTreeSet::from([
        "Unset".to_owned(),
        "None".to_owned(),
        "True".to_owned(),
        "False".to_owned(),
    ]);
    let mut names = BTreeMap::new();
    for id in declared {
        let base = hints
            .get(id)
            .map_or_else(|| source_name(id), |name| pascal(&name));
        let mut name = base.clone();
        let mut suffix = 2;
        while !used.insert(name.clone()) {
            name = format!("{base}{suffix}");
            suffix += 1;
        }
        names.insert(id.clone(), name);
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
            "get" | "put" | "post" | "delete" | "patch" => token.to_ascii_uppercase(),
            _ => token.clone(),
        });
    }
    pascal(&parts.join("_"))
}

fn python_name(wire: &str) -> String {
    let mut name = snake(wire);
    if PY_KEYWORDS.contains(&name.as_str()) {
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

fn reject_alias_cycles(declarations: &BTreeMap<SchemaId, PyDecl>, planner: &mut Planner<'_>) {
    fn cycle(
        id: &SchemaId,
        declarations: &BTreeMap<SchemaId, PyDecl>,
        active: &mut BTreeSet<SchemaId>,
        done: &mut BTreeSet<SchemaId>,
    ) -> bool {
        if done.contains(id) {
            return false;
        }
        let Some(declaration) = declarations.get(id) else {
            return false;
        };
        if !active.insert(id.clone()) {
            return true;
        }
        let targets = declaration.alias_targets();
        let found = targets
            .iter()
            .any(|target| cycle(target, declarations, active, done));
        active.remove(id);
        done.insert(id.clone());
        found
    }
    let mut done = BTreeSet::new();
    for id in declarations.keys() {
        if cycle(id, declarations, &mut BTreeSet::new(), &mut done) {
            planner.report(
                id,
                "recursive-type-alias",
                DiagnosticKind::Error,
                "an alias-only cycle evaluates eagerly at Python import time; a source object must guard recursion",
            );
        }
    }
}

fn declaration_order(declarations: &BTreeMap<SchemaId, PyDecl>) -> Vec<SchemaId> {
    let mut order = Vec::new();
    let mut visited = BTreeSet::new();
    for root in declarations.keys() {
        let mut pending = vec![(root.clone(), false)];
        while let Some((id, ready)) = pending.pop() {
            if ready {
                order.push(id);
                continue;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            pending.push((id.clone(), true));
            for target in declarations[&id].alias_targets().into_iter().rev() {
                if declarations.contains_key(&target) {
                    pending.push((target, false));
                }
            }
        }
    }
    order
}

// Emit the standalone package: models.py, the audited runtime template,
// an __init__ marker, reviewable docs and a source-addressed manifest.
pub(crate) fn render_codecs(plan: &ModelPlan) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
    if plan.has_errors() {
        return Err(plan.diagnostics.clone());
    }
    Ok(render(plan, true))
}
fn render(plan: &ModelPlan, codecs: bool) -> Vec<OutFile> {
    let mut models = String::from(
        "\"\"\"Generated Python models: source-addressed dataclass views.\n\
         \n\
         Native model construction preserves source field shapes. Associated\n\
         source codecs enforce schema constraints on decoding and mutable\n\
         encoding. Exact JSON numbers are represented by\n\
         ``json_runtime.JsonNumber`` and never by ``float``. Optional fields\n\
         start as the ``UNSET`` sentinel, distinct from ``None``; ``None``\n\
         appears only where the source schema admits null.\"\"\"\n\
         from __future__ import annotations\n\
         \n\
          import dataclasses\n\
          import typing\n\
          import types\n\
          \n\
          if typing.TYPE_CHECKING or __package__:\n\
          \x20   from . import json_runtime as _json\n\
          else:\n\
          \x20   import json_runtime as _json\n\
         \n\
         \n\
         class Unset:\n\
         \x20   \"\"\"Distinct type for optional absence; compare with ``UNSET``.\"\"\"\n\
         \n\
         \x20   __slots__ = ()\n\
         \n\
         \x20   def __repr__(self) -> str:\n\
         \x20       return \"UNSET\"\n\
         \n\
         \n\
         UNSET = Unset()\n\
         \n\
         \n",
    );
    for symbol in &plan.symbols {
        models.push_str(&format!(
            "# Source: {}\n",
            serde_json::to_string(&format!(
                "{}#{}",
                symbol.source().document(),
                symbol.source().pointer()
            ))
            .unwrap()
        ));
        models.push_str(&symbol.code);
        models.push('\n');
    }
    let names: Vec<&str> = plan.symbols.iter().map(|s| s.name()).collect();
    models.push_str(&format!(
        "\n__all__ = {}\n",
        serde_json::to_string(
            &names
                .iter()
                .copied()
                .chain(["UNSET", "Unset"])
                .collect::<Vec<_>>()
        )
        .unwrap()
    ));
    let manifest = serde_json::to_string_pretty(&serde_json::json!({
        "package": plan.package(),
        "openapi_version": plan.openapi_version(),
        "release_ready": plan.release_ready(),
        "model_only": !codecs,
        "source_codecs": codecs,
        "codec_module": codecs.then_some("model_codecs.py"),
        "python_floor": "3.11",
        "runtime": "python/json_runtime.py",
        "declarations": plan.symbols.iter().map(|s| serde_json::json!({
            "name": s.name(),
            "file": s.file(),
            "role": format!("{:?}", s.role()),
            "source": {"document": s.source().document().to_string(), "pointer": s.source().pointer()},
        })).collect::<Vec<_>>(),
        "fields": plan.fields.iter().map(|f| serde_json::json!({
            "model": f.model,
            "field": f.field,
            "wire": f.wire,
            "required": f.required,
            "nullable": f.nullable,
            "source": {"document": f.source.document().to_string(), "pointer": f.source.pointer()},
        })).collect::<Vec<_>>(),
    }))
    .expect("manifest JSON");
    let init = format!(
        "\"\"\"Generated source-addressed models and exact value types.\"\"\"\n\
         \n\
          from .models import *\n\
          \n\
          __all__ = [{}]\n",
        names
            .iter()
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mode = if codecs {
        "Source-bound model codecs are included in `model_codecs.py`. Decode validates exact JSON before native conversion; encode converts and validates again. HTTP and native package assembly use the separate HTTP plan."
    } else {
        "This model-only artifact set retains a source-linked missing-codec obligation for every selected root. Use the canonical codec plan to add schema validation and wire conversion."
    };
    let readme = format!(
        "# Generated Python models\n\n\
         Source-addressed model plan for OpenAPI {} (`{}`). {mode}\n\n\
         `release_ready` records complete SDK acceptance and remains false.\n\n\
         ## Layout\n\n\
         - `models.py`: source-addressed `@dataclasses.dataclass(kw_only=True)` views.\n\
         - `json_runtime.py`: audited exact-JSON runtime (single template source).\n\
         - `manifest.json`: every declaration and field mapped to its source address.\n\n\
         ## Semantics\n\n\
         - Exact numbers are `json_runtime.JsonNumber`; `int` is exact in Python.\n\
         - Optional absence is the `UNSET` sentinel; `None` only where the source\n\
           schema admits null (`| None`).\n\
         - Open objects keep unknown wire properties in typed private extra storage\n\
           with a read-only `extra_fields` view and collision-checked setter; closed\n\
           objects (`additionalProperties: false`) omit that storage.\n\
         - `from __future__ import annotations` makes every annotation lazy, so\n\
           mutual references between classes are safe forward references.\n\
         - Schema `default` values are ignored; titles never drive naming.\n\
         - Supported unions use native alternatives and source-bound codecs;\n\
           `oneOf` exclusivity remains a runtime assertion. Literal tags are source\n\
           constants. Unsupported intersections/representations produce diagnostics.\n\n\
         ## Floor and gates\n\n\
         The declared interpreter floor is Python 3.11. Native type checking,\n\
         installed consumers and documentation checks remain required for every\n\
         promoted package profile. Generation alone is not complete acceptance.\n",
        plan.openapi_version(),
        plan.package()
    );
    vec![
        OutFile {
            path: "python/models.py".into(),
            content: models,
        },
        OutFile {
            path: "python/json_runtime.py".into(),
            content: python_json::runtime_source().into(),
        },
        OutFile {
            path: "python/__init__.py".into(),
            content: init,
        },
        OutFile {
            path: "python/README.md".into(),
            content: readme,
        },
        OutFile {
            path: "python/manifest.json".into(),
            content: manifest,
        },
    ]
}
