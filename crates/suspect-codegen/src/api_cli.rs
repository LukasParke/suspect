//! A Go/Cobra command-line application target over the canonical generated Go SDK.
//!
//! The exposed surface is supplied by a closed, versioned mapping: it names
//! every command, flag, body and confirmation policy explicitly. HTTP
//! semantics, authentication, schema validation and exact JSON handling stay
//! in [`crate::go_http`] and its codecs; this target only binds allocated
//! native names to public CLI names and emits one self-contained Go module.
//!
//! Unsupported media, unrepresentable parameters, colliding public names and
//! invalid identity all fail here, during generation, with source-linked
//! [`crate::application::Diagnostic`]s - never at the emitted program's first use.

use std::sync::Arc;

use suspect_ir::contract::Contract;

use crate::{
    OutFile,
    application::{Diagnostic, DocumentScope},
    go_http,
};

mod emit;
mod package;
mod planning;
mod profile;
mod surface;

pub use profile::*;

/// The exact mapping profile understood by this target.
pub const PROFILE: &str = "suspect.application.cli.v1";

/// The exact application surface manifest format emitted for review.
pub const SURFACE_FORMAT: &str = "suspect.application.cli.surface.v1";

/// Registry-verified Cobra release used by the emitted application. Cobra is
/// an application-only dependency; generated SDKs never require it.
pub const COBRA_VERSION: &str = "1.10.2";

/// Immutable admitted command surface and native SDK binding plan.
#[derive(Debug)]
pub struct CliPlan {
    sdk: go_http::HttpPlan,
    sdk_files: Vec<OutFile>,
    mapping: MappingProfile,
    config: CliTargetConfig,
    commands: Vec<planning::BoundCommand>,
    /// Allocated SDK environment credential factory, when one is configured.
    credential_factory: Option<String>,
    /// The documents this application may record in its surface manifest.
    /// Every source the manifest names was admitted through it during planning.
    scope: DocumentScope,
}

impl CliPlan {
    /// The actual canonical native plan behind every command.
    #[must_use]
    pub fn sdk_plan(&self) -> &go_http::HttpPlan {
        &self.sdk
    }

    #[must_use]
    pub fn mapping(&self) -> &MappingProfile {
        &self.mapping
    }

    #[must_use]
    pub fn config(&self) -> &CliTargetConfig {
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

/// Bind the explicitly mapped operations to the actual Go SDK plan. No
/// filesystem writes, HTTP requests, environment reads or name inference occur.
///
/// # Errors
/// Unknown profile versions, unresolved selectors, colliding or reserved
/// public names, unmapped or unrepresentable parameters, unsupported media,
/// invalid runtime bounds or invalid package identity - each located at its
/// mapping pointer and its contract source.
pub fn plan_cli(
    contract: Arc<Contract>,
    mapping: MappingProfile,
    config: CliTargetConfig,
) -> Result<CliPlan, Vec<Diagnostic>> {
    planning::plan(contract, mapping, config)
}

/// The complete desired artifact set: one Go module rooted at the output
/// directory, with the canonical SDK embedded under `internal/sdk/`, the Cobra
/// application under `internal/cli/`, the executable under `cmd/<binary>/` and
/// a deterministic `application-surface.json`. Emission performs no I/O; use
/// `write_files_with_owner` / `check_files_with_owner` with a dedicated owner.
#[must_use]
pub fn emit_cli(plan: &CliPlan) -> Vec<OutFile> {
    emit::artifacts(plan)
}
