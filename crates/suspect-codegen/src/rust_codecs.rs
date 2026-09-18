//! Source-aware native Rust codecs planned from immutable model descriptors.

use crate::{
    OutFile,
    rust_models::{self, Decl, DiagnosticKind, ModelDiagnostic, ModelPlan},
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
    /// Versioned source dialect interpretation choices; the default preserves
    /// the ordinary strict dialect semantics.
    pub dialect: crate::schema_view::DialectPolicy,
}
impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            schema: Config::default(),
            json_limits: JsonLimits::default(),
            max_conversion_depth: 128,
            max_conversion_steps: 32 * 1024 * 1024,
            dialect: crate::schema_view::DialectPolicy::default(),
        }
    }
}
/// Complete immutable native package and its source model identities.
#[derive(Debug)]
pub struct CodecPlan {
    models: ModelPlan,
    files: Vec<OutFile>,
    indices: BTreeMap<(String, String), usize>,
    config: CodecConfig,
    validation_version: &'static str,
    validation_profile: &'static str,
}
impl CodecPlan {
    /// Checked validation envelope selected by this codec plan.
    #[must_use]
    pub fn validation_version(&self) -> &'static str {
        self.validation_version
    }
    #[must_use]
    pub fn validation_profile(&self) -> &'static str {
        self.validation_profile
    }
    /// Original model-only plan; its obligations are discharged by this codec package.
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }
    /// Render the complete planned package without source inference.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        self.render_for_crate("generated_models")
    }

    /// Render for a crate name already validated by the native package planner.
    pub(crate) fn render_for_crate(&self, crate_name: &str) -> Vec<OutFile> {
        let mut files = self.files.clone();
        files.extend(emit::package(
            &self.models,
            &self.indices,
            &self.config,
            crate_name,
        ));
        if self.validation_version == OwnedProgram::V2_VERSION
            && let Some(guide) = files.iter_mut().find(|file| file.path == "rust/README.md")
        {
            guide.content.push_str("\n## Scoped-applicator validation v2\n\nThis package implements the checked `oas31-jsonschema202012-static-applicators` profile. Conditional branches, dependencies, exact contains counts, overlapping property patterns, property names and unevaluated locations use fresh per-schema annotation scopes. Whole-root validation retains these assertions on decode and mutable encoding. Patterned extras use exact JSON values independently of unmatched additional-property constraints; positional arrays retain heterogeneous JSON values. These carriers do not flatten conditional requirements or substitute for their source codecs.\n");
        }
        if self.validation_version == OwnedProgram::V3_VERSION
            && let Some(guide) = files.iter_mut().find(|file| file.path == "rust/README.md")
        {
            guide.content.push_str("\n## Resource/dynamic validation v3\n\nThe checked `oas31-jsonschema202012-resources-dynamic` program enters each node's indexed physical resource and resolves dynamic anchors from the outermost actually entered resource. Candidate declarations do not select runtime targets. Logical URI metadata never replaces physical source ownership and triggers no acquisition. Targets start with fresh annotation scopes; entered resources unwind after every return and branch trial. Dynamic values and context-dependent union conversions use exact JSON carriers, not a statically chosen fallback. Whole-root codecs revalidate mutable models under their actual entry context. `validation::resources()` and `node_scopes()` expose read-only indexed metadata.\n");
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }
}

/// Base standalone package manifest: no required dependencies. The optional
/// `serde-json` feature adds pinned serde/serde_json adapters; HTTP manifests
/// splice their own optional features and dependencies onto this text.
pub(crate) fn cargo_manifest(package_name: &str, version: &str, http: bool) -> String {
    let features = if http {
        "http = [\"dep:url\"]\nreqwest-rustls = [\"http\", \"dep:reqwest\", \"reqwest/rustls-tls\"]\n"
    } else {
        ""
    };
    let dependencies = if http {
        "url = { version = \"=2.5.8\", optional = true }\nreqwest = { version = \"=0.12.28\", optional = true, default-features = false }\n"
    } else {
        ""
    };
    let include = if http {
        "[\"src/**\", \"README.md\", \"models.md\", \"http-manifest.json\", \"examples.json\", \"examples.md\", \"examples/**\"]"
    } else {
        "[\"src/**\", \"README.md\"]"
    };
    let examples = if http {
        "\n[[example]]\nname = \"validated\"\nrequired-features = [\"http\"]\n"
    } else {
        ""
    };
    format!(
        "[package]\nname = {package_name:?}\nversion = {version:?}\nedition = \"2024\"\nrust-version = \"1.88\"\npublish = false\ndescription = \"Experimental source-selected OpenAPI models and validated codecs\"\nreadme = \"README.md\"\ninclude = {include}\n\n[features]\ndefault = []\nserde-json = [\"dep:serde\", \"dep:serde_json\"]\n{features}\n[dependencies]\nserde = {{ version = \"=1.0.229\", optional = true }}\nserde_json = {{ version = \"=1.0.151\", optional = true, features = [\"raw_value\"] }}\n{dependencies}\n[workspace]\n{examples}"
    )
}
/// Plan exact, validated codecs for every public symbol and union branch.
///
/// # Errors
/// Source-linked unsupported representations, assertions, or resource policies
/// reject the entire artifact set. This is not an HTTP SDK or release certification.
pub fn plan_codecs(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    plan_codecs_with_profile(contract, roots, config, false, false)
}

/// Plan codecs with native v2 scoped applicators. Unsupported source schemas or
/// representations fail before artifacts; base closures retain v1 emission.
pub fn plan_codecs_v2(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    plan_codecs_with_profile(contract, roots, config, true, false)
}

/// Explicit canonical-resource/dynamic codecs. Ordinary closures keep their
/// established v1/v2 program and byte output through the existing entry point.
pub fn plan_codecs_v3(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    if rust_models::resources::required(&contract, roots) {
        plan_codecs_with_profile(contract, roots, config, true, true)
    } else {
        plan_codecs_v2(contract, roots, config)
    }
}

fn plan_codecs_with_profile(
    contract: Arc<Contract>,
    roots: &[SchemaId],
    config: CodecConfig,
    applicators: bool,
    resources: bool,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    let models = if resources {
        rust_models::plan_models_v3_with_policy(&contract, roots, config.dialect)
    } else if applicators {
        rust_models::plan_models_v2_with_policy(&contract, roots, config.dialect)
    } else {
        rust_models::plan_models(&contract, roots)
    };
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
    let mut validation_roots: Vec<_> = models
        .symbols()
        .iter()
        .map(|s| s.source().clone())
        .collect();
    for decl in models.declarations.values() {
        if let Decl::Enum(variants) = decl {
            validation_roots.extend(variants.iter().map(|v| v.source.clone()));
        }
    }
    validation_roots.sort();
    validation_roots.dedup();
    let mut schema_config = config.schema.clone();
    schema_config.oas30_nullable_in_31 = config.dialect.oas30_nullable_in_31;
    let compiler = OwnedCompiler::new(schema_config);
    let compiled = (if resources {
        compiler.compile_v3(contract.clone(), &validation_roots)
    } else if applicators {
        compiler.compile_v2(contract.clone(), &validation_roots)
    } else {
        compiler.compile(contract.clone(), &validation_roots)
    })
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
    let validation_files = crate::rust_validation::emit(&program).map_err(|e| {
        let source = e
            .source
            .as_ref()
            .and_then(|s| {
                contract
                    .documents()
                    .find(|(uri, _)| uri.to_string() == s.document)
                    .map(|(uri, _)| {
                        let root = SchemaId::new(uri.clone(), Default::default());
                        s.pointer.strip_prefix('/').map_or(root.clone(), |pointer| {
                            pointer.split('/').fold(root, |parent, token| {
                                parent.child(&token.replace("~1", "/").replace("~0", "~"))
                            })
                        })
                    })
            })
            .unwrap_or_else(|| fallback.clone());
        vec![finding(source, "codec-validation-emission", e.message)]
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
    let mut files = models.render()?;
    files.retain(|f| {
        !matches!(
            f.path.as_str(),
            "rust/Cargo.toml" | "rust/src/lib.rs" | "rust/src/models.rs" | "rust/README.md"
        )
    });
    files.extend(validation_files);
    Ok(CodecPlan {
        models,
        files,
        indices,
        config,
        validation_version: program.version,
        validation_profile: program.profile,
    })
}
