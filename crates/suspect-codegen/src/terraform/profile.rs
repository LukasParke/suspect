//! Closed lifecycle vocabulary. No defaults conceal lifecycle decisions.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Build identity, separate from API and lifecycle semantics. The witnessed
/// Framework version is currently 1.15.1 (Plugin Go 0.27.0, Go >=1.23.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    /// Terraform provider local name, used in resource type prefixes and examples.
    pub provider_name: String,
    /// Registry source address (`namespace/type` or `host/namespace/type`).
    pub provider_address: String,
    /// Go module path of the generated provider.
    pub module_path: String,
    /// Provider version, without a leading `v`.
    pub version: String,
    /// `go` directive of the provider module.
    pub go_version: String,
    /// `toolchain` directive of the provider module.
    pub go_toolchain: String,
    /// Exact Terraform CLI version pinned by the examples.
    pub terraform_version: String,
    /// Pinned `terraform-plugin-framework` version, without a leading `v`.
    pub framework_version: String,
    /// The generated Go SDK the provider depends on.
    pub sdk: SdkDependency,
}

/// Exact generated SDK dependency; no local replace directive is emitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkDependency {
    /// Go module path of the generated SDK.
    pub module_path: String,
    /// Required SDK module version, without a leading `v`.
    pub version: String,
}

/// Closed lifecycle mapping document, versioned by [`PROFILE`](super::PROFILE).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingProfile {
    /// Must equal [`PROFILE`](super::PROFILE).
    pub format: String,
    /// How the provider authenticates SDK calls.
    pub authentication: Authentication,
    /// Resources keyed by Terraform type suffix.
    pub resources: BTreeMap<String, ResourceMapping>,
    /// Data sources keyed by Terraform type suffix.
    pub data_sources: BTreeMap<String, DataSourceMapping>,
}

/// The first bounded profile admits anonymous APIs or one source-declared
/// bearer scheme. The allocated SDK credential constructor remains authoritative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Authentication {
    /// Anonymous API; no provider credential attribute.
    None,
    /// The single source-declared bearer scheme.
    Bearer {
        /// Security-scheme name in the source document.
        scheme: String,
    },
}

/// Terraform attribute value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    /// `types.String`.
    String,
    /// `types.Bool`.
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

/// Who supplies an attribute's value: configuration, the API, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeMode {
    /// Must be configured.
    Required,
    /// May be configured; otherwise null.
    Optional,
    /// Supplied only by the API.
    Computed,
    /// May be configured; otherwise supplied by the API.
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

/// One Terraform schema attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    /// Value type.
    pub r#type: AttributeType,
    /// Configured/computed mode.
    pub mode: AttributeMode,
    /// Redacted in CLI output (still stored in state).
    pub sensitive: bool,
    /// Read from configuration and sent, never stored in state; must be configured and sensitive.
    pub write_only: bool,
    /// Changing the value forces resource replacement.
    pub requires_replace: bool,
    /// Terraform-only trigger values; accepted only as a write-only update trigger.
    pub state_only: bool,
    /// Attribute documentation.
    pub description: String,
}

/// Explicit lifecycle of one managed resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceMapping {
    /// Resource documentation.
    pub description: String,
    /// Schema attributes keyed by name.
    pub attributes: BTreeMap<String, Attribute>,
    /// Resource identity and import format.
    pub identity: Identity,
    /// Create operation binding.
    pub create: Mutation,
    /// Read operation binding, also used for refresh and import.
    pub read: Read,
    /// Update operation binding.
    pub update: Mutation,
    /// Delete operation binding.
    pub delete: Delete,
    /// Refresh policy.
    pub refresh: Refresh,
    /// Retry policy.
    pub retry: Retry,
    /// Polling policy.
    pub polling: Polling,
    /// Failure handling policy.
    pub failure: Failure,
}

/// Explicit binding of one read-only data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSourceMapping {
    /// Data source documentation.
    pub description: String,
    /// Schema attributes keyed by name.
    pub attributes: BTreeMap<String, Attribute>,
    /// Read operation binding.
    pub read: Read,
    /// Retry policy.
    pub retry: Retry,
}

/// The attribute that identifies a resource and how it is imported.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Identity attribute; must be a computed, non-sensitive string API attribute.
    pub attribute: String,
    /// Accepted `terraform import` ID format.
    pub import: ImportFormat,
}

/// Accepted `terraform import` ID formats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    /// The exact nonempty string, with no splitting, normalization or decoding.
    OpaqueString,
}

/// Refresh policy of a resource.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refresh {
    /// Read maps authoritative remote values; missing removes resource state.
    AuthoritativeRead,
}
/// Retry policy; v1 never retries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    /// No retries.
    None,
}
/// Polling policy; v1 never polls.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polling {
    /// No polling.
    None,
}
/// Failure handling policy of a resource.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// Mapped partial responses save authoritative state and return an error.
    /// Other errors preserve prior state (create has none). Never retry.
    MappedPartialOtherwisePreserve,
}

/// A create or update operation binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    /// Bound SDK operation, by source `operationId`.
    pub operation_id: String,
    /// Request inputs taken from Terraform attributes.
    pub inputs: Vec<Input>,
    /// Declared success response and the state it maps.
    pub success: Response,
    /// Declared error responses whose mapped state is saved before the error is returned.
    pub partial: Vec<Response>,
}
/// A read operation binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    /// Bound SDK operation, by source `operationId`.
    pub operation_id: String,
    /// Request inputs taken from Terraform attributes.
    pub inputs: Vec<Input>,
    /// Declared success response and the state it maps.
    pub success: Response,
    /// Exact source-declared API error statuses, not arbitrary transport statuses.
    pub missing: Vec<u16>,
}
/// A delete operation binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delete {
    /// Bound SDK operation, by source `operationId`.
    pub operation_id: String,
    /// Request inputs taken from Terraform attributes.
    pub inputs: Vec<Input>,
    /// Expected success status.
    pub success_status: u16,
    /// Declared API error statuses that mean already deleted (treated as success).
    pub missing: Vec<u16>,
}

/// One request input sourced from a Terraform attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Source Terraform attribute.
    pub attribute: String,
    /// Where the value is placed in the request.
    pub target: InputTarget,
    /// Handling of a null attribute value.
    pub null: NullInput,
    /// When the input is sent.
    pub when: InputWhen,
}
/// Where an input value is placed in the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputTarget {
    /// A declared operation parameter.
    Parameter {
        /// Parameter location.
        location: Location,
        /// Declared parameter name.
        name: String,
    },
    /// A wire-property path. V1 admits exactly one field of a native record.
    Body {
        /// Wire property names from the body root.
        path: Vec<String>,
    },
}
/// Location of a parameter input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    /// Path parameter.
    Path,
    /// Query parameter.
    Query,
    /// Header parameter.
    Header,
    /// Cookie parameter.
    Cookie,
}
/// Handling of a null attribute value for an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullInput {
    /// Refuse the call.
    Reject,
    /// Leave the input unset; the target must be optional.
    Omit,
    /// Send an explicit null; the target must admit null.
    SendNull,
}
/// When an input is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputWhen {
    /// On every call of the bound operation.
    Always,
    /// Only for optional write-only mutation fields on update. The trigger is
    /// a configured Terraform-only attribute. Unknown triggers block apply.
    TriggerChanged {
        /// The Terraform-only trigger attribute.
        attribute: String,
    },
}

/// A declared response and how it maps into Terraform state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Exact source-declared status.
    pub status: u16,
    /// Terraform attribute -> exact wire-property path in the typed SDK payload.
    pub state: BTreeMap<String, Vec<String>>,
}
