//! A separate Terraform artifact target over the canonical generated Go SDK.
//!
//! Lifecycle intent is supplied by a closed, versioned mapping. HTTP semantics,
//! checked codecs, native names and API calls belong to [`crate::go_http`]. This
//! module is deliberately independent of the SDK language/backend registry.

use std::{ops::Range, sync::Arc};

use suspect_ir::contract::{Contract, SourceId};

use crate::{OutFile, go_http};

mod docs;
mod emit;
mod native;
mod package;
mod planning;
mod profile;

pub use profile::*;

/// Exact lifecycle profile understood by this target.
pub const PROFILE: &str = "suspect.terraform.lifecycle.v1";

/// A located refusal. Mapping pointers address the separate mapping JSON;
/// source/span address the actual canonical OpenAPI/Go binding where available.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub mapping_pointer: String,
    pub source: SourceId,
    pub at: Range<usize>,
}

/// Immutable admitted lifecycle and native SDK binding plan.
#[derive(Debug)]
pub struct ProviderPlan {
    sdk: go_http::HttpPlan,
    sdk_files: Vec<OutFile>,
    profile: MappingProfile,
    config: TargetConfig,
    resources: Vec<planning::BoundResource>,
    data_sources: Vec<planning::BoundDataSource>,
    credential: Option<String>,
}

impl ProviderPlan {
    /// The actual canonical native plan used for every operation and field.
    #[must_use]
    pub fn sdk_plan(&self) -> &go_http::HttpPlan {
        &self.sdk
    }

    #[must_use]
    pub fn mapping(&self) -> &MappingProfile {
        &self.profile
    }

    #[must_use]
    pub fn config(&self) -> &TargetConfig {
        &self.config
    }
}

/// Parse the closed profile. Unknown fields/variants are errors, never ignored.
///
/// # Errors
/// Malformed JSON or a field/variant outside the versioned mapping vocabulary.
pub fn parse_mapping(json: &str) -> Result<MappingProfile, serde_json::Error> {
    serde_json::from_str(json)
}

/// Bind explicit lifecycle operations and paths to the actual Go SDK plan.
/// No filesystem writes, HTTP requests or lifecycle-name inference occur here.
///
/// # Errors
/// Unsupported lifecycle mappings, unresolved source/native bindings, SDK
/// admission failures or invalid package/dependency identity.
pub fn plan_provider(
    contract: Arc<Contract>,
    profile: MappingProfile,
    config: TargetConfig,
) -> Result<ProviderPlan, Vec<Diagnostic>> {
    planning::plan(contract, profile, config)
}

/// Complete desired artifacts: canonical SDK under `go/`, dependent provider
/// under `terraform/`. Use `write_files_with_owner` / `check_files_with_owner`
/// with a dedicated stable target owner; emission itself performs no I/O.
#[must_use]
pub fn emit_provider(plan: &ProviderPlan) -> Vec<OutFile> {
    emit::artifacts(plan)
}
