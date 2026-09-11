//! Native PHP 8.3 SDK planning over the shared HTTP contract and owned validator.
//!
//! Planning is pure and fails before producing artifacts. The generated Composer
//! package contains native models/enums/unions, explicit omission, exact JSON
//! codecs, a bounded portable validator, an injectable HTTP client and PHPDoc.

#[cfg(not(feature = "http-protocol"))]
use std::collections::BTreeSet;
use std::sync::Arc;

use suspect_ir::contract::{Contract, SchemaId, SourceId};
#[cfg(not(feature = "http-protocol"))]
use suspect_schema::OwnedCompiler;
use suspect_schema::OwnedProgram;

pub use crate::http_contract::HttpDiagnostic;
use crate::{OutFile, http_contract};

#[path = "php_sdk/emit.rs"]
mod emit;
#[path = "php_sdk/models.rs"]
pub mod models;
#[path = "php_sdk/protocol.rs"]
#[cfg(feature = "http-protocol")]
pub mod protocol;
#[path = "php_sdk/samples.rs"]
mod samples;

#[path = "php_sdk/tests_v2.rs"]
#[cfg(test)]
mod tests_v2;

#[path = "php_sdk/tests_v3.rs"]
#[cfg(test)]
mod tests_v3;

/// Package identity and finite runtime policy; none of these redefine API semantics.
#[derive(Debug, Clone)]
pub struct PhpConfig {
    /// Composer vendor/package name.
    pub package_name: String,
    /// Exact SemVer package version.
    pub package_version: String,
    /// PHP namespace without a leading or trailing backslash.
    pub namespace: String,
    /// Explicit source-scheme to runtime environment-variable names; never values.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Maximum request-body and assembled-URL bytes.
    pub max_request_bytes: usize,
    /// Maximum response-body bytes (also enforced during the default transport read).
    pub max_response_bytes: usize,
    /// Bounded raw body prefix retained by SDK failures.
    pub max_capture_bytes: usize,
    /// Aggregate response/request header bytes, including repeated fields.
    pub max_header_bytes: usize,
    /// JSON and native conversion nesting ceiling, at most 128.
    pub max_depth: usize,
    /// Per JSON parse/serialize/native conversion node visits.
    pub max_nodes: usize,
    /// Aggregate native conversion string, member-name and numeric-token bytes.
    pub max_conversion_bytes: usize,
    /// Shared owned-schema compiler/executor policy.
    pub validation: suspect_schema::Config,
}

impl Default for PhpConfig {
    fn default() -> Self {
        Self {
            package_name: "generated/sdk".into(),
            package_version: "0.0.0".into(),
            namespace: "GeneratedSdk".into(),
            credential_env: None,
            max_request_bytes: 8 * 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            max_capture_bytes: 16 * 1024,
            max_header_bytes: 64 * 1024,
            max_depth: 128,
            max_nodes: 100_000,
            max_conversion_bytes: 32 * 1024 * 1024,
            validation: suspect_schema::Config {
                max_depth: 128,
                ..Default::default()
            },
        }
    }
}

/// Native operation symbols retain the exact source-selected wire declaration.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub input_type: String,
    pub input_constructor: String,
    pub error_type: String,
    pub success_types: Vec<(u16, String)>,
    pub error_types: Vec<(u16, String)>,
    pub(crate) wire: http_contract::Operation,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
}

/// Allocated operation argument; the canonical schema and wire identities remain public.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub name: String,
    pub source: SourceId,
    pub schema: SchemaId,
    pub wire_name: String,
    pub required: bool,
    pub location: suspect_ir::contract::ParameterLocation,
    pub style: suspect_ir::contract::ParameterStyle,
    pub explode: bool,
    pub array: bool,
    pub description: String,
}

/// Native request body member.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub name: String,
    pub source: SourceId,
    pub schema: SchemaId,
    pub required: bool,
    pub media_type: String,
    pub description: String,
}

/// Allocated success wrapper or typed exception, suitable for compatibility records.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub type_name: String,
    pub source: SourceId,
    pub schema: SchemaId,
    pub status: u16,
    pub media_type: String,
    pub success: bool,
    pub body_member: String,
    pub metadata_member: String,
    pub description: String,
}

/// Immutable admitted plan. Rendering cannot introduce new schema decisions.
#[derive(Debug)]
pub struct ModelCore {
    contract: Arc<Contract>,
    config: PhpConfig,
    models: models::ModelPlan,
    operations: Vec<PlannedOperation>,
    program: OwnedProgram,
    examples: crate::examples::ExamplePlan,
    samples: Vec<samples::NativeExample>,
}

/// Descriptive alias for callers using the other native SDK backends.
#[cfg(feature = "http-protocol")]
pub type Plan = protocol::SdkPlan;
#[cfg(not(feature = "http-protocol"))]
pub type Plan = ModelCore;
pub type SdkPlan = Plan;

impl ModelCore {
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub fn config(&self) -> &PhpConfig {
        &self.config
    }
    #[must_use]
    pub fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Complete deterministic artifact set under `php/`, ready for the shared writer.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        emit::package(self)
    }
}

/// Render a successfully admitted plan without filesystem or toolchain access.
#[must_use]
pub fn emit_sdk(plan: &Plan) -> Vec<OutFile> {
    plan.render()
}

/// Emit the exact JSON and checked portable validator independently of model admission.
/// This accepts all supported OwnedProgram opcodes, including tuples and conditionally
/// applicable assertions whose PHP model representation may require a richer profile.
///
/// # Errors
/// Malformed program metadata and unrepresentable resource policies fail before emission.
pub fn emit_validation(
    program: &OwnedProgram,
    config: &PhpConfig,
) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    let fallback = SourceId::new(
        suspect_source::Uri::parse("file:///php-validation").expect("static URI"),
        Default::default(),
    );
    native_program_check(program).map_err(|e| {
        vec![HttpDiagnostic {
            source: e
                .source
                .as_ref()
                .and_then(|s| {
                    suspect_source::Uri::parse(&s.document).ok().map(|uri| {
                        let root = SourceId::new(uri, Default::default());
                        s.pointer
                            .strip_prefix('/')
                            .map(|p| {
                                p.split('/').fold(root.clone(), |id, part| {
                                    id.child(&part.replace("~1", "/").replace("~0", "~"))
                                })
                            })
                            .unwrap_or(root)
                    })
                })
                .unwrap_or_else(|| fallback.clone()),
            at: 0..0,
            code: "php-validation-program",
            message: e.to_string(),
        }]
    })?;
    if config.namespace.len() > 128
        || !config.namespace.split('\\').all(models::identifier)
        || !valid_policy(config)
        || program.limits.max_depth > 128
        || program.limits.max_number_bytes > 65_536
        || [
            program.limits.max_errors,
            program.limits.max_equality_steps,
            program.limits.max_evaluation_steps,
        ]
        .iter()
        .any(|n| *n > i32::MAX as usize)
    {
        return Err(vec![HttpDiagnostic {
            source: fallback,
            at: 0..0,
            code: "php-resource-policy",
            message: "invalid PHP namespace or portable runtime ceilings".into(),
        }]);
    }
    Ok(emit::validation(program, config))
}

/// An additive shared opcode/profile is not native PHP admission.
fn native_program_check(program: &OwnedProgram) -> Result<(), suspect_schema::ProgramCheckError> {
    native_program_check_mode(program, true, true)
}

#[cfg(test)]
fn native_program_check_profile(
    program: &OwnedProgram,
    scoped: bool,
) -> Result<(), suspect_schema::ProgramCheckError> {
    native_program_check_mode(program, scoped, false)
}

fn native_program_check_mode(
    program: &OwnedProgram,
    scoped: bool,
    resources: bool,
) -> Result<(), suspect_schema::ProgramCheckError> {
    use suspect_schema::ProgramInstruction::*;
    program.check()?;
    for check in program.nodes.iter().flat_map(|node| &node.checks) {
        let base = matches!(
            &check.instruction,
            Always { .. }
                | Type { .. }
                | Ref { .. }
                | Properties { .. }
                | AdditionalProperties { .. }
                | Required { .. }
                | Items { .. }
                | PrefixItems { .. }
                | AllOf { .. }
                | AnyOf { .. }
                | OneOf { .. }
                | Not { .. }
                | Bound { .. }
                | MultipleOf { .. }
                | Count { .. }
                | Enum { .. }
                | Const { .. }
                | UniqueItems
                | Pattern { .. }
        );
        let v2 = matches!(
            &check.instruction,
            If { .. }
                | DependentRequired { .. }
                | DependentSchemas { .. }
                | Contains { .. }
                | PatternProperties { .. }
                | AdditionalPropertiesWithPatterns { .. }
                | PropertyNames { .. }
                | UnevaluatedProperties { .. }
                | UnevaluatedItems { .. }
        );
        let v3 = matches!(&check.instruction, DynamicRef { .. });
        if !base && !(scoped && v2) && !(resources && v3) {
            return Err(suspect_schema::ProgramCheckError{source:Some(check.source.clone()),message:"PHP has no verified native implementation for this portable validation instruction".into()});
        }
    }
    let v1 = program.version == "suspect.validation.experimental.v1"
        && program.profile == "oas31-jsonschema202012-static-subset";
    let v2 = program.version == "suspect.validation.experimental.v2"
        && program.profile == "oas31-jsonschema202012-static-applicators";
    let v3 = program.version == "suspect.validation.experimental.v3"
        && program.profile == "oas31-jsonschema202012-resources-dynamic";
    if !v1 && !(scoped && v2) && !(resources && v3) {
        return Err(suspect_schema::ProgramCheckError {
            source: program.roots.first().map(|root| root.source.clone()),
            message: "PHP requires an explicitly admitted portable validation version/profile"
                .into(),
        });
    }
    Ok(())
}

fn program_source(source: &suspect_schema::ProgramSource) -> Option<SourceId> {
    let root = SourceId::new(
        suspect_source::Uri::parse(&source.document).ok()?,
        Default::default(),
    );
    Some(
        source
            .pointer
            .strip_prefix('/')
            .map(|pointer| {
                pointer.split('/').fold(root.clone(), |id, part| {
                    id.child(&part.replace("~1", "/").replace("~0", "~"))
                })
            })
            .unwrap_or(root),
    )
}

fn valid_policy(config: &PhpConfig) -> bool {
    [
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_header_bytes,
        config.max_nodes,
        config.max_conversion_bytes,
    ]
    .iter()
    .all(|v| *v > 0 && *v <= i32::MAX as usize)
        && config.max_capture_bytes <= config.max_response_bytes
        && config.max_depth > 0
        && config.max_depth <= 128
        && config.validation.max_depth <= 128
        && config.validation.max_number_bytes <= 65_536
        && [
            config.validation.max_errors,
            config.validation.max_evaluation_steps,
            config.validation.max_equality_steps,
        ]
        .iter()
        .all(|v| *v <= i32::MAX as usize)
}

fn composer_part(part: &str, package: bool) -> bool {
    let bytes = part.as_bytes();
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if bytes.first().is_none_or(|b| !alphanumeric(*b)) {
        return false;
    }
    let mut at = 1;
    while at < bytes.len() {
        if alphanumeric(bytes[at]) {
            at += 1;
            continue;
        }
        let separator = bytes[at];
        if !b"._-".contains(&separator) {
            return false;
        }
        at += 1;
        if package && separator == b'-' && bytes.get(at) == Some(&b'-') {
            at += 1;
        }
        if bytes.get(at).is_none_or(|b| !alphanumeric(*b)) {
            return false;
        }
        at += 1;
    }
    true
}

pub(crate) fn diagnostic(
    contract: &Contract,
    source: SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        message: message.into(),
    }
}

/// Plan exactly the selected outgoing operations and their schema closure.
///
/// # Errors
/// Source-linked HTTP, native representation, dialect, opcode and resource-policy
/// diagnostics. Unsupported constructs never become releasable fallback artifacts.
#[cfg(not(feature = "http-protocol"))]
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let entry = SourceId::new(contract.entry().clone(), Default::default());
    if !contract.openapi_version().starts_with("3.1.") {
        errors.push(diagnostic(&contract, entry.child("openapi"), "php-dialect-unsupported", "the verified PHP profile requires OpenAPI 3.1; other dialects need native codec admission"));
    }
    if selected.is_empty() {
        errors.push(diagnostic(
            &contract,
            entry.clone(),
            "http-no-operations",
            "select at least one outgoing operation",
        ));
    }
    let parts: Vec<_> = config.package_name.split('/').collect();
    if parts.len() != 2
        || !composer_part(parts[0], false)
        || !composer_part(parts[1], true)
        || config.package_name.len() > 128
    {
        errors.push(diagnostic(
            &contract,
            entry.clone(),
            "php-package-name",
            "Composer identity must be lowercase vendor/package",
        ));
    }
    if semver::Version::parse(&config.package_version).is_err() {
        errors.push(diagnostic(
            &contract,
            entry.clone(),
            "php-package-version",
            "package version must be exact SemVer",
        ));
    }
    if config.namespace.len() > 128 || !config.namespace.split('\\').all(models::identifier) {
        errors.push(diagnostic(
            &contract,
            entry.clone(),
            "php-namespace",
            "namespace must consist of non-reserved ASCII PHP identifiers",
        ));
    }
    if !valid_policy(&config) {
        errors.push(diagnostic(&contract, entry.clone(), "php-resource-policy", "byte/node budgets must be positive portable 32-bit integers; depth <=128 and numeric tokens <=65536 bytes"));
    }
    let wire = match http_contract::plan(&contract, selected) {
        Ok(wire) => Some(wire),
        Err(mut more) => {
            errors.append(&mut more);
            None
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    let wire = wire.expect("checked HTTP admission");
    let reachable = contract.reachable_from(&wire.roots);
    for id in &reachable {
        if contract
            .schema(id)
            .is_some_and(|s| matches!(s.dialect(), suspect_ir::contract::SchemaDialect::OpenApi30))
        {
            errors.push(diagnostic(&contract, id.clone(), "php-dialect-unsupported", "OpenAPI 3.0 nullable/exclusive-bound projection is not admitted by the PHP model profile"));
        }
        for keyword in ["readOnly", "writeOnly"] {
            if contract
                .source(id)
                .and_then(|raw| raw.get(keyword))
                .is_some_and(|v| v != &serde_json::Value::Bool(false))
            {
                errors.push(diagnostic(
                    &contract,
                    id.child(keyword),
                    "http-directional-codec-unsupported",
                    "PHP neutral codecs require directional annotations to be absent or false",
                ));
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Every reachable public codec is an explicit root of the same checked program.
    let validation = OwnedCompiler::new(config.validation.clone())
        .compile(contract.clone(), &reachable)
        .map_err(|more| {
            more.into_iter()
                .map(|e| HttpDiagnostic {
                    source: e.source,
                    at: e.span.unwrap_or(0..0),
                    code: "php-schema-compilation",
                    message: e.message,
                })
                .collect::<Vec<_>>()
        })?;
    let program = validation.program();
    native_program_check(&program).map_err(|e| {
        vec![diagnostic(
            &contract,
            entry.clone(),
            "php-validation-program",
            e.to_string(),
        )]
    })?;
    // The PHP executor implements this entire version, including the portable
    // pattern NFA; new Rust instruction variants require an explicit match arm.
    for node in &program.nodes {
        for check in &node.checks {
            use suspect_schema::ProgramInstruction::*;
            match &check.instruction {
                Always { .. }
                | Type { .. }
                | Ref { .. }
                | Properties { .. }
                | AdditionalProperties { .. }
                | Required { .. }
                | Items { .. }
                | PrefixItems { .. }
                | AllOf { .. }
                | AnyOf { .. }
                | OneOf { .. }
                | Not { .. }
                | Bound { .. }
                | MultipleOf { .. }
                | Count { .. }
                | Enum { .. }
                | Const { .. }
                | UniqueItems
                | Pattern { .. } => {}
            }
        }
    }
    let mut used = models::reserved_symbols();
    let models = models::plan(&contract, &reachable, &wire.operations, &mut used)?;
    let mut methods = BTreeSet::from(["__construct".into(), "exchange".into()]);
    let mut operations = Vec::new();
    for op in wire.operations {
        let declaration = contract
            .operations()
            .find(|item| item.source() == &op.source)
            .expect("admitted operation");
        let declared_parameters = declaration.parameters();
        let declared_responses = declaration.responses();
        let stem = models::pascal(&op.operation_id);
        let mut fields = BTreeSet::from(["body".into(), "options".into()]);
        let parameters = op
            .parameters
            .iter()
            .map(|p| PlannedParameter {
                name: models::allocate(&models::member(&p.wire_name), &mut fields),
                source: p.source.clone(),
                schema: p.schema.clone(),
                wire_name: p.wire_name.clone(),
                required: p.required,
                location: p.location,
                style: p.style,
                explode: p.explode,
                array: p.array,
                description: declared_parameters
                    .iter()
                    .find(|d| {
                        d.name() == Some(p.wire_name.as_str()) && d.location() == Some(p.location)
                    })
                    .and_then(|d| d.description())
                    .unwrap_or_default()
                    .into(),
            })
            .collect();
        let mut success_types = Vec::new();
        let mut error_types = Vec::new();
        for response in &op.responses {
            if (200..300).contains(&response.status) {
                success_types.push((
                    response.status,
                    models::allocate(&format!("{stem}Status{}", response.status), &mut used),
                ));
            } else {
                error_types.push((
                    response.status,
                    models::allocate(&format!("{stem}Status{}Error", response.status), &mut used),
                ));
            }
        }
        let responses = op
            .responses
            .iter()
            .map(|r| {
                let success = (200..300).contains(&r.status);
                PlannedResponse {
                    type_name: if success {
                        &success_types
                    } else {
                        &error_types
                    }
                    .iter()
                    .find(|(s, _)| *s == r.status)
                    .expect("allocated status")
                    .1
                    .clone(),
                    source: r.source.clone(),
                    schema: r.schema.clone(),
                    status: r.status,
                    media_type: r.media_type.clone(),
                    success,
                    body_member: "body".into(),
                    metadata_member: "response".into(),
                    description: declared_responses
                        .iter()
                        .find(|d| d.status_key() == r.status.to_string())
                        .and_then(|d| d.description())
                        .unwrap_or_default()
                        .into(),
                }
            })
            .collect();
        let body = op.body.as_ref().map(|b| PlannedBody {
            name: "body".into(),
            source: b.source.clone(),
            schema: b.schema.clone(),
            required: b.required,
            media_type: b.media_type.clone(),
            description: declaration
                .request_body()
                .and_then(|b| b.description())
                .unwrap_or_default()
                .into(),
        });
        operations.push(PlannedOperation {
            source: op.source.clone(),
            operation_id: op.operation_id.clone(),
            method_name: models::allocate(&models::member(&op.operation_id), &mut methods),
            input_type: models::allocate(&format!("{stem}Input"), &mut used),
            input_constructor: "__construct".into(),
            error_type: models::allocate(&format!("{stem}ApiError"), &mut used),
            success_types,
            error_types,
            parameters,
            body,
            responses,
            wire: op,
        });
    }
    let examples = crate::examples::plan_examples(contract.clone(), selected, Default::default());
    let samples = samples::plan(&models, &validation, &examples);
    Ok(ModelCore {
        contract,
        config,
        models,
        operations,
        program,
        examples,
        samples,
    })
}

/// Source-selected PHP SDK over the shared rich protocol plan.
#[cfg(feature = "http-protocol")]
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan_sdk(contract, selected, config, protocol::capabilities())
}
