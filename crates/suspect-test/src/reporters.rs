//! Pure report renderers over run results: aligned console text, JUnit XML,
//! and NDJSON event lines. All functions are side-effect free and return
//! owned strings.
//!
//! Machine-readable reports are built from [`Case`]s: one per step of a
//! workflow run ([`workflow_cases`]), or one per sequence, operation or
//! exchange for the other test commands. [`junit`] renders any list of
//! cases, so every command reports the same way.

use crate::exec::{RunSummary, TestEvent};
use crate::plan::Plan;

/// One reported test case: a step of a workflow, or whatever unit a test
/// command judges.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Case {
    /// The group the case belongs to: the workflow id for a step.
    pub suite: String,
    /// The case within its group: the step id.
    pub name: String,
    /// Time spent in the case's exchanges, in milliseconds.
    pub duration_ms: u64,
    /// What the case sent, one `METHOD URL -> status` line per request.
    pub requests: Vec<String>,
    /// How the case ended.
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// How a [`Case`] ended.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Outcome {
    /// Every check held.
    Passed,
    /// At least one check failed.
    Failed {
        /// Every failed check, in the order it was reported.
        failures: Vec<Failure>,
    },
    /// The case never ran.
    Skipped {
        /// Why it did not run.
        reason: String,
    },
}

/// One failed check of a [`Case`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Failure {
    /// The check: a success criterion's rendering, or a built-in check
    /// such as `response-schema` or `transport`.
    pub check: String,
    /// What the check wanted.
    pub expected: String,
    /// What it got.
    pub actual: String,
}

impl Failure {
    /// The one-line rendering used by every report.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{}: expected {}, got {}",
            self.check, self.expected, self.actual
        )
    }
}

impl Case {
    /// A passing case with no recorded requests.
    #[must_use]
    pub fn passed(suite: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            suite: suite.into(),
            name: name.into(),
            duration_ms: 0,
            requests: Vec::new(),
            outcome: Outcome::Passed,
        }
    }

    /// A case that failed one check.
    #[must_use]
    pub fn failed(
        suite: impl Into<String>,
        name: impl Into<String>,
        check: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            outcome: Outcome::Failed {
                failures: vec![Failure {
                    check: check.into(),
                    expected: expected.into(),
                    actual: actual.into(),
                }],
            },
            ..Self::passed(suite, name)
        }
    }

    /// Whether the case failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self.outcome, Outcome::Failed { .. })
    }

    /// Whether the case never ran.
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        matches!(self.outcome, Outcome::Skipped { .. })
    }
}

/// One [`Case`] per step of every workflow in `plan`, judged from the
/// event log of its run: failed when any check of the step failed, passed
/// when it ran and none did, skipped when it never started (an earlier
/// step of its workflow failed, or an action ended the workflow or jumped
/// past it). A retried step is judged by its last attempt; a step an
/// `onFailure` goto re-ran is one case, failed if any run of it failed.
#[must_use]
pub fn workflow_cases(plan: &Plan, events: &[TestEvent]) -> Vec<Case> {
    struct Seen {
        started: bool,
        duration_ms: u64,
        requests: Vec<String>,
        failures: Vec<Failure>,
    }
    let mut cases: Vec<(String, String, Seen)> = plan
        .workflows
        .iter()
        .flat_map(|wf| {
            wf.steps.iter().map(|step| {
                (
                    wf.workflow_id.clone(),
                    step.step_id.clone(),
                    Seen {
                        started: false,
                        duration_ms: 0,
                        requests: Vec::new(),
                        failures: Vec::new(),
                    },
                )
            })
        })
        .collect();
    let mut slot = |wf: &str, step: &str| -> usize {
        if let Some(index) = cases
            .iter()
            .position(|(suite, name, _)| suite == wf && name == step)
        {
            return index;
        }
        // An event for a step the plan does not list still gets reported.
        cases.push((
            wf.to_owned(),
            step.to_owned(),
            Seen {
                started: false,
                duration_ms: 0,
                requests: Vec::new(),
                failures: Vec::new(),
            },
        ));
        cases.len() - 1
    };
    // Events address their step by ids, so each is applied through the
    // index `slot` resolves.
    let mut updates: Vec<(usize, &TestEvent)> = Vec::new();
    for event in events {
        let key = match event {
            TestEvent::StepStarted { wf, step }
            | TestEvent::RequestSent { wf, step, .. }
            | TestEvent::ResponseGot { wf, step, .. }
            | TestEvent::StepRetried { wf, step, .. }
            | TestEvent::CriterionFail { wf, step, .. } => Some((wf, step)),
            _ => None,
        };
        if let Some((wf, step)) = key {
            updates.push((slot(wf, step), event));
        }
    }
    for (index, event) in updates {
        let seen = &mut cases[index].2;
        match event {
            TestEvent::StepStarted { .. } => seen.started = true,
            // Only the last attempt decides a retried step.
            TestEvent::StepRetried { .. } => seen.failures.clear(),
            TestEvent::RequestSent { method, url, .. } => {
                seen.requests.push(format!("{method} {url}"));
            }
            TestEvent::ResponseGot {
                status,
                duration_ms,
                ..
            } => {
                if let Some(last) = seen.requests.last_mut() {
                    last.push_str(&format!(" -> {status}"));
                }
                seen.duration_ms += duration_ms;
            }
            TestEvent::CriterionFail {
                crit,
                expected,
                actual,
                ..
            } => seen.failures.push(Failure {
                check: crit.clone(),
                expected: expected.clone(),
                actual: actual.clone(),
            }),
            _ => {}
        }
    }
    cases
        .into_iter()
        .map(|(suite, name, seen)| Case {
            suite,
            name,
            duration_ms: seen.duration_ms,
            requests: seen.requests,
            outcome: if !seen.failures.is_empty() {
                Outcome::Failed {
                    failures: seen.failures,
                }
            } else if seen.started {
                Outcome::Passed
            } else {
                Outcome::Skipped {
                    reason: "not run: the workflow ended or moved on before this step".to_owned(),
                }
            },
        })
        .collect()
}

/// Per-workflow view assembled from the event log.
struct WfReport {
    id: String,
    passed: bool,
    duration_ms: u64,
    /// `(rendered line, optional status)` pairs, one per executed request.
    requests: Vec<(String, Option<u16>)>,
    /// `(step id, rendered failure)` pairs.
    failures: Vec<(String, String)>,
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
                step,
                crit,
                expected,
                actual,
            } => {
                let report = report_for(&mut reports, wf);
                report.failures.push((
                    step.clone(),
                    format!("{crit}: expected {expected}, got {actual}"),
                ));
                report.passed = false;
            }
            // The attempt being retried is replaced, failures and all.
            TestEvent::StepRetried { wf, step, .. } => {
                report_for(&mut reports, wf)
                    .failures
                    .retain(|(failed, _)| failed != step);
            }
            // The runner's verdict is the workflow's verdict.
            TestEvent::WfDone { wf, passed } => {
                report_for(&mut reports, wf).passed = *passed;
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
        for (_, failure) in &report.failures {
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

/// Renders a JUnit-XML document: `<testsuites>` named `name`, one
/// `<testsuite>` per [`Case::suite`] in first-appearance order, and one
/// `<testcase>` per case. A failed case carries a `<failure>` whose
/// message is its first failed check and whose text lists every failed
/// check and the requests the case sent; a case that never ran carries
/// `<skipped>`. Counts on every element are counts of the test cases
/// under it, so the document is consistent for any consumer.
#[must_use]
pub fn junit(name: &str, cases: &[Case], duration_ms: u64) -> String {
    let seconds = |ms: u64| format!("{:.3}", ms as f64 / 1000.0);
    let counts = |cases: &[&Case]| {
        (
            cases.len(),
            cases.iter().filter(|case| case.is_failed()).count(),
            cases.iter().filter(|case| case.is_skipped()).count(),
        )
    };
    let mut suites: Vec<(&str, Vec<&Case>)> = Vec::new();
    for case in cases {
        match suites.iter_mut().find(|(suite, _)| *suite == case.suite) {
            Some((_, members)) => members.push(case),
            None => suites.push((&case.suite, vec![case])),
        }
    }
    let all: Vec<&Case> = cases.iter().collect();
    let (tests, failures, skipped) = counts(&all);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"{}\" tests=\"{tests}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{}\">\n",
        xml_escape(name),
        seconds(duration_ms)
    ));
    for (suite, members) in &suites {
        let (tests, failures, skipped) = counts(members);
        let time: u64 = members.iter().map(|case| case.duration_ms).sum();
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{tests}\" failures=\"{failures}\" errors=\"0\" skipped=\"{skipped}\" time=\"{}\">\n",
            xml_escape(suite),
            seconds(time)
        ));
        for case in members {
            let open = format!(
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"",
                xml_escape(&case.name),
                xml_escape(suite),
                seconds(case.duration_ms)
            );
            match &case.outcome {
                Outcome::Passed => out.push_str(&format!("{open}/>\n")),
                Outcome::Skipped { reason } => out.push_str(&format!(
                    "{open}>\n      <skipped message=\"{}\"/>\n    </testcase>\n",
                    xml_escape(reason)
                )),
                Outcome::Failed { failures } => {
                    let first = failures.first();
                    let mut text: Vec<String> = failures.iter().map(Failure::line).collect();
                    text.extend(case.requests.iter().map(|request| format!("  {request}")));
                    out.push_str(&format!(
                        "{open}>\n      <failure message=\"{}\" type=\"{}\">{}</failure>\n    </testcase>\n",
                        xml_escape(&first.map(Failure::line).unwrap_or_default()),
                        xml_escape(first.map_or("failure", |failure| failure.check.as_str())),
                        xml_escape(&text.join("\n"))
                    ));
                }
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// Escapes text for an XML attribute or element body. Characters XML 1.0
/// cannot carry at all (most control characters) become spaces, so a
/// binary response quoted in a failure never makes the report unparseable.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' | '\t' | '\r' => out.push(ch),
            ch if (ch as u32) < 0x20 || matches!(ch as u32, 0xFFFE | 0xFFFF) => out.push(' '),
            ch => out.push(ch),
        }
    }
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
        TestEvent::ResponseValidated { wf, step, status } => {
            format!("  ✓ {wf}/{step} response {status} matches contract")
        }
        TestEvent::ResponseGot {
            wf,
            step,
            status,
            duration_ms,
        } => {
            format!("  ← {wf}/{step} {status} ({duration_ms}ms)")
        }
        TestEvent::CriterionOk { step, crit, .. } => format!("    ✓ {step} {crit}"),
        TestEvent::CriterionFail {
            wf,
            step,
            crit,
            expected,
            actual,
        } => {
            format!("    ✗ {wf}/{step} {crit}: expected {expected}, got {actual}")
        }
        TestEvent::StepRetried {
            wf,
            step,
            attempt,
            limit,
            delay_ms,
        } => {
            format!("  ↻ {wf}/{step} retry {attempt} of {limit} after {delay_ms}ms")
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
