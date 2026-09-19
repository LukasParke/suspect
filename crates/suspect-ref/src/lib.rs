#![deny(missing_docs)]
//! suspect-ref: the `$ref` resolution engine — workspace graph, JSON
//! pointers, cycle analysis, memoized resolution.
//!
//! Layering: [`Workspace`] owns loaded documents ([`suspect_low::LowDoc`])
//! keyed by canonical [`suspect_source::Uri`]. [`DocHandle`] exposes per-document `$ref`
//! edges, memoized pointer/edge resolution (following reference chains to a
//! node, a whole document, or a detected cycle), and a per-document cycle
//! census that classifies loops as legal schema recursion or illegal.

pub mod acquire;
mod cycles;
mod edges;
mod error;
mod provider;
mod resolve;
pub mod resource_uri;
mod workspace;

pub use cycles::{Cycle, CycleKind, CycleReport};
pub use edges::{ParsedRef, RefDiagnostic, RefEdge, parse_ref};
pub use error::{RefError, WorkspaceError};
pub use provider::{
    DocumentMetadata, DocumentProvider, ProvidedDocument, ProviderError, sha256_digest,
};
pub use resolve::{ReferenceInput, ReferenceTarget, Resolution, Step};
pub use workspace::{DocHandle, Workspace, WorkspaceBuilder, WorkspaceStats};

/// Index of a document inside a [`Workspace`].
pub use workspace::DocId;
