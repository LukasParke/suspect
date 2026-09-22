//! Closed MCP surface vocabulary. Nothing here has a default, so no exposed
//! tool, public property, advisory annotation or runtime bound can be concealed
//! by omission.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Build, identity and runtime configuration of the emitted application,
/// separate from every API semantic. The witnessed official SDK and toolchain
/// pins are [`super::MCP_SERVER_VERSION`], [`super::MCP_CLIENT_VERSION`] and
/// [`super::TYPESCRIPT_VERSION`]; any other pin is refused rather than
/// generated against an unverified dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpTargetConfig {
    /// npm identity of the single emitted private package.
    pub package_name: String,
    /// Executable name published under `bin`; also the documented command.
    pub bin_name: String,
    /// MCP server identity reported during initialization.
    pub server_name: String,
    /// Exact SemVer of both the package and the reported server version.
    pub version: String,
    /// Exact reviewed Node version, recorded for review and documentation.
    pub node_version: String,
    /// Minimum Node major written into `engines.node`.
    pub node_minimum_major: u32,
    /// Exact npm version written into `packageManager`.
    pub npm_version: String,
    /// Exact TypeScript version used to build the emitted package.
    pub typescript_version: String,
    /// Exact `@types/node` version the emitted package builds against. The
    /// official MCP SDK's own declarations require it.
    pub node_types_version: String,
    /// Exact `@modelcontextprotocol/server` version the application depends on.
    pub mcp_server_version: String,
    /// Exact `@modelcontextprotocol/client` version used by acceptance tests.
    /// The emitted package never depends on it.
    pub mcp_client_version: String,
    /// Explicit source-scheme to environment VARIABLE NAME policy. The
    /// generated SDK factory is the only credential reader; generation never
    /// reads a value and no tool input ever carries one. `null` selects an
    /// anonymous application, which is refused when the selected operations
    /// require credentials.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Environment VARIABLE NAME that may supply an absolute base URL
    /// override. `null` forbids any override. This is never a credential and
    /// never a tool input.
    pub server_url_env: Option<String>,
    /// Finite runtime bounds compiled into the executable.
    pub runtime: McpRuntime,
    /// Diagnostic policy. Every log line goes to standard error.
    pub logs: McpLogs,
}

/// Finite bounds; MCP clients cannot raise them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpRuntime {
    /// Whole-call deadline in milliseconds, 1..=3_600_000, combined with the
    /// request's own cancellation signal.
    pub call_deadline_ms: u64,
    /// Ceiling on one call's encoded arguments, checked before any codec runs,
    /// 1..=8_388_608.
    pub max_input_bytes: usize,
    /// Ceiling on one call's exact result document, 1..=8_388_608.
    pub max_result_bytes: usize,
}

/// Diagnostic policy. Standard output carries protocol traffic exclusively, so
/// every policy writes to standard error only, and no policy logs argument or
/// document values.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpLogs {
    pub policy: LogPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogPolicy {
    /// Nothing is written, not even a failure.
    Off,
    /// One line per failed call.
    Failures,
    /// One line per call outcome, successful or not.
    Calls,
}

impl LogPolicy {
    pub(super) const fn surface(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Failures => "failures",
            Self::Calls => "calls",
        }
    }
}

/// The closed, versioned tool mapping. `format` must equal [`super::PROFILE`];
/// any other profile version is refused.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingProfile {
    pub format: String,
    /// Prose describing the exposed surface, used by the generated guide.
    pub description: String,
    /// The explicit allowlist of exposed operations, in tool listing order.
    pub tools: Vec<ToolMapping>,
}

/// One exposed operation and every public name it contributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolMapping {
    /// Exact operationId, or the exact `METHOD /path` form.
    pub selector: String,
    /// Stable public tool name.
    pub name: String,
    /// Human-readable display title.
    pub title: String,
    /// Tool description shown during discovery.
    pub description: String,
    pub annotations: ToolAdvisory,
    pub input: InputBinding,
}

/// Advisory discovery hints. These are hints only: they change nothing the
/// emitted server actually does, and a client must not rely on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolAdvisory {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

/// How one tool's arguments carry the selected operation's inputs. There is no
/// filesystem-path form: a request body arrives as one projected JSON value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputBinding {
    /// Public property names keyed by exact source parameter name. Every source
    /// parameter needs exactly one entry; unknown names are refused.
    pub parameters: BTreeMap<String, PropertyMapping>,
    pub body: BodyBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyMapping {
    /// Public tool input property name.
    pub property: String,
    /// Property description projected into the tool input schema.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BodyBinding {
    /// The operation declares no request body.
    None,
    /// One finite JSON value supplied under `property`, projected from the
    /// declared body schema and decoded by the generated codec.
    JsonValue {
        property: String,
        description: String,
    },
}

/// How one tool input property carries a source value exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// A JSON string; the empty string is a real value, distinct from omission.
    String,
    /// A JSON boolean.
    Boolean,
    /// A JSON null.
    Null,
    /// An exact JSON number token carried as a string at the tool boundary.
    NumberToken,
    /// A projected JSON array.
    Array,
    /// A projected JSON object.
    Object,
    /// A projected union of the above, dispatched by JSON value kind.
    Union,
    /// One complete projected JSON request-body value.
    BodyValue,
}

impl Representation {
    /// Stable name recorded in the application surface manifest.
    #[must_use]
    pub const fn surface(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Null => "null",
            Self::NumberToken => "exact_number_token",
            Self::Array => "array",
            Self::Object => "object",
            Self::Union => "union",
            Self::BodyValue => "exact_json_value",
        }
    }
}
