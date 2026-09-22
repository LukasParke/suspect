//! A TypeScript stdio MCP server application target over the canonical
//! generated TypeScript SDK.
//!
//! The exposed surface is supplied by a closed, versioned mapping: it names
//! every tool, input property and advisory annotation explicitly. HTTP
//! semantics, authentication, schema validation and exact JSON handling stay in
//! [`crate::typescript::http`] and its codecs; this target only projects
//! allocated native models into MCP JSON Schema, binds public names to
//! allocated SDK names, and emits one self-contained ESM package.
//!
//! Unsupported media, unrepresentable schemas, colliding public names, invalid
//! identity and unverified toolchain pins all fail here, during generation,
//! with source-linked [`crate::application::Diagnostic`]s - never at the
//! emitted server's first use.
//!
//! The emitted server is stdio only. Standard output carries protocol traffic
//! exclusively; every diagnostic goes to standard error. No tool input ever
//! carries a credential or a filesystem path, and no resource, prompt, retry or
//! pagination traversal is generated.

use std::{collections::BTreeMap, sync::Arc};

use suspect_ir::contract::Contract;

use crate::{
    OutFile,
    application::{Diagnostic, DocumentScope},
    typescript::http as ts_http,
};

mod emit;
mod package;
mod planning;
mod profile;
mod schema;
mod surface;

pub use planning::RESERVED_TOOL_NAMES;
pub use profile::*;

/// The exact mapping profile understood by this target.
pub const PROFILE: &str = "suspect.application.mcp.v1";

/// The exact application surface manifest format emitted for review.
pub const SURFACE_FORMAT: &str = "suspect.application.mcp.surface.v1";

/// Registry-verified official MCP server package used by the emitted
/// application. Verified against the npm registry on 2026-09-21.
pub const MCP_SERVER_VERSION: &str = "2.0.0";

/// Registry-verified official MCP client package. It is an acceptance-testing
/// tool only: the emitted package never depends on it.
pub const MCP_CLIENT_VERSION: &str = "2.0.0";

/// Registry-verified TypeScript release used to build the emitted package.
pub const TYPESCRIPT_VERSION: &str = "5.9.3";

/// Registry-verified Node type declarations. The official MCP SDK's own
/// declarations reference `node:stream` and `Buffer`, so the emitted package
/// cannot compile without them.
pub const NODE_TYPES_VERSION: &str = "26.6.2";

/// The official MCP SDK's own minimum Node major, taken on its own.
const MCP_SDK_NODE_MINIMUM_MAJOR: u32 = 20;

/// The Node major an emitted application actually requires. A configuration
/// may require a newer runtime, never an older one.
///
/// The application depends on the official MCP SDK *and* embeds the canonical
/// Suspect TypeScript SDK as compiled-in source, so its real floor is the
/// stricter of the two. It is derived from
/// [`crate::typescript::package::NODE_MINIMUM_MAJOR`] rather than restated,
/// so raising the embedded SDK's floor can never leave an emitted application
/// declaring an `engines.node` it cannot honour.
pub const NODE_MINIMUM_MAJOR: u32 = {
    let embedded = crate::typescript::package::NODE_MINIMUM_MAJOR;
    if embedded > MCP_SDK_NODE_MINIMUM_MAJOR {
        embedded
    } else {
        MCP_SDK_NODE_MINIMUM_MAJOR
    }
};

/// Immutable admitted tool surface and native SDK binding plan.
#[derive(Debug)]
pub struct ServerPlan {
    sdk: ts_http::HttpPlan,
    sdk_files: Vec<OutFile>,
    mapping: MappingProfile,
    config: McpTargetConfig,
    tools: Vec<planning::BoundTool>,
    /// Source scheme name to environment variable name, as the generated SDK
    /// factory actually snapshots them, when a policy is configured.
    credential_bindings: Option<BTreeMap<String, String>>,
    /// The documents this application may record in its surface manifest.
    /// Every source the manifest names was admitted through it during planning.
    scope: DocumentScope,
}

impl ServerPlan {
    /// The actual canonical native plan behind every tool.
    #[must_use]
    pub fn sdk_plan(&self) -> &ts_http::HttpPlan {
        &self.sdk
    }

    #[must_use]
    pub fn mapping(&self) -> &MappingProfile {
        &self.mapping
    }

    #[must_use]
    pub fn config(&self) -> &McpTargetConfig {
        &self.config
    }
}

/// Parse the closed mapping profile. Unknown fields and variants are errors,
/// never ignored, so a mapping can never silently expose less than it names.
///
/// # Errors
/// Malformed JSON, or a field or variant outside the versioned vocabulary.
pub fn parse_mapping(json: &str) -> Result<MappingProfile, serde_json::Error> {
    serde_json::from_str(json)
}

/// Bind the explicitly mapped operations to the actual TypeScript SDK plan. No
/// filesystem writes, HTTP requests, environment reads or name inference occur.
///
/// # Errors
/// Unknown profile versions, unresolved selectors, colliding or reserved
/// public names, unmapped or unrepresentable parameters, schemas with no exact
/// tool projection, unsupported media, mandatory credentials with no
/// environment policy, invalid runtime bounds, invalid package identity and
/// unverified toolchain pins - each located at its mapping pointer and its
/// contract source.
pub fn plan_server(
    contract: Arc<Contract>,
    mapping: MappingProfile,
    config: McpTargetConfig,
) -> Result<ServerPlan, Vec<Diagnostic>> {
    planning::plan(contract, mapping, config)
}

/// The complete desired artifact set: one private ESM package rooted at the
/// output directory, with the canonical SDK embedded under `typescript/`, the
/// generated tool registrations and stdio entry point under `source/`, a
/// pinned lockfile, a guide and a deterministic `application-surface.json`.
/// Emission performs no I/O; use `write_files_with_owner` /
/// `check_files_with_owner` with a dedicated owner.
#[must_use]
pub fn emit_server(plan: &ServerPlan) -> Vec<OutFile> {
    emit::artifacts(plan)
}
