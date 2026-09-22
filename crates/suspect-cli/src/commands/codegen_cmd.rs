//! Canonical SDK generation through the shared native backend boundary.

use super::sdk::{SdkArgs, SdkCompatibilityProfile, SdkProfile};
use crate::{OutputFormat, TextFormat};
use clap::Args;
use std::path::PathBuf;
use suspect_codegen::generation_session::Input;

/// Public source-selected SDK generation options.
#[derive(Debug, Args)]
pub struct CodegenArgs {
    /// Entry OpenAPI document.
    #[arg(required_unless_present = "pins", conflicts_with = "pins")]
    pub spec: Option<PathBuf>,
    /// Immutable pin manifest for cache-only generation.
    #[arg(long)]
    pub pins: Option<PathBuf>,
    /// Cache populated by `suspect acquire`; used with --pins.
    #[arg(long, requires = "pins")]
    pub cache_dir: Option<PathBuf>,
    /// Numeric-loopback origin recorded by an explicitly acquired test manifest.
    #[arg(long, requires = "pins")]
    pub insecure_test_origin: Vec<String>,
    /// Native capability profile; unsupported contracts fail before output.
    #[arg(long, value_enum)]
    pub profile: SdkProfile,
    /// Exact operationId selector (repeatable); omit to attempt all operations.
    #[arg(long, allow_hyphen_values = true)]
    pub operation_id: Vec<String>,
    /// Native package identity, independent of the API title.
    #[arg(long)]
    pub package_name: String,
    /// Exact native-compatible package SemVer, independent of API info.version.
    #[arg(long)]
    pub package_version: String,
    /// Native import/module identity, independent of the source API semantics.
    #[arg(long)]
    pub import_name: Option<String>,
    /// Explicit versioned source interpretation (repeatable); ordinary OpenAPI
    /// semantics remain the default, independently of native feature support.
    #[arg(long, value_enum)]
    pub compatibility_profile: Vec<SdkCompatibilityProfile>,
    /// Output root for generated artifacts.
    #[arg(short, long, default_value = "codegen-out")]
    pub out: PathBuf,
    /// Check ownership and drift without writing.
    #[arg(long)]
    pub check: bool,
    /// Human-readable output or a structured report.
    #[arg(long,value_enum,default_value_t=OutputFormat::Text)]
    pub format: OutputFormat,
}

/// Generate a native SDK from the source-selected canonical contract.
///
/// # Errors
/// Propagates invalid configuration, source loading and emission failures.
pub fn codegen(args: CodegenArgs) -> anyhow::Result<i32> {
    let input = match (args.spec, args.pins) {
        (Some(path), None) => Input::File { path },
        (None, Some(manifest)) => Input::Pinned {
            manifest,
            cache_dir: args
                .cache_dir
                .unwrap_or_else(|| PathBuf::from(".suspect-cache")),
            insecure_test_origins: args.insecure_test_origin,
        },
        _ => anyhow::bail!("choose an OpenAPI input or --pins manifest"),
    }
    .normalized()?;
    super::sdk::generate(&SdkArgs {
        input,
        profile: args.profile,
        operation_id: args.operation_id,
        package_name: args.package_name,
        package_version: args.package_version,
        import_name: args.import_name,
        generation: suspect_codegen::backend::GenerationOptions {
            compatibility_profiles: args
                .compatibility_profile
                .into_iter()
                .map(|profile| profile.0)
                .collect(),
            ..Default::default()
        },
        out: args.out,
        check: args.check,
        text: TextFormat {
            format: args.format,
        },
    })
}
