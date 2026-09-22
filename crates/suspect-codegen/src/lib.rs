//! Canonical OpenAPI SDK generation through a twelve-language native registry.
//! Source-addressed contracts drive native models, checked
//! codecs, HTTP clients, documentation and compatibility reports.

pub mod admission;
#[cfg(feature = "http-protocol")]
pub mod api_cli;
#[cfg(feature = "http-protocol")]
pub mod application;
pub mod attribution;
pub mod backend;
pub mod compatibility;
pub mod credential_env;
pub mod generation_session;
pub mod sdk_defaults;
pub mod toolchain;
#[cfg(feature = "java-sdk")]
#[rustfmt::skip]
pub mod java_sdk;
#[cfg(feature = "csharp-sdk")]
#[rustfmt::skip]
pub mod csharp_sdk;
#[cfg(feature = "kotlin-sdk")]
#[rustfmt::skip]
pub mod kotlin_sdk;
#[cfg(feature = "ruby-sdk")]
#[rustfmt::skip]
pub mod ruby_sdk;
#[cfg(feature = "php-sdk")]
#[rustfmt::skip]
pub mod php_sdk;
#[cfg(feature = "dart-sdk")]
#[rustfmt::skip]
pub mod dart_sdk;
#[cfg(feature = "cpp-sdk")]
#[rustfmt::skip]
pub mod cpp_sdk;
pub mod examples;
pub mod features;
pub mod go_codecs;
pub mod go_http;
pub mod go_json;
pub mod go_models;
pub mod go_validation;
pub(crate) mod http_contract;
mod http_examples;
#[cfg(feature = "http-protocol")]
pub mod http_protocol;
#[cfg(feature = "http-protocol")]
pub mod mcp;
mod model_naming;
pub mod python_codecs;
pub mod python_http;
pub mod python_json;
pub mod python_models;
pub mod python_validation;
pub mod rust_codecs;
pub mod rust_http;
pub mod rust_models;
pub mod rust_validation;
pub mod schema_view;
pub mod swift_sdk;
#[cfg(feature = "http-protocol")]
pub mod terraform;
pub mod typescript;

use std::path::Path;

pub use suspect_artifact::{Adoption, OwnershipChangeKind, OwnershipReport};
/// One generated file.
#[derive(Debug, Clone, PartialEq)]
pub struct OutFile {
    /// Relative path within the output directory.
    pub path: String,
    /// Full file content.
    pub content: String,
}

/// Ownership-aware drift check: outputs and versioned metadata must both match.
#[must_use]
pub fn matches_disk(files: &[OutFile], root: &Path) -> bool {
    check_files(files, root).is_ok_and(|report| report.is_current())
}

/// Detailed drift for the default compiler owner, without filesystem writes.
///
/// # Errors
/// Invalid paths or ownership metadata, symlinks, and filesystem failures.
pub fn check_files(files: &[OutFile], root: &Path) -> Result<OwnershipReport, String> {
    check_files_with_owner(files, root, "suspect-codegen")
}

/// Detailed drift for one caller-selected stable logical generation owner.
///
/// # Errors
/// Invalid paths or ownership metadata, symlinks, and filesystem failures.
pub fn check_files_with_owner(
    files: &[OutFile],
    root: &Path,
    owner: &str,
) -> Result<OwnershipReport, String> {
    owned_files(files, root, owner, Adoption::Refuse).map(|batch| batch.report().clone())
}

/// Writes the compiler's complete desired file set with strict ownership.
/// Unowned and user-edited files are preserved as conflicts. Unchanged owned
/// files retain metadata; byte-identical obsolete owned files are removed.
///
/// # Errors
/// Propagates filesystem failures.
pub fn write_files(files: &[OutFile], root: &Path) -> Result<(), String> {
    write_files_with_owner(files, root, "suspect-codegen", Adoption::Refuse)
}

/// Write one stable owner's complete output set, optionally adopting identical
/// preexisting output explicitly. Adoption never takes files from another owner.
///
/// # Errors
/// Ownership conflicts, unsafe paths, malformed metadata and I/O failures.
/// Replacements are atomic per file; the whole batch is not a transaction.
pub fn write_files_with_owner(
    files: &[OutFile],
    root: &Path,
    owner: &str,
    adoption: Adoption,
) -> Result<(), String> {
    owned_files(files, root, owner, adoption)?
        .commit()
        .map_err(|error| error.to_string())
}

fn owned_files<'a>(
    files: &'a [OutFile],
    root: &Path,
    owner: &str,
    adoption: Adoption,
) -> Result<suspect_artifact::OwnedBatch<'a>, String> {
    suspect_artifact::ArtifactBatch::prepare(
        root,
        files.iter().map(|file| suspect_artifact::Artifact {
            path: Path::new(&file.path),
            content: file.content.as_bytes(),
        }),
    )
    .and_then(|batch| batch.with_ownership(owner, adoption))
    .map_err(|error| error.to_string())
}
