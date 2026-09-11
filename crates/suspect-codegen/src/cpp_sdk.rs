//! C++20 value models, exact codecs and source-backed HTTP protocol packages.
//! Shared protocol admission precedes native naming and representation planning.
//! JSON codec roots, byte policies, aggregate rules and stream items remain distinct.

pub use crate::http_contract::HttpDiagnostic;
use crate::{OutFile, examples, http_protocol};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SourceId};
use suspect_schema::{Config, OwnedProgram, ProgramInstruction};

mod emit;
mod models;
mod protocol;
mod scoped_examples;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod v2_tests;
#[cfg(test)]
mod v3_tests;
pub use models::{
    CodecDescriptor, CodecMethods, Constructor, ConstructorParameter, Extras, Field, ModelPlan,
    ModelSymbol, Shape, TagInitializer,
};
pub use protocol::{
    InputArgument, InputConstructor, PlannedAggregate, PlannedBody, PlannedCredential,
    PlannedHeader, PlannedMedia, PlannedOperation, PlannedParameter, PlannedPart, PlannedResponse,
    PlannedResponseCase, ValueKind, ValueType, capabilities,
};

/// Package identity and finite native resource policy, independent of API semantics.
#[derive(Debug, Clone)]
pub struct SdkConfig {
    pub name: String,
    pub version: String,
    pub namespace: String,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_capture_bytes: usize,
    pub max_header_bytes: usize,
    pub max_json_depth: usize,
    pub max_json_work: usize,
    pub max_part_bytes: usize,
    pub max_parts: usize,
    pub max_stream_item_bytes: usize,
    pub max_stream_buffer_bytes: usize,
    /// Explicit, versioned interpretation of 3.1/3.2 binary string markers.
    /// Ordinary JSON strings and event-stream declarations are unaffected.
    pub legacy_binary_strings: bool,
    /// Explicit runtime variable-name policy. Generation never reads values.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    pub validation: Config,
}
impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            name: "generated_sdk".into(),
            version: "0.1.0".into(),
            namespace: "generated_sdk".into(),
            max_request_bytes: 8 * 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            max_capture_bytes: 64 * 1024,
            max_header_bytes: 64 * 1024,
            max_json_depth: 128,
            max_json_work: 32 * 1024 * 1024,
            max_part_bytes: 8 * 1024 * 1024,
            max_parts: 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_stream_buffer_bytes: 64 * 1024,
            legacy_binary_strings: false,
            credential_env: None,
            validation: Config {
                max_depth: 128,
                ..Default::default()
            },
        }
    }
}

/// Immutable admitted package: language descriptors and checked schema/HTTP plans.
#[derive(Debug)]
pub struct SdkPlan {
    contract: Arc<Contract>,
    config: SdkConfig,
    models: ModelPlan,
    operations: Vec<PlannedOperation>,
    credentials: BTreeMap<SourceId, PlannedCredential>,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    aggregates: Vec<PlannedAggregate>,
    program: OwnedProgram,
    examples: examples::ExamplePlan,
    protocol: http_protocol::ProtocolPlan,
}
impl SdkPlan {
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub fn config(&self) -> &SdkConfig {
        &self.config
    }
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<SourceId, PlannedCredential> {
        &self.credentials
    }
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub(crate) fn credential_for(
        &self,
        requirement: &http_protocol::CredentialRequirement,
    ) -> &PlannedCredential {
        &self.credentials[if self.credential_env.is_some() {
            requirement.scheme().use_site().source()
        } else {
            requirement.scheme().terminal().source()
        }]
    }
    #[must_use]
    pub fn aggregates(&self) -> &[PlannedAggregate] {
        &self.aggregates
    }
    #[must_use]
    pub fn validation_program(&self) -> &OwnedProgram {
        &self.program
    }
    #[must_use]
    pub fn examples(&self) -> &examples::ExamplePlan {
        &self.examples
    }
    #[must_use]
    pub fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    /// Pure emission; source-located refusal returns no partial artifact set.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
        check_program(&self.contract, &self.program)?;
        emit::package(self)
    }
}

/// Plan the selected outgoing operations using the verified C++ capabilities.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config)
}
pub fn emit_sdk(plan: &SdkPlan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    plan.render()
}

fn validate_config(
    contract: &Contract,
    selected: &[SourceId],
    config: &SdkConfig,
) -> Result<(), Vec<HttpDiagnostic>> {
    let root = SourceId::new(contract.entry().clone(), Default::default());
    let mut errors = Vec::new();
    if selected.is_empty() {
        errors.push(diagnostic(
            contract,
            root.clone(),
            "http-no-operations",
            "select at least one outgoing operation",
        ));
    }
    if !models::valid_identifier(&config.name)
        || config.namespace.len() > 256
        || config
            .namespace
            .split("::")
            .any(|p| !models::valid_identifier(p) || p == "std")
        || !valid_version(&config.version)
    {
        errors.push(diagnostic(contract,root.clone(),"cpp-package-identity","package/namespace require non-reserved ASCII C++ identifiers and version MAJOR.MINOR.PATCH"));
    }
    if [
        config.max_request_bytes,
        config.max_response_bytes,
        config.max_capture_bytes,
        config.max_header_bytes,
        config.max_json_work,
        config.max_parts,
        config.max_stream_item_bytes,
    ]
    .iter()
    .any(|&n| n == 0 || n > i32::MAX as usize)
        || config.max_part_bytes > i32::MAX as usize
        || config.max_capture_bytes > config.max_response_bytes
        || !(1..=256).contains(&config.max_json_depth)
        || !(16384..=1024 * 1024).contains(&config.max_stream_buffer_bytes)
        || config.max_parts > 100000
        || config.validation.max_depth > 512
        || config.validation.max_number_bytes > 65536
        || config.validation.max_evaluation_steps > i32::MAX as usize
        || config.validation.max_equality_steps > i32::MAX as usize
    {
        errors.push(diagnostic(contract,root,"cpp-resource-policy","finite positive byte/work limits must fit Int32; capture <= response; JSON depth <=256; receive window 16KiB..1MiB; at most 100000 parts; validation depth <=512 and numeric tokens <=65536 bytes"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
fn valid_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 8
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
fn locate(contract: &Contract, source: &suspect_schema::ProgramSource) -> SourceId {
    let uri = contract
        .documents()
        .find(|(uri, _)| uri.as_str() == source.document)
        .map(|(uri, _)| uri.clone())
        .unwrap_or_else(|| contract.entry().clone());
    let mut id = SourceId::new(uri, Default::default());
    for token in source.pointer.split('/').skip(1) {
        id = id.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    id
}
fn check_program(contract: &Contract, program: &OwnedProgram) -> Result<(), Vec<HttpDiagnostic>> {
    check_program_resources(contract, program, true, true)
}
#[cfg(test)]
fn check_program_profile(
    contract: &Contract,
    program: &OwnedProgram,
    scoped: bool,
) -> Result<(), Vec<HttpDiagnostic>> {
    check_program_resources(contract, program, scoped, false)
}
fn check_program_resources(
    contract: &Contract,
    program: &OwnedProgram,
    scoped: bool,
    resources: bool,
) -> Result<(), Vec<HttpDiagnostic>> {
    let root = SourceId::new(contract.entry().clone(), Default::default());
    if !(resources
        && (program.version, program.profile)
            == (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE))
        && !matches!(
            (program.version, program.profile),
            (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
                | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
        )
    {
        return Err(vec![diagnostic(
            contract,
            root,
            "cpp-validation-profile-unsupported",
            "C++ supports the witnessed v1 and scoped v2 static profiles; resource/dynamic execution requires a separate native profile",
        )]);
    }
    program.check().map_err(|e| {
        vec![diagnostic(
            contract,
            e.source
                .as_ref()
                .map(|s| locate(contract, s))
                .unwrap_or_else(|| root.clone()),
            "cpp-validation-program",
            e.to_string(),
        )]
    })?;
    let limits = &program.limits;
    if limits.max_depth > 512
        || limits.max_number_bytes > 65536
        || [
            limits.max_errors,
            limits.max_equality_steps,
            limits.max_evaluation_steps,
        ]
        .iter()
        .any(|&n| n > i32::MAX as usize)
    {
        return Err(vec![diagnostic(
            contract,
            root,
            "cpp-validation-resource-policy",
            "validation depth <=512, numeric tokens <=65536 bytes, counters <=Int32.max",
        )]);
    }
    for node in &program.nodes {
        for check in &node.checks {
            let values = match &check.instruction {
                ProgramInstruction::Const { value } => std::slice::from_ref(value),
                ProgramInstruction::Enum { values } => values.as_slice(),
                _ => &[],
            };
            let mut pending = values.iter().map(|v| (v, 0)).collect::<Vec<_>>();
            while let Some((value, depth)) = pending.pop() {
                if depth > 256
                    || value
                        .as_number()
                        .is_some_and(|v| v.to_string().len() > 65536)
                {
                    return Err(vec![diagnostic(
                        contract,
                        locate(contract, &check.source),
                        "cpp-validation-literal-limit",
                        "compiled literal nesting <=256 and numeric tokens <=65536 bytes",
                    )]);
                }
                match value {
                    serde_json::Value::Array(v) => pending.extend(v.iter().map(|v| (v, depth + 1))),
                    serde_json::Value::Object(v) => {
                        pending.extend(v.values().map(|v| (v, depth + 1)))
                    }
                    _ => {}
                }
            }
            // Admission is a whitelist of witnessed native opcodes. Shared
            // program extensions must fail at their source until their native
            // semantics have been implemented, including annotation scopes.
            #[allow(unreachable_patterns)] // Also compiles against retained v1 dependencies.
            match &check.instruction {
                ProgramInstruction::Always { .. }
                | ProgramInstruction::Type { .. }
                | ProgramInstruction::Ref { .. }
                | ProgramInstruction::Properties { .. }
                | ProgramInstruction::AdditionalProperties { .. }
                | ProgramInstruction::Required { .. }
                | ProgramInstruction::Items { .. }
                | ProgramInstruction::PrefixItems { .. }
                | ProgramInstruction::AllOf { .. }
                | ProgramInstruction::AnyOf { .. }
                | ProgramInstruction::OneOf { .. }
                | ProgramInstruction::Not { .. }
                | ProgramInstruction::Bound { .. }
                | ProgramInstruction::MultipleOf { .. }
                | ProgramInstruction::Count { .. }
                | ProgramInstruction::Enum { .. }
                | ProgramInstruction::Const { .. }
                | ProgramInstruction::UniqueItems
                | ProgramInstruction::Pattern { .. } => {}
                instruction if scoped && instruction.requires_v2() => {}
                instruction if resources && instruction.requires_v3() => {}
                _ => {
                    return Err(vec![diagnostic(
                        contract,
                        locate(contract, &check.source),
                        "cpp-validation-opcode-unsupported",
                        "this C++ validation profile does not implement this scoped-applicator instruction",
                    )]);
                }
            }
        }
    }
    Ok(())
}
/// Portable validation independent of the native model-layout profile.
pub fn emit_validation_runtime(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    check_program(contract, program)?;
    Ok(emit::validation_runtime(contract, program))
}
fn valid_path(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    let mut literal = String::new();
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut name = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '}' {
                    closed = true;
                    break;
                }
                if ch == '{' || ch.is_control() {
                    return false;
                }
                name.push(ch);
            }
            if !closed || name.is_empty() {
                return false;
            }
            literal.push('x');
        } else if ch.is_ascii_alphanumeric() || "/-._~!$&'()*+,;=:@%".contains(ch) {
            literal.push(ch);
        } else {
            return false;
        }
    }
    for part in literal.split('/') {
        let mut bytes = part.bytes();
        let mut decoded = Vec::new();
        while let Some(b) = bytes.next() {
            if b == b'%' {
                let Some(a) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                    return false;
                };
                let Some(b) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                    return false;
                };
                decoded.push((a * 16 + b) as u8);
            } else {
                decoded.push(b);
            }
        }
        if decoded == b"." || decoded == b".." {
            return false;
        }
    }
    true
}
fn diagnostic(
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
