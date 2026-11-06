//! Turns a panic inside one generation step into an error value.
//!
//! Backends assert planner invariants with `expect`/indexing; a violated
//! invariant is a generator bug. At the public generation boundary such a
//! panic must not take down the caller — a language server previewing an
//! SDK, or a session generating twelve targets — so it is caught and
//! reported as an internal error that names the step and the panic site.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

thread_local! {
    /// `file:line` of the most recent panic on this thread.
    static LAST_PANIC_SITE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Installs, once per process, a hook that records each panic's location
/// on the panicking thread and then defers to the previously installed
/// hook. Recording per thread keeps concurrent generation steps from
/// seeing each other's panics, and the hook is never swapped back and
/// forth.
fn install_site_recorder() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if let Some(location) = info.location() {
                let site = format!("{}:{}", location.file(), location.line());
                LAST_PANIC_SITE.with(|slot| *slot.borrow_mut() = Some(site));
            }
            previous(info);
        }));
    });
}

/// Runs `step`, returning its value, or a report of the panic it raised.
///
/// The previously installed panic hook still runs, so `RUST_BACKTRACE`
/// works as usual. Generation steps read shared immutable inputs and build
/// fresh output, so no caller-visible state is left half-updated.
///
/// # Errors
/// The message `internal generator error in <label>: <panic> (at <site>)`.
pub fn catch<T>(label: &str, step: impl FnOnce() -> T) -> Result<T, String> {
    install_site_recorder();
    LAST_PANIC_SITE.with(|slot| slot.borrow_mut().take());
    panic::catch_unwind(AssertUnwindSafe(step)).map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        let at = LAST_PANIC_SITE
            .with(|slot| slot.borrow_mut().take())
            .map(|site| format!(" (at {site})"))
            .unwrap_or_default();
        format!(
            "internal generator error in {label}: {message}{at}; this is a bug in suspect, please report it with the input that triggered it"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::catch;

    #[test]
    fn a_panicking_step_becomes_a_located_error() {
        let error = catch("demo-backend", || -> u8 { panic!("invariant broken") }).unwrap_err();
        assert!(
            error.starts_with("internal generator error in demo-backend: invariant broken (at ")
        );
        assert!(error.contains("guard.rs:"), "{error}");
    }

    #[test]
    fn a_normal_step_returns_its_value() {
        assert_eq!(catch("demo-backend", || 7), Ok(7));
    }
}
