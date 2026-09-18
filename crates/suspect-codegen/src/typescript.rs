//! Canonical Contract → native TypeScript model planning.
//!
//! Model-only planning distinguishes unsupported lowering from runtime
//! obligations. The `codecs` module supplies validated neutral model conversion
//! for its admitted subset. HTTP clients and SDK release promotion remain open.

mod additional_properties;
pub mod codecs;
mod directional;
pub mod http;
mod intersections;
pub mod json;
pub mod package;
use crate::schema_view;
pub mod validation;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde_json::{Map, Value, json};
use suspect_gen::filters::ts_identifier;
use suspect_ir::contract::{Contract, ContractSeverity, SchemaDialect, SchemaId};

use crate::OutFile;

const MODEL_FILE: &str = "typescript/models.ts";
const DOC_FILE: &str = "typescript/models.md";

/// Requiredness applicability for OpenAPI read/write annotations. These views
/// keep every wire property; they are not policies that reject or strip data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelView {
    /// Every declared property and required entry, without directional projection.
    Neutral,
    /// `readOnly` properties stay present in the type but are not required.
    Request,
    /// `writeOnly` properties stay present in the type but are not required.
    Response,
}

impl ModelView {
    fn suffix(self) -> &'static str {
        match self {
            Self::Neutral => "",
            Self::Request => "Request",
            Self::Response => "Response",
        }
    }
}

/// A diagnostic's effect on model emission and release support claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    /// No honest representation is implemented; artifact emission is blocked.
    Error,
    /// Model types need a source-aware codec; release promotion is blocked.
    CodecObligation,
    /// An annotation is retained without adding validation behavior.
    Annotation,
}

/// A planning finding tied to the original contract address and byte span.
#[derive(Debug, Clone)]
pub struct ModelDiagnostic {
    /// Original keyword/schema identity, never a generated symbol or copied tree.
    pub source: SchemaId,
    /// Original byte range when available.
    pub at: Range<usize>,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Whether this is unsupported lowering, runtime work, or an annotation.
    pub kind: DiagnosticKind,
    /// Human-readable explanation of the remaining requirement.
    pub message: String,
}

impl ModelDiagnostic {
    /// Whether a generated SDK can be promoted before this finding is resolved.
    #[must_use]
    pub fn blocks_release(&self) -> bool {
        self.kind != DiagnosticKind::Annotation
    }
}

/// One public declaration and its documentation identity.
#[derive(Debug, Clone)]
pub struct ModelSymbol {
    source: SchemaId,
    view: ModelView,
    name: String,
    expression: Expr,
    description: String,
    source_json: String,
}

impl ModelSymbol {
    /// Original schema address.
    #[must_use]
    pub fn source(&self) -> &SchemaId {
        &self.source
    }
    /// Directional applicability of this declaration.
    #[must_use]
    pub fn view(&self) -> ModelView {
        self.view
    }
    /// Collision-allocated exported TypeScript identifier, shared by docs.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Retained native type expression for source compatibility and documentation.
    pub(crate) fn expression(&self) -> &Expr {
        &self.expression
    }
    /// Planned relative code artifact path, shared by docs.
    #[must_use]
    pub fn file(&self) -> &str {
        MODEL_FILE
    }
}

/// An immutable declaration/diagnostic snapshot. Rendering does no source
/// inference or name allocation and uses the existing artifact writer seam.
#[derive(Debug)]
pub struct ModelPlan {
    symbols: Vec<ModelSymbol>,
    diagnostics: Vec<ModelDiagnostic>,
    openapi_version: String,
    resource_validation: bool,
}

impl ModelPlan {
    /// All selected roots and reference targets, in stable source/view order.
    #[must_use]
    pub fn symbols(&self) -> &[ModelSymbol] {
        &self.symbols
    }
    /// Source-linked errors, outstanding codec work, and annotations.
    #[must_use]
    pub fn diagnostics(&self) -> &[ModelDiagnostic] {
        &self.diagnostics
    }
    /// Whether lowering errors prevent artifact emission.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::Error)
    }
    /// Whether all required model and codec semantics have been implemented.
    #[must_use]
    pub fn release_ready(&self) -> bool {
        !self.diagnostics.iter().any(ModelDiagnostic::blocks_release)
    }

    /// Render reviewable models and reference documentation from this same plan.
    ///
    /// # Errors
    /// Returns all findings if any source construct cannot be lowered. Codec
    /// obligations remain prominently documented and do not imply runtime support.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        self.render_with_codecs(None)
    }

    fn render_with_codecs(
        &self,
        codecs: Option<&BTreeMap<String, String>>,
    ) -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        if self.has_errors() {
            return Err(self.diagnostics.clone());
        }
        let implemented = codecs.is_some();
        let status = if implemented {
            "A validated model codec is implemented. HTTP transport is outside this model module."
        } else {
            "No codec or HTTP client is implemented."
        };
        let findings: Vec<_> = self
            .diagnostics
            .iter()
            .filter(|d| !implemented || d.kind != DiagnosticKind::CodecObligation)
            .collect();
        let mut code = String::from(if implemented {
            "/** Experimental contract models with source-validated model codecs. HTTP transport is outside this model module. */\n\n"
        } else {
            "/** Experimental contract models. Codecs are not implemented; do not use JSON.parse/JSON.stringify as faithful codecs. */\n\n"
        });
        code.push_str(SUPPORT_TYPES);
        let mut docs = format!(
            "# TypeScript contract models\n\nOpenAPI {}. {status}\n\nUse TypeScript with `strict`, `exactOptionalPropertyTypes` and `noUncheckedIndexedAccess`.\n\n`JsonNumber.parse` constructs an exact decimal token after validating JSON number grammar. The generated `json.ts` parser/encoder preserves exact tokens; model codecs must establish schema validity. Unbounded integers use `bigint`; only locally proven safe bounded integers use `number`. Ordinary JSON.parse/JSON.stringify are not the codec.\n\nNeutral models preserve complete requiredness. Request and response views retain all wire properties and adjust required applicability for readOnly/writeOnly. OpenAPI annotations do not imply an unconditional rule to reject or remove wire data. Implemented codecs apply exactly the same applicability proof to compiled validation: only proven directional presence requirements are relaxed, and every provided value still validates against all unmodified schema assertions.\n\nTypeScript structural assignability does not establish object exactness, numeric constraints, or oneOf exclusivity. SDK release promotion also requires complete HTTP, packaging, documentation and performance gates.\n\n",
            escape_prose(&self.openapi_version)
        );
        for symbol in &self.symbols {
            let source = source_text(&symbol.source);
            code.push_str(&declaration_comment(&symbol.description, &source, &format!("Model view: {:?}. {status} Source constraints and union exclusivity are runtime obligations.", symbol.view)));
            code.push_str(&format!(
                "export type {} = {};\n\n",
                symbol.name,
                symbol.expression.render()
            ));
            docs.push_str(&format!("## `{}`\n\n[Code](models.ts). View: `{:?}`.\n\n{}\n\nSource: `{}`\n\n```ts\nexport type {} = {};\n```\n\nOriginal schema:\n\n```json\n{}\n```\n\n", symbol.name, symbol.view, escape_prose(&symbol.description), escape_prose(&source), symbol.name, symbol.expression.render(), symbol.source_json));
        }
        docs.push_str("## Findings and remaining codec obligations\n\n");
        for diagnostic in &findings {
            docs.push_str(&format!(
                "- `{:?}` / `{}` at `{}`: {}\n",
                diagnostic.kind,
                diagnostic.code,
                escape_prose(&source_text(&diagnostic.source)),
                escape_prose(&diagnostic.message)
            ));
        }
        let manifest = json!({
            "format": "suspect-typescript-docs-v1",
            "openapiVersion": self.openapi_version,
            "releaseReady": if implemented {false} else {self.release_ready()},
            "codecsImplemented": implemented,
            "codecs": codecs.map(|names| self.symbols.iter().map(|symbol| json!({
                "name": names[&symbol.name], "model": symbol.name, "file":"model-codecs.ts",
                "view": format!("{:?}", symbol.view),
                "source": { "document":symbol.source.document().to_string(),"pointer":symbol.source.pointer() }
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "supportSymbols": ["JsonNumber", "JsonValue"],
            "symbols": self.symbols.iter().map(|symbol| json!({
                "name": symbol.name,
                "view": format!("{:?}", symbol.view),
                "file": "models.ts",
                "source": { "document": symbol.source.document().to_string(), "pointer": symbol.source.pointer() },
                "sourceComment": escape_prose(&source_text(&symbol.source)),
                "hasSourceDescription": !symbol.description.trim().is_empty(),
                "descriptionText": symbol.description,
            })).collect::<Vec<_>>(),
            "findings": findings.iter().map(|finding| json!({
                "code": finding.code, "kind": format!("{:?}", finding.kind), "message": finding.message,
                "source": { "document": finding.source.document().to_string(), "pointer": finding.source.pointer() },
            })).collect::<Vec<_>>(),
        });
        let native_readme = format!(
            "# Experimental OpenAPI contract models\n\nOpenAPI {}. {status} Native documentation alone does not establish runtime contract fidelity, and every root remains blocked from SDK release.\n\nUse strict TypeScript with exactOptionalPropertyTypes and noUncheckedIndexedAccess. Requiredness and nullability are independent. Unbounded integers use bigint; general JSON numbers use the opaque JsonNumber representation. Ordinary JSON.parse/JSON.stringify is not an implemented codec.\n\nEach reference page records its original OpenAPI source and model view. Source descriptions are rendered as literal prose. Missing source descriptions and remaining findings are recorded in docs-manifest.json; original schemas also remain in models.md. No working client examples are claimed.\n\n## Planned models\n\n{}\n",
            escape_prose(&self.openapi_version),
            self.symbols
                .iter()
                .map(|symbol| format!(
                    "- {{@link {}{}}}",
                    if implemented { "models." } else { "" },
                    symbol.name
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(vec![
            json::runtime(),
            OutFile {
                path: MODEL_FILE.into(),
                content: code,
            },
            OutFile {
                path: DOC_FILE.into(),
                content: docs,
            },
            OutFile { path: "typescript/docs-manifest.json".into(), content: serde_json::to_string_pretty(&manifest).expect("manifest JSON") },
            OutFile { path: "typescript/typedoc.json".into(), content: serde_json::to_string_pretty(&json!({
                "entryPoints": if implemented {vec!["models.ts", "model-codecs.ts"]} else {vec!["models.ts"]}, "entryPointStrategy": "resolve",
                "tsconfig": "tsconfig.docs.json", "readme": "docs-readme.md",
                "name": "OpenAPI contract models", "out": "docs/html",
                "disableSources": true, "includeVersion": false,
                "plugin": [],
                "treatWarningsAsErrors": true,
                "validation": { "invalidLink": true, "notExported": true, "notDocumented": true },
                "requiredToBeDocumented": ["TypeAlias"],
            })).expect("TypeDoc configuration") },
            OutFile { path: "typescript/tsconfig.docs.json".into(), content: serde_json::to_string_pretty(&json!({
                "compilerOptions": { "target": "ES2022", "module": "NodeNext", "moduleResolution": "NodeNext", "strict": true, "exactOptionalPropertyTypes": true, "noUncheckedIndexedAccess": true, "noEmit": true },
                "files": if implemented {vec!["models.ts", "model-codecs.ts"]} else {vec!["models.ts"]},
            })).expect("TypeScript configuration") },
            OutFile { path: "typescript/docs-readme.md".into(), content: native_readme },
        ])
    }
}

/// Plan selected schema closures and views. An empty selection emits no models;
/// unknown roots and an empty view selection are errors. Original reference
/// edges determine symbol identity, never schema titles or reference text.
#[must_use]
pub fn plan_models(contract: &Contract, roots: &[SchemaId], views: &[ModelView]) -> ModelPlan {
    plan_models_with_policy(
        contract,
        roots,
        views,
        crate::schema_view::DialectPolicy::default(),
    )
}

/// The same plan under explicit versioned dialect interpretation choices.
#[must_use]
pub fn plan_models_with_policy(
    contract: &Contract,
    roots: &[SchemaId],
    views: &[ModelView],
    policy: crate::schema_view::DialectPolicy,
) -> ModelPlan {
    let reachable = schema_view::closure(contract, roots);
    let resource_sources = resource_requirements(contract, &reachable);
    let resource_validation = !resource_sources.is_empty();
    let mut planner = Planner {
        contract,
        names: BTreeMap::new(),
        diagnostics: Vec::new(),
        policy,
    };
    for root in roots {
        if contract.schema(root).is_none() {
            planner.report(
                root.clone(),
                "unknown-model-root",
                DiagnosticKind::Error,
                "selected root is not an indexed contract schema",
            );
        }
        planner.report(root.clone(), "model-codec-unimplemented", DiagnosticKind::CodecObligation, "the selected root has no validated encoder/decoder; preserve exact numbers, JSON validity, absence/null and all schema constraints before release");
    }
    for source in resource_sources {
        planner.report(source, "resource-validation-required", DiagnosticKind::CodecObligation,
            "canonical resources and dynamic targets require the checked v3 codec; logical URI metadata cannot replace physical source identity or select a dynamic fallback statically");
    }
    if views.is_empty()
        && let Some(root) = roots.first()
    {
        planner.report(
            root.clone(),
            "missing-model-view",
            DiagnosticKind::Error,
            "at least one model view is required",
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
            problem.source,
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
        for reference in contract.schema(id).unwrap().references() {
            if let Some(target) = &reference.target {
                declared.insert(target.clone());
            }
        }
    }
    planner.names = allocate_names(contract, &declared, views);
    let mut symbols = Vec::new();
    for ((source, view), name) in planner.names.clone() {
        let raw = contract.schema(&source).unwrap().raw();
        let description = schema_view::description(contract.schema(&source).unwrap()).to_owned();
        let expression = planner.lower(&source, view);
        symbols.push(ModelSymbol {
            source,
            view,
            name,
            expression,
            description,
            source_json: serde_json::to_string_pretty(raw)
                .expect("JSON serialization is infallible"),
        });
    }
    planner.check_alias_cycles(&symbols);
    planner
        .diagnostics
        .extend(additional_properties::check(contract, &symbols));
    planner
        .diagnostics
        .extend(intersections::check(contract, &symbols));
    planner.diagnostics.sort_by(|a, b| {
        (&a.source, a.kind, a.code, &a.message).cmp(&(&b.source, b.kind, b.code, &b.message))
    });
    planner.diagnostics.dedup_by(|a, b| {
        a.source == b.source && a.code == b.code && a.kind == b.kind && a.message == b.message
    });
    ModelPlan {
        symbols,
        diagnostics: planner.diagnostics,
        openapi_version: contract.openapi_version().into(),
        resource_validation,
    }
}

/// Executable resource requirements from the canonical effective closure. This
/// is the same source/dialect boundary used by model obligations and admission.
fn resource_requirements(contract: &Contract, closure: &[SchemaId]) -> BTreeSet<SchemaId> {
    let mut requirements = BTreeSet::new();
    for id in closure {
        let Some(schema) = contract.schema(id) else {
            continue;
        };
        if schema.ignores_ref_siblings() {
            continue;
        }
        if let Some(source) = contract
            .resource_scope(id)
            .and_then(|scope| scope.base_source())
        {
            requirements.insert(source.clone());
        }
        for keyword in ["$id", "$anchor", "$dynamicAnchor", "$dynamicRef"] {
            if schema.raw().get(keyword).is_some() {
                requirements.insert(id.child(keyword));
            }
        }
    }
    requirements
}

#[derive(Debug, Clone)]
pub(crate) enum Expr {
    At(SchemaId, Box<Self>),
    Any,
    Never,
    Primitive(Primitive),
    Literal(Literal),
    Reference(String),
    Array(Box<Self>),
    Object(Vec<FieldPlan>, Option<Box<Self>>),
    Union(Vec<Self>),
    Intersection(Vec<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Primitive {
    Null,
    Boolean,
    String,
    SafeInteger,
    Integer,
    Number,
    AnyNumber,
}

#[derive(Debug, Clone)]
pub(crate) enum Literal {
    Boolean(bool),
    String(String),
    Integer { value: String, safe: bool },
}

#[derive(Debug, Clone)]
pub(crate) struct FieldPlan {
    pub(crate) name: String,
    pub(crate) required: bool,
    pub(crate) expression: Expr,
    source: SchemaId,
    description: String,
}

impl Expr {
    fn render(&self) -> String {
        match self {
            Self::At(_, expression) => expression.render(),
            Self::Any => "JsonValue".into(),
            Self::Never => "never".into(),
            Self::Reference(value) => value.clone(),
            Self::Primitive(kind) => match kind {
                Primitive::Null => "null",
                Primitive::Boolean => "boolean",
                Primitive::String => "string",
                Primitive::SafeInteger => "number",
                Primitive::Integer => "bigint",
                Primitive::Number => "JsonNumber",
                Primitive::AnyNumber => "number | bigint | JsonNumber",
            }
            .into(),
            Self::Literal(value) => match value {
                Literal::Boolean(value) => value.to_string(),
                Literal::String(value) => quote(value),
                Literal::Integer { value, safe } => {
                    format!("{value}{}", if *safe { "" } else { "n" })
                }
            },
            Self::Array(item) => format!("({})[]", item.render()),
            Self::Object(fields, extra) => {
                let mut items = fields
                    .iter()
                    .map(|field| {
                        format!(
                            "{}{}{}: {}",
                            declaration_comment(&field.description, &source_text(&field.source), &format!("Wire property: {}. This model view makes presence {}. Nullability is represented independently. Source constraints remain codec obligations.", escape_prose(&quote(&field.name)), if field.required { "required" } else { "optional" })),
                            quote(&field.name),
                            if field.required { "" } else { "?" },
                            field.expression.render()
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(extra) = extra {
                    items.push(format!("[key: string]: {}", extra.render()));
                }
                format!("{{\n{}\n}}", items.join(";\n"))
            }
            Self::Union(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Self::render)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            Self::Intersection(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Self::render)
                    .collect::<Vec<_>>()
                    .join(" & ")
            ),
        }
    }

    fn union(items: Vec<Self>) -> Self {
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| !matches!(item.unlocated(), Self::Never))
            .collect();
        if items
            .iter()
            .any(|item| matches!(item.unlocated(), Self::Any))
        {
            return Self::Any;
        }
        match items.len() {
            0 => Self::Never,
            1 => items.into_iter().next().unwrap(),
            _ => Self::Union(items),
        }
    }

    fn intersection(items: Vec<Self>) -> Self {
        if items
            .iter()
            .any(|item| matches!(item.unlocated(), Self::Never))
        {
            return Self::Never;
        }
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| !matches!(item.unlocated(), Self::Any))
            .collect();
        match items.len() {
            0 => Self::Any,
            1 => items.into_iter().next().unwrap(),
            _ => Self::Intersection(items),
        }
    }

    fn unlocated(&self) -> &Self {
        match self {
            Self::At(_, expression) => expression.unlocated(),
            _ => self,
        }
    }
}

struct Planner<'a> {
    contract: &'a Contract,
    names: BTreeMap<(SchemaId, ModelView), String>,
    diagnostics: Vec<ModelDiagnostic>,
    policy: crate::schema_view::DialectPolicy,
}

impl Planner<'_> {
    fn report(
        &mut self,
        source: SchemaId,
        code: &'static str,
        kind: DiagnosticKind,
        message: impl Into<String>,
    ) {
        let at = self.contract.source_span(&source).unwrap_or(0..0);
        self.diagnostics.push(ModelDiagnostic {
            source,
            at,
            code,
            kind,
            message: message.into(),
        });
    }

    fn lower(&mut self, id: &SchemaId, view: ModelView) -> Expr {
        Expr::At(id.clone(), Box::new(self.lower_inner(id, view)))
    }

    fn lower_inner(&mut self, id: &SchemaId, view: ModelView) -> Expr {
        let Some(schema) = self.contract.schema(id) else {
            self.report(
                id.clone(),
                "missing-schema",
                DiagnosticKind::Error,
                "schema position was not indexed by the canonical contract",
            );
            return Expr::Never;
        };
        let raw_value = schema_view::raw(schema);
        let raw = &*raw_value;
        if let Some(boolean) = raw.as_bool() {
            return if boolean { Expr::Any } else { Expr::Never };
        }
        let Some(raw) = raw.as_object() else {
            return Expr::Never;
        };
        let oas30 = matches!(schema.dialect(), SchemaDialect::OpenApi30);
        let mut terms = Vec::new();
        if raw.contains_key("$ref") {
            if let Some(target) = schema
                .references()
                .iter()
                .find(|reference| reference.keyword == "$ref")
                .and_then(|reference| reference.target.as_ref())
            {
                if let Some(name) = self.names.get(&(target.clone(), view)) {
                    terms.push(Expr::Reference(name.clone()));
                } else {
                    self.report(
                        id.child("$ref"),
                        "unplanned-reference",
                        DiagnosticKind::Error,
                        "reference target has no allocated symbol",
                    );
                }
            } else {
                self.report(
                    id.child("$ref"),
                    "unresolved-model-reference",
                    DiagnosticKind::Error,
                    "reference has no resolved contract target",
                );
            }
            // OAS 3.0 Reference Objects do not apply Schema Object siblings.
            if oas30 {
                return Expr::intersection(terms);
            }
            if [
                "type",
                "enum",
                "const",
                "anyOf",
                "oneOf",
                "properties",
                "items",
            ]
            .iter()
            .any(|keyword| raw.contains_key(*keyword))
                && self.numeric_composition(id, &mut BTreeSet::new())
            {
                self.report(id.child("$ref"), "numeric-composition-representation", DiagnosticKind::Error, "a numeric reference and its siblings require a shared exact representation plan before they can be intersected");
            }
        }
        for keyword in [
            "$dynamicRef",
            "not",
            "if",
            "then",
            "else",
            "dependentSchemas",
            "dependentRequired",
            "patternProperties",
            "propertyNames",
            "unevaluatedProperties",
            "unevaluatedItems",
            "prefixItems",
            "contains",
            "minContains",
            "maxContains",
        ] {
            if raw.contains_key(keyword) {
                self.report(id.child(keyword), "applicator-validation-required", DiagnosticKind::CodecObligation, format!("`{keyword}` is enforced by the checked applicator codec; native types preserve values without flattening evaluated scopes"));
            }
        }
        for keyword in [
            "minimum",
            "maximum",
            "exclusiveMinimum",
            "exclusiveMaximum",
            "multipleOf",
            "minLength",
            "maxLength",
            "pattern",
            "minItems",
            "maxItems",
            "uniqueItems",
            "minProperties",
            "maxProperties",
        ] {
            if raw.contains_key(keyword) {
                self.report(id.child(keyword), "schema-validation-required", DiagnosticKind::CodecObligation, format!("the codec must enforce `{keyword}` for applicable instance types; TypeScript alone does not establish it"));
            }
        }
        for keyword in [
            "format",
            "contentEncoding",
            "contentMediaType",
            "contentSchema",
            "readOnly",
            "writeOnly",
            "discriminator",
        ] {
            if raw.contains_key(keyword) {
                self.report(id.child(keyword), "schema-annotation-retained", DiagnosticKind::Annotation, format!("`{keyword}` is retained; no extra validation, data stripping, or inferred union member is introduced"));
            }
        }
        if raw.contains_key("nullable") && !oas30 {
            self.report(id.child("nullable"), "nonstandard-nullable-annotation", DiagnosticKind::Annotation, "OpenAPI 3.1+ does not give nullable validation semantics; use a null type or union");
        }
        let object_constraints = ["properties", "required", "additionalProperties"]
            .iter()
            .any(|keyword| raw.contains_key(*keyword));
        let array_constraints = raw.contains_key("items") || raw.contains_key("prefixItems");
        if raw.contains_key("type") || object_constraints || array_constraints {
            let types = self.types(id, raw, oas30);
            let mut alternatives = Vec::new();
            for kind in types {
                alternatives.push(match kind.as_str() {
                    "null" => Expr::Primitive(Primitive::Null),
                    "boolean" => Expr::Primitive(Primitive::Boolean),
                    "string" => Expr::Primitive(Primitive::String),
                    "integer" => Expr::Primitive(if safe_integer(self.contract, id) {
                        Primitive::SafeInteger
                    } else {
                        Primitive::Integer
                    }),
                    "number" => Expr::Primitive(Primitive::Number),
                    "json-number" => Expr::Primitive(Primitive::AnyNumber),
                    "object" => self.object(id, raw, view),
                    "array" => Expr::Array(Box::new(if raw.contains_key("prefixItems") {
                        // Positional and remaining items can have different native
                        // representations. A checked exact-JSON item carrier keeps
                        // every value; the source program enforces the tuple rules.
                        Expr::Any
                    } else if raw.contains_key("items") {
                        self.lower(&id.child("items"), view)
                    } else {
                        Expr::Any
                    })),
                    _ => Expr::Never,
                });
            }
            terms.push(Expr::union(alternatives));
        }
        if let Some(values) = raw.get("enum") {
            if let Some(values) = values.as_array() {
                terms.push(Expr::union(
                    values
                        .iter()
                        .enumerate()
                        .map(|(i, value)| {
                            self.literal(&id.child("enum").child(&i.to_string()), value, raw, id)
                        })
                        .collect(),
                ));
            } else {
                self.report(
                    id.child("enum"),
                    "invalid-schema-enum",
                    DiagnosticKind::Error,
                    "enum must be an array",
                );
            }
        }
        if let Some(value) = raw.get("const") {
            if oas30 {
                self.report(
                    id.child("const"),
                    "unsupported-oas30-const",
                    DiagnosticKind::Error,
                    "const is not an OpenAPI 3.0 Schema Object keyword",
                );
            }
            terms.push(self.literal(&id.child("const"), value, raw, id));
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(items) = raw.get(keyword) {
                if let Some(items) = items.as_array().filter(|items| !items.is_empty()) {
                    if (keyword == "allOf" || raw.contains_key("type"))
                        && self.numeric_composition(id, &mut BTreeSet::new())
                    {
                        self.report(id.child(keyword), "numeric-composition-representation", DiagnosticKind::Error, "numeric intersections need a shared exact representation plan; bigint/number/JsonNumber aliases must not be intersected as incompatible JS representations");
                    }
                    if keyword == "oneOf" {
                        self.report(id.child(keyword), "oneof-exclusivity-required", DiagnosticKind::CodecObligation, "validate exactly one declared branch; a TypeScript union does not enforce exclusivity");
                    }
                    let members = (0..items.len())
                        .map(|index| self.lower(&id.child(keyword).child(&index.to_string()), view))
                        .collect();
                    terms.push(if keyword == "allOf" {
                        Expr::intersection(members)
                    } else {
                        Expr::union(members)
                    });
                } else {
                    self.report(
                        id.child(keyword),
                        "invalid-schema-composition",
                        DiagnosticKind::Error,
                        format!("`{keyword}` must be a nonempty schema array"),
                    );
                }
            }
        }
        Expr::intersection(terms)
    }

    fn types(&mut self, id: &SchemaId, raw: &Map<String, Value>, oas30: bool) -> Vec<String> {
        let mut types = match raw.get("type") {
            None => [
                "null",
                "boolean",
                "string",
                "json-number",
                "array",
                "object",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            Some(Value::String(kind)) => vec![kind.clone()],
            Some(Value::Array(kinds))
                if !oas30 && !kinds.is_empty() && kinds.iter().all(Value::is_string) =>
            {
                kinds
                    .iter()
                    .map(|kind| kind.as_str().unwrap().into())
                    .collect()
            }
            Some(_) => {
                self.report(
                    id.child("type"),
                    "invalid-schema-type",
                    DiagnosticKind::Error,
                    "type must declare a valid type or a nonempty OpenAPI 3.1+ type array",
                );
                Vec::new()
            }
        };
        for kind in &types {
            if !matches!(
                kind.as_str(),
                "null"
                    | "boolean"
                    | "string"
                    | "integer"
                    | "number"
                    | "array"
                    | "object"
                    | "json-number"
            ) || raw.contains_key("type") && (kind == "json-number" || oas30 && kind == "null")
            {
                self.report(
                    id.child("type"),
                    "unsupported-schema-type",
                    DiagnosticKind::Error,
                    format!("unsupported type `{kind}` in this schema dialect"),
                );
            }
        }
        if oas30
            && raw.get("nullable") == Some(&Value::Bool(true))
            && raw.get("type").is_some_and(Value::is_string)
        {
            types.push("null".into());
        }
        // The versioned OAS-3.0-nullable interpretation on OAS 3.1/3.2 nodes:
        // the same-object type gains (or loses) "null".
        if !oas30 && self.policy.oas30_nullable_in_31 {
            match raw.get("nullable") {
                Some(&Value::Bool(true)) if raw.get("type").is_some() => types.push("null".into()),
                Some(&Value::Bool(false)) => types.retain(|kind| kind != "null"),
                _ => {}
            }
        }
        types
    }

    fn object(&mut self, id: &SchemaId, raw: &Map<String, Value>, view: ModelView) -> Expr {
        let properties = raw.get("properties").and_then(Value::as_object);
        if raw.contains_key("properties") && properties.is_none() {
            self.report(
                id.child("properties"),
                "invalid-schema-properties",
                DiagnosticKind::Error,
                "properties must be an object",
            );
        }
        let mut required = BTreeSet::new();
        if let Some(value) = raw.get("required") {
            match value.as_array() {
                Some(values) if values.iter().all(Value::is_string) => required.extend(
                    values
                        .iter()
                        .map(|value| value.as_str().unwrap().to_owned()),
                ),
                _ => self.report(
                    id.child("required"),
                    "invalid-schema-required",
                    DiagnosticKind::Error,
                    "required must be an array of property names",
                ),
            }
        }
        let pattern_extras = raw
            .get("patternProperties")
            .and_then(Value::as_object)
            .is_some_and(|patterns| !patterns.is_empty());
        let closed =
            !pattern_extras && raw.get("additionalProperties") == Some(&Value::Bool(false));
        let typed_extra = !pattern_extras
            && raw
                .get("additionalProperties")
                .is_some_and(Value::is_object);
        let mut fields = Vec::new();
        for (name, _) in properties.into_iter().flatten() {
            let property_id = id.child("properties").child(name);
            let description = self
                .contract
                .schema(&property_id)
                .map(schema_view::description)
                .unwrap_or("")
                .to_owned();
            let directional = match view {
                ModelView::Neutral => false,
                ModelView::Request => {
                    self.directional_annotation(&property_id, "readOnly", &mut BTreeSet::new())
                }
                ModelView::Response => {
                    self.directional_annotation(&property_id, "writeOnly", &mut BTreeSet::new())
                }
            };
            let present = required.remove(name) && !directional;
            fields.push(FieldPlan {
                name: name.clone(),
                required: present,
                expression: self.lower(&property_id, view),
                source: property_id,
                description,
            });
        }
        for name in required {
            fields.push(FieldPlan {
                name,
                required: true,
                expression: if closed {
                    Expr::Never
                } else if typed_extra {
                    self.lower(&id.child("additionalProperties"), view)
                } else {
                    Expr::Any
                },
                source: id.child("required"),
                description: "Required key without a declared property description.".into(),
            });
        }
        self.report(id.clone(), "object-validation-required", DiagnosticKind::CodecObligation, "the codec must check object identity, required presence, exact property rules and all applicable compositions; TypeScript structural assignability is insufficient");
        let extra = if pattern_extras {
            // Every matching pattern applies, and overlaps are conjunctive.
            // A single static index-signature type cannot encode those rules.
            // Keep exact JSON values and let the source-bound v2 validator decide.
            Some(Box::new(Expr::Any))
        } else if closed {
            if fields.is_empty() {
                Some(Box::new(Expr::Never))
            } else {
                None
            }
        } else if typed_extra {
            Some(Box::new(
                self.lower(&id.child("additionalProperties"), view),
            ))
        } else {
            Some(Box::new(Expr::Any))
        };
        Expr::Object(fields, extra)
    }

    // Direct reference annotations use canonical targets, including external
    // references. The applicability proof below is the single authority shared
    // with the codec validation projection, so model requiredness and runtime
    // requiredness cannot disagree.
    fn directional_annotation(
        &mut self,
        id: &SchemaId,
        keyword: &str,
        seen: &mut BTreeSet<SchemaId>,
    ) -> bool {
        let contract = self.contract;
        let mut violations = |child: SchemaId, applicator: &'static str| {
            self.report(
                child,
                "directional-annotation-evaluation",
                DiagnosticKind::Error,
                format!("`{keyword}` under `{applicator}` requires evaluated annotation applicability before a directional requiredness view can be generated"),
            );
        };
        directional_annotation_walk(contract, id, keyword, seen, &mut violations)
    }

    fn literal(
        &mut self,
        id: &SchemaId,
        value: &Value,
        schema: &Map<String, Value>,
        owner: &SchemaId,
    ) -> Expr {
        match value {
            Value::Null => Expr::Primitive(Primitive::Null),
            Value::Bool(value) => Expr::Literal(Literal::Boolean(*value)),
            Value::String(value) => Expr::Literal(Literal::String(value.clone())),
            Value::Number(number) => {
                if schema.get("type").is_some_and(|types| {
                    types.as_str() == Some("number")
                        || types.as_array().is_some_and(|types| {
                            types.iter().any(|kind| kind.as_str() == Some("number"))
                        })
                }) {
                    self.report(id.clone(), "decimal-literal-representation", DiagnosticKind::Error, "numeric literal equality needs the exact decimal codec's canonical value representation");
                    return Expr::Never;
                }
                if let Some(integer) = integral_token(&number.to_string()) {
                    if let Some((lower, upper)) = safe_integer_bounds(self.contract, owner)
                        && !integer
                            .parse::<i64>()
                            .is_ok_and(|integer| (lower..=upper).contains(&integer))
                    {
                        // A safe bounded schema cannot accept this literal.
                        // Do not round it merely because the overall type uses number.
                        return Expr::Never;
                    }
                    Expr::Literal(Literal::Integer {
                        value: integer,
                        safe: safe_integer(self.contract, owner),
                    })
                } else {
                    self.report(id.clone(), "decimal-literal-representation", DiagnosticKind::Error, "nonintegral or exceptionally large numeric literals need a canonical exact decimal representation");
                    Expr::Never
                }
            }
            Value::Array(_) | Value::Object(_) => {
                self.report(id.clone(), "structured-literal-representation", DiagnosticKind::Error, "object/array const and enum equality needs a dedicated literal representation and codec plan");
                Expr::Never
            }
        }
    }

    // Stop at container boundaries: a nested numeric property does not make
    // the containing object intersection a numeric representation conflict.
    fn numeric_composition(&self, id: &SchemaId, seen: &mut BTreeSet<SchemaId>) -> bool {
        if !seen.insert(id.clone()) {
            return false;
        }
        let Some(schema) = self.contract.schema(id) else {
            return false;
        };
        let raw = schema_view::raw(schema);
        // Enum/const can constrain numbers without declaring a primitive type.
        // Their native literal representation must participate in composition
        // planning just as an explicit integer/number type does.
        if raw.get("const").is_some_and(Value::is_number)
            || raw
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|members| members.iter().any(Value::is_number))
        {
            return true;
        }
        if raw
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "number" | "integer"))
        {
            return true;
        }
        if raw
            .get("type")
            .and_then(Value::as_array)
            .is_some_and(|kinds| {
                kinds
                    .iter()
                    .any(|kind| matches!(kind.as_str(), Some("number" | "integer")))
            })
        {
            return true;
        }
        schema
            .references()
            .iter()
            .filter_map(|reference| reference.target.as_ref())
            .any(|target| self.numeric_composition(target, seen))
            || ["allOf", "anyOf", "oneOf"].iter().any(|keyword| {
                raw.get(*keyword)
                    .and_then(Value::as_array)
                    .is_some_and(|members| {
                        (0..members.len()).any(|i| {
                            self.numeric_composition(&id.child(keyword).child(&i.to_string()), seen)
                        })
                    })
            })
    }

    fn check_alias_cycles(&mut self, symbols: &[ModelSymbol]) {
        let by_name: BTreeMap<_, _> = symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol))
            .collect();
        for symbol in symbols {
            if alias_reaches(
                &symbol.expression,
                &symbol.name,
                &by_name,
                &mut BTreeSet::new(),
            ) {
                self.report(symbol.source.clone(), "unguarded-schema-recursion", DiagnosticKind::Error, "recursive aliases need an object or array boundary to be representable in TypeScript");
            }
        }
    }
}

// Single applicability authority for OpenAPI read/write annotations, shared by
// the model tracer's directional views and the codec validation projection, so
// both requiredness decisions cannot disagree. A position is proven annotated
// when its schema object declares the keyword itself or the annotation is
// reachable through an unconditional `$ref` target chain. Annotations
// reachable only through applicators are never proven; each applicator child
// carrying the annotation is reported with its position and enclosing keyword
// so unsupported applicability stays source-linked instead of guessed. OAS 3.0
// Reference Objects carry no schema siblings, so a ref-only position cannot
// itself declare the annotation, but its canonical target is still followed.
fn directional_annotation_walk(
    contract: &Contract,
    id: &SchemaId,
    keyword: &str,
    seen: &mut BTreeSet<SchemaId>,
    violations: &mut dyn FnMut(SchemaId, &'static str),
) -> bool {
    if !seen.insert(id.clone()) {
        return false;
    }
    let Some(schema) = contract.schema(id) else {
        return false;
    };
    let raw = schema.raw();
    let ref_only =
        matches!(schema.dialect(), SchemaDialect::OpenApi30) && raw.get("$ref").is_some();
    let mut present = !ref_only && raw.get(keyword) == Some(&Value::Bool(true));
    // JSON Schema Validation 2020-12 §9.4 combines applicable occurrences as
    // true when any occurrence is true; a false $ref sibling is not an override.
    for reference in schema.references() {
        if reference.keyword == "$dynamicRef" {
            violations(id.child("$dynamicRef"), "$dynamicRef");
        } else if let Some(target) = &reference.target {
            present |= directional_annotation_walk(contract, target, keyword, seen, violations);
        }
    }
    if !ref_only {
        for applicator in ["allOf", "anyOf", "oneOf", "if", "then", "else", "not"] {
            if let Some(value) = raw.get(applicator) {
                let children = value.as_array().map_or_else(
                    || vec![id.child(applicator)],
                    |members| {
                        (0..members.len())
                            .map(|i| id.child(applicator).child(&i.to_string()))
                            .collect()
                    },
                );
                for child in children {
                    if directional_annotation_walk(contract, &child, keyword, seen, violations) {
                        violations(child, applicator);
                    }
                }
            }
        }
    }
    present
}

// Proven directional requiredness exception for one declared property of one
// object schema position. Returns `true` only when the shared walk proves the
// view's annotation applicable to that exact property position; an
// unestablishable applicator child is returned so callers cannot guess.
fn directional_required_exception(
    contract: &Contract,
    object: &SchemaId,
    property: &str,
    view: ModelView,
) -> Result<bool, (SchemaId, &'static str)> {
    let keyword = match view {
        ModelView::Neutral => return Ok(false),
        ModelView::Request => "readOnly",
        ModelView::Response => "writeOnly",
    };
    let property = object.child("properties").child(property);
    let mut violation: Option<(SchemaId, &'static str)> = None;
    let present = directional_annotation_walk(
        contract,
        &property,
        keyword,
        &mut BTreeSet::new(),
        &mut |child, applicator| violation = Some((child, applicator)),
    );
    violation.map_or(Ok(present), Err)
}

fn alias_reaches<'a>(
    expr: &'a Expr,
    target: &str,
    symbols: &BTreeMap<&str, &'a ModelSymbol>,
    seen: &mut BTreeSet<&'a str>,
) -> bool {
    match expr {
        Expr::At(_, expression) => alias_reaches(expression, target, symbols, seen),
        Expr::Reference(name) => {
            name == target
                || seen.insert(name.as_str())
                    && symbols.get(name.as_str()).is_some_and(|symbol| {
                        alias_reaches(&symbol.expression, target, symbols, seen)
                    })
        }
        Expr::Union(items) | Expr::Intersection(items) => items
            .iter()
            .any(|item| alias_reaches(item, target, symbols, seen)),
        _ => false,
    }
}

fn allocate_names(
    contract: &Contract,
    ids: &BTreeSet<SchemaId>,
    views: &[ModelView],
) -> BTreeMap<(SchemaId, ModelView), String> {
    let hints = crate::model_naming::Hints::new(contract);
    let mut proposed: BTreeMap<String, Vec<(SchemaId, ModelView)>> = BTreeMap::new();
    for id in ids {
        let pointer = id
            .pointer()
            .strip_prefix("/components/schemas/")
            .unwrap_or(id.pointer());
        let decoded = pointer
            .split('/')
            .map(|token| token.replace("~1", "/").replace("~0", "~"))
            .collect::<Vec<_>>()
            .join("_");
        let base = hints
            .get(id)
            .map(|name| ts_identifier(&crate::rust_models::pascal(&name)))
            .unwrap_or_else(|| {
                ts_identifier(if decoded.is_empty() {
                    "Schema"
                } else {
                    &decoded
                })
            });
        for view in views.iter().copied().collect::<BTreeSet<_>>() {
            proposed
                .entry(format!("{base}{}", view.suffix()))
                .or_default()
                .push((id.clone(), view));
        }
    }
    let reserved = ["JsonValue", "JsonNumber", "$jsonNumber"];
    let mut used: BTreeSet<String> = proposed
        .keys()
        .cloned()
        .chain(reserved.map(str::to_owned))
        .collect();
    let mut names = BTreeMap::new();
    for (base, members) in proposed {
        if members.len() == 1 && !reserved.contains(&base.as_str()) {
            names.insert(members[0].clone(), base);
            continue;
        }
        for (source, view) in members {
            let identity = format!("{}#{:?}", source_text(&source), view);
            let hash = identity
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
                });
            let stem = format!("{base}_{hash:016x}");
            let mut name = stem.clone();
            let mut suffix = 2;
            while !used.insert(name.clone()) {
                name = format!("{stem}_{suffix}");
                suffix += 1;
            }
            names.insert((source, view), name);
        }
    }
    names
}

fn safe_integer(contract: &Contract, id: &SchemaId) -> bool {
    safe_integer_bounds(contract, id).is_some()
}

fn safe_integer_bounds(contract: &Contract, id: &SchemaId) -> Option<(i64, i64)> {
    let (lower, upper) = schema_view::integer_interval(contract.schema(id)?)?;
    (lower >= -9_007_199_254_740_991 && upper <= 9_007_199_254_740_991)
        .then_some((i64::try_from(lower).ok()?, i64::try_from(upper).ok()?))
}

// Exact decimal normalization for integer literals; no f64 conversion. Bounds
// cap source expansion, not the mathematical domain of bigint model values.
fn integral_token(token: &str) -> Option<String> {
    let (mantissa, exponent) = token
        .split_once(['e', 'E'])
        .map_or(Some((token, 0_i64)), |(mantissa, exponent)| {
            Some((mantissa, exponent.parse().ok()?))
        })?;
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{whole}{fraction}");
    let shift = exponent.checked_sub(i64::try_from(fraction.len()).ok()?)?;
    if shift >= 0 {
        let shift = usize::try_from(shift).ok()?;
        if digits.len().checked_add(shift)? > 4096 {
            return None;
        }
        digits.extend(std::iter::repeat_n('0', shift));
    } else {
        let remove = usize::try_from(shift.checked_neg()?).ok()?;
        if remove > digits.len()
            || !digits[digits.len() - remove..]
                .bytes()
                .all(|byte| byte == b'0')
        {
            return None;
        }
        digits.truncate(digits.len() - remove);
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        Some("0".into())
    } else {
        Some(format!("{}{digits}", if negative { "-" } else { "" }))
    }
}

fn quote(text: &str) -> String {
    serde_json::to_string(text)
        .expect("JSON string")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
fn source_text(id: &SchemaId) -> String {
    format!("{}#{}", id.document(), id.pointer())
}
fn escape_prose(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('`', "&#96;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('{', "&#123;")
        .replace('}', "&#125;")
        .replace('@', "&#64;")
        .replace('*', "&#42;")
        .replace('\\', "&#92;")
        .replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ")
}

// Source text is literal prose, not TSDoc tags, Markdown directives, HTML or
// examples. A generated prefix prevents source indentation becoming a code block.
fn declaration_comment(description: &str, source: &str, remarks: &str) -> String {
    let description = if description.trim().is_empty() {
        "No description is provided by the source schema."
    } else {
        description
    };
    format!(
        "/**\n * Source description: {}\n *\n * @remarks\n * OpenAPI source: {}\n *\n * {remarks}\n */\n",
        escape_prose(description)
            .replace('_', "&#95;")
            .replace('~', "&#126;"),
        escape_prose(source)
    )
}

const SUPPORT_TYPES: &str = r#"import type { JsonNumber, JsonValue } from './json.js';
export { JsonNumber } from './json.js';
export type { JsonValue } from './json.js';

"#;
