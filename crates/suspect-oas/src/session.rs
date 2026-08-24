#![deny(missing_docs)]
use std::sync::Arc;

use suspect_low::{NodeRef, Pointer, SpecFamily};
use suspect_ref::{Resolution, Workspace, WorkspaceError};

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
        Ok(OpenApi::new(
            self,
            version,
            handle.id(),
            handle.doc().root(),
        ))
    }

    /// Resolves a `$ref` value node to its target node.
    ///
    /// Hot path: local (`#/a/b`) and same-workspace external pointers
    /// navigate the session-owned tree by KEY (`root.get("components")
    /// .get("schemas")...`), which is O(pointer depth) with no tree-cursor
    /// work over the multi-megabyte document. Only anchors and exotic refs
    /// fall back to the original byte-range derivation.
    pub(crate) fn resolve<'s>(&'s self, ref_value: NodeRef<'_>) -> Result<NodeRef<'s>, CycleGuard> {
        use suspect_low::{Pointer, ValueKind};
        if std::env::var_os("SUSPECT_TRACE").is_some() {
            eprintln!(
                "[trace] resolve enter range={:?}",
                ref_value.syntax().byte_range()
            );
        }
        let uri = ref_value.syntax().doc().uri().clone();
        let raw_text = String::from_utf8_lossy(ref_value.scalar_bytes()).into_owned();

        // Fast path: parseable pointer with a doc part we can join against
        // this document's directory (or fragment-only).
        let ref_range = ref_value.syntax().byte_range();
        let handle: suspect_ref::DocHandle<'s> = self.ws.get(&uri).ok_or(CycleGuard)?;
        let _ = ref_range;
        if let Ok(ptr) = Pointer::parse(raw_text.trim_start_matches('#')) {
            return self.navigate(&handle, &ptr);
        }
        let _ = ValueKind::Null; // keep ValueKind import used on all paths

        // Fallback: original byte-range derivation (anchors etc.).
        let range = ref_value.syntax().byte_range();
        let handle = self.ws.get(&uri).ok_or(CycleGuard)?;
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

    /// Navigates `ptr` from the document root, following top-level `$ref`
    /// chains iteratively.
    ///
    /// A seen-set over landed byte ranges detects every cycle shape
    /// (direct self-`$ref`, mutual wrappers) and degrades with
    /// [`CycleGuard`], matching the legacy resolver's contract. Legal
    /// recursion (`Node.next: $ref Node`) lands on a *structured* target
    /// that has no further top-level `$ref` and terminates.
    fn navigate<'s>(
        &'s self,
        handle: &suspect_ref::DocHandle<'s>,
        ptr: &Pointer,
    ) -> Result<NodeRef<'s>, CycleGuard> {
        use std::collections::HashSet;

        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut current = ptr.clone();
        loop {
            let mut node: Option<NodeRef<'s>> = Some(handle.doc().root());
            for seg in current.tokens() {
                let cur = node.take().ok_or(CycleGuard)?;
                let resolved = cur.resolved();
                match resolved.kind() {
                    suspect_low::ValueKind::Object => {
                        let mut next = None;
                        for e in resolved.entries() {
                            if e.key == seg.as_ref() {
                                next = e.value;
                                break;
                            }
                        }
                        node = next;
                    }
                    suspect_low::ValueKind::Array => {
                        let idx: usize = seg.as_ref().parse().map_err(|_| CycleGuard)?;
                        node = resolved.items().into_iter().nth(idx);
                    }
                    _ => return Err(CycleGuard),
                }
            }
            let landed = node.ok_or(CycleGuard)?;
            let key = (landed.byte_range().start, landed.byte_range().end);
            if !seen.insert(key) {
                return Err(CycleGuard);
            }
            match landed.get("$ref") {
                Some(rv) => {
                    let text = String::from_utf8_lossy(rv.scalar_bytes());
                    let Some(rest) = text.strip_prefix('#') else {
                        return Ok(landed);
                    };
                    current = Pointer::parse(rest).map_err(|_| CycleGuard)?;
                }
                None => return Ok(landed),
            }
        }
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
