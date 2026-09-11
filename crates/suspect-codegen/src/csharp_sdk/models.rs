//! Native C# representation planning. Validation and null-domain proofs come
//! from the owned compiler; this module never reconstructs validation rules.
use super::{HttpDiagnostic, allocate, diagnostic, exported};
pub use crate::rust_models::RepresentationRole;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{Contract, SchemaId};
pub type Key = (SchemaId, RepresentationRole);

/// Native type expression. Optional presence is a property concern, separate
/// from `Nullable`, which represents a value-domain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsType {
    Native(&'static str),
    Number,
    Integer,
    Json,
    Named(Key),
    Nullable(Box<CsType>),
    List(Box<CsType>),
    Dict(Box<CsType>),
}
#[derive(Debug, Clone)]
pub enum CsDecl {
    Record {
        fields: Vec<CsField>,
        extras: Option<CsType>,
    },
    /// Erased C# aliases retain a distinct source-aware codec, not a wrapper object.
    Alias(CsType),
    Literals {
        values: Vec<(String, String)>,
    },
    Union {
        branches: Vec<CsBranch>,
    },
}
#[derive(Debug, Clone)]
pub struct CsBranch {
    pub name: String,
    pub source: SchemaId,
    pub ty: CsType,
}
#[derive(Debug, Clone)]
pub struct CsField {
    pub name: String,
    pub wire: String,
    pub ty: CsType,
    pub required: bool,
    pub nullable: bool,
    pub source: SchemaId,
    pub description: String,
}
/// Actual C# object-initializer obligation, independently of wire requiredness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldInitialization {
    /// The generated property carries the C# `required` modifier.
    Required,
    /// The property is `Optional<T>` and starts absent.
    Absent,
    /// A source-required singleton tag is initialized to this allocated enum member.
    Singleton {
        symbol: Key,
        member: String,
        token: String,
    },
}
#[derive(Debug)]
pub struct ModelPlan {
    pub names: BTreeMap<Key, String>,
    pub declarations: BTreeMap<Key, CsDecl>,
    pub(crate) nullable: BTreeMap<SchemaId, bool>,
    deprecated: BTreeSet<Key>,
}
impl ModelPlan {
    /// Whether emission marks this actual declaration with `System.Obsolete`.
    #[must_use]
    pub fn is_deprecated(&self, key: &Key) -> bool {
        self.deprecated.contains(key) && !matches!(self.declarations[key], CsDecl::Alias(_))
    }
    /// Construction rule shared by emission and native compatibility capture.
    #[must_use]
    pub fn field_initialization(&self, field: &CsField) -> FieldInitialization {
        if !field.required {
            return FieldInitialization::Absent;
        }
        self.singleton(&field.ty)
            .unwrap_or(FieldInitialization::Required)
    }
    fn singleton(&self, ty: &CsType) -> Option<FieldInitialization> {
        let CsType::Named(key) = ty else {
            return None;
        };
        if self.nullable[&key.0] {
            return None;
        }
        match &self.declarations[key] {
            CsDecl::Alias(inner) => self.singleton(inner),
            CsDecl::Literals { values } if values.len() == 1 => {
                Some(FieldInitialization::Singleton {
                    symbol: key.clone(),
                    member: values[0].0.clone(),
                    token: values[0].1.clone(),
                })
            }
            _ => None,
        }
    }
    /// Allocated codec/declaration names keyed by stable source and representation role.
    #[must_use]
    pub fn names(&self) -> &BTreeMap<Key, String> {
        &self.names
    }
    /// Retained native fields, aliases, literal tokens and sealed union branches.
    #[must_use]
    pub fn declarations(&self) -> &BTreeMap<Key, CsDecl> {
        &self.declarations
    }
    /// Native null domain. Context-sensitive v3 schemas use a conservative
    /// representation; exact membership is checked under the active resources.
    #[must_use]
    pub fn is_nullable(&self, id: &SchemaId) -> Option<bool> {
        self.nullable.get(id).copied()
    }
    /// Native type of an admitted public codec root (aliases are erased).
    #[must_use]
    pub fn native_type(&self, id: &SchemaId) -> String {
        self.render_type(&CsType::Named(key(id)))
    }
    /// Suffix shared by Decode/Encode methods for this exact source root.
    #[must_use]
    pub fn codec_name(&self, id: &SchemaId) -> &str {
        &self.names[&key(id)]
    }
    /// Whether this source codec uses a JSON-value carrier instead of a nominal
    /// model. The carrier is still validated on both decode and mutable encode.
    #[must_use]
    pub fn is_json_carrier(&self, id: &SchemaId) -> bool {
        fn carrier(plan: &ModelPlan, ty: &CsType) -> bool {
            match ty {
                CsType::Json => true,
                CsType::Named(key) => match &plan.declarations[key] {
                    CsDecl::Alias(inner) => carrier(plan, inner),
                    _ => false,
                },
                _ => false,
            }
        }
        carrier(self, &CsType::Named(key(id)))
    }
    #[must_use]
    pub fn render_type(&self, ty: &CsType) -> String {
        self.render_in(ty, None)
    }
    pub(crate) fn qualified_type(&self, ty: &CsType, namespace: &str) -> String {
        self.render_in(ty, Some(namespace))
    }
    fn render_in(&self, ty: &CsType, namespace: Option<&str>) -> String {
        let nominal = |name: &str| {
            namespace.map_or_else(|| name.to_owned(), |ns| format!("global::{ns}.{name}"))
        };
        match ty {
            CsType::Native(t @ ("Never" | "JsonNull")) => nominal(t),
            CsType::Native(t) => (*t).into(),
            CsType::Number => nominal("JsonNumber"),
            CsType::Integer => nominal("JsonInteger"),
            CsType::Json => "global::System.Text.Json.JsonElement".into(),
            CsType::Nullable(inner) => {
                let t = self.render_in(inner, namespace);
                if t.ends_with('?') { t } else { format!("{t}?") }
            }
            CsType::Named(k) => match &self.declarations[k] {
                CsDecl::Alias(inner) => self.render_in(inner, namespace),
                _ => format!(
                    "{}{}",
                    nominal(&self.names[k]),
                    if self.nullable[&k.0] { "?" } else { "" }
                ),
            },
            CsType::List(inner) => format!(
                "global::System.Collections.Generic.List<{}>",
                self.render_in(inner, namespace)
            ),
            CsType::Dict(inner) => format!(
                "global::System.Collections.Generic.Dictionary<string, {}>",
                self.render_in(inner, namespace)
            ),
        }
    }
}
pub(crate) fn key(id: &SchemaId) -> Key {
    (id.clone(), RepresentationRole::Model)
}
pub(crate) fn plan(
    contract: &Contract,
    roots: &[SchemaId],
    nullable: BTreeMap<SchemaId, bool>,
    scoped: bool,
    resources: bool,
    sensitive: BTreeSet<SchemaId>,
) -> Result<ModelPlan, Vec<HttpDiagnostic>> {
    let reachable = if resources {
        contract.effective_schema_closure(roots)
    } else {
        contract.reachable_from(roots)
    };
    let mut declared: BTreeSet<_> = roots.iter().cloned().collect();
    for id in &reachable {
        let schema = contract.schema(id).expect("compiled schema");
        if schema.raw().as_object().is_some_and(|r| {
            types(r).iter().any(|t| t == "object")
                || ["oneOf", "anyOf", "enum", "const"]
                    .iter()
                    .any(|k| r.contains_key(*k))
        }) {
            declared.insert(id.clone());
        }
        for reference in schema.references() {
            if let Some(target) = &reference.target {
                declared.insert(target.clone());
            }
        }
    }
    let mut used: BTreeSet<_> = super::reserved_types()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let hints = crate::model_naming::Hints::new(contract);
    let names = declared
        .iter()
        .map(|id| {
            (
                key(id),
                allocate(&source_name(contract, id, &hints), &mut used),
            )
        })
        .collect();
    let mut planner = Planner {
        contract,
        names,
        nullable,
        errors: Vec::new(),
        scoped,
        resources,
        sensitive,
    };
    for id in &reachable {
        planner.check_shape(id);
    }
    let declarations: BTreeMap<_, _> = declared
        .iter()
        .map(|id| (key(id), planner.lower(id)))
        .collect();
    reject_alias_cycles(&declarations, &mut planner);
    if !planner.errors.is_empty() {
        return Err(planner.errors);
    }
    Ok(ModelPlan {
        names: planner.names,
        declarations,
        nullable: planner.nullable,
        deprecated: declared
            .iter()
            .filter(|id| {
                contract.source(id).and_then(|v| v.get("deprecated")) == Some(&Value::Bool(true))
            })
            .map(key)
            .collect(),
    })
}
struct Planner<'a> {
    contract: &'a Contract,
    names: BTreeMap<Key, String>,
    nullable: BTreeMap<SchemaId, bool>,
    errors: Vec<HttpDiagnostic>,
    scoped: bool,
    resources: bool,
    sensitive: BTreeSet<SchemaId>,
}
impl Planner<'_> {
    fn report(&mut self, id: &SchemaId, code: &'static str, message: &str) {
        self.errors
            .push(diagnostic(self.contract, id.clone(), code, message));
    }
    fn check_shape(&mut self, id: &SchemaId) {
        // The checked v2 codec can retain native JSON for conditional domains,
        // tuple/intersection shapes and untyped constraints. No assertion is
        // erased: the full source program guards both conversions.
        if self.scoped {
            return;
        }
        let Some(raw) = self.contract.schema(id).and_then(|s| s.raw().as_object()) else {
            return;
        };
        for keyword in ["allOf", "not", "prefixItems"] {
            if raw.contains_key(keyword) {
                self.report(&id.child(keyword), "csharp-representation-unsupported", &format!("{keyword} has portable validation, but its native C# intersection/tuple representation is not implemented"));
            }
        }
        if raw.contains_key("$ref") && raw.keys().any(|k| k != "$ref" && !annotation(k)) {
            self.report(
                id,
                "csharp-ref-sibling-unsupported",
                "reference assertion siblings require a native intersection representation",
            );
        }
        if (raw.contains_key("oneOf") || raw.contains_key("anyOf"))
            && (raw
                .keys()
                .any(|k| !matches!(k.as_str(), "oneOf" | "anyOf") && !annotation(k))
                || (raw.contains_key("oneOf") && raw.contains_key("anyOf")))
        {
            self.report(
                id,
                "csharp-union-intersection-unsupported",
                "union assertion siblings require a native intersection representation",
            );
        }
        if !raw.contains_key("type")
            && !["$ref", "oneOf", "anyOf", "enum", "const"]
                .iter()
                .any(|k| raw.contains_key(*k))
            && raw.keys().any(|k| !annotation(k))
        {
            self.report(id, "csharp-untyped-constraints-unsupported", "untyped assertions do not imply a native type; conditional-domain model lowering is not implemented");
        }
    }
    fn full_type(&mut self, id: &SchemaId) -> CsType {
        if self.names.contains_key(&key(id)) {
            return CsType::Named(key(id));
        }
        match self.lower(id) {
            CsDecl::Alias(ty) => ty,
            _ => {
                self.report(
                    id,
                    "csharp-unplanned-declaration",
                    "inline model has no allocated declaration",
                );
                CsType::Native("Never")
            }
        }
    }
    fn wrap(&self, id: &SchemaId, ty: CsType) -> CsType {
        if self.nullable[id] && !matches!(ty, CsType::Json | CsType::Native("JsonNull")) {
            CsType::Nullable(Box::new(ty))
        } else {
            ty
        }
    }
    fn lower(&mut self, id: &SchemaId) -> CsDecl {
        let schema = self.contract.schema(id).expect("compiled schema");
        let raw = match schema.raw() {
            Value::Bool(true) => return CsDecl::Alias(CsType::Json),
            Value::Bool(false) => return CsDecl::Alias(CsType::Native("Never")),
            Value::Object(raw) => raw,
            _ => unreachable!("compiler checked schema"),
        };
        if self.resources && schema.ignores_ref_siblings() {
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone())
                .expect("compiled reference");
            return CsDecl::Alias(self.full_type(&target));
        }
        if self.resources
            && (raw.contains_key("$dynamicRef")
                || self.sensitive.contains(id)
                    && (raw.contains_key("oneOf") || raw.contains_key("anyOf")))
        {
            // Never substitute the static initial target's native type. Union
            // membership trials would also discard the enclosing dynamic scope.
            return CsDecl::Alias(CsType::Json);
        }
        if self.scoped && carrier_shape(raw) {
            return CsDecl::Alias(CsType::Json);
        }
        if raw.contains_key("$ref") {
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone())
                .expect("compiled reference");
            return CsDecl::Alias(self.full_type(&target));
        }
        for keyword in ["oneOf", "anyOf"] {
            if let Some(members) = raw.get(keyword).and_then(Value::as_array) {
                let mut used = BTreeSet::from([
                    self.names[&key(id)].clone(),
                    "Value".into(),
                    "Equals".into(),
                    "GetHashCode".into(),
                    "ToString".into(),
                    "GetType".into(),
                    "MemberwiseClone".into(),
                    "Finalize".into(),
                ]);
                let mut branches = Vec::new();
                for (index, member) in members.iter().enumerate() {
                    if only_null(member) {
                        continue;
                    }
                    let source = id.child(keyword).child(&index.to_string());
                    let label = member
                        .get("$ref")
                        .and_then(Value::as_str)
                        .map(|r| exported(r.rsplit('/').next().unwrap_or("Branch")))
                        .unwrap_or_else(|| format!("Variant{}", index + 1));
                    branches.push(CsBranch {
                        name: allocate(&label, &mut used),
                        ty: self.full_type(&source),
                        source,
                    });
                }
                return if branches.is_empty() {
                    CsDecl::Alias(CsType::Native("JsonNull"))
                } else {
                    CsDecl::Union { branches }
                };
            }
        }
        let literals = raw.get("const").map(|v| vec![v]).or_else(|| {
            raw.get("enum")
                .and_then(Value::as_array)
                .map(|v| v.iter().collect::<Vec<_>>())
        });
        if let Some(values) = literals {
            let nonnull = values
                .iter()
                .filter(|v| !v.is_null())
                .copied()
                .collect::<Vec<_>>();
            if nonnull.is_empty() {
                return CsDecl::Alias(CsType::Native(if self.nullable[id] {
                    "JsonNull"
                } else {
                    "Never"
                }));
            }
            if nonnull.iter().all(|v| v.is_string() || v.is_boolean()) {
                let mut used = BTreeSet::from([self.names[&key(id)].clone()]);
                let mut seen = BTreeSet::new();
                let values = nonnull
                    .into_iter()
                    .filter_map(|v| {
                        let token = v.to_string();
                        seen.insert(token.clone()).then(|| {
                            (
                                allocate(
                                    &exported(v.as_str().unwrap_or(if v == &Value::Bool(true) {
                                        "True"
                                    } else {
                                        "False"
                                    })),
                                    &mut used,
                                ),
                                token,
                            )
                        })
                    })
                    .collect();
                return CsDecl::Literals { values };
            }
            if nonnull.iter().all(|v| v.is_number()) {
                // Numeric literal membership is mathematical, while the native
                // value retains its spelling (1, 1.0 and 1e0 remain distinct tokens).
                return CsDecl::Alias(self.wrap(
                    id,
                    if types(raw) == ["integer"] {
                        CsType::Integer
                    } else {
                        CsType::Number
                    },
                ));
            }
            if self.scoped {
                return CsDecl::Alias(CsType::Json);
            }
            self.report(id, "csharp-literal-representation-unsupported", "mixed numeric/non-numeric and compound literals require a native literal-union representation");
            return CsDecl::Alias(CsType::Native("Never"));
        }
        let mut ts = types(raw);
        ts.retain(|t| t != "null");
        if ts.contains(&"number".into()) {
            ts.retain(|t| t != "integer");
        }
        if ts.len() > 1 {
            self.report(&id.child("type"), "csharp-type-union-unsupported", "multiple non-null type alternatives require an explicit native union representation");
            return CsDecl::Alias(CsType::Native("Never"));
        }
        let ty = match ts.first().map(String::as_str) {
            Some("object") => return self.object(id, raw),
            Some("array") => CsType::List(Box::new(
                if self.scoped && raw.contains_key("prefixItems") {
                    // `items` constrains only the suffix; it cannot supply the native
                    // value type of heterogeneous prefix positions.
                    CsType::Json
                } else if raw.contains_key("items") {
                    self.full_type(&id.child("items"))
                } else {
                    CsType::Json
                },
            )),
            Some("string") => CsType::Native("string"),
            Some("boolean") => CsType::Native("bool"),
            Some("integer") => CsType::Integer,
            Some("number") => CsType::Number,
            None if raw.contains_key("type") => CsType::Native("JsonNull"),
            None => CsType::Json,
            _ => unreachable!("compiled type"),
        };
        CsDecl::Alias(self.wrap(id, ty))
    }
    fn object(&mut self, id: &SchemaId, raw: &Map<String, Value>) -> CsDecl {
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
                self.report(&id.child("required"), "csharp-undeclared-required-unsupported", "required undeclared properties need a dedicated native construction representation");
            }
        }
        let mut used = BTreeSet::from([
            self.names[&key(id)].clone(),
            "Extra".into(),
            "Equals".into(),
            "GetHashCode".into(),
            "ToString".into(),
            "EqualityContract".into(),
            "Clone".into(),
            "Deconstruct".into(),
            "GetType".into(),
            "MemberwiseClone".into(),
            "Finalize".into(),
        ]);
        let fields = properties
            .into_iter()
            .flatten()
            .map(|(wire, value)| {
                let source = id.child("properties").child(wire);
                CsField {
                    name: allocate(&exported(wire), &mut used),
                    wire: wire.clone(),
                    ty: self.full_type(&source),
                    required: required.contains(wire.as_str()),
                    nullable: self.nullable[&source],
                    source,
                    description: value
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                }
            })
            .collect();
        let extras = if self.scoped
            && raw
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|p| !p.is_empty())
        {
            // Pattern keys can carry different domains from unmatched extras,
            // including when additionalProperties is false. Preserve every key
            // and let the complete scoped program apply all matching assertions.
            Some(CsType::Json)
        } else {
            match raw.get("additionalProperties") {
                Some(Value::Bool(false)) => None,
                Some(v) if v.is_object() => Some(self.full_type(&id.child("additionalProperties"))),
                _ => Some(CsType::Json),
            }
        };
        CsDecl::Record { fields, extras }
    }
}
fn carrier_shape(raw: &Map<String, Value>) -> bool {
    if raw.contains_key("$ref") && raw.keys().any(|k| k != "$ref" && !annotation(k)) {
        return true;
    }
    if (raw.contains_key("oneOf") || raw.contains_key("anyOf"))
        && (raw
            .keys()
            .any(|k| !matches!(k.as_str(), "oneOf" | "anyOf") && !annotation(k))
            || (raw.contains_key("oneOf") && raw.contains_key("anyOf")))
    {
        return true;
    }
    let mut domains = types(raw);
    domains.retain(|t| t != "null");
    if domains.iter().any(|t| t == "number") {
        domains.retain(|t| t != "integer");
    }
    domains.len() > 1
}
fn reject_alias_cycles(declarations: &BTreeMap<Key, CsDecl>, planner: &mut Planner<'_>) {
    fn refs(ty: &CsType, out: &mut Vec<Key>) {
        match ty {
            CsType::Named(k) => out.push(k.clone()),
            CsType::Nullable(t) | CsType::List(t) | CsType::Dict(t) => refs(t, out),
            _ => {}
        }
    }
    fn visit(
        k: &Key,
        decls: &BTreeMap<Key, CsDecl>,
        active: &mut BTreeSet<Key>,
        done: &mut BTreeSet<Key>,
    ) -> bool {
        if done.contains(k) {
            return false;
        }
        let Some(CsDecl::Alias(ty)) = decls.get(k) else {
            return false;
        };
        if !active.insert(k.clone()) {
            return true;
        }
        let mut targets = Vec::new();
        refs(ty, &mut targets);
        let cycle = targets.iter().any(|k| visit(k, decls, active, done));
        active.remove(k);
        done.insert(k.clone());
        cycle
    }
    let mut done = BTreeSet::new();
    for k in declarations.keys() {
        if visit(k, declarations, &mut BTreeSet::new(), &mut done) {
            planner.report(
                &k.0,
                "csharp-recursive-alias-unsupported",
                "an erased alias cycle needs a nominal object or union to guard recursion",
            );
        }
    }
}
fn types(raw: &Map<String, Value>) -> Vec<String> {
    match raw.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}
fn only_null(value: &Value) -> bool {
    value.get("type") == Some(&Value::String("null".into()))
        || value.get("const") == Some(&Value::Null)
        || value
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|vs| !vs.is_empty() && vs.iter().all(Value::is_null))
}
fn annotation(key: &str) -> bool {
    key.starts_with("x-")
        || matches!(
            key,
            "$id"
                | "$schema"
                | "$anchor"
                | "$defs"
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
        )
}
fn source_name(contract: &Contract, id: &SchemaId, hints: &crate::model_naming::Hints) -> String {
    for operation in contract.operations() {
        if id.document() == operation.source().document()
            && let Some(tail) = id
                .pointer()
                .strip_prefix(&format!("{}/", operation.source().pointer()))
        {
            let stem = exported(operation.operation_id().unwrap_or("Operation"));
            if let Some(tail) = tail.strip_prefix("requestBody/content/application~1json/schema") {
                return format!("{stem}Request{}", pointer_name(tail));
            }
            if let Some(response) = tail.strip_prefix("responses/")
                && let Some((status, tail)) =
                    response.split_once("/content/application~1json/schema")
            {
                return format!("{stem}Response{status}{}", pointer_name(tail));
            }
            for parameter in operation.parameters() {
                if let Some(schema) = parameter.schema()
                    && id.document() == schema.id().document()
                    && (id.pointer() == schema.id().pointer()
                        || id
                            .pointer()
                            .starts_with(&format!("{}/", schema.id().pointer())))
                {
                    return format!(
                        "{stem}{}Parameter{}",
                        exported(parameter.name().unwrap_or("Value")),
                        pointer_name(&id.pointer()[schema.id().pointer().len()..])
                    );
                }
            }
            if let Some(hint) = hints.get(id) {
                return exported(&hint);
            }
            if let Some((_, tail)) = tail.split_once("/itemSchema") {
                return format!("{stem}Item{}", pointer_name(tail));
            }
            return format!("{stem}{}", pointer_name(tail));
        }
    }
    if let Some(component) = id.pointer().strip_prefix("/components/schemas/") {
        let (name, tail) = component.split_once('/').unwrap_or((component, ""));
        return format!(
            "{}{}",
            exported(&name.replace("~1", "/").replace("~0", "~")),
            pointer_name(tail)
        );
    }
    let name = pointer_name(id.pointer());
    if name.is_empty() {
        "Value".into()
    } else {
        name
    }
}
fn pointer_name(pointer: &str) -> String {
    let mut tokens = pointer.split('/').filter(|s| !s.is_empty());
    let mut result = String::new();
    while let Some(token) = tokens.next() {
        match token {
            "properties" | "$defs" => result.push_str(&exported(
                &tokens
                    .next()
                    .unwrap_or(token)
                    .replace("~1", "/")
                    .replace("~0", "~"),
            )),
            "items" => result.push_str("Item"),
            "oneOf" | "anyOf" => result.push_str("Variant"),
            "content" => {
                tokens.next();
            }
            "schema" => {}
            _ => result.push_str(&exported(&token.replace("~1", "/").replace("~0", "~"))),
        }
    }
    result
}
