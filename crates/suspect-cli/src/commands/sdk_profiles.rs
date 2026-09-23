//! Editor/automation discovery from the canonical backend registry.

use std::io::{self, Write};

use serde_json::json;
use suspect_codegen::backend::Backend;
use suspect_codegen::http_protocol::CompatibilityProfile;

use crate::{OutputFormat, commands::application_cmd::Target};

/// Which profile family to list. Application targets are separate profiles
/// with their own mappings, commands and output-root owners, so they are never
/// mixed into the native SDK backend inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum ProfileKind {
    /// Native SDK backends (the default).
    Sdk,
    /// Application targets generated over an embedded canonical SDK.
    Applications,
}

/// List exactly the profiles compiled into this binary, without source IO.
///
/// # Errors
/// Propagates output failures.
pub fn list(format: OutputFormat) -> anyhow::Result<i32> {
    let mut output = io::stdout().lock();
    match format {
        OutputFormat::Json | OutputFormat::Sarif => {
            serde_json::to_writer_pretty(
                &mut output,
                &json!({"format":"suspect.sdk.profiles.v1",
                    "profiles":Backend::ALL.iter().map(|backend|json!({"profile":backend.name(),
                        "directory":backend.artifact_directory(),"description":backend.description(),"experimental":true})).collect::<Vec<_>>(),
                    "compatibilityProfiles":CompatibilityProfile::ALL.iter().map(|profile|profile.name()).collect::<Vec<_>>()
                }),
            )?;
            writeln!(output)?;
        }
        OutputFormat::Text => {
            for backend in Backend::ALL {
                writeln!(
                    output,
                    "{}\t{}\t{}",
                    backend.name(),
                    backend.artifact_directory(),
                    backend.description()
                )?;
            }
        }
    }
    output.flush()?;
    Ok(0)
}

/// List the application targets compiled into this binary: the command that
/// generates each one, the mapping profile it accepts, the surface manifest it
/// records and the stable owner of its output root. Discovery only - no source
/// IO, and no relation to the native SDK backend inventory.
///
/// # Errors
/// Propagates output failures.
pub fn list_applications(format: OutputFormat) -> anyhow::Result<i32> {
    let mut output = io::stdout().lock();
    let targets = [Target::Cli, Target::Mcp];
    match format {
        OutputFormat::Json | OutputFormat::Sarif => {
            serde_json::to_writer_pretty(
                &mut output,
                &json!({"format":"suspect.application.profiles.v1",
                    "profiles":targets.iter().map(|target|json!({"command":target.command(),
                        "profile":target.profile(),
                        "surfaceFormat":target.surface_format(),"owner":target.owner(),
                        "requires":["--mapping","--target-config"],"experimental":true})).collect::<Vec<_>>()}),
            )?;
            writeln!(output)?;
        }
        OutputFormat::Text => {
            for target in targets {
                writeln!(
                    output,
                    "{}\t{}\t{}\t{}",
                    target.command(),
                    target.profile(),
                    target.surface_format(),
                    target.owner()
                )?;
            }
        }
    }
    output.flush()?;
    Ok(0)
}
