//! Latency measurement for the language server, under editor-shaped load.
//!
//! The point is not to make the numbers look good. It is to notice when a
//! change turns a feature from instant into noticeable, or from noticeable
//! into a spinner. Every sample is a real request against the real binary
//! over real pipes, issued the way an editor issues it — which means the
//! numbers include the wait behind anything already in flight, because that
//! wait is exactly what a user feels.
//!
//! Percentiles rather than means: a mean hides the tail completely, and the
//! tail is where a queued lock shows up.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

/// One measured round trip.
#[derive(Debug, Clone)]
pub struct Sample {
    /// The scenario that produced it.
    pub scenario: &'static str,
    /// Wall-clock time of the whole batch this sample came from. Equal to
    /// the sample's own latency for the last request of a batch, and much
    /// larger for the rest — which is how you tell "this handler is slow"
    /// from "everything is queued".
    pub batch: Duration,
    /// The request method.
    pub method: String,
    /// How long the answer took.
    pub elapsed: Duration,
    /// Whether it was answered at all.
    pub answered: bool,
    /// Size of the answer in bytes, when there was one.
    pub bytes: usize,
}

/// Collects samples and reports them.
#[derive(Debug, Default)]
pub struct Bench {
    samples: Vec<Sample>,
}

impl Bench {
    /// A fresh recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one timed request.
    pub fn record(
        &mut self,
        scenario: &'static str,
        method: &str,
        elapsed: Duration,
        answer: &Result<Value, String>,
    ) {
        self.record_in_batch(scenario, method, elapsed, elapsed, answer);
    }

    /// As [`Bench::record`], with the enclosing batch's wall time.
    pub fn record_in_batch(
        &mut self,
        scenario: &'static str,
        method: &str,
        elapsed: Duration,
        batch: Duration,
        answer: &Result<Value, String>,
    ) {
        let (answered, bytes) = match answer {
            Ok(value) => (true, serde_json::to_vec(value).map_or(0, |v| v.len())),
            Err(_) => (false, 0),
        };
        self.samples.push(Sample {
            scenario,
            batch,
            method: method.to_owned(),
            elapsed,
            answered,
            bytes,
        });
    }

    /// Records a whole timed batch.
    pub fn record_batch(
        &mut self,
        scenario: &'static str,
        batch: &[crate::support::editor::Timed],
    ) {
        for entry in batch {
            let answer = entry
                .answer
                .as_ref()
                .map(|v| v.clone())
                .map_err(|e| e.to_string());
            self.record_in_batch(scenario, &entry.method, entry.elapsed, entry.batch, &answer);
        }
    }

    /// The longest batch seen — the figure a person waiting for the editor
    /// to settle actually experiences, as against a single request's latency.
    #[must_use]
    pub fn slowest_batch(&self) -> Option<Duration> {
        self.samples.iter().map(|s| s.batch).max()
    }

    /// Every sample taken.
    #[must_use]
    #[allow(dead_code)]
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// Requests the server never answered.
    #[must_use]
    pub fn unanswered(&self) -> Vec<&Sample> {
        self.samples.iter().filter(|s| !s.answered).collect()
    }

    /// Total number of samples.
    #[must_use]
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// The slowest sample, if there is one.
    #[must_use]
    pub fn slowest(&self) -> Option<&Sample> {
        self.samples.iter().max_by_key(|s| s.elapsed)
    }

    /// Wall-clock span covered by the samples.
    #[must_use]
    #[allow(dead_code)]
    pub fn span(&self) -> Duration {
        self.samples.iter().map(|s| s.elapsed).sum()
    }

    /// Bytes returned per method, so a slow method can be read as "large"
    /// or "slow".
    #[must_use]
    pub fn bytes_by_method(&self) -> BTreeMap<String, usize> {
        let mut out: BTreeMap<String, usize> = BTreeMap::new();
        for sample in self.samples.iter().filter(|s| s.answered) {
            *out.entry(sample.method.clone()).or_default() += sample.bytes;
        }
        out
    }

    /// Per-method statistics, sorted by method name.
    #[must_use]
    pub fn by_method(&self) -> BTreeMap<String, Stats> {
        let mut grouped: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
        for sample in &self.samples {
            if !sample.answered {
                continue;
            }
            grouped
                .entry(sample.method.clone())
                .or_default()
                .push(sample.elapsed);
        }
        grouped
            .into_iter()
            .map(|(method, times)| (method, Stats::of(times)))
            .collect()
    }

    /// Per-scenario statistics.
    #[must_use]
    pub fn by_scenario(&self) -> BTreeMap<&'static str, Stats> {
        let mut grouped: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
        for sample in self.samples.iter().filter(|s| s.answered) {
            grouped
                .entry(sample.scenario)
                .or_default()
                .push(sample.elapsed);
        }
        grouped
            .into_iter()
            .map(|(scenario, times)| (scenario, Stats::of(times)))
            .collect()
    }

    /// A table a human can read, printed to stdout by the calling test.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "\n  {} requests measured, {} unanswered, slowest {:.0}ms\n\n",
            self.count(),
            self.unanswered().len(),
            self.slowest().map_or(0.0, |s| ms(s.elapsed))
        ));
        out.push_str("  method                              n     p50     p90     p95     p99    max   mean\n");
        out.push_str(
            "  ---------------------------------------------------------------------------\n",
        );
        for (method, stats) in self.by_method() {
            out.push_str(&format!(
                "  {method:<34}{:>4}  {:>6.0}  {:>6.0}  {:>6.0}  {:>6.0}  {:>6.0}  {:>6.0}\n",
                stats.count,
                ms(stats.p50),
                ms(stats.p90),
                ms(stats.p95),
                ms(stats.p99),
                ms(stats.max),
                ms(stats.mean)
            ));
        }
        out.push_str("\n  scenario                            n     p50     p90     p95    max\n");
        out.push_str(
            "  ---------------------------------------------------------------------------\n",
        );
        for (scenario, stats) in self.by_scenario() {
            out.push_str(&format!(
                "  {scenario:<34}{:>4}  {:>6.0}  {:>6.0}  {:>6.0}  {:>6.0}\n",
                stats.count,
                ms(stats.p50),
                ms(stats.p90),
                ms(stats.p95),
                ms(stats.max)
            ));
        }
        out
    }

    /// The same data as JSON, for comparing two runs.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let methods: Vec<Value> = self
            .by_method()
            .into_iter()
            .map(|(method, stats)| {
                serde_json::json!({
                    "method": method,
                    "n": stats.count,
                    "p50_ms": round(ms(stats.p50)),
                    "p90_ms": round(ms(stats.p90)),
                    "p95_ms": round(ms(stats.p95)),
                    "p99_ms": round(ms(stats.p99)),
                    "max_ms": round(ms(stats.max)),
                })
            })
            .collect();
        serde_json::json!({
            "requests": self.count(),
            "unanswered": self.unanswered().len(),
            "methods": methods,
        })
    }

    /// Writes [`Bench::to_json`] somewhere a later run can compare against.
    pub fn write_json(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.to_json().to_string());
    }
}

/// Percentiles for one group of samples.
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    /// Number of samples.
    pub count: usize,
    /// Median.
    pub p50: Duration,
    /// 90th percentile.
    pub p90: Duration,
    /// 95th percentile.
    pub p95: Duration,
    /// 99th percentile.
    pub p99: Duration,
    /// Slowest.
    pub max: Duration,
    /// Arithmetic mean.
    pub mean: Duration,
}

impl Stats {
    /// Summarises a set of durations.
    #[must_use]
    pub fn of(mut times: Vec<Duration>) -> Self {
        if times.is_empty() {
            return Self {
                count: 0,
                p50: Duration::ZERO,
                p90: Duration::ZERO,
                p95: Duration::ZERO,
                p99: Duration::ZERO,
                max: Duration::ZERO,
                mean: Duration::ZERO,
            };
        }
        times.sort_unstable();
        let total: Duration = times.iter().sum();
        Self {
            count: times.len(),
            p50: percentile(&times, 0.50),
            p90: percentile(&times, 0.90),
            p95: percentile(&times, 0.95),
            p99: percentile(&times, 0.99),
            max: *times.last().unwrap_or(&Duration::ZERO),
            mean: total / times.len() as u32,
        }
    }
}

/// Nearest-rank percentile over a sorted slice.
fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (fraction * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let stats = Stats::of((1..=100).map(at).collect());
        assert_eq!(stats.count, 100);
        assert_eq!(stats.p50, at(50));
        assert_eq!(stats.p90, at(90));
        assert_eq!(stats.p95, at(95));
        assert_eq!(stats.p99, at(99));
        assert_eq!(stats.max, at(100));
    }

    #[test]
    fn a_single_sample_is_every_percentile() {
        let stats = Stats::of(vec![at(42)]);
        assert_eq!((stats.p50, stats.p95, stats.max), (at(42), at(42), at(42)));
    }

    #[test]
    fn unanswered_requests_are_counted_not_hidden() {
        let mut bench = Bench::new();
        bench.record("s", "textDocument/hover", at(5), &Ok(Value::Null));
        bench.record(
            "s",
            "textDocument/hover",
            at(30_000),
            &Err("never".to_owned()),
        );
        assert_eq!(bench.count(), 2);
        assert_eq!(bench.unanswered().len(), 1);
        // The unanswered one is excluded from the latency figures rather
        // than being folded into a 30-second mean.
        let stats = bench.by_method()["textDocument/hover"];
        assert_eq!(stats.count, 1);
        assert_eq!(stats.max, at(5));
    }

    #[test]
    fn an_empty_group_is_all_zeroes_rather_than_a_panic() {
        let stats = Stats::of(Vec::new());
        assert_eq!(stats.count, 0);
        assert_eq!(stats.p95, Duration::ZERO);
    }
}
