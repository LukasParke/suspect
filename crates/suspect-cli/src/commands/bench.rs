//! `suspect bench`: wall-clock micro-report of the pipeline stages on one
//! fixture. Plain `std::time::Instant` timing (no criterion); each stage
//! runs `--iters` times and the mean is reported.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use suspect_lint::Linter;
use suspect_low::LowDoc;
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};

use crate::output;
use crate::OutputFormat;

/// Mean wall-clock milliseconds per pipeline stage.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    /// Fixture path as given on the command line.
    pub fixture: String,
    /// Iterations each stage was run; the mean over these is reported.
    pub iters: usize,
    /// Mean milliseconds to read and parse the fixture.
    pub parse_ms: f64,
    /// Mean milliseconds to build the low-level node model.
    pub low_model_ms: f64,
    /// Mean milliseconds to resolve `$ref`s (workspace build).
    pub resolve_ms: f64,
    /// Mean milliseconds to run the validator.
    pub validate_ms: f64,
    /// Mean milliseconds to run the linter.
    pub lint_ms: f64,
}

/// Times fallible `f` over `iters` runs; returns the mean milliseconds.
///
/// # Errors
/// Propagates the first failure from `f`.
fn mean_ms(iters: usize, mut f: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<f64> {
    let start = Instant::now();
    for _ in 0..iters {
        f()?;
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0 / iters.max(1) as f64)
}

/// Loads the fixture as a low document.
///
/// # Errors
/// IO or path canonicalization failures.
fn load_low(fixture: &Path) -> anyhow::Result<LowDoc> {
    let source = Source::from_path(fixture)?;
    let uri = Uri::from_path(fixture)?;
    Ok(LowDoc::parse(uri, source))
}

/// Benchmarks one fixture end-to-end.
///
/// # Errors
/// IO or parse failures on the fixture; workspace load failures.
pub fn bench_of(fixture: &Path, iters: usize) -> anyhow::Result<BenchReport> {
    let shown = fixture.display().to_string();
    let entry = shown.clone();

    // Stage 1: raw load (IO + decode).
    let parse_ms = mean_ms(iters, || {
        Source::from_path(fixture)?;
        Ok(())
    })?;

    // Stage 2: syntax + low model.
    let low_model_ms = mean_ms(iters, || {
        let doc = load_low(fixture)?;
        std::hint::black_box(doc.sniff_family());
        std::hint::black_box(doc.root());
        Ok(())
    })?;

    // Stage 3: full `$ref` closure.
    let resolve_ms = mean_ms(iters, || {
        let ws = WorkspaceBuilder::new().build()?;
        ws.load_all(&entry)?;
        Ok(())
    })?;

    // Stage 4: semantic validation over the loaded closure. Non-OpenAPI
    // fixtures report zero instead of failing the benchmark.
    let validate_ms = mean_ms(iters, || {
        let ws = WorkspaceBuilder::new().build()?;
        ws.load_all(&entry)?;
        let session = Session::new(Arc::new(ws));
        match suspect_validate::validate_entry(&session, &entry) {
            Ok(diags) => std::hint::black_box(diags.len()),
            Err(e) => {
                eprintln!("bench: validation unavailable: {e}");
                0
            }
        };
        Ok(())
    })?;

    // Stage 5: default ruleset lint.
    let lint_ms = mean_ms(iters, || {
        let doc = load_low(fixture)?;
        std::hint::black_box(Linter::spectral_default().run(&doc).len());
        Ok(())
    })?;

    Ok(BenchReport {
        fixture: shown,
        iters,
        parse_ms,
        low_model_ms,
        resolve_ms,
        validate_ms,
        lint_ms,
    })
}

/// `suspect bench <FIXTURE> [--iters N]`: prints the stage table or JSON.
///
/// # Errors
/// See [`bench_of`]; JSON serialization failures.
pub fn bench(fixture: &Path, iters: usize, format: OutputFormat) -> anyhow::Result<i32> {
    let report = bench_of(fixture, iters)?;
    match format {
        OutputFormat::Text => {
            println!("fixture: {} ({} iters/stage)", report.fixture, report.iters);
            println!("  parse     {:>10.2} ms", report.parse_ms);
            println!("  low model {:>10.2} ms", report.low_model_ms);
            println!("  resolve   {:>10.2} ms", report.resolve_ms);
            println!("  validate  {:>10.2} ms", report.validate_ms);
            println!("  lint      {:>10.2} ms", report.lint_ms);
        }
        OutputFormat::Json => output::print_json(&report)?,
    }
    Ok(0)
}
