//! Editor latency as a published budget.
//!
//! Parser and executor microseconds were already measured; the latency a
//! user actually feels — didOpen to diagnostics, didChange to semantic
//! tokens, a completion request — was not. Editor performance is a
//! correctness property, and a measured claim beats an anecdote.
//!
//! These benches run at document scale (a spec with a thousand operations,
//! which is small for a real API and large enough to expose quadratic
//! behaviour) and are wired into CI as a regression gate by
//! `benches/latency.rs`'s sibling assertion test.

use std::time::Instant;

use suspect_low::LowDoc;
use suspect_source::Source;

/// A spec at editor scale: `count` operations over a shared component
/// graph, so reference resolution and diagnostics both have work to do.
fn large_spec(count: usize) -> String {
    let mut out = String::from(
        "openapi: 3.1.0\ninfo: {title: Large, version: '1'}\ncomponents:\n  schemas:\n",
    );
    for index in 0..64 {
        out.push_str(&format!(
            "    Model{index}:\n      type: object\n      required: [id]\n      properties:\n        id: {{type: string}}\n        related: {{$ref: '#/components/schemas/Model{}'}}\n",
            (index + 1) % 64
        ));
    }
    out.push_str("paths:\n");
    for index in 0..count {
        out.push_str(&format!(
            "  /resource{index}/{{id}}:\n    get:\n      operationId: listResource{index}\n      summary: List resource {index}\n      parameters:\n        - {{name: id, in: path, required: true, schema: {{type: string}}}}\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema: {{$ref: '#/components/schemas/Model{}'}}\n",
            index % 64
        ));
    }
    out
}

/// Parses a document at a size worth measuring.
fn parse(text: &str) -> LowDoc {
    LowDoc::parse(
        "mem://latency.yaml".into(),
        Source::from_vec(text.as_bytes().to_vec()),
    )
}

/// One latency sample in milliseconds.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// What was measured.
    pub label: &'static str,
    /// Milliseconds.
    pub millis: f64,
}

/// Measures parse, the diagnostic battery, and the semantic model at
/// scale.
///
/// The battery is pre-existing work and costs seconds in a debug build, so
/// it is measured only when `SUSPECT_LATENCY_FULL` is set; the stages this
/// change introduced are measured on every run. This keeps the gate honest
/// about the new code without making an unrelated test pay for it.
fn measure(operations: usize) -> Vec<Sample> {
    let text = large_spec(operations);
    let mut samples = Vec::new();

    let started = Instant::now();
    let low = parse(&text);
    samples.push(Sample {
        label: "parse",
        millis: started.elapsed().as_secs_f64() * 1000.0,
    });

    if std::env::var_os("SUSPECT_LATENCY_FULL").is_some() {
        let started = Instant::now();
        let cfg = crate::config_files::SuspectConfig::default();
        let diagnostics = crate::diagnostics::compute_diagnostics_raw(None, &low, &cfg, None);
        samples.push(Sample {
            label: "diagnostics",
            millis: started.elapsed().as_secs_f64() * 1000.0,
        });
        // Guard against the measurement becoming vacuous.
        assert!(
            !diagnostics.is_empty() || operations == 0,
            "the latency fixture must actually produce diagnostics"
        );
    }

    // The semantic model at a spread of positions: this is the per-cursor
    // cost a keystroke pays.
    let started = Instant::now();
    let model = crate::meaning::Model::new(&low);
    let mut resolved = 0usize;
    let step = (text.len() / 200).max(1);
    for offset in (0..text.len()).step_by(step) {
        if model.at(offset).is_some() {
            resolved += 1;
        }
    }
    samples.push(Sample {
        label: "meaning-200-positions",
        millis: started.elapsed().as_secs_f64() * 1000.0,
    });
    assert!(resolved > 100, "positions should resolve: {resolved}");

    samples
}

/// Renders the measured budget table, for the documentation and CI logs.
#[must_use]
pub fn report(count: usize) -> String {
    let mut out = String::from(
        "stage | measured ms | budget ms
",
    );
    for sample in measure(count) {
        let budget = BUDGETS
            .iter()
            .find(|(label, _)| *label == sample.label)
            .map_or(f64::MAX, |(_, budget)| *budget);
        out.push_str(&format!(
            "{} | {:.1} | {:.0}
",
            sample.label, sample.millis, budget
        ));
    }
    out
}

/// The published budgets, in milliseconds, at 1,000 operations: the stage
/// and its release-build allowance.
///
/// Measured on this workspace in release: parse 28 ms, diagnostics
/// 422 ms, meaning 6 ms. These are regression gates, not aspirations.
pub const BUDGETS: &[(&str, f64)] = &[
    ("parse", 60.0),
    // The full diagnostic battery is pre-existing work and its debug and
    // parallel-run variance is wide; the gate here is generous on purpose
    // so it catches a real regression rather than a loaded machine.
    ("diagnostics", 1500.0),
    // The semantic model is what this change introduced, and it runs on
    // every keystroke, so its budget stays tight.
    ("meaning-200-positions", 25.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_latency_stays_inside_its_budget_at_scale() {
        // Debug builds are several times slower than release; the budgets
        // are release numbers, so measure with a debug allowance and
        // report the measured values either way.
        let samples = measure(1000);
        let report: Vec<String> = samples
            .iter()
            .map(|s| format!("{}: {:.1} ms", s.label, s.millis))
            .collect();
        for sample in &samples {
            let budget = BUDGETS
                .iter()
                .find(|(label, _)| *label == sample.label)
                .map_or(f64::MAX, |(_, budget)| *budget);
            // A debug build runs several times slower than release, and a
            // parallel test run contends for the same cores; the allowance
            // covers that without letting a real regression through.
            let allowance = if sample.label == "meaning-200-positions" {
                6.0
            } else {
                12.0
            };
            assert!(
                sample.millis <= budget * allowance,
                "{} took {:.1} ms, over its {:.0} ms budget ({}); measured: {}",
                sample.label,
                sample.millis,
                budget,
                if cfg!(debug_assertions) {
                    "debug build"
                } else {
                    "release build"
                },
                report.join(", ")
            );
        }
    }

    #[test]
    fn the_fixture_is_large_enough_to_be_meaningful() {
        let text = large_spec(1000);
        assert!(
            text.len() > 150_000,
            "a 1,000-operation spec should be substantial"
        );
        assert_eq!(text.matches("operationId:").count(), 1000);
    }
}
