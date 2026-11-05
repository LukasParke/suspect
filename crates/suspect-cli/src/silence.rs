//! Process-level output silencing for machine-readable runs.
//!
//! A machine-readable gate must emit exactly one document on stdout. The
//! stages it drives (contract, codegen, breaking, tests) report progress in
//! their own words, so a JSON or SARIF run redirects their output to the
//! null device and keeps only the aggregate report. A text run is
//! untouched: stage progress is part of the human experience.
//!
//! On unix the redirect is `dup2` onto descriptors 1 and 2. On Windows,
//! Rust's std looks up `GetStdHandle` on every write and child processes
//! inherit the process's standard handles, so the redirect swaps those with
//! `SetStdHandle` (CRT descriptors are not used by std).
//!
//! The redirect is process-global and therefore confined to one gate's
//! scope, guarded by a restore-on-drop so an early return cannot leave the
//! process writing to a closed descriptor.

use std::io::Write;

/// Redirects the process's stdout and stderr to the null device until
/// dropped.
pub struct Silenced {
    #[cfg(unix)]
    saved_out: Option<i32>,
    #[cfg(unix)]
    saved_err: Option<i32>,
    #[cfg(windows)]
    saved: Option<windows::Saved>,
}

/// Pushes buffered std output to the descriptor it was written for, so text
/// written before a swap is not emitted after it.
fn flush_std() {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
}

impl Silenced {
    /// Starts silencing. On targets other than unix and Windows this is a
    /// no-op.
    #[must_use]
    pub fn start() -> Self {
        flush_std();
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let null = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .ok();
            match null {
                Some(null) => {
                    let null_fd = null.as_raw_fd();
                    let saved_out = unsafe { libc::dup(libc::STDOUT_FILENO) };
                    let saved_err = unsafe { libc::dup(libc::STDERR_FILENO) };
                    unsafe {
                        libc::dup2(null_fd, libc::STDOUT_FILENO);
                        libc::dup2(null_fd, libc::STDERR_FILENO);
                    }
                    // The null handle is dropped here on purpose: the
                    // descriptors it created are now owned by the process.
                    drop(null);
                    Self {
                        saved_out: Some(saved_out),
                        saved_err: Some(saved_err),
                    }
                }
                None => Self {
                    saved_out: None,
                    saved_err: None,
                },
            }
        }
        #[cfg(windows)]
        {
            Self {
                saved: windows::Saved::redirect_to_null(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self {}
        }
    }
}

impl Drop for Silenced {
    fn drop(&mut self) {
        flush_std();
        #[cfg(unix)]
        {
            if let Some(saved) = self.saved_out.take() {
                unsafe {
                    libc::dup2(saved, libc::STDOUT_FILENO);
                    libc::close(saved);
                }
            }
            if let Some(saved) = self.saved_err.take() {
                unsafe {
                    libc::dup2(saved, libc::STDERR_FILENO);
                    libc::close(saved);
                }
            }
        }
        #[cfg(windows)]
        {
            if let Some(saved) = self.saved.take() {
                saved.restore();
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;
    // winbase.h: STD_OUTPUT_HANDLE = (DWORD)-11, STD_ERROR_HANDLE = (DWORD)-12.
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    // kernel32 is linked by std on every Windows target.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> Handle;
        fn SetStdHandle(n_std_handle: u32, handle: Handle) -> i32;
    }

    /// The original standard handles and the open `NUL` file that replaces
    /// them. The file must outlive the swap: its handle is what std and
    /// spawned children write to.
    pub(super) struct Saved {
        out: Handle,
        err: Handle,
        _null: std::fs::File,
    }

    impl Saved {
        pub(super) fn redirect_to_null() -> Option<Self> {
            let null = std::fs::OpenOptions::new().write(true).open("NUL").ok()?;
            let handle = null.as_raw_handle();
            // SAFETY: plain Win32 calls on the current process's handle table.
            unsafe {
                let out = GetStdHandle(STD_OUTPUT_HANDLE);
                let err = GetStdHandle(STD_ERROR_HANDLE);
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
                SetStdHandle(STD_ERROR_HANDLE, handle);
                Some(Self {
                    out,
                    err,
                    _null: null,
                })
            }
        }

        pub(super) fn restore(self) {
            // SAFETY: restores the handles saved by `redirect_to_null`; the
            // originals were never closed.
            unsafe {
                SetStdHandle(STD_OUTPUT_HANDLE, self.out);
                SetStdHandle(STD_ERROR_HANDLE, self.err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Silenced;
    use std::io::Write;
    use std::process::Command;

    const ROLE: &str = "SUSPECT_SILENCE_TEST_ROLE";
    const CHILD: &str = "silence::tests::silenced_child";

    fn self_command(role: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().expect("test binary"));
        cmd.args([
            "--exact",
            CHILD,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role);
        cmd
    }

    /// Writes straight to the process handles; `print!` would go to the
    /// test harness's capture instead.
    fn emit(text: &str) {
        let _ = std::io::stdout().write_all(text.as_bytes());
        let _ = std::io::stderr().write_all(text.as_bytes());
    }

    #[test]
    #[ignore = "invoked in a subprocess by silenced_output_is_discarded_and_restored"]
    fn silenced_child() {
        match std::env::var(ROLE).as_deref() {
            Ok("grandchild") => emit("from-grandchild\n"),
            Ok("child") => {
                emit("before\n");
                {
                    let _silence = Silenced::start();
                    emit("hidden\n");
                    // Children inherit the redirected handles.
                    let status = self_command("grandchild")
                        .status()
                        .expect("spawn grandchild");
                    assert!(status.success());
                    // Unterminated on purpose: a line still buffered when the
                    // guard drops must not reach the real stdout.
                    let _ = std::io::stdout().write_all(b"hidden-partial");
                }
                emit("\nafter\n");
            }
            _ => {}
        }
    }

    #[test]
    fn silenced_output_is_discarded_and_restored() {
        let output = self_command("child").output().expect("spawn child");
        assert!(output.status.success(), "{output:?}");
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            let before = text.find("before\n").unwrap_or_else(|| panic!("{text}"));
            let after = text.find("\nafter\n").unwrap_or_else(|| panic!("{text}"));
            assert!(before < after, "{text}");
            assert!(!text.contains("hidden"), "{text}");
            assert!(!text.contains("from-grandchild"), "{text}");
        }
    }
}
