//! Source-selected Kotlin/JVM SDK with native models and coroutine protocol clients.
pub use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::{OutFile, examples::ExamplePlan};
use std::{collections::BTreeMap, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::OwnedProgram;

mod emit;
mod environment;
pub mod models;
mod protocol;
mod rich_emit;
mod samples;
pub mod validation;
mod validation_v3;

pub const KOTLIN_VERSION: &str = "2.4.20";
pub const COROUTINES_VERSION: &str = "1.11.0";
pub const DOKKA_VERSION: &str = "2.2.0";

/// Maven/package identity; does not add API semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkConfig {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub package_name: String,
    /// Explicit runtime variable names, bound to used source security schemes.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
}
impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            group_id: "generated".into(),
            artifact_id: "sdk".into(),
            version: "0.0.0".into(),
            package_name: "generated.sdk".into(),
            credential_env: None,
        }
    }
}

/// A native type binding. Only Model references are canonical JSON codec roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    Model(SchemaId),
    Json,
    String,
    Boolean,
    Number,
    Bytes,
    Unit,
    Named(String),
    List(Box<NativeType>),
    Flow(Box<NativeType>),
    Presence(Box<NativeType>),
}
impl NativeType {
    pub fn kotlin(&self, models: &models::ModelPlan) -> String {
        match self {
            Self::Model(id) => models.get(id).kotlin_type.clone(),
            Self::Json => "JsonValue".into(),
            Self::String => "String".into(),
            Self::Boolean => "Boolean".into(),
            Self::Number => "JsonNumber".into(),
            Self::Bytes => "ByteArray".into(),
            Self::Unit => "Unit".into(),
            Self::Named(name) => name.clone(),
            Self::List(ty) => format!("List<{}>", ty.kotlin(models)),
            Self::Flow(ty) => format!("kotlinx.coroutines.flow.Flow<{}>", ty.kotlin(models)),
            Self::Presence(ty) => format!("Presence<{}>", ty.kotlin(models)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedCredential {
    pub source: SourceId,
    pub scheme_name: String,
    pub name: String,
    pub kotlin_type: String,
    pub wire: wire::CredentialRequirement,
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub source: SourceId,
    pub schema: SchemaId,
    pub wire_name: String,
    pub name: String,
    pub kotlin_type: String,
    pub codec_name: String,
    pub required: bool,
    pub wire: wire::ParameterPlan,
}

#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub name: String,
    pub ty: NativeType,
    pub codec_name: String,
    pub wire: wire::HeaderPlan,
}

/// Finite named form/multipart field. Binary parts carry bytes, never JSON stand-ins.
#[derive(Debug, Clone)]
pub struct PlannedPart {
    pub name: String,
    pub ty: NativeType,
    pub value_type: NativeType,
    pub wrapper: Option<String>,
    pub headers_type: Option<String>,
    pub headers: Vec<PlannedHeader>,
    pub wire: wire::PartPlan,
}

#[derive(Debug, Clone)]
pub struct PlannedForm {
    pub name: String,
    pub source: SourceId,
    pub multipart: bool,
    pub rules: wire::ObjectRules,
    pub fields: Vec<PlannedPart>,
    pub additional: Option<Box<PlannedPart>>,
}

#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub name: String,
    pub ty: NativeType,
    pub schema: Option<SchemaId>,
    pub codec_name: Option<String>,
    pub form: Option<PlannedForm>,
    pub wire: wire::MediaPlan,
}

#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub name: String,
    pub ty: NativeType,
    pub kotlin_type: String,
    pub required: bool,
    pub media: Vec<PlannedMedia>,
    pub choice_type: Option<String>,
}

/// One native status/media/disposition constructor. Defaults have success and
/// failure members distinguished using the *actual* status at runtime.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub source: SourceId,
    pub status: wire::ResponseStatus,
    pub status_key: String,
    pub success: bool,
    pub variant_name: String,
    pub constructor: String,
    pub ty: NativeType,
    pub kotlin_type: String,
    pub schema: Option<SchemaId>,
    pub codec_name: Option<String>,
    pub media: Option<PlannedMedia>,
    pub headers_type: Option<String>,
    pub headers: Vec<PlannedHeader>,
    pub disposition: wire::ResponseBodyDisposition,
    pub stream: bool,
    pub response_index: usize,
    pub media_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub input_type: String,
    pub constructor: String,
    pub result_type: String,
    pub result_data_type: Option<String>,
    pub result_data: Option<NativeType>,
    pub error_type: String,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
    pub flow: bool,
    pub credentials: Vec<PlannedCredential>,
    pub wire: wire::OperationPlan,
}
impl PlannedOperation {
    pub fn input_has_default(&self) -> bool {
        self.parameters.iter().all(|p| !p.required)
            && self.body.as_ref().is_none_or(|b| !b.required)
    }
    /// Only the simple single-scheme view; full security is retained in `wire`.
    pub fn security_scheme_name(&self) -> &str {
        self.credentials
            .first()
            .map_or("", |c| c.scheme_name.as_str())
    }
}

#[derive(Debug)]
pub struct Plan {
    contract: Arc<Contract>,
    config: SdkConfig,
    models: models::ModelPlan,
    operations: Vec<PlannedOperation>,
    credentials: BTreeMap<SourceId, PlannedCredential>,
    program: OwnedProgram,
    protocol: wire::ProtocolPlan,
    examples: ExamplePlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
}
pub type SdkPlan = Plan;
impl Plan {
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    pub fn config(&self) -> &SdkConfig {
        &self.config
    }
    pub fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    pub fn credentials(&self) -> &BTreeMap<SourceId, PlannedCredential> {
        &self.credentials
    }
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub(crate) fn credential_source<'a>(
        &self,
        requirement: &'a wire::CredentialRequirement,
    ) -> &'a SourceId {
        requirement.scheme().terminal().source()
    }
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    pub fn protocol(&self) -> &wire::ProtocolPlan {
        &self.protocol
    }
    pub fn examples(&self) -> &ExamplePlan {
        &self.examples
    }
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
        emit::package(self)
    }
}

/// Native adapter fence. Unsupported directions/representations have additional
/// located native admission guards; compatibility profiles are never implicit.
pub fn capabilities() -> wire::Capabilities {
    protocol::capabilities()
}
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    plan_sdk_with_profiles(contract, selected, config, &Default::default())
}

/// Explicit interpretation entry point used by the shared backend registry.
/// Native protocol support never implicitly opts into compatibility semantics.
pub fn plan_sdk_with_profiles(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
    profiles: &std::collections::BTreeSet<wire::CompatibilityProfile>,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, profiles, true, false)
}

/// Explicit scoped-applicator SDK seam used to verify v2 adoption. Standard
/// closures keep their original v1 program and emitted runtime bytes.
pub fn plan_sdk_v2(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, &Default::default(), true, false)
}

/// Explicit canonical-resource and dynamic-reference SDK compilation.
/// It does not acquire documents or interpret a dynamic fallback as static.
pub fn plan_sdk_v3(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SdkConfig,
) -> Result<Plan, Vec<HttpDiagnostic>> {
    protocol::plan(contract, selected, config, &Default::default(), true, true)
}
pub fn emit_sdk(plan: &Plan) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    plan.render()
}

fn valid_coordinate(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn valid_package(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !["java", "kotlin"].contains(&s.split('.').next().unwrap_or(""))
        && s.split('.').all(|part| {
            !models::keyword(part)
                && part.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}
pub(super) fn diagnostic(
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
