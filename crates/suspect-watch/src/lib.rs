#![deny(missing_docs)]
//! suspect-watch: debounced filesystem watching for document trees.
//!
//! [`watch`] spawns a dedicated thread running a `notify` recommended
//! watcher over the given roots and coalesces bursts of modifications
//! (editor saves, formatter rewrites) into single [`DebounceEvent`]s
//! delivered on a plain channel. Only `yaml`, `yml`, and `json` files are
//! reported; editor temporaries (`.swp`, `~` suffix, `#` prefix) are
//! ignored so vim/emacs churn never triggers a rebuild.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use notify::{Event, RecursiveMode, Watcher, recommended_watcher};

/// Extensions that count as watched documents.
const WATCHED_EXTENSIONS: &[&str] = &["yaml", "yml", "json"];

/// Upper bound on how long the watcher thread sleeps between stop checks.
/// Also the polling granularity while idle; debounce flushes use exact
/// remaining-time waits capped at this.
const POLL: Duration = Duration::from_millis(50);

/// Handle to a running watcher. Send `()` on [`WatcherHandle::stop`] — or
/// simply drop the handle — to terminate the watcher thread.
#[must_use = "dropping the handle stops the watcher; keep it alive for as long as watching is needed"]
pub struct WatcherHandle {
    /// Shutdown signal: sending `()` (or dropping the sender) ends the thread.
    pub stop: Sender<()>,
}

/// A coalesced burst of file modifications.
#[derive(Debug, Clone)]
pub enum DebounceEvent {
    /// Paths modified during one quiet-debounce window, deduplicated.
    Changed(Vec<PathBuf>),
}

/// Watches `roots` recursively for yaml/yml/json modifications, coalescing
/// bursts within `debounce` into one [`DebounceEvent::Changed`] delivered on
/// `tx`. Returns after spawning the watcher thread; drop [`WatcherHandle`]
/// or send `()` on its `stop` channel to terminate.
///
/// # Errors
/// Watcher construction or root registration failures (missing paths,
/// permission denied).
pub fn watch(
    roots: Vec<PathBuf>,
    debounce: Duration,
    tx: Sender<DebounceEvent>,
) -> io::Result<WatcherHandle> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    // Raw notify events flow to this channel; the thread below owns both
    // ends of the state machine.
    let (event_tx, event_rx) = mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher = recommended_watcher(event_tx).map_err(notify_to_io)?;
    for root in &roots {
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(notify_to_io)?;
    }

    std::thread::Builder::new()
        .name("suspect-watch".into())
        .spawn(move || {
            // Keep the watcher alive for the lifetime of the thread: it
            // unregisters all watches when dropped.
            let _watcher = watcher;
            let mut pending: Vec<PathBuf> = Vec::new();
            let mut last_fire: Option<Instant> = None;
            loop {
                match stop_rx.try_recv() {
                    Ok(()) | Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => {}
                }
                // Quiet long enough with something buffered? Flush once.
                if let Some(fired) = last_fire
                    && !pending.is_empty()
                    && fired.elapsed() >= debounce
                {
                    if tx
                        .send(DebounceEvent::Changed(std::mem::take(&mut pending)))
                        .is_err()
                    {
                        break;
                    }
                    last_fire = None;
                    continue;
                }
                // Wait for the next notify event, or until the debounce
                // window expires, whichever comes first (capped by POLL so
                // shutdown stays responsive).
                let wait = match last_fire {
                    Some(fired) => (debounce - fired.elapsed()).min(POLL),
                    None => POLL,
                };
                match event_rx.recv_timeout(wait) {
                    Ok(Ok(event)) => {
                        let changed: Vec<PathBuf> =
                            event.paths.into_iter().filter(|p| is_watched(p)).collect();
                        if !changed.is_empty() {
                            merge(&mut pending, changed);
                            last_fire = Some(Instant::now());
                        }
                    }
                    // A notify backend hiccup is not fatal: keep watching.
                    Ok(Err(_)) => {}
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })?;

    Ok(WatcherHandle { stop: stop_tx })
}
/// Flattens a [`notify::Error`] into [`std::io::Error`].
fn notify_to_io(err: notify::Error) -> io::Error {
    match err.kind {
        notify::ErrorKind::Io(io_err) => io_err,
        _ => io::Error::other(err.to_string()),
    }
}

/// Merges `paths` into `pending`, normalizing each and skipping duplicates.
fn merge(pending: &mut Vec<PathBuf>, paths: Vec<PathBuf>) {
    for path in paths {
        let normalized = normalize(&path);
        if !pending.contains(&normalized) {
            pending.push(normalized);
        }
    }
}

/// Normalizes a path: canonical form when resolvable, original otherwise
/// (the file may already be gone by the time we report it).
#[must_use]
pub fn normalize(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Whether a path names a watched document: right extension and not an
/// editor temporary (`.swp` suffix, `~` backup suffix, `#`/`.` lock prefix).
pub fn is_watched(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.')
        || name.starts_with('#')
        || name.ends_with('~')
        || name.ends_with(".swp")
    {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if WATCHED_EXTENSIONS.contains(&ext)
    )
}

#[cfg(test)]
mod tests;
