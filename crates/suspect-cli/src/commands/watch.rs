//! `suspect watch`: re-run a command whenever watched documents change.
//!
//! Thin glue over [`suspect_watch`]: one immediate run, then kill-and-
//! respawn the child on every debounced change burst under `roots`.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::Duration;
use suspect_watch::{self as watcher, DebounceEvent};

/// Debounce window coalescing editor save bursts before a re-run.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Owns the current child process; kills it on drop so a terminating
/// parent never leaves an orphaned runner behind.
struct ChildGuard {
    child: Option<Child>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawns `command` with inherited stdout/stderr; empty command means no-op.
fn spawn(command: &[String]) -> anyhow::Result<Option<Child>> {
    let Some((program, args)) = command.split_first() else {
        return Ok(None);
    };
    Ok(Some(Command::new(program).args(args).spawn().map_err(
        |e| anyhow::anyhow!("failed to spawn `{program}`: {e}"),
    )?))
}

/// Kills and reaps the guarded child, if any.
fn stop(guard: &mut ChildGuard) {
    if let Some(child) = guard.child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    guard.child = None;
}

/// Runs `command` once immediately, then kills and re-runs it whenever
/// files under `roots` change. Blocks until the watcher stops (handle
/// dropped or shutdown signalled). Ctrl-C follows default terminal
/// behavior: it terminates this process, and the OS reaps the child via
/// the process group; normal exits go through `ChildGuard`.
///
/// # Errors
/// Watcher setup failure or failure to spawn the command.
pub fn watch(roots: &[PathBuf], command: &[String]) -> anyhow::Result<i32> {
    let (tx, rx) = mpsc::channel();
    let _handle = watcher::watch(roots.to_vec(), DEBOUNCE, tx)?;

    println!(
        "[suspect-watch] watching {} root(s); running `{}`",
        roots.len(),
        command.join(" ")
    );
    let mut guard = ChildGuard {
        child: spawn(command)?,
    };
    while let Ok(event) = rx.recv() {
        let DebounceEvent::Changed(changed) = event;
        println!(
            "[suspect-watch] {} file(s) changed; re-running",
            changed.len()
        );
        stop(&mut guard);
        guard.child = spawn(command)?;
    }
    // Watcher ended (its handle dropped elsewhere): adopt final exit code.
    Ok(guard
        .child
        .as_mut()
        .and_then(|c| c.wait().ok().and_then(|s| s.code()))
        .unwrap_or(0))
}
