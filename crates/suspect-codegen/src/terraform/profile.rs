//! Closed lifecycle vocabulary. No defaults conceal lifecycle decisions.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Build identity, separate from API and lifecycle semantics. The witnessed
/// Framework version is currently 1.15.1 (Plugin Go 0.27.0, Go >=1.23.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub provider_name: String,
    pub provider_address: String,
    pub module_path: String,
    pub version: String,
    pub go_version: String,
    pub go_toolchain: String,
    pub terraform_version: String,
    pub framework_version: String,
    pub sdk: SdkDependency,
}

/// Exact generated SDK dependency; no local replace directive is emitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkDependency {
    pub module_path: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingProfile {
    pub format: String,
    pub authentication: Authentication,
    pub resources: BTreeMap<String, ResourceMapping>,
    pub data_sources: BTreeMap<String, DataSourceMapping>,
}

/// The first bounded profile admits anonymous APIs or one source-declared
/// bearer scheme. The allocated SDK credential constructor remains authoritative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Authentication {
    None,
    Bearer { scheme: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    String,
    Bool,
}

impl AttributeType {
    pub(super) fn go(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::Bool => "Bool",
        }
    }
    pub(super) fn hcl(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Bool => "bool",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeMode {
    Required,
    Optional,
    Computed,
    OptionalComputed,
}

impl AttributeMode {
    pub(super) fn configured(self) -> bool {
        self != Self::Computed
    }
    pub(super) fn computed(self) -> bool {
        matches!(self, Self::Computed | Self::OptionalComputed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub r#type: AttributeType,
    pub mode: AttributeMode,
    pub sensitive: bool,
    pub write_only: bool,
    pub requires_replace: bool,
    /// Terraform-only trigger values; accepted only as a write-only update trigger.
    pub state_only: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceMapping {
    pub description: String,
    pub attributes: BTreeMap<String, Attribute>,
    pub identity: Identity,
    pub create: Mutation,
    pub read: Read,
    pub update: Mutation,
    pub delete: Delete,
    pub refresh: Refresh,
    pub retry: Retry,
    pub polling: Polling,
    pub failure: Failure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSourceMapping {
    pub description: String,
    pub attributes: BTreeMap<String, Attribute>,
    pub read: Read,
    pub retry: Retry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub attribute: String,
    pub import: ImportFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    /// The exact nonempty string, with no splitting, normalization or decoding.
    OpaqueString,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refresh {
    /// Read maps authoritative remote values; missing removes resource state.
    AuthoritativeRead,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    None,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polling {
    None,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// Mapped partial responses save authoritative state and return an error.
    /// Other errors preserve prior state (create has none). Never retry.
    MappedPartialOtherwisePreserve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub operation_id: String,
    pub inputs: Vec<Input>,
    pub success: Response,
    pub partial: Vec<Response>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub operation_id: String,
    pub inputs: Vec<Input>,
    pub success: Response,
    /// Exact source-declared API error statuses, not arbitrary transport statuses.
    pub missing: Vec<u16>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delete {
    pub operation_id: String,
    pub inputs: Vec<Input>,
    pub success_status: u16,
    pub missing: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub attribute: String,
    pub target: InputTarget,
    pub null: NullInput,
    pub when: InputWhen,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputTarget {
    Parameter {
        location: Location,
        name: String,
    },
    /// A wire-property path. V1 admits exactly one field of a native record.
    Body {
        path: Vec<String>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Path,
    Query,
    Header,
    Cookie,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullInput {
    Reject,
    Omit,
    SendNull,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputWhen {
    Always,
    /// Only for optional write-only mutation fields on update. The trigger is
    /// a configured Terraform-only attribute. Unknown triggers block apply.
    TriggerChanged {
        attribute: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub status: u16,
    /// Terraform attribute -> exact wire-property path in the typed SDK payload.
    pub state: BTreeMap<String, Vec<String>>,
}
