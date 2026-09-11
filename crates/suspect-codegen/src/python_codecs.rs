//! Source-aware native Python codecs planned from immutable model descriptors.
//!
//! Companion to `rust_codecs`: consumes the retained `pub(crate)`
//! `PyDecl`/`PyType` descriptors of `python_models` (never re-parsed Python
//! source) and emits typed `<Model>Codec` values whose `decode`/`encode`
//! are source-bound: they validate through the same checked `OwnedProgram`
//! the host validation emitter produces, and convert via exact per-schema
//! codecs, never broad `Any` casts.

use crate::{
    OutFile,
    python_models::ModelPlan,
    rust_models::{DiagnosticKind, ModelDiagnostic},
};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};

mod emit;

/// Finite exact JSON transport policy embedded in the generated codecs.
#[derive(Debug, Clone, Copy)]
pub struct JsonLimits {
    /// Maximum input bytes.
    pub max_input_bytes: usize,
    /// Maximum output bytes.
    pub max_output_bytes: usize,
    /// Maximum JSON nesting (at most 256).
    pub max_depth: usize,
    /// Maximum parser/writer work.
    pub max_work: usize,
}

impl Default for JsonLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 8 * 1024 * 1024,
            max_output_bytes: 8 * 1024 * 1024,
            max_depth: 128,
            max_work: 32 * 1024 * 1024,
        }
    }
}

/// Schema, JSON and native conversion limits. Each codec call has fresh budgets.
#[derive(Debug, Clone)]
pub struct CodecConfig {
    /// Owned schema compilation and evaluation policy.
    pub schema: Config,
    /// Exact JSON parsing/writing policy.
    pub json_limits: JsonLimits,
    /// Maximum nested conversion expressions, at most 256.
    pub max_conversion_depth: usize,
    /// Shared conversion visits and copied string bytes.
    pub max_conversion_steps: usize,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            schema: Config::default(),
            json_limits: JsonLimits::default(),
            max_conversion_depth: 128,
            max_conversion_steps: 32 * 1024 * 1024,
        }
    }
}

/// Complete immutable native Python package and its source model identities.
#[derive(Debug)]
pub struct CodecPlan {
    models: ModelPlan,
    files: Vec<OutFile>,
    /// Wire map: (document, pointer) -> validation program root index.
    indices: BTreeMap<(String, String), usize>,
    validation: OwnedProgram,
    config: CodecConfig,
}

impl CodecPlan {
    /// Original model-only plan; its codec obligations are discharged here.
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }

    /// The exact checked profile and instructions executed by the native codecs.
    #[must_use]
    pub fn validation_program(&self) -> &OwnedProgram {
        &self.validation
    }

    /// Render the complete planned Python package (models, runtimes, codecs).
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        let mut files = self.files.clone();
        files.extend(emit::package(&self.models, &self.indices, &self.config));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }

    /// Validation root index for a source identity, for review tooling.
    #[must_use]
    pub fn root_index(&self, document: &str, pointer: &str) -> Option<usize> {
        self.indices
            .get(&(document.to_owned(), pointer.to_owned()))
            .copied()
    }
}

/// Plan exact, validated Python codecs for every public symbol and union branch.
///
/// # Errors
/// Source-linked unsupported representations, assertions, or resource policies
/// reject the entire artifact set. This is not an HTTP SDK or release certification.
pub fn plan_codecs(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    let models = crate::python_models::plan_models(&contract, roots);
    if models.has_errors() {
        return Err(models.diagnostics().to_vec());
    }
    let fallback = roots
        .first()
        .cloned()
        .unwrap_or_else(|| SchemaId::new(contract.entry().clone(), Default::default()));
    let finding = |source: SchemaId, code, message: String| ModelDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        kind: DiagnosticKind::Error,
        message,
    };
    if config.max_conversion_depth > 256 || config.json_limits.max_depth > 256 {
        return Err(vec![finding(
            fallback,
            "codec-resource-policy",
            "conversion and JSON nesting must not exceed the native stack ceiling of 256".into(),
        )]);
    }
    // Public/referenced model sources plus every union branch source become
    // validation program roots.
    let mut validation_roots: Vec<_> = models
        .symbols()
        .iter()
        .map(|s| s.source().clone())
        .collect();
    for decl in models.declarations.values() {
        decl.collect_validation_roots(&mut validation_roots);
    }
    validation_roots.sort();
    validation_roots.dedup();
    validation_roots = crate::schema_view::closure(&contract, &validation_roots);
    let compiler = OwnedCompiler::new(config.schema.clone());
    let compiled = if crate::python_models::requires_resources(&contract, &validation_roots) {
        compiler.compile_v3(contract.clone(), &validation_roots)
    } else {
        compiler.compile_v2(contract.clone(), &validation_roots)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| ModelDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "codec-schema-compilation",
                kind: DiagnosticKind::Error,
                message: format!("{:?}: {}", e.kind, e.message),
            })
            .collect::<Vec<_>>()
    })?;
    let program = compiled.program();
    let validation_files = crate::python_validation::emit(&program).map_err(|e| {
        vec![finding(
            fallback.clone(),
            "codec-validation-emission",
            e.message,
        )]
    })?;
    let indices: BTreeMap<_, _> = program
        .roots
        .iter()
        .map(|r| {
            (
                (r.source.document.clone(), r.source.pointer.clone()),
                r.target,
            )
        })
        .collect();
    let mut files = crate::python_models::render_codecs(&models)?;
    files.extend(validation_files);
    Ok(CodecPlan {
        models,
        files,
        indices,
        validation: program,
        config,
    })
}
