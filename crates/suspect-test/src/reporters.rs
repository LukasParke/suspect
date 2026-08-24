//! Pure report renderers over run results: aligned console text, JUnit XML,
//! and NDJSON event lines. All functions are side-effect free and return
//! owned strings.

use crate::exec::{RunSummary, TestEvent};

/// Per-workflow view assembled from the event log.
struct WfReport {
    id: String,
    passed: bool,
    duration_ms: u64,
    /// `(rendered line, optional status)` pairs, one per executed request.
    requests: Vec<(String, Option<u16>)>,
    failures: Vec<String>,
}

impl WfReport {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            passed: true,
            duration_ms: 0,
            requests: Vec::new(),
            failures: Vec::new(),
        }
    }
}

/// Renders an aligned console summary: one block per workflow with
/// PASS/FAIL, one line per request with its response status, criterion
/// failures, and a final totals line.
#[must_use]
pub fn console(summary: &RunSummary, events_log: &[TestEvent]) -> String {
    let mut reports: Vec<WfReport> = Vec::new();
    for ev in events_log {
        match ev {
            TestEvent::WfStarted { id } => reports.push(WfReport::new(id)),
            TestEvent::RequestSent {
                wf, method, url, ..
            } => {
                report_for(&mut reports, wf)
                    .requests
                    .push((format!("{method} {url}"), None));
            }
            TestEvent::ResponseGot {
                wf,
                status,
                duration_ms,
                ..
            } => {
                let report = report_for(&mut reports, wf);
                if let Some(last) = report.requests.last_mut() {
                    last.1 = Some(*status);
                }
                report.duration_ms += duration_ms;
            }
            TestEvent::CriterionFail {
                wf,
                crit,
                expected,
                actual,
                ..
            } => {
                let report = report_for(&mut reports, wf);
                report
                    .failures
                    .push(format!("{crit}: expected {expected}, got {actual}"));
                report.passed = false;
            }
            TestEvent::WfDone { wf, passed: false } => {
                report_for(&mut reports, wf).passed = false;
            }
            _ => {}
        }
    }

    let width = reports.iter().map(|r| r.id.len()).max().unwrap_or(0);
    let mut out = String::from("suspect test report\n===================\n");
    for report in &reports {
        let verdict = if report.passed { "PASS" } else { "FAIL" };
        let label = format!(
            "{} {}",
            report.id,
            ".".repeat(width.saturating_sub(report.id.len()) + 3)
        );
        out.push_str(&format!(
            "{:<width$} {} ({} ms)\n",
            label, verdict, report.duration_ms
        ));
        for (line, status) in &report.requests {
            match status {
                Some(status) => out.push_str(&format!("  {line} -> {status}\n")),
                None => out.push_str(&format!("  {line}\n")),
            }
        }
        for failure in &report.failures {
            out.push_str(&format!("  FAIL {failure}\n"));
        }
    }
    if reports.is_empty() {
        out.push_str("(no workflows ran)\n");
    }
    out.push_str(&format!(
        "{} passed, {} failed, {} skipped in {} ms\n",
        summary.passed, summary.failed, summary.skipped, summary.duration_ms
    ));
    out
}

/// Finds or appends the report slot for a workflow id.
fn report_for<'r>(reports: &'r mut Vec<WfReport>, wf: &str) -> &'r mut WfReport {
    if let Some(idx) = reports.iter().position(|r| r.id == wf) {
        return &mut reports[idx];
    }
    reports.push(WfReport::new(wf));
    reports.last_mut().expect("just pushed")
}

/// Renders a JUnit-XML document from the aggregate summary:
/// `<testsuites>` wrapping one `<testsuite>` whose `failures` attribute
/// mirrors [`RunSummary::failed`] and whose `skipped` attribute mirrors
/// [`RunSummary::skipped`] (`"0"` when nothing was skipped).
#[must_use]
pub fn junit(summary: &RunSummary) -> String {
    let total = summary.passed + summary.failed + summary.skipped;
    let time_s = format!("{:.3}", summary.duration_ms as f64 / 1000.0);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites tests=\"{total}\" failures=\"{}\" errors=\"0\" time=\"{time_s}\">\n",
        summary.failed
    ));
    out.push_str(&format!(
        "  <testsuite name=\"suspect\" tests=\"{total}\" failures=\"{}\" errors=\"0\" skipped=\"{}\" time=\"{time_s}\">\n",
        summary.failed, summary.skipped
    ));
    if total > 0 {
        if summary.failed > 0 {
            out.push_str(&format!(
                "    <testcase name=\"suspect-run\" classname=\"suspect\" time=\"{time_s}\">\n      <failure message=\"{} test(s) failed\"/>\n    </testcase>\n",
                summary.failed
            ));
        } else {
            out.push_str(&format!(
                "    <testcase name=\"suspect-run\" classname=\"suspect\" time=\"{time_s}\"/>\n"
            ));
        }
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

/// Renders one JSON object per event, newline-delimited.
#[must_use]
pub fn ndjson(events: &[TestEvent]) -> String {
    events
        .iter()
        .filter_map(|event| serde_json::to_string(event).ok())
        .map(|line| format!("{line}\n"))
        .collect()
}

/// One-line human rendering of a single event, for streaming printers.
#[must_use]
pub fn event_line(event: &TestEvent) -> String {
    match event {
        TestEvent::WfStarted { id } => format!("▶ workflow {id}"),
        TestEvent::StepStarted { wf, step } => format!("  · {wf}/{step}"),
        TestEvent::RequestSent { wf, step, .. } => {
            format!("  → {wf}/{step} request sent")
        }
        TestEvent::ResponseGot {
            wf,
            step,
            status,
            duration_ms,
        } => {
            format!("  ← {wf}/{step} {status} ({duration_ms}ms)")
        }
        TestEvent::CriterionOk { step, .. } => format!("    ✓ {step} criterion met"),
        TestEvent::CriterionFail {
            wf,
            step,
            crit,
            expected,
            actual,
        } => {
            format!(
                "    ✗ {wf}/{step} criterion {crit} failed: expected {expected:?}, got {actual:?}"
            )
        }
        TestEvent::OutputSet { wf, key, value } => {
            format!("    ⤷ {wf} output {key} = {value}")
        }
        TestEvent::WfDone { wf, passed } => {
            if *passed {
                format!("✔ workflow {wf} passed")
            } else {
                format!("✘ workflow {wf} failed")
            }
        }
        TestEvent::RunDone { passed, failed } => {
            format!("= run complete: {passed} passed, {failed} failed")
        }
    }
}
