//! Retained Java declarations, independent of emitted source text and other
//! languages' type systems. The owned compiler proves the null domain; this
//! planner chooses native representations for that same selected closure.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedOutcome, OwnedProgram, OwnedSchema};

use super::{HttpDiagnostic, PackageConfig, diagnostic};

/// A native type expression. Presence is separate from the JSON value domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaType {
    /// Unicode string, with no implicit format coercion.
    String,
    /// Boxed boolean; a non-null schema still rejects Java null.
    Boolean,
    /// Exact JSON decimal, including mathematically integral values.
    Number,
    /// The immutable, sealed JSON domain. Its null is `JsonNull.INSTANCE`.
    Json,
    /// Only `JsonNull.INSTANCE`.
    Null,
    /// No constructible native value.
    Never,
    /// A source-bound declaration or erased alias.
    Named(SchemaId),
    /// A value-domain Java null, independent of property presence.
    Nullable(Box<Self>),
    /// An immutable snapshot of homogeneous items.
    List(Box<Self>),
}

/// A model member and its actual public builder/accessor identity.
#[derive(Debug, Clone)]
pub struct JavaField {
    pub name: String,
    pub wire: String,
    pub ty: JavaType,
    pub required: bool,
    pub nullable: bool,
    pub source: SchemaId,
    pub description: String,
    /// A required, source-proved singleton. This is not a schema default.
    pub fixed: Option<Value>,
    /// The optional builder method that restores absence.
    pub omit_method: Option<String>,
}

/// The ordered arguments of a generated factory or constructor.
#[derive(Debug, Clone)]
pub struct JavaArgument {
    pub name: String,
    pub ty: JavaType,
    pub source: SchemaId,
    pub nullable: bool,
}

/// A retained public construction signature, shared by docs and compatibility.
#[derive(Debug, Clone)]
pub struct JavaConstructor {
    /// `builder` for objects; `new VariantN` for union arms.
    pub name: String,
    pub arguments: Vec<JavaArgument>,
}

/// One explicit native union arm, with its own checked codec binding.
#[derive(Debug, Clone)]
pub struct JavaVariant {
    pub name: String,
    pub source: SchemaId,
    pub ty: JavaType,
    pub constructor: JavaConstructor,
}

/// A source literal and its allocated native constant. Numeric spelling is
/// retained when a mathematically equal value is decoded.
#[derive(Debug, Clone)]
pub struct JavaLiteral {
    pub name: String,
    pub value: Value,
}

/// The complete, retained native declaration. Aliases have their own codec
/// holder class, but callers use the alias's resolved native type.
#[derive(Debug, Clone)]
pub enum JavaDeclaration {
    Object {
        fields: Vec<JavaField>,
        extras: Option<JavaType>,
        constructor: JavaConstructor,
    },
    Alias(JavaType),
    Union {
        exclusive: bool,
        variants: Vec<JavaVariant>,
    },
    Literals {
        values: Vec<JavaLiteral>,
    },
}

/// A binding to the emitted `OwnedProgram`, never a Java-text lookup.
#[derive(Debug, Clone)]
pub struct JavaCodecBinding {
    pub source: SchemaId,
    pub holder: String,
    pub field: &'static str,
    pub root: usize,
    pub native_type: String,
}

/// One immutable declaration and documentation identity.
#[derive(Debug, Clone)]
pub struct JavaSymbol {
    source: SchemaId,
    name: String,
    description: String,
    declaration: JavaDeclaration,
    nullable: bool,
    codec: JavaCodecBinding,
}

impl JavaSymbol {
    #[must_use]
    pub fn source(&self) -> &SchemaId {
        &self.source
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    #[must_use]
    pub const fn declaration(&self) -> &JavaDeclaration {
        &self.declaration
    }
    #[must_use]
    pub const fn nullable(&self) -> bool {
        self.nullable
    }
    #[must_use]
    pub const fn codec(&self) -> &JavaCodecBinding {
        &self.codec
    }
}

/// Flattened member identities for consumers that do not need the full graph.
#[derive(Debug, Clone)]
pub struct JavaFieldWire {
    pub model: String,
    pub field: String,
    pub wire: String,
    pub native_type: String,
    pub required: bool,
    pub nullable: bool,
    pub source: SchemaId,
}

/// Native model, construction and codec descriptors. Callers receive immutable
/// views; a render never allocates a second set of symbols.
#[derive(Debug)]
pub struct JavaModelPlan {
    symbols: Vec<JavaSymbol>,
    names: BTreeMap<SchemaId, String>,
    positions: BTreeMap<SchemaId, usize>,
    fields: Vec<JavaFieldWire>,
}

impl JavaModelPlan {
    #[must_use]
    pub fn symbols(&self) -> &[JavaSymbol] {
        &self.symbols
    }
    #[must_use]
    pub fn fields(&self) -> &[JavaFieldWire] {
        &self.fields
    }
    #[must_use]
    pub fn names(&self) -> &BTreeMap<SchemaId, String> {
        &self.names
    }
    #[must_use]
    pub fn symbol(&self, id: &SchemaId) -> Option<&JavaSymbol> {
        self.positions
            .get(id)
            .map(|&position| &self.symbols[position])
    }
    #[must_use]
    pub fn codec(&self, id: &SchemaId) -> &JavaCodecBinding {
        &self.symbols[self.positions[id]].codec
    }
    #[must_use]
    pub fn native_type(&self, id: &SchemaId) -> String {
        self.render_type(&JavaType::Named(id.clone()))
    }
    #[must_use]
    pub fn render_type(&self, ty: &JavaType) -> String {
        match ty {
            JavaType::String => "String".into(),
            JavaType::Boolean => "Boolean".into(),
            JavaType::Number => "JsonNumber".into(),
            JavaType::Json => "JsonValue".into(),
            JavaType::Null => "JsonNull".into(),
            JavaType::Never => "Never".into(),
            JavaType::Nullable(inner) => self.render_type(inner),
            JavaType::List(inner) => format!("java.util.List<{}>", self.render_type(inner)),
            JavaType::Named(id) => match &self.symbols[self.positions[id]].declaration {
                JavaDeclaration::Alias(inner) => self.render_type(inner),
                _ => self.names[id].clone(),
            },
        }
    }

    /// Lower one already validated example through these exact declarations.
    pub(crate) fn example(
        &self,
        id: &SchemaId,
        value: &Value,
        compiled: &OwnedSchema,
    ) -> Option<String> {
        self.example_type(&JavaType::Named(id.clone()), value, compiled, 0)
    }

    fn example_type(
        &self,
        ty: &JavaType,
        value: &Value,
        compiled: &OwnedSchema,
        depth: usize,
    ) -> Option<String> {
        if depth > 64 {
            return None;
        }
        Some(match ty {
            JavaType::String => q(value.as_str()?),
            JavaType::Boolean => value.as_bool()?.to_string(),
            JavaType::Number => format!("JsonNumber.parse({})", q(&value.as_number()?.to_string())),
            JavaType::Json => format!("JsonRuntime.parse({})", q(&value.to_string())),
            JavaType::Null if value.is_null() => "JsonNull.INSTANCE".into(),
            JavaType::Null => return None,
            JavaType::Never => return None,
            JavaType::Nullable(inner) => {
                if value.is_null() {
                    "null".into()
                } else {
                    self.example_type(inner, value, compiled, depth + 1)?
                }
            }
            JavaType::List(inner) => {
                let values = value
                    .as_array()?
                    .iter()
                    .map(|v| self.example_type(inner, v, compiled, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                // Arrays.asList supports source-nullable elements; List.of does not.
                format!("java.util.Arrays.asList({})", values.join(", "))
            }
            JavaType::Named(id) => {
                let symbol = self.symbol(id)?;
                if let JavaDeclaration::Alias(inner) = &symbol.declaration {
                    return self.example_type(inner, value, compiled, depth + 1);
                }
                if value.is_null()
                    && symbol.nullable
                    && !matches!(symbol.declaration, JavaDeclaration::Union { .. })
                {
                    return Some("null".into());
                }
                match &symbol.declaration {
                    JavaDeclaration::Alias(_) => unreachable!(),
                    JavaDeclaration::Object {
                        fields,
                        extras,
                        constructor,
                    } => {
                        let object = value.as_object()?;
                        let args = constructor
                            .arguments
                            .iter()
                            .map(|a| {
                                let field = fields.iter().find(|f| f.source == a.source)?;
                                self.example_type(
                                    &a.ty,
                                    object.get(&field.wire)?,
                                    compiled,
                                    depth + 1,
                                )
                            })
                            .collect::<Option<Vec<_>>>()?;
                        let mut expression =
                            format!("{}.builder({})", symbol.name, args.join(", "));
                        for field in fields.iter().filter(|f| !f.required) {
                            if let Some(value) = object.get(&field.wire) {
                                expression.push_str(&format!(
                                    ".{}({})",
                                    field.name,
                                    self.example_type(&field.ty, value, compiled, depth + 1)?
                                ));
                            }
                        }
                        if let Some(extra) = extras {
                            for (key, value) in object {
                                if !fields.iter().any(|f| &f.wire == key) {
                                    expression.push_str(&format!(
                                        ".putAdditionalProperty({}, {})",
                                        q(key),
                                        self.example_type(extra, value, compiled, depth + 1)?
                                    ));
                                }
                            }
                        }
                        expression.push_str(".build()");
                        expression
                    }
                    JavaDeclaration::Literals { values } => {
                        // Keep alternate exact numeric spellings (including inside
                        // compound literals) instead of canonicalizing a constant.
                        let spelling = serde_json::to_vec(value).expect("JSON example");
                        if let Some(literal) = values.iter().find(|literal| {
                            serde_json::to_vec(&literal.value).expect("JSON literal") == spelling
                        }) {
                            format!("{}.{}", symbol.name, literal.name)
                        } else {
                            format!("{}.decode({})", symbol.name, q(&value.to_string()))
                        }
                    }
                    JavaDeclaration::Union { variants, .. } => {
                        let branch = variants.iter().find(|v| {
                            matches!(compiled.validate(&v.source, value), OwnedOutcome::Valid)
                        })?;
                        format!(
                            "new {}.{}({})",
                            symbol.name,
                            branch.name,
                            self.example_type(&branch.ty, value, compiled, depth + 1)?
                        )
                    }
                }
            }
        })
    }
}

pub(crate) fn plan_models(
    contract: &Contract,
    roots: &[SchemaId],
    config: &PackageConfig,
    compiled: &OwnedSchema,
    program: &OwnedProgram,
) -> Result<JavaModelPlan, Vec<HttpDiagnostic>> {
    let reachable = crate::schema_view::closure(contract, roots);
    let mut errors = Vec::new();
    let mut nullable = BTreeMap::new();
    for id in &reachable {
        match compiled.validate(id, &Value::Null) {
            OwnedOutcome::Valid => {
                nullable.insert(id.clone(), true);
            }
            OwnedOutcome::Invalid(_) => {
                nullable.insert(id.clone(), false);
            }
            OwnedOutcome::EvaluationFailure(f) => errors.push(diagnostic(
                contract,
                f.source,
                "java-nullability-unproved",
                format!("null-domain proof did not complete: {}", f.message),
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut used = reserved_types();
    used.insert(config.api_name.clone());
    let names: BTreeMap<_, _> = reachable
        .iter()
        .map(|id| (id.clone(), allocate(&source_name(contract, id), &mut used)))
        .collect();
    let mut planner = Planner {
        contract,
        names,
        nullable,
        errors,
        scoped: matches!(
            program.version,
            OwnedProgram::V2_VERSION | OwnedProgram::V3_VERSION
        ),
    };
    for id in &reachable {
        planner.check_shape(id);
    }
    let declarations: BTreeMap<_, _> = reachable
        .iter()
        .map(|id| (id.clone(), planner.lower(id)))
        .collect();
    reject_alias_cycles(&declarations, &mut planner);
    if !planner.errors.is_empty() {
        return Err(planner.errors);
    }
    let symbols: Vec<_> = declarations
        .into_iter()
        .map(|(id, declaration)| {
            let root = program
                .roots
                .iter()
                .find(|r| {
                    r.source.document == id.document().as_str() && r.source.pointer == id.pointer()
                })
                .expect("every reachable source is a compiled root");
            JavaSymbol {
                name: planner.names[&id].clone(),
                description: crate::schema_view::description(
                    contract.schema(&id).expect("compiled schema"),
                )
                .into(),
                nullable: planner.nullable[&id],
                codec: JavaCodecBinding {
                    source: id.clone(),
                    holder: planner.names[&id].clone(),
                    field: "CODEC",
                    root: root.target,
                    native_type: String::new(),
                },
                source: id,
                declaration,
            }
        })
        .collect();
    let positions = symbols
        .iter()
        .enumerate()
        .map(|(i, s)| (s.source.clone(), i))
        .collect();
    let mut plan = JavaModelPlan {
        symbols,
        positions,
        names: planner.names,
        fields: Vec::new(),
    };
    let native_types: Vec<_> = plan
        .symbols
        .iter()
        .map(|s| plan.native_type(&s.source))
        .collect();
    for (symbol, native) in plan.symbols.iter_mut().zip(native_types) {
        symbol.codec.native_type = native;
    }
    for symbol in &plan.symbols {
        if let JavaDeclaration::Object { fields, .. } = &symbol.declaration {
            for field in fields {
                plan.fields.push(JavaFieldWire {
                    model: symbol.name.clone(),
                    field: field.name.clone(),
                    wire: field.wire.clone(),
                    native_type: plan.render_type(&field.ty),
                    required: field.required,
                    nullable: field.nullable,
                    source: field.source.clone(),
                });
            }
        }
    }
    Ok(plan)
}

struct Planner<'a> {
    contract: &'a Contract,
    names: BTreeMap<SchemaId, String>,
    nullable: BTreeMap<SchemaId, bool>,
    errors: Vec<HttpDiagnostic>,
    scoped: bool,
}

impl Planner<'_> {
    fn report(&mut self, id: &SchemaId, code: &'static str, message: impl Into<String>) {
        self.errors
            .push(diagnostic(self.contract, id.clone(), code, message));
    }
    fn check_shape(&mut self, id: &SchemaId) {
        let Some(schema) = self.contract.schema(id) else {
            return;
        };
        let view = crate::schema_view::raw(schema);
        let Some(raw) = view.as_object() else {
            return;
        };
        for keyword in ["allOf", "not", "prefixItems"] {
            if !self.scoped && raw.contains_key(keyword) {
                self.report(&id.child(keyword), "java-representation-unsupported", format!("{keyword} has portable validation, but its native intersection/tuple representation is not implemented"));
            }
        }
        if !self.scoped
            && raw.contains_key("$ref")
            && raw.keys().any(|k| {
                k != "$ref"
                    && !annotation(k)
                    && !matches!(
                        k.as_str(),
                        "type"
                            | "enum"
                            | "const"
                            | "minimum"
                            | "maximum"
                            | "exclusiveMinimum"
                            | "exclusiveMaximum"
                            | "multipleOf"
                            | "minLength"
                            | "maxLength"
                            | "pattern"
                            | "minItems"
                            | "maxItems"
                            | "uniqueItems"
                            | "minProperties"
                            | "maxProperties"
                    )
            })
        {
            self.report(
                id,
                "java-ref-sibling-unsupported",
                "reference assertion siblings require a native intersection representation",
            );
        }
        if !self.scoped
            && (raw.contains_key("oneOf") || raw.contains_key("anyOf"))
            && (raw
                .keys()
                .any(|k| !matches!(k.as_str(), "oneOf" | "anyOf") && !annotation(k))
                || raw.contains_key("oneOf") && raw.contains_key("anyOf"))
        {
            self.report(
                id,
                "java-union-sibling-unsupported",
                "union assertion siblings require a native intersection representation",
            );
        }
        if !self.scoped
            && !raw.contains_key("type")
            && !["$ref", "oneOf", "anyOf", "enum", "const"]
                .iter()
                .any(|k| raw.contains_key(*k))
            && raw.keys().any(|k| !annotation(k))
        {
            self.report(
                id,
                "java-untyped-constraints-unsupported",
                "untyped assertions do not imply a native object or scalar type",
            );
        }
        for keyword in ["enum", "const"] {
            if let Some(value) = raw.get(keyword)
                && value.to_string().len() > 16_000
            {
                self.report(
                    &id.child(keyword),
                    "java-literal-resource-limit",
                    "native literal declarations are limited to 16000 serialized UTF-8 bytes",
                );
            }
        }
        if id.pointer().len() + id.document().as_str().len() > 16_000 {
            self.report(
                id,
                "java-source-resource-limit",
                "source identity exceeds the Java constant-pool profile limit",
            );
        }
    }
    fn wrap(&self, id: &SchemaId, ty: JavaType) -> JavaType {
        if self.nullable[id]
            && !matches!(
                ty,
                JavaType::Json | JavaType::Null | JavaType::Never | JavaType::Named(_)
            )
        {
            JavaType::Nullable(Box::new(ty))
        } else {
            ty
        }
    }
    fn lower(&mut self, id: &SchemaId) -> JavaDeclaration {
        let schema = self.contract.schema(id).expect("compiled schema");
        let view = crate::schema_view::raw(schema);
        let raw = match view.as_ref() {
            Value::Bool(true) => return JavaDeclaration::Alias(JavaType::Json),
            Value::Bool(false) => return JavaDeclaration::Alias(JavaType::Never),
            Value::Object(raw) => raw,
            _ => unreachable!("owned compiler admitted schema"),
        };
        if self.scoped && scoped_json_carrier(raw) {
            return JavaDeclaration::Alias(JavaType::Json);
        }
        if raw.contains_key("$ref") {
            let target = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.clone())
                .expect("resolved compiled reference");
            return JavaDeclaration::Alias(JavaType::Named(target));
        }
        for keyword in ["oneOf", "anyOf"] {
            if let Some(branches) = raw.get(keyword).and_then(Value::as_array) {
                let variants = (0..branches.len())
                    .map(|i| {
                        let source = id.child(keyword).child(&i.to_string());
                        let name = format!("Variant{}", i + 1);
                        let ty = JavaType::Named(source.clone());
                        JavaVariant {
                            name: name.clone(),
                            source: source.clone(),
                            ty: ty.clone(),
                            constructor: JavaConstructor {
                                name: format!("new {name}"),
                                arguments: vec![JavaArgument {
                                    name: "value".into(),
                                    ty,
                                    source: source.clone(),
                                    nullable: self.nullable[&source],
                                }],
                            },
                        }
                    })
                    .collect();
                return JavaDeclaration::Union {
                    exclusive: keyword == "oneOf",
                    variants,
                };
            }
        }
        if let Some(values) = raw
            .get("const")
            .map(|v| vec![v.clone()])
            .or_else(|| raw.get("enum").and_then(Value::as_array).cloned())
        {
            if values.iter().all(Value::is_null) {
                return JavaDeclaration::Alias(if self.nullable[id] {
                    JavaType::Null
                } else {
                    JavaType::Never
                });
            }
            let mut used = BTreeSet::from(["CODEC".into()]);
            let values = values
                .into_iter()
                .filter(|v| !v.is_null())
                .enumerate()
                .map(|(index, value)| {
                    let base = match &value {
                        Value::String(s) => crate::rust_models::snake(s).to_ascii_uppercase(),
                        Value::Bool(v) => if *v { "TRUE" } else { "FALSE" }.into(),
                        _ => format!("VALUE_{}", index + 1),
                    };
                    JavaLiteral {
                        name: allocate(&bounded(&base), &mut used),
                        value,
                    }
                })
                .collect();
            return JavaDeclaration::Literals { values };
        }
        let mut types = types(raw);
        types.retain(|t| t != "null");
        if types.contains(&"number".into()) {
            types.retain(|t| t != "integer");
        }
        if types.len() > 1 {
            self.report(&id.child("type"), "java-type-union-unsupported", "multiple non-null type alternatives require an explicit native union representation");
            return JavaDeclaration::Alias(JavaType::Never);
        }
        let ty = match types.first().map(String::as_str) {
            Some("object") => return self.object(id, raw),
            Some("array") => JavaType::List(Box::new(if raw.contains_key("items") {
                JavaType::Named(id.child("items"))
            } else {
                JavaType::Json
            })),
            Some("string") => JavaType::String,
            Some("boolean") => JavaType::Boolean,
            Some("integer" | "number") => JavaType::Number,
            None if raw.contains_key("type") => JavaType::Null,
            None => JavaType::Json,
            _ => unreachable!("compiled type"),
        };
        JavaDeclaration::Alias(self.wrap(id, ty))
    }
    fn object(&mut self, id: &SchemaId, raw: &Map<String, Value>) -> JavaDeclaration {
        let properties = raw.get("properties").and_then(Value::as_object);
        let required: BTreeSet<_> = raw
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for wire in &required {
            if !self.scoped && !properties.is_some_and(|p| p.contains_key(*wire)) {
                self.report(
                    &id.child("required"),
                    "java-undeclared-required-unsupported",
                    "required undeclared keys need a native construction representation",
                );
            }
        }
        let mut used = reserved_members();
        let fields: Vec<_> = properties
            .into_iter()
            .flatten()
            .map(|(wire, _raw)| {
                if wire.len() > 4096 {
                    self.report(
                        &id.child("properties").child(wire),
                        "java-property-resource-limit",
                        "wire names are limited to 4096 UTF-8 bytes",
                    );
                }
                let source = id.child("properties").child(wire);
                let name = allocate(&member(wire), &mut used);
                let is_required = required.contains(wire.as_str());
                let field_view =
                    crate::schema_view::raw(self.contract.schema(&source).expect("compiled field"));
                let fixed = is_required
                    .then(|| {
                        field_view.get("const").cloned().or_else(|| {
                            field_view
                                .get("enum")
                                .and_then(Value::as_array)
                                .filter(|v| v.len() == 1)
                                .map(|v| v[0].clone())
                        })
                    })
                    .flatten();
                JavaField {
                    omit_method: (!is_required)
                        .then(|| format!("omit{}", crate::rust_models::pascal(&name))),
                    name,
                    wire: wire.clone(),
                    ty: JavaType::Named(source.clone()),
                    required: is_required,
                    nullable: self.nullable[&source],
                    source,
                    description: field_view
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    fixed,
                }
            })
            .collect();
        let arguments: Vec<_> = fields
            .iter()
            .filter(|f| f.required && f.fixed.is_none())
            .map(|f| JavaArgument {
                name: f.name.clone(),
                ty: f.ty.clone(),
                source: f.source.clone(),
                nullable: f.nullable,
            })
            .collect();
        if arguments.len() > 200 || fields.len() > 512 {
            self.report(id, "java-constructor-resource-limit", "native objects support at most 200 required construction arguments and 512 declared fields");
        }
        let extras = if self.scoped
            && raw
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|p| !p.is_empty())
        {
            // Pattern and non-pattern keys can have different/intersecting value
            // types. Preserve all keys in the immutable JSON domain; the parent
            // checked codec enforces every matching pattern and extra rule.
            Some(JavaType::Json)
        } else {
            match raw.get("additionalProperties") {
                Some(Value::Bool(false)) => None,
                Some(value) if value.is_object() => {
                    Some(JavaType::Named(id.child("additionalProperties")))
                }
                _ => Some(JavaType::Json),
            }
        };
        JavaDeclaration::Object {
            fields,
            extras,
            constructor: JavaConstructor {
                name: "builder".into(),
                arguments,
            },
        }
    }
}

fn reject_alias_cycles(
    declarations: &BTreeMap<SchemaId, JavaDeclaration>,
    planner: &mut Planner<'_>,
) {
    fn targets(ty: &JavaType, result: &mut Vec<SchemaId>) {
        match ty {
            JavaType::Named(id) => result.push(id.clone()),
            JavaType::Nullable(inner) | JavaType::List(inner) => targets(inner, result),
            _ => {}
        }
    }
    fn visit(
        id: &SchemaId,
        declarations: &BTreeMap<SchemaId, JavaDeclaration>,
        active: &mut BTreeSet<SchemaId>,
        done: &mut BTreeSet<SchemaId>,
    ) -> bool {
        if done.contains(id) {
            return false;
        }
        let Some(JavaDeclaration::Alias(ty)) = declarations.get(id) else {
            return false;
        };
        if !active.insert(id.clone()) {
            return true;
        }
        let mut references = Vec::new();
        targets(ty, &mut references);
        let cycle = references
            .iter()
            .any(|target| visit(target, declarations, active, done));
        active.remove(id);
        done.insert(id.clone());
        cycle
    }
    let mut done = BTreeSet::new();
    for id in declarations.keys() {
        if visit(id, declarations, &mut BTreeSet::new(), &mut done) {
            planner.report(id, "java-recursive-alias-unsupported", "an erased recursive alias requires a nominal object or union to guard its Java type");
        }
    }
}

fn scoped_json_carrier(raw: &Map<String, Value>) -> bool {
    // A dynamic use is context-sensitive; its initial target is never a native
    // static type substitute. Whole-root codecs supply the actual entered scope.
    if raw.contains_key("$dynamicRef") {
        return true;
    }
    if ["allOf", "not", "prefixItems"]
        .iter()
        .any(|k| raw.contains_key(*k))
    {
        return true;
    }
    if (raw.contains_key("oneOf") || raw.contains_key("anyOf"))
        && (raw
            .keys()
            .any(|k| !matches!(k.as_str(), "oneOf" | "anyOf") && !annotation(k))
            || raw.contains_key("oneOf") && raw.contains_key("anyOf"))
    {
        return true;
    }
    if raw.contains_key("$ref")
        && raw.keys().any(|k| {
            k != "$ref"
                && !annotation(k)
                && !matches!(
                    k.as_str(),
                    "type"
                        | "enum"
                        | "const"
                        | "minimum"
                        | "maximum"
                        | "exclusiveMinimum"
                        | "exclusiveMaximum"
                        | "multipleOf"
                        | "minLength"
                        | "maxLength"
                        | "pattern"
                        | "minItems"
                        | "maxItems"
                        | "uniqueItems"
                        | "minProperties"
                        | "maxProperties"
                )
        })
    {
        return true;
    }
    if !raw.contains_key("type")
        && !["$ref", "oneOf", "anyOf", "enum", "const"]
            .iter()
            .any(|k| raw.contains_key(*k))
        && raw.keys().any(|k| !annotation(k))
    {
        return true;
    }
    let mut kinds = types(raw);
    kinds.retain(|v| v != "null");
    if kinds.iter().any(|v| v == "number") {
        kinds.retain(|v| v != "integer");
    }
    kinds.len() > 1
}

fn types(raw: &Map<String, Value>) -> Vec<String> {
    match raw.get("type") {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}
fn annotation(key: &str) -> bool {
    matches!(
        key,
        "nullable" | "contentMediaType" | "contentEncoding" | "contentSchema"
    ) || key.starts_with("x-")
        || matches!(
            key,
            "$id"
                | "$dynamicAnchor"
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
        )
}

fn source_name(contract: &Contract, id: &SchemaId) -> String {
    for operation in contract.operations() {
        if id.document() == operation.source().document()
            && let Some(tail) = id
                .pointer()
                .strip_prefix(&format!("{}/", operation.source().pointer()))
        {
            let stem = crate::rust_models::pascal(operation.operation_id().unwrap_or("Operation"));
            if let Some(rest) = tail.strip_prefix("requestBody/content/application~1json/schema") {
                return bounded(&format!("{stem}Request{}", optional_pointer_name(rest)));
            }
            if let Some(rest) = tail.strip_prefix("responses/")
                && let Some((status, rest)) = rest.split_once("/content/application~1json/schema")
                && !status.contains('/')
            {
                let successes = operation
                    .responses()
                    .iter()
                    .filter(|r| {
                        r.status_key()
                            .parse::<u16>()
                            .is_ok_and(|n| (200..300).contains(&n))
                    })
                    .count();
                let label = if status.parse::<u16>().is_ok_and(|n| (200..300).contains(&n)) {
                    if successes == 1 {
                        "Response".into()
                    } else {
                        format!("Response{status}")
                    }
                } else {
                    format!("Error{status}")
                };
                return bounded(&format!("{stem}{label}{}", optional_pointer_name(rest)));
            }
            for parameter in operation.parameters() {
                let prefix = format!("{}/schema", parameter.source().pointer());
                if let Some(rest) = id.pointer().strip_prefix(&prefix) {
                    let name = crate::rust_models::pascal(parameter.name().unwrap_or("Parameter"));
                    return bounded(&format!(
                        "{stem}{name}Parameter{}",
                        optional_pointer_name(rest)
                    ));
                }
            }
            return bounded(&format!("{stem}{}", pointer_name(tail)));
        }
    }
    if let Some(component) = id.pointer().strip_prefix("/components/schemas/") {
        let (name, rest) = component.split_once('/').unwrap_or((component, ""));
        return bounded(&format!(
            "{}{}",
            crate::rust_models::pascal(&name.replace("~1", "/").replace("~0", "~")),
            optional_pointer_name(rest)
        ));
    }
    bounded(&pointer_name(id.pointer()))
}
fn optional_pointer_name(pointer: &str) -> String {
    if pointer.is_empty() {
        String::new()
    } else {
        pointer_name(pointer)
    }
}
fn pointer_name(pointer: &str) -> String {
    let mut tokens = pointer.split('/').filter(|s| !s.is_empty());
    let mut value = String::new();
    while let Some(token) = tokens.next() {
        match token {
            // A wire property named "items"/"properties" is still its literal
            // name. Only structural pointer positions carry a schema role.
            "properties" | "$defs" | "definitions" => {
                if let Some(name) = tokens.next() {
                    value.push_str(&crate::rust_models::pascal(
                        &name.replace("~1", "/").replace("~0", "~"),
                    ));
                }
            }
            "oneOf" | "anyOf" => {
                value.push_str("Variant");
                if let Some(index) = tokens.next().and_then(|v| v.parse::<usize>().ok()) {
                    value.push_str(&(index + 1).to_string());
                }
            }
            "items" => value.push_str("Item"),
            "additionalProperties" => value.push_str("AdditionalProperty"),
            "content" | "application~1json" | "schema" => {}
            _ => value.push_str(&crate::rust_models::pascal(
                &token.replace("~1", "/").replace("~0", "~"),
            )),
        }
    }
    if value.is_empty() {
        "Value".into()
    } else {
        value
    }
}
pub(crate) fn bounded(name: &str) -> String {
    name.chars().take(100).collect()
}

/// Escape Java source literals without Java's pre-lexical Unicode escape trap.
pub(crate) fn q(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' || c == '\u{7f}' => out.push_str(&format!("\\{:03o}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
pub(crate) fn javadoc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("*/", "*&#47;")
        .replace('\\', "&#92;")
        .replace('@', "&#64;")
}

const KEYWORDS: &[&str] = &[
    "_",
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "true",
    "false",
    "null",
    "var",
    "yield",
    "record",
    "sealed",
    "permits",
    "when",
];
#[must_use]
pub fn is_java_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !KEYWORDS.contains(&name)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}
pub(crate) fn member(wire: &str) -> String {
    let value = bounded(&crate::rust_models::pascal(wire));
    let mut name = format!("{}{}", value[..1].to_ascii_lowercase(), &value[1..]);
    if !is_java_identifier(&name) {
        name.push('_');
    }
    name
}
pub(crate) fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut value = base.to_owned();
    let mut suffix = 2;
    // The artifact writer must be safe on case-insensitive filesystems too.
    while used.iter().any(|name| name.eq_ignore_ascii_case(&value)) {
        value = format!("{base}{suffix}");
        suffix += 1;
    }
    used.insert(value.clone());
    value
}
pub(crate) fn reserved_members() -> BTreeSet<String> {
    [
        "builder",
        "build",
        "getClass",
        "hashCode",
        "equals",
        "toString",
        "wait",
        "notify",
        "notifyAll",
        "additionalProperties",
        "putAdditionalProperty",
        "read",
        "write",
        "encode",
        "decode",
        "codec",
        "snapshot",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
pub(crate) fn reserved_types() -> BTreeSet<String> {
    let mut names = reserved_type_names();
    names.extend(
        ["TreeSet", "Comparator", "ValidationResources"]
            .into_iter()
            .map(str::to_owned),
    );
    names.extend(
        [
            "ExactHttp",
            "SecureRandom",
            "HexFormat",
            "BufferedInputStream",
            "Socket",
            "EOFException",
            "SSLSocket",
            "SSLSocketFactory",
            "SSLParameters",
            "SSLSession",
            "IDN",
            "SocketTimeoutException",
            "Consumer",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "Bytes",
            "NoContent",
            "ResponseBody",
            "EventStream",
            "RequestOptions",
            "HttpWire",
            "WireValue",
            "WireCodec",
            "Protocol",
            "Credential",
            "Authorization",
            "CredentialContext",
            "CredentialProvider",
            "InputStream",
            "OutputStream",
            "ArrayBlockingQueue",
            "AtomicLong",
            "NoSuchElementException",
            "Spliterator",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        ["TimeoutException", "InterruptedException", "LongConsumer"]
            .into_iter()
            .map(str::to_owned),
    );
    names
}
fn reserved_type_names() -> BTreeSet<String> {
    [
        "JsonRuntime",
        "JsonValue",
        "JsonNumber",
        "JsonString",
        "JsonBoolean",
        "JsonArray",
        "JsonObject",
        "JsonNull",
        "JsonError",
        "Presence",
        "Never",
        "ModelCodec",
        "Validation",
        "CodecException",
        "HttpRuntime",
        "SdkException",
        "SdkExamples",
        "Builder",
        "String",
        "Boolean",
        "Object",
        "Enum",
        "Record",
        "Class",
        "StringBuilder",
        "Integer",
        "Long",
        "Character",
        "Number",
        "Double",
        "Float",
        "Byte",
        "Short",
        "Void",
        "System",
        "Math",
        "Thread",
        "Throwable",
        "Exception",
        "RuntimeException",
        "IllegalArgumentException",
        "IllegalStateException",
        "NullPointerException",
        "ClassCastException",
        "ArithmeticException",
        "UnsupportedOperationException",
        "AssertionError",
        "AutoCloseable",
        "Comparable",
        "Override",
        "SuppressWarnings",
        "FunctionalInterface",
        "BigInteger",
        "ByteBuffer",
        "StandardCharsets",
        "CharacterCodingException",
        "CodingErrorAction",
        "IOException",
        "InputStream",
        "ByteArrayOutputStream",
        "Map",
        "List",
        "Set",
        "Collection",
        "Collections",
        "Arrays",
        "Objects",
        "ArrayList",
        "ArrayDeque",
        "LinkedHashMap",
        "HashMap",
        "HashSet",
        "IdentityHashMap",
        "TreeMap",
        "Locale",
        "Optional",
        "Iterator",
        "URI",
        "URL",
        "ProxySelector",
        "Proxy",
        "SocketAddress",
        "InetSocketAddress",
        "Duration",
        "HttpClient",
        "HttpRequest",
        "HttpResponse",
        "HttpHeaders",
        "HttpTimeoutException",
        "CompletableFuture",
        "CompletionStage",
        "CompletionException",
        "ExecutionException",
        "CancellationException",
        "TimeUnit",
        "Executors",
        "Executor",
        "ExecutorService",
        "Future",
        "ScheduledFuture",
        "ScheduledThreadPoolExecutor",
        "RejectedExecutionException",
        "ConcurrentHashMap",
        "Flow",
        "Function",
        "BiFunction",
        "Supplier",
        "Consumer",
        "IntConsumer",
        "AtomicReference",
        "AtomicBoolean",
        "java",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
