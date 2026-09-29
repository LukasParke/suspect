//! File-descriptor level output silencing for machine-readable runs.
//!
//! A machine-readable gate must emit exactly one document on stdout. The
//! stages it drives (contract, codegen, breaking, tests) report progress in
//! their own words, so a JSON or SARIF run redirects their output to
//! `/dev/null` and keeps only the aggregate report. A text run is
//! untouched: stage progress is part of the human experience.
//!
//! The redirect is process-global and therefore confined to one gate's
//! scope, guarded by a restore-on-drop so an early return cannot leave the
//! process writing to a closed descriptor.

/// Redirects the process's stdout and stderr to `/dev/null` until dropped.
pub struct Silenced {
    #[cfg(unix)]
    saved_out: Option<i32>,
    #[cfg(unix)]
    saved_err: Option<i32>,
}

impl Silenced {
    /// Starts silencing. On non-unix targets this is a no-op.
    #[must_use]
    pub fn start() -> Self {
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
        #[cfg(not(unix))]
        {
            Self {}
        }
    }
}

impl Drop for Silenced {
    fn drop(&mut self) {
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
    }
}
