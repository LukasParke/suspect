//! The workspace graph: loading documents, handing out handles, and
//! breadth-first traversal of external references.
//!
//! Concurrency model: documents live forever in an append-only slot table
//! behind a `RwLock`; the `Uri → DocId` map makes loads idempotent. A mutex
//! serializes the check-then-insert critical section so concurrent openers
//! of the same document converge on one `DocId` instead of racing duplicate
//! parses. A `loading` set additionally detects re-entrant requests for a
//! document that is mid-load (only possible through pathological
//! self-referential spellings; reported as [`RefError::MissingDoc`] rather
//! than deadlocking a non-reentrant mutex).
//!
//! Cycle safety: file A referencing B referencing A cannot loop — the second
//! request for A hits the `Uri` map and resolves to the existing slot.
//!
//! Remote (`http:`/`https:`) references are always denied in v1.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use dashmap::DashMap;
use rustc_hash::FxHasher;
use suspect_low::{LowDoc, NodeRef, Pointer};
use suspect_source::{Source, Uri};

use crate::cycles::{self, CycleReport};
use crate::edges::{EdgeMeta, RefEdge};
use crate::error::{RefError, WorkspaceError};
use crate::resolve::{MemoVal, Resolution};

/// Index of a document inside a [`Workspace`]'s slot table.
pub type DocId = usize;

type FxDashMap<K, V> = DashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Builder for a [`Workspace`].
#[derive(Debug, Clone)]
pub struct WorkspaceBuilder {
    root: Option<PathBuf>,
    max_doc_size: u64,
    max_docs: usize,
    depth_cap: usize,
}

impl Default for WorkspaceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceBuilder {
    /// Creates a builder with defaults: `max_docs = 10_000`,
    /// `max_doc_size = 64 MiB`, `depth_cap = 256`.
    #[must_use]
    pub fn new() -> Self {
        Self { root: None, max_doc_size: 64 << 20, max_docs: 10_000, depth_cap: 256 }
    }

    /// Base directory against which relative CLI entries are resolved.
    #[must_use]
    pub fn root(mut self, path: impl Into<PathBuf>) -> Self {
        self.root = Some(path.into());
        self
    }

    /// Upper bound on documents a workspace will hold.
    #[must_use]
    pub fn max_docs(mut self, n: usize) -> Self {
        self.max_docs = n;
        self
    }

    /// Upper bound on a single document's byte size.
    #[must_use]
    pub fn max_doc_size(mut self, n: u64) -> Self {
        self.max_doc_size = n;
        self
    }

    /// Depth cap for resolution chains (default 256).
    #[must_use]
    pub fn depth_cap(mut self, n: usize) -> Self {
        self.depth_cap = n;
        self
    }

    /// Builds the (empty) workspace.
    ///
    /// # Errors
    /// Never currently fails; kept fallible for future validation.
    pub fn build(self) -> Result<Workspace, WorkspaceError> {
        Ok(Workspace {
            root: self.root,
            max_doc_size: self.max_doc_size,
            max_docs: self.max_docs,
            depth_cap: self.depth_cap,
            slots: RwLock::new(Vec::new()),
            uris: FxDashMap::default(),
            memos: FxDashMap::default(),
            edges_cache: FxDashMap::default(),
            edge_meta: FxDashMap::default(),
            anchors: FxDashMap::default(),
            ids: FxDashMap::default(),
            loading: Mutex::new(HashSet::new()),
            memo_hits: AtomicU64::new(0),
            memo_misses: AtomicU64::new(0),
            cycles_found: AtomicU64::new(0),
        })
    }
}

/// Aggregated counters for a workspace's lifetime so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceStats {
    /// Loaded documents.
    pub docs: usize,
    /// `$ref` edges scanned across all loaded documents.
    pub edges: usize,
    /// Memo cache hits across all resolutions.
    pub memo_hits: u64,
    /// Memo cache misses across all resolutions.
    pub memo_misses: u64,
    /// Cycles discovered by census runs so far.
    pub cycles: usize,
}

/// The `$ref` resolution engine over a set of loaded documents.
///
/// Documents are loaded idempotently by canonical [`Uri`] and live for the
/// workspace's lifetime; handles borrow from it. Three lazily-populated
/// caches back resolution: a one-shot per-document scan (edges, anchors,
/// `$id` bases) and a memo table keyed by `(DocId, pointer)` storing the
/// outcome of following `$ref` chains — a final node, a whole document, or
/// a detected [`Resolution::Cycle`]. Because memo entries are lifetime-free
/// and chain following is deterministic, cached hits are exact, and
/// [`WorkspaceStats`] exposes hit/miss counters.
///
/// Remote policy: `http:`/`https:` references are always rejected with
/// [`RefError::RemoteDenied`] — v1 never performs network fetches.
pub struct Workspace {
    pub(crate) root: Option<PathBuf>,
    pub(crate) max_doc_size: u64,
    pub(crate) max_docs: usize,
    pub(crate) depth_cap: usize,
    // SAFETY invariant: append-only; entries are never removed or replaced,
    // which is what justifies extending borrows to 'ws in `extend_doc`.
    pub(crate) slots: RwLock<Vec<Arc<LowDoc>>>,
    pub(crate) uris: FxDashMap<Uri, DocId>,
    pub(crate) memos: FxDashMap<(DocId, Box<str>), MemoVal>,
    pub(crate) edges_cache: FxDashMap<DocId, Arc<Vec<RefEdge>>>,
    pub(crate) edge_meta: FxDashMap<DocId, Arc<EdgeMeta>>,
    pub(crate) anchors: FxDashMap<DocId, Arc<HashMap<String, Pointer>>>,
    pub(crate) ids: FxDashMap<DocId, Arc<HashMap<Pointer, String>>>,
    pub(crate) loading: Mutex<HashSet<Uri>>,
    pub(crate) memo_hits: AtomicU64,
    pub(crate) memo_misses: AtomicU64,
    pub(crate) cycles_found: AtomicU64,
}


impl Workspace {
    /// Opens an entry (filesystem path or absolute URI) and returns a handle.
    /// Relative paths resolve against the builder root, else the current
    /// directory.
    ///
    /// # Errors
    /// Invalid entry spelling, I/O failure, or a [`RefError`] from loading.
    pub fn open(&self, entry: &str) -> Result<DocHandle<'_>, WorkspaceError> {
        let uri = self.resolve_entry(entry)?;
        let id = self.load_uri(&uri)?;
        Ok(DocHandle { ws: self, id })
    }

    /// Returns a handle if this document URI is already loaded.
    #[must_use]
    pub fn get(&self, uri: &Uri) -> Option<DocHandle<'_>> {
        let id = self.uris.get(uri).map(|e| *e)?;
        Some(DocHandle { ws: self, id })
    }

    /// Number of loaded documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.uris.len()
    }

    /// True when no document is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.uris.is_empty()
    }

    /// URIs of all loaded documents, sorted for determinism.
    #[must_use]
    pub fn uris(&self) -> Vec<Uri> {
        let mut out: Vec<Uri> = self.uris.iter().map(|e| e.key().clone()).collect();
        out.sort();
        out
    }

    /// Loads `entry` and every document reachable through its external
    /// refs, breadth-first (one wave of parallel edge scans per frontier).
    /// Whole-workspace dedup means diamonds load shared leaves once.
    ///
    /// # Errors
    /// [`WorkspaceError::TooManyDocs`] when the frontier would exceed
    /// `max_docs`; propagates load errors otherwise.
    pub fn load_all(&self, entry: &str) -> Result<usize, WorkspaceError> {
        use rayon::prelude::*;

        let first = self.open(entry)?;
        let mut total = 1usize;
        let mut frontier = vec![first.id()];
        loop {
            // Parallel scan of the wave's edges; sequential load afterwards
            // (loads serialize on the loading mutex anyway).
            let mut candidates: Vec<Uri> = frontier
                .par_iter()
                .flat_map_iter(|&id| {
                    let edges = self.edges_of(id);
                    edges
                        .iter()
                        .filter_map(|e| match &e.parsed {
                            crate::edges::ParsedRef::External { uri, .. } => Some(uri.clone()),
                            _ => None,
                        })
                        .collect::<Vec<Uri>>()
                })
                .collect();
            candidates.sort();
            candidates.dedup();

            let mut next = Vec::new();
            for uri in candidates {
                if self.uris.contains_key(&uri) {
                    continue;
                }
                if total >= self.max_docs {
                    return Err(WorkspaceError::TooManyDocs { max: self.max_docs });
                }
                let id = self.load_uri(&uri)?;
                total += 1;
                next.push(id);
            }
            if next.is_empty() {
                return Ok(total);
            }
            frontier = next;
        }
    }

    /// Lifetime counters and sizes for this workspace.
    #[must_use]
    pub fn stats(&self) -> WorkspaceStats {
        WorkspaceStats {
            docs: self.uris.len(),
            edges: self.edges_cache.iter().map(|e| e.value().len()).sum(),
            memo_hits: self.memo_hits.load(Ordering::Relaxed),
            memo_misses: self.memo_misses.load(Ordering::Relaxed),
            cycles: self.cycles_found.load(Ordering::Relaxed) as usize,
        }
    }

    // ---- internals -----------------------------------------------------

    pub(crate) fn extend_doc<'ws>(&'ws self, id: DocId) -> Option<&'ws LowDoc> {
        let guard = match self.slots.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let arc = guard.get(id)?.clone();
        drop(guard);
        let r: &LowDoc = &arc;
        // SAFETY: `slots` is append-only and its payloads immutable (see the
        // field invariant), so the reference stays valid as long as `&'ws self`
        // does even though the local Arc count is dropped here — the table
        // holds another strong reference for the workspace's whole life.
        Some(unsafe { std::mem::transmute::<&LowDoc, &'ws LowDoc>(r) })
    }

    fn resolve_entry(&self, entry: &str) -> Result<Uri, WorkspaceError> {
        if let Ok(uri) = Uri::parse(entry) {
            return Ok(uri);
        }
        let base = self.root.clone().unwrap_or_else(|| PathBuf::from("."));
        Uri::from_path(&base.join(entry)).map_err(|_| WorkspaceError::InvalidEntry(entry.to_owned()))
    }

    /// Idempotent single-document load. Concurrent callers converge on the
    /// same `DocId`; re-entrant requests for an in-flight document error
    /// instead of deadlocking.
    pub(crate) fn load_uri(&self, uri: &Uri) -> Result<DocId, RefError> {
        if let Some(id) = self.uris.get(uri).map(|e| *e) {
            return Ok(id);
        }
        let mut guard = lock(&self.loading);
        if let Some(id) = self.uris.get(uri).map(|e| *e) {
            return Ok(id);
        }
        if !guard.insert(uri.clone()) {
            // Mid-load on this thread's own stack; waiting would deadlock.
            return Err(RefError::MissingDoc { uri: uri.to_string() });
        }
        let res = self.load_uncached(uri);
        guard.remove(uri);
        res
    }

    fn load_uncached(&self, uri: &Uri) -> Result<DocId, RefError> {
        if uri.is_remote() {
            return Err(RefError::RemoteDenied { uri: uri.to_string() });
        }
        let Some(path) = uri.as_path() else {
            return Err(RefError::MissingDoc { uri: uri.to_string() });
        };
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.len() > self.max_doc_size {
                return Err(RefError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("document {} exceeds max_doc_size ({})", path.display(), self.max_doc_size),
                )));
            }
        let source = Source::from_path(&path).map_err(RefError::Io)?;
        let doc = LowDoc::parse(uri.clone(), source);
        let mut guard = match self.slots.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let id = guard.len();
        guard.push(Arc::new(doc));
        drop(guard);
        self.uris.insert(uri.clone(), id);
        Ok(id)
    }

    /// Lazily scans a document once, populating edges, metadata, anchors,
    /// and `$id` indexes. Concurrent first-touches may both scan; only one
    /// set of results wins each insert (scans are deterministic).
    pub(crate) fn ensure_scanned(&self, d: DocId) {
        if self.edges_cache.contains_key(&d) {
            return;
        }
        let Some(doc) = self.extend_doc(d) else { return };
        let scanned = crate::edges::scan(doc);
        self.edges_cache
            .entry(d)
            .or_insert_with(|| Arc::new(scanned.edges));
        self.edge_meta
            .entry(d)
            .or_insert_with(|| Arc::new(scanned.meta));
        self.anchors
            .entry(d)
            .or_insert_with(|| Arc::new(scanned.anchors));
        self.ids.entry(d).or_insert_with(|| Arc::new(scanned.ids));
    }

    pub(crate) fn edges_of(&self, d: DocId) -> Arc<Vec<RefEdge>> {
        self.ensure_scanned(d);
        self.edges_cache.get(&d).map(|e| e.clone()).unwrap_or_default()
    }

    pub(crate) fn meta_of(&self, d: DocId) -> Arc<EdgeMeta> {
        self.ensure_scanned(d);
        self.edge_meta
            .get(&d)
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    pub(crate) fn anchors_of(&self, d: DocId) -> Arc<HashMap<String, Pointer>> {
        self.ensure_scanned(d);
        self.anchors.get(&d).map(|e| e.clone()).unwrap_or_default()
    }

    pub(crate) fn ids_of(&self, d: DocId) -> Arc<HashMap<Pointer, String>> {
        self.ensure_scanned(d);
        self.ids.get(&d).map(|e| e.clone()).unwrap_or_default()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// A borrowed view of one loaded document inside its workspace.
pub struct DocHandle<'ws> {
    pub(crate) ws: &'ws Workspace,
    id: DocId,
}

impl<'ws> std::fmt::Debug for DocHandle<'ws> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocHandle").field("id", &self.id).finish()
    }
}

impl<'ws> DocHandle<'ws> {
    /// Slot index of this document.
    #[must_use]
    pub fn id(&self) -> DocId {
        self.id
    }

    /// The parsed document.
    #[must_use]
    pub fn doc(&self) -> &'ws LowDoc {
        match self.ws.extend_doc(self.id) {
            Some(d) => d,
            // Unreachable: handles are only minted for loaded documents and
            // the slot table never shrinks.
            None => unreachable!("DocHandle references a document that was dropped"),
        }
    }

    /// This document's canonical URI.
    #[must_use]
    pub fn uri(&self) -> &'ws Uri {
        self.doc().uri()
    }

    /// The owning workspace.
    #[must_use]
    pub fn workspace(&self) -> &'ws Workspace {
        self.ws
    }

    /// This document's `$ref` edges, scanned lazily and cached.
    #[must_use]
    pub fn edges(&self) -> Arc<Vec<RefEdge>> {
        self.ws.edges_of(self.id)
    }

    /// Resolves edge number `edge` of this document, following chains and
    /// consulting the memo cache.
    ///
    /// # Errors
    /// Missing pointers/documents, denied remotes, invalid refs, depth cap.
    pub fn resolve_edge(&self, edge: usize) -> Result<Resolution<'ws>, RefError> {
        let edges = self.ws.edges_of(self.id);
        let Some(edge) = edges.get(edge) else {
            return Err(RefError::InvalidRef {
                raw: String::new(),
                reason: format!("edge index {edge} out of range"),
            });
        };
        let mv = self.ws.resolve_edge_memo(self.id, &edge.path, &edge.raw)?;
        self.ws.materialize(&mv)
    }

    /// Resolves an RFC 6901 pointer against a loaded document, following
    /// `$ref` chains from wherever it lands. Memoized.
    ///
    /// # Errors
    /// See [`Self::resolve_edge`].
    pub fn resolve_pointer(
        &self,
        target_doc: DocId,
        pointer: &Pointer,
    ) -> Result<Resolution<'ws>, RefError> {
        let mv = self.ws.resolve_memo(target_doc, pointer)?;
        self.ws.materialize(&mv)
    }

    /// Resolves the value of a `$ref` key (`node` must be that string
    /// value). Applies `$id` base inheritance along the node's ancestor
    /// chain.
    ///
    /// # Errors
    /// See [`Self::resolve_edge`]; additionally [`RefError::InvalidRef`]
    /// when `node` is not a string.
    pub fn resolve_ref_value(&self, node: NodeRef<'ws>) -> Result<Resolution<'ws>, RefError> {
        let raw = node.as_str().ok_or_else(|| RefError::InvalidRef {
            raw: String::from_utf8_lossy(node.scalar_bytes()).into_owned(),
            reason: "node is not a $ref string value".to_owned(),
        })?;
        // Locate the owning document via its syntax-level URI.
        let doc_uri = node.syntax().doc().uri();
        let id = self
            .ws
            .uris
            .iter()
            .find(|e| e.key() == doc_uri)
            .map(|e| *e.value())
            .ok_or_else(|| RefError::MissingDoc { uri: doc_uri.to_string() })?;
        // The value node's own pointer is one token deeper than the
        // containing mapping; $id inheritance walks mapping prefixes.
        let full = node.path_from_root();
        let containing = full.parent().unwrap_or_default();
        let mv = self.ws.resolve_edge_memo(id, &containing, raw)?;
        self.ws.materialize(&mv)
    }

    /// Enumerate and classify the reference cycles among this document's
    /// edges.
    #[must_use]
    pub fn cycles(&self) -> CycleReport {
        let report = cycles::census(self.ws, self.id);
        if !report.cycles.is_empty() {
            self.ws
                .cycles_found
                .fetch_add(report.cycles.len() as u64, Ordering::Relaxed);
        }
        report
    }
}
