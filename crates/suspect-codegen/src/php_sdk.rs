//! Native PHP 8.3 SDK planning over the shared HTTP contract and owned validator.
//!
//! Planning is pure and fails before producing artifacts. The generated Composer
//! package contains native models/enums/unions, explicit omission, exact JSON
//! codecs, a bounded portable validator, an injectable HTTP client and PHPDoc.

use std::sync::Arc;

use suspect_ir::contract::{Contract, SourceId};
use suspect_schema::OwnedProgram;

pub use crate::http_contract::HttpDiagnostic;
use crate::OutFile;

#[path = "php_sdk/emit.rs"]
mod emit;
#[path = "php_sdk/models.rs"]
pub mod models;
#[path = "php_sdk/protocol.rs"]
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
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
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
            sdk_defaults: None,
            attribution: None,
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

/// Immutable admitted plan. Rendering cannot introduce new schema decisions.
#[derive(Debug)]
pub(crate) struct ModelCore {
    contract: Arc<Contract>,
    config: PhpConfig,
    models: models::ModelPlan,
    program: OwnedProgram,
    examples: crate::examples::ExamplePlan,
    samples: Vec<samples::NativeExample>,
}

/// Descriptive alias for callers using the other native SDK backends.
pub type Plan = protocol::SdkPlan;
/// Backend-neutral alias for [`Plan`].
pub type SdkPlan = Plan;

impl ModelCore {
    /// Canonical contract the plan was built from.
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// Configuration the plan was admitted under.
    #[must_use]
    pub fn config(&self) -> &PhpConfig {
        &self.config
    }
    /// Native model symbols for every reachable codec schema.
    #[must_use]
    pub fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    /// Checked validation program executed by the generated runtime.
    #[must_use]
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    /// Per-operation source examples used by generated docs.
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
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

/// Source-selected PHP SDK over the shared rich protocol plan.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PhpConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan_sdk(contract, selected, config, protocol::capabilities())
}
