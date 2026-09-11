#![deny(missing_docs)]
use std::sync::Arc;

use suspect_low::{NodeRef, SpecFamily};
use suspect_ref::{Workspace, WorkspaceError};

/// OpenAPI 3.x version of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OasVersion {
    /// OpenAPI 3.0.x (JSON-Schema-Subset dialect, `nullable` keyword).
    V30,
    /// OpenAPI 3.1.x (JSON Schema 2020-12 dialect).
    V31,
    /// OpenAPI 3.2.x (JSON Schema 2020-12 dialect plus tag `kind`/`parent`
    /// and `info.summary`).
    V32,
}

impl OasVersion {
    /// Sniffs an OpenAPI 3.x version from a parsed document.
    #[must_use]
    pub fn sniff(doc: &suspect_low::LowDoc) -> Option<OasVersion> {
        match doc.sniff_family() {
            SpecFamily::Oas30 => Some(OasVersion::V30),
            SpecFamily::Oas31 => Some(OasVersion::V31),
            SpecFamily::Oas32 => Some(OasVersion::V32),
            _ => None,
        }
    }

    /// True for 3.1+ semantics (JSON Schema 2020-12 style schemas).
    #[must_use]
    pub const fn is_31_plus(self) -> bool {
        matches!(self, OasVersion::V31 | OasVersion::V32)
    }
}

/// Model construction errors.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The entry document sniffs as some other specification family
    /// (AsyncAPI, Overlay, Arazzo, ...), not OpenAPI 3.x.
    #[error("document is not OpenAPI 3.x (family: {family:?})")]
    NotOpenApi {
        /// The detected [`SpecFamily`] of the offending entry document.
        family: SpecFamily,
    },
    /// A `$ref` chain cycled while building the model. Views degrade to
    /// their raw form via [`CycleGuard`]; this error is raised only where a
    /// cycle cannot be degraded away.
    #[error("ref chain cycled while building the model")]
    Cycle,
    /// The underlying workspace failed to load or resolve a document.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

impl From<suspect_low::SpecFamily> for ModelError {
    fn from(family: suspect_low::SpecFamily) -> Self {
        ModelError::NotOpenApi { family }
    }
}

/// Marker returned when a `$ref` chain cycles; views degrade to their raw
/// (unresolved) form instead of looping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleGuard;

/// Entry point for building typed views over a workspace.
///
/// Immutable: the workspace resolves and loads documents through interior
/// mutability, so views only ever need `&Session`, and every [`NodeRef`]
/// they hand out is valid for the session borrow.
pub struct Session {
    ws: Arc<Workspace>,
    pub(crate) reference_registry: std::sync::Mutex<crate::scoped_refs::ReferenceRegistry>,
}

impl Session {
    /// Starts a session over a shared workspace.
    #[must_use]
    pub fn new(ws: Arc<Workspace>) -> Self {
        Self {
            ws,
            reference_registry: std::sync::Mutex::new(Default::default()),
        }
    }

    /// Loads an entry document (plus its external-`$ref` closure) and
    /// returns the typed root view.
    ///
    /// # Errors
    /// Workspace load errors; the entry not being an OpenAPI 3.x document.
    pub fn load(&self, entry: &str) -> Result<OpenApi<'_>, ModelError> {
        self.ws.load_all(entry).map_err(ModelError::Workspace)?;
        self.open(entry)
    }

    /// Opens only the OpenAPI entry document. Typed traversal follows
    /// references as needed; arbitrary `$ref` keys in example/default data
    /// cannot cause eager document loading through this entry point.
    ///
    /// # Errors
    /// Workspace entry load errors or a non-OpenAPI 3.x entry document.
    pub fn open(&self, entry: &str) -> Result<OpenApi<'_>, ModelError> {
        let handle = self.ws.open(entry).map_err(ModelError::Workspace)?;
        let version = OasVersion::sniff(handle.doc()).ok_or(ModelError::NotOpenApi {
            family: handle.doc().sniff_family(),
        })?;
        Ok(OpenApi::new(
            self,
            version,
            handle.id(),
            handle.doc().root(),
        ))
    }

    /// Resolves through the workspace's canonical decoder and iterative
    /// reference chain, preserving document identity across every hop.
    pub(crate) fn resolve<'s>(&'s self, ref_value: NodeRef<'_>) -> Result<NodeRef<'s>, CycleGuard> {
        self.resolve_scoped(ref_value, false)
    }

    /// Follows exactly one reference hop for source-preserving traversals.
    pub(crate) fn resolve_target<'s>(
        &'s self,
        ref_value: NodeRef<'_>,
    ) -> Result<NodeRef<'s>, CycleGuard> {
        let target = self.reference_target(ref_value).map_err(|_| CycleGuard)?;
        self.ws
            .get_by_id(target.doc)
            .ok_or(CycleGuard)?
            .node_at_pointer(&target.pointer)
            .map_err(|_| CycleGuard)
    }

    /// The workspace this session resolves and loads documents through.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.ws
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("docs", &self.ws.len())
            .finish()
    }
}

/// A typed OpenAPI 3.x document view.
#[derive(Debug)]
pub struct OpenApi<'s> {
    pub(crate) session: &'s Session,
    pub(crate) version: OasVersion,
    #[allow(dead_code)] // exposed via doc_id()
    pub(crate) doc: suspect_ref::DocId,
    pub(crate) root: NodeRef<'s>,
}

impl<'s> OpenApi<'s> {
    pub(crate) fn new(
        session: &'s Session,
        version: OasVersion,
        doc: suspect_ref::DocId,
        root: NodeRef<'s>,
    ) -> Self {
        Self {
            session,
            version,
            doc,
            root,
        }
    }

    /// Family of this document (diagnostics).
    #[must_use]
    pub fn family(&self) -> SpecFamily {
        SpecFamily::Unknown
    }
}
