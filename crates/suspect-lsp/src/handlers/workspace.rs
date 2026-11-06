//! Workspace, semantic-index and per-document cache plumbing shared by every handler.

use std::sync::Arc;

use suspect_source::Uri;

use crate::Backend;
use crate::meaning;
use crate::state::{self};

impl Backend {
    /// The semantic index for the current content, building it off-lock.
    ///
    /// `State::cached_index` answers the common case with a read guard. A
    /// miss is built with no lock held at all: the build walks the whole
    /// workspace, and holding the write lock through it blocks every other
    /// request the editor has in flight. Two requests that miss together may
    /// both build; the second store wins and both are equivalent.
    pub(crate) async fn index(
        &self,
        ws: &std::sync::Arc<suspect_ref::Workspace>,
    ) -> std::sync::Arc<crate::meaning::Index> {
        let generation = self.state.read().await.generation();
        if let Some(index) = self.state.read().await.cached_index(generation) {
            return index;
        }
        // Single-flight: requests that miss together (a burst right after
        // a keystroke invalidates the cache) line up here. The first builds;
        // every waiter re-checks the cache under the lock and finds the
        // finished answer instead of building it again.
        let _guard = self.index_build_lock.lock().await;
        if let Some(index) = self.state.read().await.cached_index(generation) {
            return index;
        }
        let built = tokio::task::spawn_blocking({
            let ws = ws.clone();
            move || std::sync::Arc::new(crate::meaning::Index::build(&ws))
        })
        .await;
        // A panicked build must not fail the request: an empty index answers
        // the question with less detail rather than not at all.
        let index = built.unwrap_or_default();
        self.state
            .write()
            .await
            .store_index(generation, index.clone());
        index
    }

    /// A whole-document response from the cache, if the content is unchanged
    /// since it was computed.
    ///
    /// The response types are big — links for a 63k-line specification
    /// serialise to 1.7MB — so the cache stores `Arc`s and a hit is a
    /// pointer clone rather than a payload copy.
    pub(crate) async fn doc_cached<T>(
        &self,
        uri: &Uri,
        pick: fn(&state::DocCache) -> Option<std::sync::Arc<T>>,
    ) -> Option<std::sync::Arc<T>> {
        let st = self.state.read().await;
        let (epoch, cache) = st.doc_cache.get(uri)?;
        if *epoch != st.generation() {
            return None;
        }
        pick(cache)
    }

    /// Records a whole-document response against the epoch it was computed
    /// for. Stale entries are replaced rather than merged: a lookup checks
    /// the epoch before trusting anything, so an edit landing mid-compute
    /// cannot resurrect a stale answer.
    pub(crate) async fn doc_store<T>(
        &self,
        uri: &Uri,
        epoch: u64,
        set: fn(&mut state::DocCache, Option<std::sync::Arc<T>>),
        value: std::sync::Arc<T>,
    ) {
        let mut st = self.state.write().await;
        let entry = st
            .doc_cache
            .entry(uri.clone())
            .or_insert_with(|| (epoch, state::DocCache::default()));
        if entry.0 != epoch {
            *entry = (epoch, state::DocCache::default());
        }
        set(&mut entry.1, Some(value));
    }

    /// Returns the workspace, building it on first use and making sure the
    /// given document plus its `$ref` closure is loaded. Best-effort.
    pub(crate) async fn workspace_for(&self, uri: &Uri) -> Option<Arc<suspect_ref::Workspace>> {
        // A cached workspace is cloned out under a *read* guard. Taking a
        // write lock here deadlocked any handler already holding a read
        // guard: tokio's RwLock is write-preferring, so the writer waits
        // for a reader that is this same task, and never wakes. One such
        // call wedged the whole server, because the queued writer then
        // blocked every later reader too.
        let cached = self.state.read().await.workspace.clone();
        let ws = match cached {
            Some(ws) => ws,
            None => {
                let mut st = self.state.write().await;
                st.ensure_workspace()?
            }
        };
        if ws.get(uri).is_none()
            && let Some(path) = uri.as_path()
        {
            let _ = ws.load_all(&path.to_string_lossy());
        }
        Some(ws)
    }

    /// A closed workspace mirroring the current editor state: every
    /// document of the entry's disk closure contributes its live buffer
    /// when open, its on-disk bytes otherwise. Compiling against this
    /// sees what the user sees — admission reviews and rendered previews
    /// track unsaved edits instead of the last saved state.
    ///
    /// On-demand by design: the provider snapshot is built per request
    /// (hashing the closure's bytes), and the closed workspace re-parses
    /// documents as it opens them. That is the correct cost for a request
    /// like "review my contract"; the per-keystroke path stays the
    /// incremental reparse.
    pub(crate) async fn live_workspace(&self, uri: &Uri) -> Option<Arc<suspect_ref::Workspace>> {
        let disk = self.workspace_for(uri).await?;
        let st = self.state.read().await;
        let mut provided = Vec::new();
        for open_uri in disk.uris() {
            let Some(handle) = disk.get(&open_uri) else {
                continue;
            };
            let disk_bytes = handle.doc().inner().bytes().to_vec();
            let bytes: std::sync::Arc<[u8]> = match st.docs.get(&open_uri) {
                Some(doc) if doc.text.as_bytes() != disk_bytes.as_slice() => {
                    doc.text.clone().into_bytes().into()
                }
                _ => disk_bytes.into(),
            };
            // The handle's URI is the workspace's own identity for this
            // document (file-based workspaces have no provider metadata);
            // requested == effective keeps every $ref that resolved
            // before resolving the same way here.
            match suspect_ref::ProvidedDocument::new(
                handle.uri().clone(),
                handle.uri().clone(),
                bytes,
            ) {
                Ok(document) => provided.push(document),
                Err(_) => return None,
            }
        }
        let provider = suspect_ref::DocumentProvider::new(provided).ok()?;
        suspect_ref::WorkspaceBuilder::new()
            .document_provider(std::sync::Arc::new(provider))
            .build()
            .ok()
            .map(Arc::new)
    }

    /// Builds the reference index before anyone asks for it.
    ///
    /// An open document is a near-certainty that a hover follows, so the
    /// workspace walk happens now — off the interaction path, on the
    /// blocking pool, without the state lock held while it runs. Editing
    /// invalidates the result, so the next edit warms it again and the
    /// build never lands in front of a cursor move.
    pub(crate) fn warm_index(&self) {
        // Debounced: one whole-workspace build per editing pause, not one
        // per keystroke. The sleep means a typing burst of N changes
        // schedules N of these tasks but exactly one survives to build;
        // the rest observe a newer generation on wake and stand down.
        const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            // The generation this task was scheduled for: only the task
            // owned by the LAST change of an editing pause builds.
            let scheduled = state.read().await.generation();
            tokio::time::sleep(SETTLE).await;
            // The editor kept moving: a newer task owns the warm.
            if state.read().await.generation() != scheduled {
                return;
            }
            let (ws, generation) = {
                let st = state.read().await;
                let Some(ws) = st.workspace.as_ref() else {
                    return;
                };
                (Arc::clone(ws), st.generation())
            };
            let built =
                tokio::task::spawn_blocking(move || Arc::new(meaning::Index::build(&ws))).await;
            let Ok(index) = built else {
                return;
            };
            let mut st = state.write().await;
            // A document changed while we were walking the workspace: what
            // we built is already stale, so leave the cache to whoever
            // needs it next.
            if st.generation() == generation {
                st.index_cache = Some((generation, index));
            }
        });
    }

    /// Any usable ref workspace: cached one, else rebuilt from the root.
    pub(crate) async fn any_workspace(&self) -> Option<Arc<suspect_ref::Workspace>> {
        {
            let st = self.state.read().await;
            if let Some(ws) = &st.workspace {
                return Some(ws.clone());
            }
        }
        // Apply the configured document cap when rebuilding: without it a
        // `suspect.ref.maxDocs` setting parses but never limits loading.
        let (root, max_docs) = {
            let st = self.state.read().await;
            (st.root.clone(), st.config.max_docs())
        };
        let root = root?;
        let ws = suspect_ref::WorkspaceBuilder::new()
            .max_docs(max_docs)
            .root(&root)
            .build()
            .ok()?;
        ws.load_all("main.yaml").ok()?;
        Some(Arc::new(ws))
    }

    /// Invalidates the cached workspace so the next request rebuilds it.
    pub(crate) async fn drop_workspace_cache(&self) {
        self.state.write().await.workspace = None;
    }
}
