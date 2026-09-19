//! Model codecs from the typed language plan and compiled owned assertions.
//!
//! This API plans the requested model views (neutral, request, response) of
//! admitted OAS 3.1 / JSON Schema 2020-12 models. It rejects unsupported model
//! or validation features, and directional annotations whose applicability
//! cannot be established, before returning artifacts. HTTP clients and release
//! certification remain separate.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramSource};

use super::{DiagnosticKind, Expr, Literal, ModelDiagnostic, ModelPlan, ModelView, Primitive};
use crate::OutFile;

/// Resource policies for generated validation and model conversion.
#[derive(Debug, Clone)]
pub struct CodecConfig {
    /// Exact schema compilation and per-call validation limits.
    pub validation: Config,
    /// Maximum nested conversion expressions, including reference/source nodes.
    pub max_conversion_depth: usize,
    /// Shared conversion visits across branches, containers and merge work.
    pub max_conversion_steps: usize,
    /// Maximum decimal digits when materializing a native mathematical integer.
    pub max_integer_digits: usize,
    /// Versioned source dialect interpretation choices; the default preserves
    /// the ordinary strict dialect semantics.
    pub dialect: crate::schema_view::DialectPolicy,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            validation: Config::default(),
            max_conversion_depth: 256,
            max_conversion_steps: 100_000,
            max_integer_digits: 4096,
            dialect: crate::schema_view::DialectPolicy::default(),
        }
    }
}

/// Immutable model, runtime, compiled program and documentation artifacts.
pub struct CodecPlan {
    models: ModelPlan,
    files: Vec<OutFile>,
    interfaces: BTreeMap<String, Value>,
    validation_profile: (&'static str, &'static str),
}

impl std::fmt::Debug for CodecPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecPlan")
            .field("models", &self.models.symbols().len())
            .field("artifacts", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl CodecPlan {
    /// The original standalone model plan. Its codec obligations describe what
    /// model-only rendering requires; this plan's artifacts implement them.
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }

    /// Native codec behavior from the actual checked per-view program. Keys are
    /// allocated model names. Sources and prose stay on the model records; exact
    /// literal operands and scoped instruction/limit semantics remain intact.
    #[must_use]
    pub fn interfaces(&self) -> &BTreeMap<String, Value> {
        &self.interfaces
    }

    /// Exact checked executable version/profile pair. Ordinary base closures
    /// retain v1 even though admission uses the additive `compile_v2` API.
    #[must_use]
    pub fn validation_profile(&self) -> (&'static str, &'static str) {
        self.validation_profile
    }

    /// Deterministic generated artifacts, without rereading or reinterpreting
    /// source schemas. Use the shared artifact writer for filesystem changes.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        self.files.clone()
    }
}

/// Plan neutral models and their validated codecs from one owned Contract.
/// All public referenced symbols receive their own codec and validation root.
///
/// # Errors
/// Returns source-linked model, schema compilation, or resource-policy errors.
/// No artifact set is returned unless every selected closure is representable
/// and every source assertion is supported by the owned validator. Successful
/// planning does not certify a complete SDK or provide an HTTP transport.
pub fn plan_codecs(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    plan_codecs_with_views(contract, roots, &[ModelView::Neutral], config)
}

/// Plan the selected model views and their validated codecs from one owned
/// Contract. `Neutral`, `Request` and `Response` may be combined in one
/// package; every (source, view) symbol receives its own codec bound to a
/// validation program projected for that view. Request/response projections
/// relax only the required presence proven directional by the shared
/// annotation applicability walk; all other assertions, branch validation and
/// recursion are retained, and every provided value still validates. Program
/// modules are deduplicated by content and named deterministically, so a
/// closure without directional annotations emits exactly one validation
/// program regardless of the requested views.
///
/// # Errors
/// Returns source-linked model, schema compilation, or resource-policy errors.
/// No artifact set is returned unless every selected closure is representable
/// and every source assertion is supported by the owned validator. Directional
/// annotations whose applicability cannot be established fail planning
/// explicitly. Successful planning does not certify a complete SDK or provide
/// an HTTP transport.
pub fn plan_codecs_with_views(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    views: &[ModelView],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    let models = super::plan_models_with_policy(&contract, roots, views, config.dialect);
    if models.has_errors() {
        let mut errors = models.diagnostics.clone();
        for error in &mut errors {
            // Keep the existing codec API's schema-capability classification
            // when the unsupported directional profile is detected earlier by
            // the shared model view. Model-only APIs retain the precise code.
            if error.code == "oas30-directional-required-unsupported" {
                error.code = "codec-schema-compilation";
            }
        }
        return Err(errors);
    }
    let fallback = roots
        .first()
        .cloned()
        .unwrap_or_else(|| SchemaId::new(contract.entry().clone(), Default::default()));
    if views.is_empty() {
        return Err(vec![finding(
            &contract,
            fallback,
            "missing-model-view",
            "at least one model view is required".into(),
        )]);
    }
    let mut errors = Vec::new();
    for (name, limit) in [
        ("max_conversion_depth", config.max_conversion_depth),
        ("max_conversion_steps", config.max_conversion_steps),
        ("max_integer_digits", config.max_integer_digits),
    ] {
        if limit as u128 > 9_007_199_254_740_991 {
            errors.push(finding(
                &contract,
                fallback.clone(),
                "codec-resource-policy",
                format!("{name} must be a JavaScript safe metadata integer"),
            ));
        }
    }
    // Discharge only obligations whose semantics are enforced below. New model
    // obligations must deliberately establish their corresponding runtime gate.
    for diagnostic in &models.diagnostics {
        if diagnostic.kind == DiagnosticKind::CodecObligation
            && !matches!(
                diagnostic.code,
                "model-codec-unimplemented"
                    | "schema-validation-required"
                    | "oneof-exclusivity-required"
                    | "object-validation-required"
                    | "applicator-validation-required"
                    | "resource-validation-required"
            )
        {
            errors.push(finding(
                &contract,
                diagnostic.source.clone(),
                "codec-obligation-unsupported",
                diagnostic.message.clone(),
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let validation_roots: Vec<_> = models
        .symbols
        .iter()
        .map(|symbol| symbol.source.clone())
        .collect();
    let mut validation_config = config.validation;
    validation_config.oas30_nullable_in_31 = config.dialect.oas30_nullable_in_31;
    let compiler = OwnedCompiler::new(validation_config);
    let schema = if models.resource_validation {
        compiler.compile_v3(contract.clone(), &validation_roots)
    } else {
        compiler.compile_v2(contract.clone(), &validation_roots)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| ModelDiagnostic {
                source: error.source,
                at: error.span.unwrap_or(0..0),
                code: "codec-schema-compilation",
                kind: DiagnosticKind::Error,
                message: format!("{:?}: {}", error.kind, error.message),
            })
            .collect::<Vec<_>>()
    })?;
    let program = schema.program();
    let view_programs = super::directional::view_programs(&contract, program.clone(), views)
        .map_err(|error| vec![error])?;
    let emit_validation = |emitted: &OwnedProgram| -> Result<Vec<OutFile>, Vec<ModelDiagnostic>> {
        super::validation::emit(emitted).map_err(|error| {
            vec![finding(
                &contract,
                error.source.as_ref().map_or_else(
                    || fallback.clone(),
                    |source| source_id(&contract, source, &fallback),
                ),
                "codec-validation-emission",
                error.message,
            )]
        })
    };
    let mut validation_files = emit_validation(view_programs.base())?;
    for (file, projected) in view_programs.programs.iter().skip(1) {
        let module = emit_validation(projected)?
            .into_iter()
            .find(|artifact| artifact.path == "typescript/validation-program.ts")
            .map(|artifact| OutFile {
                path: (*file).to_owned(),
                content: artifact.content,
            })
            .expect("validation emission always contains its program module");
        validation_files.push(module);
    }
    let indices: BTreeMap<_, _> = models
        .symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| (symbol.name.as_str(), index))
        .collect();
    let names: BTreeMap<_, _> = models
        .symbols
        .iter()
        .map(|symbol| (symbol.name.clone(), format!("{}Codec", symbol.name)))
        .collect();
    let mut reserved: BTreeSet<_> = names.values().cloned().collect();
    reserved.extend(
        [
            "Codec",
            "ModelCodecError",
            "JsonLimits",
            "ValidationSource",
            "ValidationFinding",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let create = private_name("__suspectCreateCodec", &mut reserved);
    let lazy = private_name("__suspectCreateLazyCodec", &mut reserved);
    let model_types = private_name("__SuspectModels", &mut reserved);
    let conversion_type = private_name("__SuspectConversionProgram", &mut reserved);
    let mut aliases: BTreeMap<(&'static str, usize), String> = BTreeMap::new();
    for (file, _) in &view_programs.programs {
        for (index, _) in program.roots.iter().enumerate() {
            let alias = private_name(&format!("__suspectValidate{index}"), &mut reserved);
            aliases.insert((*file, index), alias);
        }
    }
    let validator_imports = view_programs
        .programs
        .iter()
        .map(|(file, _)| {
            let imported = (0..program.roots.len())
                .map(|index| format!("validateRoot{index} as {}", aliases[&(*file, index)]))
                .collect::<Vec<_>>()
                .join(", ");
            let module = file
                .rsplit('/')
                .next()
                .expect("module path has a file name")
                .replace(".ts", ".js");
            format!("import {{ {imported} }} from './{module}';")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let root_indices = program
        .roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            (
                (root.source.document.clone(), root.source.pointer.clone()),
                index,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expressions: Vec<_> = models
        .symbols
        .iter()
        .map(|symbol| descriptor(&symbol.expression, &indices))
        .collect();
    let interfaces = models.symbols.iter().map(|symbol| {
        let file = view_programs.bindings[&symbol.view];
        let projected = &view_programs.programs.iter().find(|(name, _)| *name == file).expect("view program exists").1;
        let root = &projected.roots[root_indices[&(symbol.source.document().to_string(), symbol.source.pointer().to_owned())]];
        (symbol.name.clone(), json!({
            "name":names[&symbol.name],
            "validation":super::validation::interface(projected, root.target),
            "conversion":{"maxDepth":config.max_conversion_depth,"maxSteps":config.max_conversion_steps,"maxIntegerDigits":config.max_integer_digits},
            "encode":"revalidate-current-wire-value",
        }))
    }).collect();
    let expression_declarations = expressions.iter().enumerate().map(|(index, expression)| format!("const __suspectExpression{index}: NonNullable<{conversion_type}['symbols'][number]> = {};",script_json(expression))).collect::<Vec<_>>().join("\n");
    let mut module = format!(
        "// Generated model conversions and checked schema program share one Contract.\nimport {{ createCodec as {create}, createLazyCodec as {lazy}, type ConversionProgram as {conversion_type}, type ModelCodec as Codec }} from './codecs.js';\nimport type * as {model_types} from './models.js';\n{validator_imports}\nexport type {{ ModelCodec as Codec }} from './codecs.js';\nexport {{ ModelCodecError }} from './codecs.js';\nexport type {{ JsonLimits }} from './json.js';\nexport type {{ ValidationSource, ValidationFinding }} from './validation.js';\n\n{expression_declarations}\n\n"
    );
    let resource_scopes = program
        .resource_context
        .as_ref()
        .map(|context| {
            let scopes: BTreeMap<_, _> = program
                .nodes
                .iter()
                .zip(&context.node_scopes)
                .map(|(node, (resource, _, _))| {
                    (
                        serde_json::to_string(&[
                            node.source.document.as_str(),
                            node.source.pointer.as_str(),
                        ])
                        .unwrap(),
                        resource,
                    )
                })
                .collect();
            module.push_str(&format!(
                "const __suspectResourceScopes = {};\n",
                script_json(&json!(scopes))
            ));
            ",resourceScopes:__suspectResourceScopes"
        })
        .unwrap_or("");
    for (index, symbol) in models.symbols.iter().enumerate() {
        let file = view_programs.bindings[&symbol.view];
        let validator = &aliases[&(
            file,
            root_indices[&(
                symbol.source.document().to_string(),
                symbol.source.pointer().to_owned(),
            )],
        )];
        let reachable = reachable_expressions(index, &models.symbols, &indices);
        let table = reachable
            .iter()
            .map(|target| format!("{target}:__suspectExpression{target}"))
            .collect::<Vec<_>>()
            .join(",");
        module.push_str(&super::declaration_comment(&symbol.description, &super::source_text(&symbol.source), &format!("Validated encode/decode for {}. Model view: {:?}. HTTP transport is outside this codec module.", symbol.name, symbol.view)));
        module.push_str(&format!("export const {}: Codec<{model_types}.{}> = /* @__PURE__ */ {lazy}(() => {create}({}, {index}, {{symbols:Object.assign([],{{{table}}}),maxDepth:{},maxSteps:{},maxIntegerDigits:{}{resource_scopes}}}, {validator}));\n\n", names[&symbol.name], symbol.name, script_json(&source(&symbol.source)),config.max_conversion_depth,config.max_conversion_steps,config.max_integer_digits));
    }
    let mut files = BTreeMap::new();
    for mut file in models
        .render_with_codecs(Some(&names))
        .expect("model errors checked before codec planning")
        .into_iter()
        .chain(validation_files)
        .chain([
            OutFile {
                path: "typescript/codecs.ts".into(),
                content: include_str!("codecs.ts").into(),
            },
            OutFile {
                path: "typescript/model-codecs.ts".into(),
                content: module,
            },
            OutFile {
                path: "typescript/codecs.md".into(),
                content: documentation(&models, &names),
            },
        ])
    {
        if file.path == "typescript/docs-manifest.json" {
            let mut manifest: Value =
                serde_json::from_str(&file.content).expect("model docs manifest");
            manifest["validationVersion"] = json!(program.version);
            manifest["validationProfile"] = json!(program.profile);
            file.content = serde_json::to_string_pretty(&manifest).unwrap();
        }
        if program.version == OwnedProgram::V2_VERSION
            && matches!(
                file.path.as_str(),
                "typescript/codecs.md" | "typescript/docs-readme.md"
            )
        {
            file.content
                .push_str(super::validation::SCOPED_DOCUMENTATION);
        }
        if program.version == OwnedProgram::V3_VERSION
            && matches!(
                file.path.as_str(),
                "typescript/codecs.md" | "typescript/docs-readme.md"
            )
        {
            file.content
                .push_str(super::validation::RESOURCE_DOCUMENTATION);
        }
        if let Some(previous) = files.insert(file.path.clone(), file.clone()) {
            assert_eq!(
                previous.content, file.content,
                "shared runtime artifact must have one identity"
            );
        }
    }
    Ok(CodecPlan {
        models,
        files: files.into_values().collect(),
        interfaces,
        validation_profile: (program.version, program.profile),
    })
}

pub(super) fn finding(
    contract: &Contract,
    source: SchemaId,
    code: &'static str,
    message: String,
) -> ModelDiagnostic {
    ModelDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        kind: DiagnosticKind::Error,
        message,
    }
}

pub(super) fn source_id(
    contract: &Contract,
    source: &ProgramSource,
    fallback: &SchemaId,
) -> SchemaId {
    let Some((uri, _)) = contract
        .documents()
        .find(|(uri, _)| uri.to_string() == source.document)
    else {
        return fallback.clone();
    };
    let root = SchemaId::new(uri.clone(), Default::default());
    source
        .pointer
        .strip_prefix('/')
        .map_or(root.clone(), |pointer| {
            pointer.split('/').fold(root, |parent, token| {
                parent.child(&token.replace("~1", "/").replace("~0", "~"))
            })
        })
}

fn source(id: &SchemaId) -> Value {
    json!({"document":id.document().to_string(),"pointer":id.pointer()})
}

fn script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn private_name(base: &str, reserved: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 0;
    while !reserved.insert(name.clone()) {
        suffix += 1;
        name = format!("{base}_{suffix}");
    }
    name
}

fn reachable_expressions(
    root: usize,
    symbols: &[super::ModelSymbol],
    indices: &BTreeMap<&str, usize>,
) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if !reached.insert(index) {
            continue;
        }
        expression_references(&symbols[index].expression, indices, &mut pending);
    }
    reached
}

fn expression_references(
    expression: &Expr,
    indices: &BTreeMap<&str, usize>,
    pending: &mut Vec<usize>,
) {
    match expression {
        Expr::At(_, inner) | Expr::Array(inner) => expression_references(inner, indices, pending),
        Expr::Reference(name) => pending.push(indices[name.as_str()]),
        Expr::Object(fields, extra) => {
            for field in fields {
                expression_references(&field.expression, indices, pending);
            }
            if let Some(extra) = extra {
                expression_references(extra, indices, pending);
            }
        }
        Expr::Union(items) | Expr::Intersection(items) => {
            for item in items {
                expression_references(item, indices, pending);
            }
        }
        Expr::Any | Expr::Never | Expr::Primitive(_) | Expr::Literal(_) => {}
    }
}

fn descriptor(expression: &Expr, indices: &BTreeMap<&str, usize>) -> Value {
    match expression {
        Expr::At(id, expression) => {
            json!({"kind":"source","source":source(id),"expression":descriptor(expression, indices)})
        }
        Expr::Any => json!({"kind":"any"}),
        Expr::Never => json!({"kind":"never"}),
        Expr::Primitive(kind) => json!({"kind":match kind {
            Primitive::Null=>"null", Primitive::Boolean=>"boolean", Primitive::String=>"string", Primitive::SafeInteger=>"safeInteger",
            Primitive::Integer=>"integer", Primitive::Number=>"number", Primitive::AnyNumber=>"anyNumber",
        }}),
        Expr::Literal(Literal::Boolean(value)) => json!({"kind":"literal","value":value}),
        Expr::Literal(Literal::String(value)) => json!({"kind":"literal","value":value}),
        Expr::Literal(Literal::Integer { value, safe }) => {
            json!({"kind":"integerLiteral","value":value,"safe":safe})
        }
        Expr::Reference(name) => json!({"kind":"reference","target":indices[name.as_str()]}),
        Expr::Array(item) => json!({"kind":"array","item":descriptor(item,indices)}),
        Expr::Object(fields, extra) => {
            json!({"kind":"object","fields":fields.iter().map(|field| json!({"name":field.name,"required":field.required,"expression":descriptor(&field.expression,indices)})).collect::<Vec<_>>(),"extra":extra.as_ref().map(|expr|descriptor(expr,indices))})
        }
        Expr::Union(alternatives) => {
            json!({"kind":"union","alternatives":alternatives.iter().map(|expr|descriptor(expr,indices)).collect::<Vec<_>>()})
        }
        Expr::Intersection(members) => {
            json!({"kind":"intersection","members":members.iter().map(|expr|descriptor(expr,indices)).collect::<Vec<_>>()})
        }
    }
}

fn documentation(models: &ModelPlan, names: &BTreeMap<String, String>) -> String {
    let mut docs = String::from(
        "# TypeScript model codecs\n\nThese experimental view-aware codecs consume exact JSON text. Each decoded value first passes the compiled source schema of its model view; conversion then preserves native representations, every permitted property, omitted members and null. Request and response views apply exactly the shared directional applicability proof to compiled validation: only required presence proven directional for the view is relaxed, every other assertion is unchanged, no wire property is stripped, no default is injected, and every provided value still validates. This is not an HTTP client or a complete SDK release. Only the emitted closure and the validator's documented subset are admitted. Unsupported features cause planning errors.\n\nUse `FooCodec.decode(text)` and `FooCodec.encode(value)` from `model-codecs.ts`. Parsing occurs once during decode and source validation occurs once per decode/encode. Encode serializes the input, reparses exact wire values, and validates that wire value. Own enumerable string-named data properties with undefined are omitted, with requiredness checked afterward; null is retained and undefined array members fail. No defaults or source-data coercion is applied.\n\nIntegers use bigint unless the model proves a safe native number range; general decimals retain JsonNumber. When more than one validated union branch can represent a value, the first convertible branch in the deterministic model plan wins. A conversion/evaluation limit never becomes an ordinary branch mismatch. Intersections merge typed projections recursively, preserving generic passthrough values without overwriting typed fields.\n\nModelCodecError distinguishes json, invalid, evaluation, representation and limit failures, with original document/schema and instance pointers where available. JSON representation, schema evaluation and model conversion have independent limits; these are not a total allocation or byte-work bound. Generated modules and standard JavaScript builtins are trusted.\n\n## Public codecs\n\n",
    );
    for symbol in &models.symbols {
        docs.push_str(&format!(
            "- `{}` → `{}` ({:?} view); source `{}`.\n",
            names[&symbol.name],
            symbol.name,
            symbol.view,
            super::escape_prose(&super::source_text(&symbol.source))
        ));
    }
    docs
}
