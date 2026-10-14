//! Load report renderers: Artillery-style console blocks (per interval and
//! final), a versioned JSON document, and JUnit XML with one test case per
//! `ensure` check. All functions are pure and return owned strings.

use std::fmt::Write as _;

use super::metrics::{MetricsSnapshot, Quantiles};
use super::{IntervalReport, LoadSummary};
use crate::duration::format_duration;

/// JSON report schema version.
pub const JSON_VERSION: u32 = 1;

/// Renders one interval block.
#[must_use]
pub fn console_interval(report: &IntervalReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "--------------------------------------------------------------"
    );
    let _ = writeln!(
        out,
        "{:>8}  phase: {}  (interval {}s, total {}s)",
        format_duration(std::time::Duration::from_secs_f64(report.at_s)),
        report.phase,
        report.interval.elapsed_s.round(),
        report.cumulative.elapsed_s.round()
    );
    let _ = writeln!(
        out,
        "--------------------------------------------------------------"
    );
    out.push_str(&metrics_block(&report.interval, false));
    out
}

/// Renders the final summary: scenario table, phases, cumulative metrics,
/// per-step table, hooks and `ensure` verdicts.
#[must_use]
pub fn console_final(summary: &LoadSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "suspect load report");
    let _ = writeln!(out, "===================");
    let _ = writeln!(out, "target:    {}", summary.target);
    let _ = writeln!(
        out,
        "duration:  {}{}",
        format_duration(std::time::Duration::from_secs_f64(summary.duration_s)),
        if summary.interrupted {
            "  (interrupted)"
        } else {
            ""
        }
    );
    let _ = writeln!(out, "seed:      {}", summary.seed);
    out.push('\n');

    let _ = writeln!(out, "scenarios");
    let width = summary
        .scenarios
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(0)
        .max(8);
    for s in &summary.scenarios {
        let _ = writeln!(
            out,
            "  {:<width$}  weight {:<5} {:>5.1}%  -> {}",
            s.name, s.weight, s.share_pct, s.workflow
        );
    }
    out.push('\n');

    let _ = writeln!(out, "phases");
    for p in &summary.phases {
        let _ = writeln!(
            out,
            "  {:>8}  {:<28} {}{}",
            format!(
                "+{}",
                format_duration(std::time::Duration::from_secs_f64(p.started_at_s))
            ),
            p.label,
            p.kind,
            if p.expected_arrivals > 0 {
                format!(
                    "  ({} arrivals over {})",
                    p.expected_arrivals,
                    format_duration(std::time::Duration::from_secs_f64(p.planned_s))
                )
            } else {
                format!(
                    "  ({})",
                    format_duration(std::time::Duration::from_secs_f64(p.planned_s))
                )
            }
        );
    }
    out.push('\n');

    if !summary.hooks.is_empty() {
        let _ = writeln!(out, "hooks");
        for h in &summary.hooks {
            let _ = writeln!(
                out,
                "  {:<10} {:<28} {} ({} ms)",
                h.role,
                h.workflow,
                if h.ok { "PASS" } else { "FAIL" },
                h.duration_ms
            );
        }
        out.push('\n');
    }

    let _ = writeln!(out, "summary (all phases)");
    out.push_str(&metrics_block(&summary.aggregate, true));

    if !summary.aggregate.steps.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "steps");
        let sw = summary
            .aggregate
            .steps
            .iter()
            .map(|s| s.scenario.len() + s.step.len() + 1)
            .max()
            .unwrap_or(0)
            .max(10);
        let _ = writeln!(
            out,
            "  {:<sw$}  {:>8} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8}",
            "scenario/step", "requests", "errors", "contract", "p50", "p95", "p99", "max"
        );
        for s in &summary.aggregate.steps {
            let errors: u64 = s.errors.values().sum();
            let _ = writeln!(
                out,
                "  {:<sw$}  {:>8} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8}",
                format!("{}/{}", s.scenario, s.step),
                s.requests,
                errors,
                s.contract_failures,
                ms(s.response_time.median),
                ms(s.response_time.p95),
                ms(s.response_time.p99),
                ms(s.response_time.max),
            );
        }
    }

    if !summary.ensure.checks.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "ensure");
        for c in &summary.ensure.checks {
            let mark = if c.passed {
                "✓"
            } else if c.strict {
                "✗"
            } else {
                "!"
            };
            let actual = match (&c.actual, &c.error) {
                (_, Some(e)) => format!("  ({e})"),
                (Some(a), None) => format!("  (actual {})", trim_num(*a)),
                (None, None) => String::new(),
            };
            let _ = writeln!(
                out,
                "  {mark} {}{}{}",
                c.requirement,
                actual,
                if !c.strict && !c.passed {
                    "  [non-strict]"
                } else {
                    ""
                }
            );
        }
        let _ = writeln!(
            out,
            "{}",
            if summary.ensure.ok() {
                "all ensure checks passed"
            } else {
                "ENSURE FAILED"
            }
        );
    }
    out
}

fn metrics_block(m: &MetricsSnapshot, cumulative: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "http.requests ......................: {}",
        m.http.requests
    );
    let _ = writeln!(
        out,
        "http.responses .....................: {}",
        m.http.responses
    );
    let _ = writeln!(
        out,
        "http.request_rate ..................: {:.1}/s",
        m.http.request_rate
    );
    for (code, n) in &m.http.codes {
        let _ = writeln!(out, "http.codes.{code} ....................: {n}");
    }
    out.push_str(&quantile_lines("http.response_time", &m.http.response_time));
    if m.http.ttfb.count > 0 {
        let _ = writeln!(
            out,
            "http.ttfb (p50/p95/p99) ............: {} / {} / {}",
            ms(m.http.ttfb.median),
            ms(m.http.ttfb.p95),
            ms(m.http.ttfb.p99)
        );
    }
    let _ = writeln!(
        out,
        "http.error_rate ....................: {:.2}%",
        m.http.error_rate
    );
    let _ = writeln!(
        out,
        "contract.failures ..................: {} ({:.2}% of responses)",
        m.contract.failures, m.contract.failure_rate
    );
    for (class, n) in m
        .errors
        .iter()
        .filter(|(k, v)| k.as_str() != "total" && **v > 0)
    {
        let _ = writeln!(out, "errors.{class} .....................: {n}");
    }
    let _ = writeln!(
        out,
        "vusers.created .....................: {}",
        m.vusers.created
    );
    for (name, c) in &m.vusers.by_scenario {
        let _ = writeln!(out, "vusers.created_by_name.{name} ......: {}", c.created);
    }
    let _ = writeln!(
        out,
        "vusers.completed ...................: {}",
        m.vusers.completed
    );
    let _ = writeln!(
        out,
        "vusers.failed ......................: {}",
        m.vusers.failed
    );
    if m.vusers.skipped > 0 {
        let _ = writeln!(
            out,
            "vusers.skipped (maxVusers) .........: {}",
            m.vusers.skipped
        );
    }
    if m.vusers.aborted > 0 {
        let _ = writeln!(
            out,
            "vusers.aborted (grace) .............: {}",
            m.vusers.aborted
        );
    }
    if m.vusers.session_length.count > 0 {
        out.push_str(&quantile_lines(
            "vusers.session_length",
            &m.vusers.session_length,
        ));
    }
    if cumulative {
        let _ = writeln!(
            out,
            "bytes down / up ....................: {} / {}",
            bytes(m.http.downloaded_bytes),
            bytes(m.http.uploaded_bytes)
        );
    }
    if m.dropped > 0 {
        let _ = writeln!(
            out,
            "metrics.dropped ....................: {}  (recorder queue full; generator overloaded)",
            m.dropped
        );
    }
    out
}

fn quantile_lines(prefix: &str, q: &Quantiles) -> String {
    if q.count == 0 {
        return format!("{prefix}: (no samples)\n");
    }
    let mut out = String::new();
    let _ = writeln!(out, "{prefix}:");
    let _ = writeln!(out, "  min ......: {}", ms(q.min));
    let _ = writeln!(out, "  max ......: {}", ms(q.max));
    let _ = writeln!(out, "  mean .....: {}", ms(q.mean));
    let _ = writeln!(out, "  median ...: {}", ms(q.median));
    let _ = writeln!(out, "  p95 ......: {}", ms(q.p95));
    let _ = writeln!(out, "  p99 ......: {}", ms(q.p99));
    out
}

fn ms(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2}s", v / 1000.0)
    } else if v >= 10.0 {
        format!("{v:.0}ms")
    } else {
        format!("{v:.1}ms")
    }
}

fn bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    let f = n as f64;
    if f >= KIB * KIB * KIB {
        format!("{:.2} GiB", f / (KIB * KIB * KIB))
    } else if f >= KIB * KIB {
        format!("{:.2} MiB", f / (KIB * KIB))
    } else if f >= KIB {
        format!("{:.1} KiB", f / KIB)
    } else {
        format!("{n} B")
    }
}

fn trim_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Renders the versioned JSON report.
#[must_use]
pub fn json(summary: &LoadSummary) -> serde_json::Value {
    serde_json::json!({
        "version": JSON_VERSION,
        "tool": "suspect load",
        "run": {
            "started_at_ms": summary.started_at_ms,
            "duration_s": summary.duration_s,
            "interrupted": summary.interrupted,
            "target": summary.target,
            "seed": summary.seed,
            "ensure_ok": summary.ensure.ok(),
        },
        "scenarios": summary.scenarios,
        "phases": summary.phases,
        "hooks": summary.hooks,
        "intervals": summary.intervals,
        "aggregate": summary.aggregate,
        "ensure": summary.ensure,
    })
}

/// Renders JUnit XML: one `<testcase>` per `ensure` check (so CI shows
/// which SLO broke), plus one for the run itself when no checks exist.
#[must_use]
pub fn junit(summary: &LoadSummary) -> String {
    let checks = &summary.ensure.checks;
    let time_s = format!("{:.3}", summary.duration_s);
    let failures = checks.iter().filter(|c| !c.passed && c.strict).count();
    let skipped = checks.iter().filter(|c| !c.passed && !c.strict).count();
    let total = checks.len().max(1);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuites tests=\"{total}\" failures=\"{failures}\" errors=\"0\" time=\"{time_s}\">"
    );
    let _ = writeln!(
        out,
        "  <testsuite name=\"suspect-load\" tests=\"{total}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{time_s}\">"
    );
    if checks.is_empty() {
        let _ = writeln!(
            out,
            "    <testcase name=\"load run\" classname=\"suspect.load\" time=\"{time_s}\"/>"
        );
    }
    for c in checks {
        let name = xml_escape(&c.requirement);
        if c.passed {
            let _ = writeln!(
                out,
                "    <testcase name=\"{name}\" classname=\"suspect.load.ensure\" time=\"0\"/>"
            );
            continue;
        }
        let detail = match (&c.actual, &c.error) {
            (_, Some(e)) => e.clone(),
            (Some(a), None) => format!("actual {}", trim_num(*a)),
            (None, None) => "condition false".to_owned(),
        };
        let _ = writeln!(
            out,
            "    <testcase name=\"{name}\" classname=\"suspect.load.ensure\" time=\"0\">"
        );
        if c.strict {
            let _ = writeln!(out, "      <failure message=\"{}\"/>", xml_escape(&detail));
        } else {
            let _ = writeln!(
                out,
                "      <skipped message=\"non-strict: {}\"/>",
                xml_escape(&detail)
            );
        }
        let _ = writeln!(out, "    </testcase>");
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
