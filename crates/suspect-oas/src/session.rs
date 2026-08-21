use std::sync::Arc;

use suspect_low::{NodeRef, SpecFamily};
use suspect_ref::{Resolution, Workspace, WorkspaceError};



/// OpenAPI 3.x version of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OasVersion {
    V30,
    V31,
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
    #[error("document is not OpenAPI 3.x (family: {family:?})")]
    NotOpenApi { family: SpecFamily },
    #[error("ref chain cycled while building the model")]
    Cycle,
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
}

impl Session {
    /// Starts a session over a shared workspace.
    #[must_use]
    pub fn new(ws: Arc<Workspace>) -> Self {
        Self { ws }
    }

    /// Loads an entry document (plus its external-`$ref` closure) and
    /// returns the typed root view.
    ///
    /// # Errors
    /// Workspace load errors; the entry not being an OpenAPI 3.x document.
    pub fn load(&self, entry: &str) -> Result<OpenApi<'_>, ModelError> {
        self.ws.load_all(entry).map_err(ModelError::Workspace)?;
        let handle = self.ws.open(entry).map_err(ModelError::Workspace)?;
        let version = OasVersion::sniff(handle.doc()).ok_or(ModelError::NotOpenApi {
            family: handle.doc().sniff_family(),
        })?;
        Ok(OpenApi::new(self, version, handle.id(), handle.doc().root()))
    }

    /// Resolves a `$ref` value node to its target node.
    pub(crate) fn resolve<'s>(&'s self, ref_value: NodeRef<'_>) -> Result<NodeRef<'s>, CycleGuard> {
        let uri = ref_value.syntax().doc().uri().clone();
        let range = ref_value.syntax().byte_range();
        let handle = self.ws.get(&uri).ok_or(CycleGuard)?;
        // NodeRef is invariant in its lifetime, so re-derive the same node
        // from the workspace-borrowed document to get session-lifetime output
        let mut raw = handle
            .doc()
            .inner()
            .root()
            .raw()
            .descendant_for_byte_range(range.start, range.end.saturating_sub(1))
            .ok_or(CycleGuard)?;
        while raw.byte_range() != range {
            raw = raw.parent().ok_or(CycleGuard)?;
        }
        let node = NodeRef::new(suspect_syntax::SNode::new(handle.doc().inner(), raw));
        match handle.resolve_ref_value(node) {
            Ok(Resolution::Node(target)) => Ok(target),
            _ => Err(CycleGuard),
        }
    }

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
        Self { session, version, doc, root }
    }

    /// Family of this document (diagnostics).
    #[must_use]
    pub fn family(&self) -> SpecFamily {
        SpecFamily::Unknown
    }
}
