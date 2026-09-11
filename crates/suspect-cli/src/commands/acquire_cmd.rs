//! Public arguments for explicit pinned source acquisition.

use std::{path::PathBuf, time::Duration};

use clap::Args;
use suspect_ref::acquire::{AcquireOptions, RefreshPolicy, parse_utc_timestamp};

use crate::OutputFormat;

/// Acquire or verify a complete, immutable retrieval manifest.
#[derive(Debug, Args)]
pub struct AcquireArgs {
    /// Versioned manifest containing exact retrieval URIs and SHA-256 pins.
    pub manifest: PathBuf,
    /// Content-addressed document cache.
    #[arg(long, default_value = ".suspect-cache")]
    pub cache_dir: PathBuf,
    /// Verify cache bytes without source-file reads or network requests.
    #[arg(long, conflicts_with_all = ["refresh", "stale_before"])]
    pub offline: bool,
    /// Refetch every declared pin and reject byte or redirect drift.
    #[arg(long, conflicts_with = "stale_before")]
    pub refresh: bool,
    /// Refetch pins older than this UTC YYYY-MM-DDTHH:MM:SSZ timestamp.
    #[arg(long)]
    pub stale_before: Option<String>,
    /// Per-document byte ceiling.
    #[arg(long, default_value_t = 64 << 20)]
    pub max_bytes: u64,
    /// Total byte ceiling across the declared closure.
    #[arg(long, default_value_t = 256 << 20)]
    pub max_total_bytes: u64,
    /// End-to-end acquisition deadline in seconds.
    #[arg(long, default_value_t = 30)]
    pub timeout_seconds: u64,
    /// Exact destination origin allowed for a manifest-declared cross-origin redirect.
    #[arg(long)]
    pub allow_redirect_origin: Vec<String>,
    /// Explicit HTTP origin for a numeric-loopback test fixture.
    #[arg(long)]
    pub insecure_test_origin: Vec<String>,
    /// Human-readable status or a structured provenance/diagnostic report.
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

/// Run the public acquisition command with explicit policies.
///
/// # Errors
/// Invalid command policy, runtime setup or output failures.
pub fn acquire(args: AcquireArgs) -> anyhow::Result<i32> {
    let refresh = match args.stale_before {
        Some(before) => RefreshPolicy::StaleOnly {
            before: parse_utc_timestamp(&before)?,
        },
        None if args.refresh => RefreshPolicy::All,
        None => RefreshPolicy::Never,
    };
    super::acquire::run(
        &args.manifest,
        AcquireOptions {
            cache_dir: args.cache_dir,
            offline: args.offline,
            refresh,
            max_bytes: args.max_bytes,
            max_total_bytes: args.max_total_bytes,
            timeout: Duration::from_secs(args.timeout_seconds),
            allowed_redirect_origins: args.allow_redirect_origin,
            insecure_test_origins: args.insecure_test_origin,
            ..Default::default()
        },
        args.format,
    )
}
