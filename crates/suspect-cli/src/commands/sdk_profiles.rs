//! Editor/automation discovery from the canonical backend registry.

use std::io::{self, Write};

use serde_json::json;
use suspect_codegen::backend::Backend;
use suspect_codegen::http_protocol::CompatibilityProfile;

use crate::OutputFormat;

/// List exactly the profiles compiled into this binary, without source IO.
///
/// # Errors
/// Propagates output failures.
pub fn list(format: OutputFormat) -> anyhow::Result<i32> {
    let mut output = io::stdout().lock();
    match format {
        OutputFormat::Json => {
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
