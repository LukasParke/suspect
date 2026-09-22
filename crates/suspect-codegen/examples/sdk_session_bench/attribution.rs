//! Process counters sampled outside the wall-clock/allocation interval.
//!
//! These observations help locate variability; they cannot identify thermal
//! throttling, physical IO bytes, or a per-phase peak from cumulative counters.

use anyhow::{Context, Result, ensure};
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    user_micros: u64,
    system_micros: u64,
    input_blocks: u64,
    output_blocks: u64,
    minor_faults: u64,
    major_faults: u64,
    voluntary_switches: u64,
    involuntary_switches: u64,
    peak_rss_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Delta {
    pub user_cpu_ms: f64,
    pub system_cpu_ms: f64,
    pub cpu_ms: f64,
    pub input_block_operations: u64,
    pub output_block_operations: u64,
    pub minor_page_faults: u64,
    pub major_page_faults: u64,
    pub voluntary_context_switches: u64,
    pub involuntary_context_switches: u64,
    /// Lifetime process high-water marks, not peaks attributed to this phase.
    pub process_peak_rss_before_bytes: u64,
    pub process_peak_rss_after_bytes: u64,
}

impl Snapshot {
    #[cfg(unix)]
    pub fn now() -> Result<Self> {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes the complete out parameter on success;
        // no value is read after a failed call. RUSAGE_SELF is a valid selector.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        ensure!(
            status == 0,
            "getrusage failed: {}",
            std::io::Error::last_os_error()
        );
        let usage = unsafe { usage.assume_init() };
        let micros = |value: libc::timeval| -> Result<u64> {
            let seconds = u64::try_from(value.tv_sec).context("negative CPU seconds")?;
            let remainder = u64::try_from(value.tv_usec).context("negative CPU microseconds")?;
            ensure!(remainder < 1_000_000, "invalid CPU microsecond remainder");
            seconds
                .checked_mul(1_000_000)
                .and_then(|n| n.checked_add(remainder))
                .context("CPU counter overflow")
        };
        let count = |value: libc::c_long| u64::try_from(value).context("negative process counter");
        // Darwin reports bytes; Linux and the other supported Unix targets
        // report KiB. Fail on unknown hosts rather than guessing a unit.
        let scale = if cfg!(target_os = "macos") {
            1
        } else if cfg!(target_os = "linux") {
            1024
        } else {
            anyhow::bail!("process RSS attribution is supported on Linux and macOS")
        };
        Ok(Self {
            user_micros: micros(usage.ru_utime)?,
            system_micros: micros(usage.ru_stime)?,
            input_blocks: count(usage.ru_inblock)?,
            output_blocks: count(usage.ru_oublock)?,
            minor_faults: count(usage.ru_minflt)?,
            major_faults: count(usage.ru_majflt)?,
            voluntary_switches: count(usage.ru_nvcsw)?,
            involuntary_switches: count(usage.ru_nivcsw)?,
            peak_rss_bytes: count(usage.ru_maxrss)?
                .checked_mul(scale)
                .context("RSS overflow")?,
        })
    }

    #[cfg(not(unix))]
    pub fn now() -> Result<Self> {
        anyhow::bail!("process resource attribution requires Linux or macOS")
    }

    pub fn since(self, before: Self) -> Result<Delta> {
        let difference = |after: u64, before: u64| {
            after
                .checked_sub(before)
                .context("process resource counter moved backwards")
        };
        let user_cpu_ms = difference(self.user_micros, before.user_micros)? as f64 / 1000.0;
        let system_cpu_ms = difference(self.system_micros, before.system_micros)? as f64 / 1000.0;
        Ok(Delta {
            user_cpu_ms,
            system_cpu_ms,
            cpu_ms: user_cpu_ms + system_cpu_ms,
            input_block_operations: difference(self.input_blocks, before.input_blocks)?,
            output_block_operations: difference(self.output_blocks, before.output_blocks)?,
            minor_page_faults: difference(self.minor_faults, before.minor_faults)?,
            major_page_faults: difference(self.major_faults, before.major_faults)?,
            voluntary_context_switches: difference(
                self.voluntary_switches,
                before.voluntary_switches,
            )?,
            involuntary_context_switches: difference(
                self.involuntary_switches,
                before.involuntary_switches,
            )?,
            process_peak_rss_before_bytes: before.peak_rss_bytes,
            process_peak_rss_after_bytes: self.peak_rss_bytes,
        })
    }
}
