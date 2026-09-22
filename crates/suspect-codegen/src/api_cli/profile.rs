//! Closed CLI surface vocabulary. Nothing here has a default, so no public
//! command, flag, body or confirmation decision can be concealed by omission.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Build and runtime identity of the emitted application, separate from every
/// API semantic. The witnessed Cobra version is [`super::COBRA_VERSION`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliTargetConfig {
    /// Go module path of the single emitted module, e.g. `example.com/team/ctl`.
    pub module_path: String,
    /// Executable name; also the root command name and the `cmd/<name>/` directory.
    pub binary_name: String,
    /// Exact SemVer reported by `--version`; version requirements are rejected.
    pub version: String,
    /// `go` directive of the emitted module.
    pub go_version: String,
    /// `toolchain` directive of the emitted module.
    pub go_toolchain: String,
    /// Explicit source-scheme to environment VARIABLE NAME policy. The
    /// generated SDK factory is the only credential reader; generation never
    /// reads a value. `null` selects an anonymous application.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Finite runtime bounds compiled into the executable.
    pub runtime: CliRuntime,
    /// Machine-output policy for successful and documented-failure documents.
    pub output: CliOutput,
}

/// Finite bounds; callers of the emitted binary cannot raise them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliRuntime {
    /// Whole-request deadline in milliseconds, 1..=3_600_000.
    pub request_deadline_ms: u64,
    /// Ceiling on a request body read from a file or standard input, checked
    /// before any decode, 1..=8_388_608.
    pub max_input_bytes: usize,
}

/// Machine-output policy. Both formats write the SDK codec's exact bytes; the
/// indented form only inserts whitespace between already-encoded tokens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliOutput {
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// One compact exact JSON document per command, followed by a newline.
    CompactJson,
    /// The same exact tokens, indented with two spaces.
    PrettyJson,
}

impl OutputFormat {
    pub(super) fn surface(self) -> &'static str {
        match self {
            Self::CompactJson => "compact_json",
            Self::PrettyJson => "pretty_json",
        }
    }
}

/// The closed, versioned command mapping. `format` must equal
/// [`super::PROFILE`]; any other profile version is refused.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingProfile {
    pub format: String,
    /// Long help for the root command.
    pub description: String,
    /// The explicit allowlist of exposed operations, in command-tree order.
    pub commands: Vec<CommandMapping>,
}

/// One exposed operation and every public name it contributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandMapping {
    /// Exact operationId, or the exact `METHOD /path` form.
    pub selector: String,
    /// Command path under the root, 1..=4 lower-case segments.
    pub path: Vec<String>,
    /// One-line help.
    pub summary: String,
    /// Long help.
    pub description: String,
    /// Flag names keyed by exact source parameter name. Every source
    /// parameter needs exactly one entry; unknown names are refused.
    pub parameters: BTreeMap<String, ParameterMapping>,
    pub body: BodyPolicy,
    pub confirmation: ConfirmationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterMapping {
    /// Long flag name without leading dashes.
    pub flag: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BodyPolicy {
    /// The operation declares no request body.
    None,
    /// One finite JSON document supplied by `--body-file <path>`, where `-`
    /// reads standard input. There is no inline-value form.
    JsonDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfirmationPolicy {
    NotRequired,
    /// The command prints its target operation and refuses to send unless
    /// `--confirm <token>` matches, or `--confirm -` reads the exact token
    /// from the first line of standard input.
    Required {
        token: String,
    },
}

/// How one CLI flag carries a source parameter exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// A string flag; the empty string is a real value, distinct from omission.
    String,
    /// A boolean flag; `--flag=false` is a real value, distinct from omission.
    Boolean,
    /// A text flag parsed by the SDK's exact integer tokenizer.
    Integer,
    /// A text flag parsed by the SDK's exact number tokenizer.
    Number,
}

impl Representation {
    /// Stable name recorded in the application surface manifest.
    #[must_use]
    pub const fn surface(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Integer => "exact_integer",
            Self::Number => "exact_number",
        }
    }
}
