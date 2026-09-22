//! Pinned Go module metadata for the single emitted application module.
//!
//! Cobra is an application-only dependency, pinned to a registry-verified
//! release together with the exact indirect requirements and module sums its
//! own `go.mod` selects. The embedded SDK is a package of this same module, so
//! it is never a requirement and never carries a nested module or a sum.
use super::{COBRA_VERSION, CliPlan};
use crate::OutFile;

pub(super) fn artifacts(plan: &CliPlan) -> [OutFile; 2] {
    let config = &plan.config;
    [
        OutFile {
            path: "go.mod".into(),
            content: format!(
                "module {}\n\ngo {}\n\ntoolchain {}\n\nrequire github.com/spf13/cobra v{COBRA_VERSION}\n\n{}",
                config.module_path,
                config.go_version,
                config.go_toolchain,
                include_str!("dependencies.mod"),
            ),
        },
        OutFile {
            path: "go.sum".into(),
            content: include_str!("dependencies.sum").into(),
        },
    ]
}
