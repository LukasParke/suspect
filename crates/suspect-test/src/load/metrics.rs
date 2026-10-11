//! Load metrics: compact per-step samples, the aggregator that folds them
//! into cumulative and interval windows, and the serializable
//! [`MetricsSnapshot`] that reports and `ensure` read.
//!
//! Latencies live in HDR histograms (microsecond resolution, 3 significant
//! figures, mergeable). Per-step histograms use 2 significant figures to
//! bound memory on large plans.

use std::collections::BTreeMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use crate::exec::{ErrorClass, StepSample, StepVerdict};

/// Highest trackable latency (60 s); slower samples saturate.
const MAX_LATENCY_US: u64 = 60_000_000;

/// Index of a scenario in the run's scenario table; hooks use
/// [`HOOK_SCENARIO`].
pub type ScenarioIdx = u16;

/// Scenario index used for `before`/`after` hook workflows.
pub const HOOK_SCENARIO: ScenarioIdx = u16::MAX;

/// One event flowing from virtual users to the aggregator.
#[derive(Debug, Clone)]
pub enum Event {
    /// A step finished.
    Step {
        /// Owning scenario.
        scenario: ScenarioIdx,
        /// Step index within the workflow.
        step: u16,
        /// Measurements.
        sample: StepSample,
    },
    /// A virtual user was created and assigned a scenario.
    VuCreated {
        /// Assigned scenario.
        scenario: ScenarioIdx,
    },
    /// A virtual user finished.
    VuDone {
        /// Its scenario.
        scenario: ScenarioIdx,
        /// Whether every iteration passed.
        ok: bool,
        /// Wall-clock session length.
        duration: Duration,
    },
    /// An arrival was dropped because `maxVusers` was reached.
    VuSkipped,
    /// Users still running when the grace period ended.
    VuAborted {
        /// How many.
        count: u64,
    },
    /// Samples the recorder could not enqueue.
    Dropped {
        /// How many.
        count: u64,
    },
}

/// Latency distribution summary in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Quantiles {
    /// Recorded values.
    pub count: u64,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// 50th percentile.
    pub median: f64,
    /// 75th percentile.
    pub p75: f64,
    /// 90th percentile.
    pub p90: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// 99.9th percentile.
    pub p999: f64,
}

impl Quantiles {
    fn from_hist(h: &Histogram<u64>) -> Self {
        if h.is_empty() {
            return Self::default();
        }
        let ms = |us: u64| us as f64 / 1000.0;
        Self {
            count: h.len(),
            min: ms(h.min()),
            max: ms(h.max()),
            mean: h.mean() / 1000.0,
            median: ms(h.value_at_quantile(0.5)),
            p75: ms(h.value_at_quantile(0.75)),
            p90: ms(h.value_at_quantile(0.9)),
            p95: ms(h.value_at_quantile(0.95)),
            p99: ms(h.value_at_quantile(0.99)),
            p999: ms(h.value_at_quantile(0.999)),
        }
    }

    /// Looks up a field by its report name (`p95`, `median`, `p50`, ...).
    #[must_use]
    pub fn get(&self, field: &str) -> Option<f64> {
        Some(match field {
            "count" => self.count as f64,
            "min" => self.min,
            "max" => self.max,
            "mean" => self.mean,
            "median" | "p50" => self.median,
            "p75" => self.p75,
            "p90" => self.p90,
            "p95" => self.p95,
            "p99" => self.p99,
            "p999" | "p99.9" => self.p999,
            _ => return None,
        })
    }
}

/// HTTP-level rollup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HttpStats {
    /// Requests sent.
    pub requests: u64,
    /// Responses received (requests minus transport failures).
    pub responses: u64,
    /// Requests per second over the window.
    pub request_rate: f64,
    /// Responses by status code.
    pub codes: BTreeMap<u16, u64>,
    /// Response latency.
    pub response_time: Quantiles,
    /// Time to first byte when transports report it.
    pub ttfb: Quantiles,
    /// Response body bytes.
    pub downloaded_bytes: u64,
    /// Request body bytes.
    pub uploaded_bytes: u64,
    /// Percent of requests that failed transport or criteria.
    pub error_rate: f64,
}

/// Contract-criteria rollup — the suspect-specific headline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContractStats {
    /// Steps whose response arrived but failed at least one criterion.
    pub failures: u64,
    /// Percent of responses failing criteria.
    pub failure_rate: f64,
}

/// Per-scenario user counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VuCounts {
    /// Users created.
    pub created: u64,
    /// Users that finished with every iteration passing.
    pub completed: u64,
    /// Users with a failed iteration.
    pub failed: u64,
}

/// Virtual-user lifecycle rollup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VuStats {
    /// Users created.
    pub created: u64,
    /// Users that finished with every iteration passing.
    pub completed: u64,
    /// Users with a failed iteration.
    pub failed: u64,
    /// Arrivals skipped because `maxVusers` was reached.
    pub skipped: u64,
    /// Users aborted at the end of the grace period.
    pub aborted: u64,
    /// Session length distribution.
    pub session_length: Quantiles,
    /// Per-scenario counts keyed by scenario name.
    pub by_scenario: BTreeMap<String, VuCounts>,
}

/// Per-step rollup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepStats {
    /// Scenario name.
    pub scenario: String,
    /// Step id.
    pub step: String,
    /// `METHOD /path` of the target operation.
    pub operation: String,
    /// Requests sent.
    pub requests: u64,
    /// Responses by status code.
    pub codes: BTreeMap<u16, u64>,
    /// Response latency.
    pub response_time: Quantiles,
    /// Criteria failures.
    pub contract_failures: u64,
    /// Transport failures by class.
    pub errors: BTreeMap<String, u64>,
}

/// A point-in-time view of one window (interval or cumulative).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MetricsSnapshot {
    /// Window length in seconds.
    pub elapsed_s: f64,
    /// HTTP rollup.
    pub http: HttpStats,
    /// Transport failures by class (`timeout`, `connect`, ...), plus `total`.
    pub errors: BTreeMap<String, u64>,
    /// Criteria rollup.
    pub contract: ContractStats,
    /// User lifecycle.
    pub vusers: VuStats,
    /// Per-step rollups, in scenario/step order.
    pub steps: Vec<StepStats>,
    /// Samples lost to a full recorder queue.
    pub dropped: u64,
}

impl MetricsSnapshot {
    /// Resolves a dotted metric name (`http.response_time.p95`,
    /// `http.codes.503`, `errors.timeout`, `vusers.failed`,
    /// `contract.failure_rate`, `scenario.<name>.step.<id>.response_time.p99`,
    /// `scenario.<name>.vusers.completed`). Counters that simply have no
    /// entries (`http.codes.503` when no 503 was seen) resolve to `0`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f64> {
        let parts: Vec<&str> = name.split('.').collect();
        match parts.as_slice() {
            ["http", "requests"] => Some(self.http.requests as f64),
            ["http", "responses"] => Some(self.http.responses as f64),
            ["http", "request_rate"] => Some(self.http.request_rate),
            ["http", "error_rate"] => Some(self.http.error_rate),
            ["http", "downloaded_bytes"] => Some(self.http.downloaded_bytes as f64),
            ["http", "uploaded_bytes"] => Some(self.http.uploaded_bytes as f64),
            [
                "http",
                "codes",
                class @ ("1xx" | "2xx" | "3xx" | "4xx" | "5xx"),
            ] => {
                let hundreds = u16::from(class.as_bytes()[0] - b'0');
                Some(
                    self.http
                        .codes
                        .iter()
                        .filter(|(c, _)| *c / 100 == hundreds)
                        .map(|(_, n)| *n as f64)
                        .sum(),
                )
            }
            ["http", "codes", code] => {
                let code: u16 = code.parse().ok()?;
                Some(*self.http.codes.get(&code).unwrap_or(&0) as f64)
            }
            ["http", "response_time", field] => self.http.response_time.get(field),
            ["http", "response_time", "p99", "9"] => Some(self.http.response_time.p999),
            ["http", "ttfb", field] => self.http.ttfb.get(field),
            ["errors", "total"] => Some(*self.errors.get("total").unwrap_or(&0) as f64),
            ["errors", class] => Some(*self.errors.get(*class).unwrap_or(&0) as f64),
            ["contract", "failures"] => Some(self.contract.failures as f64),
            ["contract", "failure_rate"] => Some(self.contract.failure_rate),
            ["vusers", "created"] => Some(self.vusers.created as f64),
            ["vusers", "completed"] => Some(self.vusers.completed as f64),
            ["vusers", "failed"] => Some(self.vusers.failed as f64),
            ["vusers", "skipped"] => Some(self.vusers.skipped as f64),
            ["vusers", "aborted"] => Some(self.vusers.aborted as f64),
            ["vusers", "session_length", field] => self.vusers.session_length.get(field),
            ["scenario", scenario, "vusers", field] => {
                let counts = self.vusers.by_scenario.get(*scenario)?;
                Some(match *field {
                    "created" => counts.created as f64,
                    "completed" => counts.completed as f64,
                    "failed" => counts.failed as f64,
                    _ => return None,
                })
            }
            ["scenario", scenario, "step", step, rest @ ..] => {
                let s = self
                    .steps
                    .iter()
                    .find(|s| s.scenario == *scenario && s.step == *step)?;
                match rest {
                    ["requests"] => Some(s.requests as f64),
                    ["contract_failures"] => Some(s.contract_failures as f64),
                    ["codes", code] => {
                        let code: u16 = code.parse().ok()?;
                        Some(*s.codes.get(&code).unwrap_or(&0) as f64)
                    }
                    ["errors", class] => Some(*s.errors.get(*class).unwrap_or(&0) as f64),
                    ["response_time", field] => s.response_time.get(field),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// Identity of a step for per-step accounting.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct StepKey {
    scenario: ScenarioIdx,
    step: u16,
}

struct StepAcc {
    requests: u64,
    codes: BTreeMap<u16, u64>,
    latency: Histogram<u64>,
    contract_failures: u64,
    errors: BTreeMap<ErrorClass, u64>,
}

impl StepAcc {
    fn new() -> Self {
        Self {
            requests: 0,
            codes: BTreeMap::new(),
            latency: Histogram::new_with_bounds(1, MAX_LATENCY_US, 2).expect("valid bounds"),
            contract_failures: 0,
            errors: BTreeMap::new(),
        }
    }
}

/// Names used to label per-step rows in snapshots.
pub trait StepNames: Send + Sync {
    /// `(scenario name, step id, "METHOD /path")` for a step key.
    fn names(&self, scenario: ScenarioIdx, step: u16) -> (String, String, String);
    /// Scenario name for user counts.
    fn scenario_name(&self, scenario: ScenarioIdx) -> String;
}

/// One accounting window. The aggregator keeps two: cumulative and the
/// current reporting interval.
pub struct Window {
    requests: u64,
    responses: u64,
    codes: BTreeMap<u16, u64>,
    latency: Histogram<u64>,
    ttfb: Histogram<u64>,
    bytes_in: u64,
    bytes_out: u64,
    errors: BTreeMap<ErrorClass, u64>,
    contract_failures: u64,
    vus_created: u64,
    vus_completed: u64,
    vus_failed: u64,
    vus_skipped: u64,
    vus_aborted: u64,
    session_length: Histogram<u64>,
    by_scenario: BTreeMap<ScenarioIdx, VuCounts>,
    steps: BTreeMap<StepKey, StepAcc>,
    dropped: u64,
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Window {
    /// An empty window.
    #[must_use]
    pub fn new() -> Self {
        Self {
            requests: 0,
            responses: 0,
            codes: BTreeMap::new(),
            latency: Histogram::new_with_bounds(1, MAX_LATENCY_US, 3).expect("valid bounds"),
            ttfb: Histogram::new_with_bounds(1, MAX_LATENCY_US, 3).expect("valid bounds"),
            bytes_in: 0,
            bytes_out: 0,
            errors: BTreeMap::new(),
            contract_failures: 0,
            vus_created: 0,
            vus_completed: 0,
            vus_failed: 0,
            vus_skipped: 0,
            vus_aborted: 0,
            session_length: Histogram::new_with_bounds(1, MAX_LATENCY_US * 60, 3)
                .expect("valid bounds"),
            by_scenario: BTreeMap::new(),
            steps: BTreeMap::new(),
            dropped: 0,
        }
    }

    /// Folds one event in.
    pub fn record(&mut self, event: &Event) {
        match event {
            Event::Step {
                scenario,
                step,
                sample,
            } => self.record_step(*scenario, *step, sample),
            Event::VuCreated { scenario } => {
                self.vus_created += 1;
                self.by_scenario.entry(*scenario).or_default().created += 1;
            }
            Event::VuDone {
                scenario,
                ok,
                duration,
            } => {
                let entry = self.by_scenario.entry(*scenario).or_default();
                if *ok {
                    self.vus_completed += 1;
                    entry.completed += 1;
                } else {
                    self.vus_failed += 1;
                    entry.failed += 1;
                }
                let _ = self
                    .session_length
                    .record(micros(*duration).clamp(1, MAX_LATENCY_US * 60));
            }
            Event::VuSkipped => self.vus_skipped += 1,
            Event::VuAborted { count } => self.vus_aborted += count,
            Event::Dropped { count } => self.dropped += count,
        }
    }

    fn record_step(&mut self, scenario: ScenarioIdx, step: u16, sample: &StepSample) {
        self.requests += 1;
        self.bytes_out += sample.bytes_out as u64;
        let us = micros(sample.latency).clamp(1, MAX_LATENCY_US);
        let acc = self
            .steps
            .entry(StepKey { scenario, step })
            .or_insert_with(StepAcc::new);
        acc.requests += 1;
        match sample.verdict {
            StepVerdict::Transport(class) => {
                *self.errors.entry(class).or_insert(0) += 1;
                *acc.errors.entry(class).or_insert(0) += 1;
                // Transport failures are not latency samples of the service.
            }
            verdict => {
                self.responses += 1;
                self.bytes_in += sample.bytes_in as u64;
                let _ = self.latency.record(us);
                let _ = acc.latency.record(us);
                if let Some(ttfb) = sample.ttfb {
                    let _ = self.ttfb.record(micros(ttfb).clamp(1, MAX_LATENCY_US));
                }
                if let Some(status) = sample.status {
                    *self.codes.entry(status).or_insert(0) += 1;
                    *acc.codes.entry(status).or_insert(0) += 1;
                }
                if let StepVerdict::CriteriaFailed(_) = verdict {
                    self.contract_failures += 1;
                    acc.contract_failures += 1;
                }
            }
        }
    }

    /// Renders the window as a snapshot covering `elapsed`.
    #[must_use]
    pub fn snapshot(&self, elapsed: Duration, names: &dyn StepNames) -> MetricsSnapshot {
        let secs = elapsed.as_secs_f64();
        let transport_failures: u64 = self.errors.values().sum();
        let mut errors: BTreeMap<String, u64> = self
            .errors
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), *v))
            .collect();
        errors.insert("total".to_owned(), transport_failures);
        let pct = |num: u64, den: u64| {
            if den == 0 {
                0.0
            } else {
                num as f64 * 100.0 / den as f64
            }
        };
        MetricsSnapshot {
            elapsed_s: secs,
            http: HttpStats {
                requests: self.requests,
                responses: self.responses,
                request_rate: if secs > 0.0 {
                    self.requests as f64 / secs
                } else {
                    0.0
                },
                codes: self.codes.clone(),
                response_time: Quantiles::from_hist(&self.latency),
                ttfb: Quantiles::from_hist(&self.ttfb),
                downloaded_bytes: self.bytes_in,
                uploaded_bytes: self.bytes_out,
                error_rate: pct(transport_failures + self.contract_failures, self.requests),
            },
            errors,
            contract: ContractStats {
                failures: self.contract_failures,
                failure_rate: pct(self.contract_failures, self.responses),
            },
            vusers: VuStats {
                created: self.vus_created,
                completed: self.vus_completed,
                failed: self.vus_failed,
                skipped: self.vus_skipped,
                aborted: self.vus_aborted,
                session_length: Quantiles::from_hist(&self.session_length),
                by_scenario: self
                    .by_scenario
                    .iter()
                    .map(|(idx, counts)| (names.scenario_name(*idx), counts.clone()))
                    .collect(),
            },
            steps: self
                .steps
                .iter()
                .map(|(key, acc)| {
                    let (scenario, step, operation) = names.names(key.scenario, key.step);
                    StepStats {
                        scenario,
                        step,
                        operation,
                        requests: acc.requests,
                        codes: acc.codes.clone(),
                        response_time: Quantiles::from_hist(&acc.latency),
                        contract_failures: acc.contract_failures,
                        errors: acc
                            .errors
                            .iter()
                            .map(|(k, v)| (k.as_str().to_owned(), *v))
                            .collect(),
                    }
                })
                .collect(),
            dropped: self.dropped,
        }
    }
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Names;
    impl StepNames for Names {
        fn names(&self, scenario: ScenarioIdx, step: u16) -> (String, String, String) {
            (
                format!("s{scenario}"),
                format!("step{step}"),
                "GET /x".into(),
            )
        }
        fn scenario_name(&self, scenario: ScenarioIdx) -> String {
            format!("s{scenario}")
        }
    }

    fn sample(ms: u64, status: Option<u16>, verdict: StepVerdict) -> StepSample {
        StepSample {
            status,
            latency: Duration::from_millis(ms),
            ttfb: Some(Duration::from_millis(ms / 2)),
            bytes_out: 10,
            bytes_in: 100,
            verdict,
        }
    }

    #[test]
    fn percentiles_and_rates() {
        let mut w = Window::new();
        for ms in 1..=100u64 {
            w.record(&Event::Step {
                scenario: 0,
                step: 0,
                sample: sample(ms, Some(200), StepVerdict::Passed),
            });
        }
        w.record(&Event::Step {
            scenario: 0,
            step: 1,
            sample: sample(5, Some(500), StepVerdict::CriteriaFailed(1)),
        });
        w.record(&Event::Step {
            scenario: 0,
            step: 1,
            sample: sample(10_000, None, StepVerdict::Transport(ErrorClass::Timeout)),
        });
        w.record(&Event::VuCreated { scenario: 0 });
        w.record(&Event::VuDone {
            scenario: 0,
            ok: false,
            duration: Duration::from_secs(3),
        });
        let snap = w.snapshot(Duration::from_secs(10), &Names);

        assert_eq!(snap.http.requests, 102);
        assert_eq!(snap.http.responses, 101);
        assert!((snap.http.request_rate - 10.2).abs() < 1e-9);
        assert_eq!(snap.http.codes[&200], 100);
        assert_eq!(snap.http.codes[&500], 1);
        // HDR at 3 sig figs: p50 of 1..100ms plus one 5ms sample ≈ 50ms.
        assert!(
            (snap.http.response_time.median - 50.0).abs() < 1.0,
            "{:?}",
            snap.http.response_time
        );
        assert!((snap.http.response_time.p99 - 99.0).abs() < 1.5);
        assert_eq!(snap.errors["timeout"], 1);
        assert_eq!(snap.errors["total"], 1);
        assert_eq!(snap.contract.failures, 1);
        // (1 transport + 1 contract) / 102 requests.
        assert!((snap.http.error_rate - 1.9607).abs() < 0.01);
        assert_eq!(snap.vusers.failed, 1);
        assert_eq!(snap.vusers.by_scenario["s0"].created, 1);
        assert_eq!(snap.steps.len(), 2);

        assert_eq!(snap.get("http.codes.500"), Some(1.0));
        assert_eq!(snap.get("http.codes.503"), Some(0.0));
        assert_eq!(snap.get("http.codes.2xx"), Some(100.0));
        assert_eq!(snap.get("errors.timeout"), Some(1.0));
        assert_eq!(snap.get("errors.connect"), Some(0.0));
        assert_eq!(snap.get("scenario.s0.step.step1.requests"), Some(2.0));
        assert_eq!(snap.get("scenario.s0.step.step1.codes.500"), Some(1.0));
        assert_eq!(snap.get("scenario.s0.vusers.failed"), Some(1.0));
        assert!(
            snap.get("scenario.s0.step.step0.response_time.p95")
                .is_some()
        );
        assert_eq!(snap.get("nope.metric"), None);
        assert_eq!(snap.get("http.response_time.p42"), None);
    }

    #[test]
    fn empty_window_is_all_zero() {
        let snap = Window::new().snapshot(Duration::ZERO, &Names);
        assert_eq!(snap.http.requests, 0);
        assert_eq!(snap.http.response_time, Quantiles::default());
        assert_eq!(snap.get("http.response_time.p95"), Some(0.0));
        assert_eq!(snap.get("http.error_rate"), Some(0.0));
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    struct Names;
    impl StepNames for Names {
        fn names(&self, s: ScenarioIdx, step: u16) -> (String, String, String) {
            (format!("s{s}"), format!("step{step}"), String::new())
        }
        fn scenario_name(&self, s: ScenarioIdx) -> String {
            format!("s{s}")
        }
    }

    /// `cargo test --release -p suspect-test aggregator_ingest -- --ignored --nocapture`
    #[test]
    #[ignore = "perf measurement; run explicitly in release mode"]
    fn aggregator_ingest() {
        let mut w = Window::new();
        let n = 2_000_000u64;
        let started = std::time::Instant::now();
        for i in 0..n {
            w.record(&Event::Step {
                scenario: (i % 3) as u16,
                step: (i % 4) as u16,
                sample: StepSample {
                    status: Some(200 + (i % 2) as u16),
                    latency: Duration::from_micros(100 + (i % 5000)),
                    ttfb: Some(Duration::from_micros(50)),
                    bytes_out: 10,
                    bytes_in: 100,
                    verdict: StepVerdict::Passed,
                },
            });
        }
        let el = started.elapsed();
        let _ = w.snapshot(el, &Names);
        eprintln!(
            "aggregator ingest: {n} samples in {:.3}s = {:.0} ns/sample ({:.2}M samples/s)",
            el.as_secs_f64(),
            el.as_secs_f64() * 1e9 / n as f64,
            n as f64 / el.as_secs_f64() / 1e6
        );
    }
}
