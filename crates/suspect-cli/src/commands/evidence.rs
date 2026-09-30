//! `suspect evidence`: the SDK verification matrix as data.
//!
//! The per-backend acceptance suites are opt-in (`#[ignore]`) because they
//! need a native toolchain, which means "which backend is verified against
//! which toolchain on which date" is normally a matter of inference. This
//! command prints the declared coverage — backend × feature → evidence
//! file, whether that file exists, whether it is claimed or deliberately
//! unclaimed — so CI can publish it and a reviewer can read it.
//!
//! It is deliberately a *declaration* report, not a test result: it says
//! what evidence is wired up. The scheduled native job pairs it with actual
//! run results; see `.github/workflows/native-evidence.yml`.

use std::path::Path;

use clap::Args;
use serde::Serialize;
use suspect_codegen::features::{BACKENDS, FEATURES, claim_for, is_supported};

use crate::OutputFormat;

/// One backend/feature coverage row.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// Backend id.
    pub backend: String,
    /// Feature id.
    pub feature: String,
    /// Feature title.
    pub title: String,
    /// Whether the feature is claimed for this backend.
    pub claimed: bool,
    /// The acceptance test that proves it, when claimed.
    pub evidence: Option<String>,
    /// Whether that evidence file exists in the tree.
    pub evidence_present: bool,
    /// When a behavioral fragment also covers it.
    pub fragment: bool,
}

/// The whole matrix.
#[derive(Debug, Serialize)]
pub struct Matrix {
    /// Format identifier.
    pub format: String,
    /// Backends in registry order.
    pub backends: Vec<String>,
    /// Rows: one per backend/feature pair that is claimed.
    pub claimed: Vec<Row>,
    /// Feature/backend pairs deliberately unclaimed, with no evidence.
    pub unclaimed: Vec<Row>,
    /// Evidence files referenced by a claim but missing from the tree.
    pub missing_evidence: Vec<String>,
}

/// Format identifier.
const FORMAT: &str = "suspect.sdk-evidence.v1";

/// One evidence review.
#[derive(Debug, Args)]
pub struct EvidenceArgs {
    /// Path to the behavioral fragment directory.
    #[arg(long, default_value = "crates/suspect-codegen/tests/behavioral")]
    pub fragments: String,
    /// Output format for the matrix.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

/// Builds the matrix from the feature manifest and the tree.
///
/// # Errors
/// IO failures reading the fragment directory.
pub fn evidence(args: &EvidenceArgs) -> anyhow::Result<i32> {
    let matrix = build_matrix(Path::new(&args.fragments))?;

    match args.format {
        OutputFormat::Json | OutputFormat::Sarif => {
            println!("{}", serde_json::to_string_pretty(&matrix)?);
        }
        OutputFormat::Text => {
            println!(
                "{}  ({} backends, {} claims, {} unclaimed pairs)",
                matrix.format,
                matrix.backends.len(),
                matrix.claimed.len(),
                matrix.unclaimed.len()
            );
            println!();
            let width = matrix
                .backends
                .iter()
                .map(String::len)
                .max()
                .unwrap_or(4)
                .max(4);
            for row in &matrix.claimed {
                let present = if row.evidence_present { "ok " } else { "MISS" };
                let fragment = if row.fragment { " +fragment" } else { "" };
                println!(
                    "{present}  {:<width$}  {:<26}  {}{fragment}",
                    row.backend,
                    row.feature,
                    row.evidence.as_deref().unwrap_or("-"),
                    width = width
                );
            }
            if !matrix.missing_evidence.is_empty() {
                println!();
                println!("missing evidence files:");
                for path in &matrix.missing_evidence {
                    println!("  {path}");
                }
            }
        }
    }

    // A claim whose evidence file is gone is a broken promise.
    Ok(i32::from(!matrix.missing_evidence.is_empty()))
}

/// Builds the matrix.
fn build_matrix(fragments: &Path) -> anyhow::Result<Matrix> {
    let covered = fragment_claims(fragments)?;
    let mut claimed = Vec::new();
    let mut unclaimed = Vec::new();
    let mut missing_evidence = Vec::new();

    for (feature_index, feature) in FEATURES.iter().enumerate() {
        for (backend_index, backend) in BACKENDS.iter().enumerate() {
            let evidence = claim_for(feature_index, backend_index);
            let row = Row {
                backend: (*backend).to_owned(),
                feature: feature.id.to_owned(),
                title: feature.title.to_owned(),
                claimed: is_supported(feature_index, backend_index),
                evidence: evidence.map(str::to_owned),
                evidence_present: evidence.is_some_and(|file| {
                    Path::new("crates/suspect-codegen/tests")
                        .join(file)
                        .is_file()
                }),
                fragment: covered.contains(&(feature.id.to_owned(), (*backend).to_owned())),
            };
            if row.claimed {
                if !row.evidence_present {
                    missing_evidence.push(format!(
                        "crates/suspect-codegen/tests/{}",
                        row.evidence.as_deref().unwrap_or("?")
                    ));
                }
                claimed.push(row);
            } else {
                unclaimed.push(row);
            }
        }
    }

    Ok(Matrix {
        format: FORMAT.to_owned(),
        backends: BACKENDS.iter().map(|b| (*b).to_owned()).collect(),
        claimed,
        unclaimed,
        missing_evidence,
    })
}

/// Which (feature, backend) pairs a behavioral fragment already covers.
fn fragment_claims(
    fragments: &Path,
) -> anyhow::Result<std::collections::BTreeSet<(String, String)>> {
    let mut covered = std::collections::BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(fragments) else {
        return Ok(covered);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let doc = suspect_low::LowDoc::parse(
            suspect_source::Uri::from_path(&path).unwrap_or_else(|_| {
                suspect_source::Uri::parse("mem://fragment.yaml").expect("mem uri")
            }),
            suspect_source::Source::from_vec(text.into_bytes()),
        );
        let Some(feature) = doc.root().get("x-suspect-feature").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(behaviors) = doc.root().get("x-suspect-behavior") {
            for entry in behaviors.entries() {
                covered.insert((feature.to_owned(), entry.key.to_owned()));
            }
        }
    }
    Ok(covered)
}
