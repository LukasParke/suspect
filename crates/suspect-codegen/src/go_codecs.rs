//! Source-aware Go codecs from retained native descriptors and portable checks.
pub use crate::rust_codecs::{CodecConfig, JsonLimits};
use crate::{
    OutFile,
    go_models::ModelPlan,
    rust_models::{DiagnosticKind, ModelDiagnostic},
};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_schema::{OwnedCompiler, OwnedProgram};
mod emit;
#[derive(Debug)]
pub struct CodecPlan {
    models: ModelPlan,
    files: Vec<OutFile>,
    validation: OwnedProgram,
}
impl CodecPlan {
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }
    pub fn render(&self) -> Vec<OutFile> {
        self.files.clone()
    }
    /// The checked program actually executed by these codecs (v1 or scoped v2).
    pub fn validation_program(&self) -> &OwnedProgram {
        &self.validation
    }
}
pub fn plan_codecs(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    let models = crate::go_models::plan_models(&contract, roots);
    if models.has_errors() {
        return Err(models.diagnostics().to_vec());
    }
    let source = roots
        .first()
        .cloned()
        .unwrap_or_else(|| SchemaId::new(contract.entry().clone(), Default::default()));
    if config.max_conversion_depth > 256 || config.json_limits.max_depth > 256 {
        return Err(vec![ModelDiagnostic {
            at: contract.source_span(&source).unwrap_or(0..0),
            source,
            code: "codec-resource-policy",
            kind: DiagnosticKind::Error,
            message: "Go codec conversion/JSON depth must not exceed 256".into(),
        }]);
    }
    let validation_roots = crate::schema_view::closure(
        &contract,
        &models
            .symbols()
            .iter()
            .map(|s| s.source().clone())
            .collect::<Vec<_>>(),
    );
    let compiler = OwnedCompiler::new(config.schema.clone());
    let validator = if crate::go_models::requires_resources(&contract, &validation_roots) {
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
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let program = validator.program();
    let mut files = crate::go_models::render_codecs(&models)?;
    files.extend(crate::go_validation::emit(&program).map_err(|e| {
        vec![ModelDiagnostic {
            at: contract.source_span(&source).unwrap_or(0..0),
            source,
            code: "codec-validation-emission",
            kind: DiagnosticKind::Error,
            message: e.message,
        }]
    })?);
    let indices: BTreeMap<_, _> = program
        .roots
        .iter()
        .map(|root| {
            (
                (root.source.document.clone(), root.source.pointer.clone()),
                root.target,
            )
        })
        .collect();
    files.extend(emit::render(&models, &indices, &config, &program));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(CodecPlan {
        models,
        files,
        validation: program,
    })
}
