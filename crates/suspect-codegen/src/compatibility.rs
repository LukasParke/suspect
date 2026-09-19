//! Source-aware SDK compatibility over canonical contracts and native plans.
//!
//! Wire compatibility means that old requests remain accepted and new responses
//! satisfy the old contract. Native compatibility concerns upgrading a generated
//! package at existing call sites. Neither implies the other. Inclusion proofs
//! deliberately cover a small fragment; unsupported proofs and failed native
//! planning are findings, never a successful compatibility guarantee.

use std::{collections::BTreeSet, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use suspect_ir::contract::{Contract, SourceId};

use crate::backend::{Backend, GenerationOptions, TargetConfig};

#[cfg(feature = "cpp-sdk")]
#[path = "compatibility/cpp.rs"]
mod cpp;

#[cfg(feature = "csharp-sdk")]
#[path = "compatibility/csharp.rs"]
mod csharp;
#[cfg(feature = "kotlin-sdk")]
#[path = "compatibility/kotlin.rs"]
mod kotlin;
#[path = "compatibility/native.rs"]
mod native;
#[path = "compatibility/native_models.rs"]
mod native_models;
#[cfg(feature = "php-sdk")]
#[path = "compatibility/php.rs"]
mod php;
#[path = "compatibility/provenance.rs"]
mod provenance;
#[cfg(feature = "ruby-sdk")]
#[path = "compatibility/ruby.rs"]
mod ruby;
#[path = "compatibility/schema.rs"]
mod schema;
#[path = "compatibility/swift_models.rs"]
mod swift_models;
#[path = "compatibility/wire.rs"]
mod wire;

#[cfg(feature = "dart-sdk")]
#[path = "compatibility/dart.rs"]
mod dart;

#[cfg(feature = "java-sdk")]
#[path = "compatibility/java.rs"]
mod java;

pub use native::{
    NativeModel, NativeOperation, NativeSnapshot, PlanFinding, PlanStatus, RuntimeProvenance,
};

pub const REPORT_FORMAT: &str = "suspect-sdk-compatibility-v1";

/// `Compatible` is a proof within the explicitly recorded scope, not a promise
/// about arbitrary server implementations or all JSON Schema vocabularies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Impact {
    Compatible,
    Breaking,
    PotentiallyBreaking,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Request,
    Response,
}

/// An original source location. Spans are metadata, never matching keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub document: String,
    pub pointer: String,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl Location {
    pub(crate) fn at(contract: &Contract, source: &SourceId) -> Self {
        Self {
            document: source.document().to_string(),
            pointer: source.pointer().to_owned(),
            span: contract.source_span(source).map(|span| SourceSpan {
                start: span.start,
                end: span.end,
            }),
        }
    }

    pub(crate) fn identifies(&self, source: &SourceId) -> bool {
        self.document == source.document().as_str() && self.pointer == source.pointer()
    }

    pub(crate) fn same_address(&self, other: &Self) -> bool {
        self.document == other.document && self.pointer == other.pointer
    }
}

/// Before/after evidence is structured contract data or a native-plan
/// descriptor. It is never a declaration extracted from emitted source text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub code: String,
    pub impact: Impact,
    pub subject: String,
    pub operation_id_before: Option<String>,
    pub operation_id_after: Option<String>,
    pub direction: Option<Direction>,
    pub source_before: Option<Location>,
    pub source_after: Option<Location>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub message: String,
    pub migration: String,
    /// Why a proof did not succeed, or the sufficient rule that did prove it.
    pub reasoning: Vec<String>,
    /// Fine-grained source changes, including changes behind external `$ref`s.
    pub schema_deltas: Vec<SchemaDelta>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaDelta {
    pub keyword: String,
    pub source_before: Option<Location>,
    pub source_after: Option<Location>,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub compatible_changes: usize,
    pub breaking_changes: usize,
    pub potentially_breaking_changes: usize,
    pub unknowns: usize,
}

impl Summary {
    fn from_changes<'a>(changes: impl Iterator<Item = &'a Change>) -> Self {
        let mut summary = Self::default();
        for change in changes {
            match change.impact {
                Impact::Compatible => summary.compatible_changes += 1,
                Impact::Breaking => summary.breaking_changes += 1,
                Impact::PotentiallyBreaking => summary.potentially_breaking_changes += 1,
                Impact::Unknown => summary.unknowns += 1,
            }
        }
        summary
    }

    /// In particular, an unknown result is never a green compatibility result.
    #[must_use]
    pub fn is_proven_compatible(&self) -> bool {
        self.breaking_changes == 0 && self.potentially_breaking_changes == 0 && self.unknowns == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFingerprint {
    pub document: String,
    /// SHA-256 of owned normalized JSON, not of original formatting bytes.
    pub normalized_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub entry: String,
    pub openapi_version: String,
    pub generator: String,
    pub generator_version: String,
    pub documents: Vec<DocumentFingerprint>,
    pub selected_operations: Vec<String>,
    /// Versioned interpretation is part of the captured semantic configuration.
    #[serde(default)]
    pub generation: GenerationOptions,
}

/// A reusable in-memory snapshot. Capturing does no input IO, package writes,
/// native toolchain execution or publishing. Native descriptors are serializable;
/// the canonical contract is kept owned for subsequent wire comparison.
#[derive(Debug)]
pub struct CompatibilitySnapshot {
    contract: Arc<Contract>,
    selected: Vec<SourceId>,
    pub metadata: SnapshotMetadata,
    pub native: Vec<NativeSnapshot>,
}

impl CompatibilitySnapshot {
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeReport {
    pub backend: Backend,
    pub before: Option<NativeSnapshot>,
    pub after: Option<NativeSnapshot>,
    pub changes: Vec<Change>,
    pub summary: Summary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub format: String,
    pub before: SnapshotMetadata,
    pub after: SnapshotMetadata,
    pub wire: Vec<Change>,
    pub native: Vec<NativeReport>,
    pub summary: Summary,
    pub scope: Vec<String>,
}

impl CompatibilityReport {
    #[must_use]
    pub fn is_proven_compatible(&self) -> bool {
        self.summary.is_proven_compatible()
    }

    /// A compact migration note, with both original source addresses. JSON
    /// serialization of the report retains complete descriptor/delta evidence.
    #[must_use]
    pub fn migration_notes(&self) -> String {
        fn prose(value: &str) -> String {
            value
                .chars()
                .flat_map(|c| match c {
                    '\n' | '\r' => " ".chars().collect::<Vec<_>>(),
                    '`' | '<' | '>' | '&' => format!("&#{};", c as u32).chars().collect(),
                    c if c.is_control() => c.escape_default().collect(),
                    c => vec![c],
                })
                .collect()
        }
        let mut text = format!(
            "# SDK compatibility\n\nGenerator: {} {}.\n\n{} breaking; {} potentially breaking; {} unknown.\n\n",
            prose(&self.after.generator),
            prose(&self.after.generator_version),
            self.summary.breaking_changes,
            self.summary.potentially_breaking_changes,
            self.summary.unknowns,
        );
        let sections = std::iter::once(("Wire contract", self.wire.as_slice())).chain(
            self.native
                .iter()
                .map(|target| (target.backend.name(), target.changes.as_slice())),
        );
        for (name, changes) in sections {
            text.push_str(&format!("## {}\n\n", prose(name)));
            if changes.is_empty() {
                text.push_str("No differences found within the recorded scope.\n\n");
            }
            for change in changes {
                text.push_str(&format!(
                    "- **{:?}** — {}: {} {}\n",
                    change.impact,
                    prose(&change.subject),
                    prose(&change.message),
                    prose(&change.migration)
                ));
                for (label, source) in [
                    ("Before", &change.source_before),
                    ("After", &change.source_after),
                ] {
                    if let Some(source) = source {
                        text.push_str(&format!(
                            "  - {label}: `{}#{}`\n",
                            prose(&source.document),
                            prose(&source.pointer)
                        ));
                    }
                }
                for reason in &change.reasoning {
                    text.push_str(&format!("  - {}\n", prose(reason)));
                }
            }
            text.push('\n');
        }
        text
    }
}

/// Invalid comparison configuration. Contract/planner limitations are instead
/// retained as unknown findings so removals and other usable deltas survive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityError(pub String);

impl std::fmt::Display for CompatibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CompatibilityError {}

/// Capture one selected SDK surface. An empty selection means all outgoing
/// operations. Use [`compare`] when selected IDs may have been removed/renamed.
pub fn snapshot(
    contract: Arc<Contract>,
    operation_ids: &[String],
    targets: &[TargetConfig],
) -> Result<CompatibilitySnapshot, CompatibilityError> {
    snapshot_with_options(
        contract,
        operation_ids,
        targets,
        &GenerationOptions::default(),
    )
}

/// Capture the same interpretation used by canonical SDK generation and sessions.
pub fn snapshot_with_options(
    contract: Arc<Contract>,
    operation_ids: &[String],
    targets: &[TargetConfig],
    generation: &GenerationOptions,
) -> Result<CompatibilitySnapshot, CompatibilityError> {
    let selected = wire::select_one(&contract, operation_ids)?;
    capture(contract, selected, targets, generation)
}

/// Compare two canonical inputs under identical target/package configuration.
/// Selection is resolved jointly: an ID missing on one side is a removal or
/// addition, and a unique unchanged method/path can identify an ID rename.
pub fn compare(
    old: Arc<Contract>,
    new: Arc<Contract>,
    operation_ids: &[String],
    targets: &[TargetConfig],
) -> Result<CompatibilityReport, CompatibilityError> {
    compare_with_targets(old, new, operation_ids, targets, targets)
}

/// Also compare package names, import identities and target additions/removals.
/// Version-only changes are recorded without inventing a source break.
pub fn compare_with_targets(
    old: Arc<Contract>,
    new: Arc<Contract>,
    operation_ids: &[String],
    old_targets: &[TargetConfig],
    new_targets: &[TargetConfig],
) -> Result<CompatibilityReport, CompatibilityError> {
    compare_with_options(
        old,
        new,
        operation_ids,
        (old_targets, &GenerationOptions::default()),
        (new_targets, &GenerationOptions::default()),
    )
}

/// Resolve selection jointly while retaining each side's target and explicit
/// interpretation choices. A profile change is an unknown until independently
/// proved; it is never hidden by identical source bytes or native signatures.
pub fn compare_with_options(
    old: Arc<Contract>,
    new: Arc<Contract>,
    operation_ids: &[String],
    old_configuration: (&[TargetConfig], &GenerationOptions),
    new_configuration: (&[TargetConfig], &GenerationOptions),
) -> Result<CompatibilityReport, CompatibilityError> {
    let (old_selected, new_selected) = wire::select_pair(&old, &new, operation_ids)?;
    let old = capture(old, old_selected, old_configuration.0, old_configuration.1)?;
    let new = capture(new, new_selected, new_configuration.0, new_configuration.1)?;
    Ok(compare_snapshots(&old, &new))
}

/// Compare retained plans without replanning. A different operation selection
/// is a real change to the generated surface and is reported accordingly.
#[must_use]
pub fn compare_snapshots(
    old: &CompatibilitySnapshot,
    new: &CompatibilitySnapshot,
) -> CompatibilityReport {
    let mut wire = wire::compare(&old.contract, &new.contract, &old.selected, &new.selected);
    if old.metadata.generation.compatibility_profiles
        != new.metadata.generation.compatibility_profiles
    {
        let mut finding = change(
            "wire-interpretation-profile-changed",
            Impact::Unknown,
            "source interpretation",
            "Explicit interpretation profiles differ; equal OpenAPI declarations do not establish equivalent wire values.",
        );
        finding.before = Some(serde_json::json!(old.metadata.generation));
        finding.after = Some(serde_json::json!(new.metadata.generation));
        finding.migration = "Review the versioned profile contract and verify the affected request/response bytes before accepting this transition.".into();
        wire.changes.push(finding);
    }
    let native = native::compare(&old.native, &new.native, &wire.matches, &wire.schemas);
    let summary = Summary::from_changes(
        wire.changes
            .iter()
            .chain(native.iter().flat_map(|target| &target.changes)),
    );
    CompatibilityReport {
        format: REPORT_FORMAT.into(),
        before: old.metadata.clone(),
        after: new.metadata.clone(),
        wire: wire.changes,
        native,
        summary,
        scope: vec![
            "Outgoing operations in the recorded selection and their resolved schema closure.".into(),
            "Wire variance: old requests must fit new inputs; new responses must fit old outputs.".into(),
            "Static reference-aware structural equality and conservative sufficient inclusion rules; no general JSON Schema equivalence claim.".into(),
            "Native descriptors come from the selected language plans and recorded interpretation options; incomplete descriptors and planner refusals remain unknown.".into(),
            "Runtime implementation, ABI, dependency resolution, server behavior and performance are not established by native descriptor equality.".into(),
        ],
    }
}

fn capture(
    contract: Arc<Contract>,
    selected: Vec<SourceId>,
    targets: &[TargetConfig],
    generation: &GenerationOptions,
) -> Result<CompatibilitySnapshot, CompatibilityError> {
    let unique: BTreeSet<_> = targets.iter().map(|target| target.backend).collect();
    if unique.len() != targets.len() {
        return Err(CompatibilityError(
            "select each compatibility backend only once".into(),
        ));
    }
    let mut targets = targets.to_vec();
    targets.sort_by_key(|target| target.backend);
    let native = targets
        .iter()
        .map(|target| {
            if let Some(message) = crate::backend::configuration_error(target) {
                let mut snapshot = native::capture(contract.clone(), &[], target, generation);
                snapshot.status = PlanStatus::Unavailable;
                snapshot.findings.push(PlanFinding {
                    code: "sdk-package".into(),
                    source: None,
                    message: message.into(),
                });
                snapshot
            } else {
                native::capture(contract.clone(), &selected, target, generation)
            }
        })
        .collect();
    let metadata = SnapshotMetadata {
        generation: generation.clone(),
        entry: contract.entry().to_string(),
        openapi_version: contract.openapi_version().into(),
        generator: "suspect-codegen".into(),
        generator_version: env!("CARGO_PKG_VERSION").into(),
        documents: contract
            .documents()
            .map(|(uri, value)| DocumentFingerprint {
                document: uri.to_string(),
                normalized_sha256: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(value).expect("contract JSON"))
                ),
            })
            .collect(),
        selected_operations: contract
            .operations()
            .filter(|op| selected.contains(op.source()))
            .map(|op| {
                op.operation_id().map(str::to_owned).unwrap_or_else(|| {
                    format!(
                        "{} {}",
                        op.method().as_str(),
                        op.path_template().unwrap_or("")
                    )
                })
            })
            .collect(),
    };
    Ok(CompatibilitySnapshot {
        contract,
        selected,
        metadata,
        native,
    })
}

pub(crate) fn change(
    code: &str,
    impact: Impact,
    subject: impl Into<String>,
    message: impl Into<String>,
) -> Change {
    Change {
        code: code.into(),
        impact,
        subject: subject.into(),
        operation_id_before: None,
        operation_id_after: None,
        direction: None,
        source_before: None,
        source_after: None,
        before: None,
        after: None,
        message: message.into(),
        migration: String::new(),
        reasoning: Vec::new(),
        schema_deltas: Vec::new(),
    }
}
