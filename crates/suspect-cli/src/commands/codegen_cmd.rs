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
    /// Defaults to `.suspect.yaml`'s `codegen.profile` when omitted.
    #[arg(long, value_enum)]
    pub profile: Option<SdkProfile>,
    /// Exact operationId selector (repeatable); omit to attempt all operations.
    #[arg(long, allow_hyphen_values = true)]
    pub operation_id: Vec<String>,
    /// Native package identity, independent of the API title.
    #[arg(long)]
    pub package_name: String,
    /// Exact native-compatible package SemVer, independent of API info.version.
    /// Defaults to `.suspect.yaml`'s `codegen.package_version` when omitted.
    #[arg(long)]
    pub package_version: Option<String>,
    /// Native import/module identity, independent of the source API semantics.
    #[arg(long)]
    pub import_name: Option<String>,
    /// Explicit versioned source interpretation (repeatable); ordinary OpenAPI
    /// semantics remain the default, independently of native feature support.
    #[arg(long, value_enum)]
    pub compatibility_profile: Vec<SdkCompatibilityProfile>,
    /// Generation options file: a JSON document holding
    /// `compatibility_profiles`, `credential_env` and `sdk_defaults`
    /// (the same schema `codegen-session` accepts). Flag-declared
    /// compatibility profiles are added on top.
    #[arg(long, value_name = "FILE")]
    pub defaults: Option<PathBuf>,
    /// Output root for generated artifacts. Defaults to `.suspect.yaml`'s
    /// `codegen.out`, then `codegen-out`.
    #[arg(short, long)]
    pub out: Option<PathBuf>,
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
pub fn codegen(mut args: CodegenArgs) -> anyhow::Result<i32> {
    // The `.suspect.yaml` codegen section supplies flag defaults, the
    // same precedence `suspect docs` uses for style and output: an
    // explicit flag always wins, the file fills what the flag omitted,
    // and with neither the historical defaults apply.
    let settings_source = match args.spec.as_deref().or(args.pins.as_deref()) {
        Some(entry) => suspect_config::for_invocation(Some(entry))
            .map_err(|e| anyhow::anyhow!("{e}"))
            .ok(),
        None => suspect_config::for_invocation(None).ok(),
    };
    if let Some(loaded) = settings_source {
        let codegen = &loaded.settings.codegen;
        if args.profile.is_none()
            && let Some(profile) = &codegen.profile
        {
            let Some(resolved) = super::sdk::profile_by_name(profile) else {
                return Err(anyhow::anyhow!(
                    "codegen profile `{profile}` is not one of the native profiles"
                ));
            };
            args.profile = Some(resolved);
        }
        if args.package_version.is_none() {
            args.package_version = codegen.package_version.clone();
        }
        if args.out.is_none()
            && let Some(out) = &codegen.out
        {
            args.out = Some(loaded.root.join(out));
        }
    }
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
    // Generation options: the `--defaults` file carries the policy-shaped
    // knobs (credential env mapping, SDK defaults incl. pagination and
    // OAuth lifecycle); the flags add interpretation choices on top.
    let mut generation = match &args.defaults {
        Some(path) => {
            let bytes =
                std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
            serde_json::from_slice::<suspect_codegen::backend::GenerationOptions>(&bytes).map_err(
                |e| anyhow::anyhow!("invalid generation options {}: {e}", path.display()),
            )?
        }
        None => suspect_codegen::backend::GenerationOptions::default(),
    };
    for profile in args.compatibility_profile {
        generation.compatibility_profiles.insert(profile.0);
    }
    // Everything the settings could not supply has a historical default;
    // the profile is the one selection with no default, so name it.
    let profile = args.profile.unwrap_or_else(|| {
        eprintln!("--profile is required (or set codegen.profile in .suspect.yaml)");
        std::process::exit(2);
    });
    super::sdk::generate(&SdkArgs {
        input,
        shared: None,
        profile,
        operation_id: args.operation_id,
        package_name: args.package_name,
        package_version: args.package_version.unwrap_or_else(|| "0.1.0".to_owned()),
        import_name: args.import_name,
        generation,
        out: args.out.unwrap_or_else(|| PathBuf::from("codegen-out")),
        check: args.check,
        text: TextFormat {
            format: args.format,
        },
    })
}
