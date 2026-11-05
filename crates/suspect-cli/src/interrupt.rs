//! The one Ctrl-C listener.
//!
//! tokio's SIGINT handler is process-wide and permanent once registered, so
//! the binary registers it once, at start-up, on a dedicated thread, and
//! commands never call `tokio::signal::ctrl_c` themselves. What the first
//! Ctrl-C does depends on what the running command opted into:
//!
//! - nothing (the default): exit 130 at once, the status a shell reports
//!   for death by SIGINT;
//! - [`graceful`]: signal the command, which drains and returns its own
//!   status (usually [`Exit::Interrupted`]);
//! - [`server`]: exit 0 at once, because Ctrl-C is how a server stops.
//!
//! A second Ctrl-C always exits 130 at once.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::watch;

use crate::exit::{self, Exit};

const ABORT: u8 = 0;
const GRACEFUL: u8 = 1;
const SERVER: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(ABORT);
static STATE: OnceLock<watch::Sender<bool>> = OnceLock::new();

fn state() -> &'static watch::Sender<bool> {
    STATE.get_or_init(|| watch::channel(false).0)
}

/// Starts the one SIGINT listener and returns once it is registered. First
/// Ctrl-C: signal a [`graceful`] command, exit 0 for a [`server`], else
/// abort with 130. Second Ctrl-C: abort with 130.
///
/// Without `install` (library use, in-process tests) nothing listens, and
/// [`graceful`] receivers never flip.
pub fn install() {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("suspect-interrupt".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                let _ = ready_tx.send(());
                return;
            };
            runtime.block_on(async move {
                let listener = Listener::new();
                let _ = ready_tx.send(());
                let Ok(mut listener) = listener else { return };
                let mut first = true;
                while listener.recv().await {
                    if !first {
                        exit::abort(Exit::Interrupted);
                    }
                    first = false;
                    match MODE.load(Ordering::SeqCst) {
                        GRACEFUL => {
                            state().send_replace(true);
                        }
                        SERVER => exit::abort(Exit::Clean),
                        _ => exit::abort(Exit::Interrupted),
                    }
                }
            });
        });
    // A failed spawn or registration leaves the platform default (death by
    // signal) in place.
    if spawned.is_ok() {
        let _ = ready_rx.recv();
    }
}

/// Opts the running command in to graceful shutdown. The receiver flips to
/// `true` on the first Ctrl-C; the command drains and returns its status.
#[must_use]
pub fn graceful() -> watch::Receiver<bool> {
    MODE.store(GRACEFUL, Ordering::SeqCst);
    state().subscribe()
}

/// Marks the running command as a server: the first Ctrl-C exits 0.
pub fn server() {
    MODE.store(SERVER, Ordering::SeqCst);
}

/// Registers SIGINT (Unix) or Ctrl-C (Windows) synchronously, so `install`
/// can return only once a Ctrl-C would be caught.
struct Listener {
    #[cfg(unix)]
    inner: tokio::signal::unix::Signal,
    #[cfg(windows)]
    inner: tokio::signal::windows::CtrlC,
}

impl Listener {
    fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        let inner = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        #[cfg(windows)]
        let inner = tokio::signal::windows::ctrl_c()?;
        Ok(Self { inner })
    }

    /// `false` once the listener can deliver no more signals.
    async fn recv(&mut self) -> bool {
        self.inner.recv().await.is_some()
    }
}
