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

use crate::OutputFormat;
use crate::output;

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
    /// Mean milliseconds for an LSP-style edit cycle: incremental reparse of
    /// a 1 KB insertion into the parsed document.
    pub lsp_edit_ms: f64,
    /// Mean milliseconds to compile every component schema (JSON Schema
    /// 2020-12 program construction); zero when the document has none.
    pub schema_compile_ms: f64,
    /// Mean milliseconds to evaluate `$..['$ref']` over the whole document.
    pub jsonpath_ms: f64,
    /// Mean milliseconds to apply a 5-action overlay to the document
    /// (skipped as zero when the target cannot take updates).
    pub overlay_ms: f64,
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

    // Stage 6: LSP edit cycle — incremental reparse after a 1 KB insertion
    // at 40% of the document. Mirrors didChange handling in the server.
    let lsp_edit_ms = {
        let source = Source::from_path(fixture)?;
        let uri = Uri::from_path(fixture)?;
        let base: Vec<u8> = source.bytes().to_vec();
        let doc = suspect_syntax::SourceDoc::parse(uri.clone(), Source::from_vec(base.clone()));
        let len = base.len();
        let at = len * 2 / 5;
        // 1 KB whitespace insertion: structurally inert, exactly 1024 bytes,
        // so Edit::from_bytes' point math stays exact.
        let insertion = vec![b' '; 1024];
        let mut edited: Vec<u8> = Vec::with_capacity(len + insertion.len());
        edited.extend_from_slice(&base[..at]);
        edited.extend_from_slice(&insertion);
        edited.extend_from_slice(&base[at..]);
        let edit = suspect_syntax::Edit::from_bytes(&doc, at, at, insertion.len());
        mean_ms(iters, || {
            // Buffer hand-off mirrors what the server pays per didChange.
            let reparsed = doc.reparse(
                Source::from_vec(edited.clone()),
                std::slice::from_ref(&edit),
            );
            std::hint::black_box(reparsed.has_errors());
            Ok(())
        })?
    };

    // Stage 7: JSON Schema compile of all component schemas.
    let schema_compile_ms = mean_ms(iters, || {
        let doc = load_low(fixture)?;
        let schemas = doc.root().get("components").and_then(|c| c.get("schemas"));
        let Some(schemas) = schemas else {
            return Ok(());
        };
        let compiler = suspect_schema::Compiler::new(suspect_schema::Config::default());
        let mut compiled = 0usize;
        for (_name, node) in schemas
            .entries()
            .into_iter()
            .filter_map(|e| e.value.map(|v| (e.key, v)))
        {
            if compiler.compile(node).is_ok() {
                compiled += 1;
            }
        }
        std::hint::black_box(compiled);
        Ok(())
    })?;

    // Stage 8: descendant $ref query across the document.
    let jsonpath_ms = mean_ms(iters, || {
        let doc = load_low(fixture)?;
        let query = suspect_jsonpath::Path::parse("$..['$ref']")?;
        std::hint::black_box(query.query(doc.root()).len());
        Ok(())
    })?;

    // Stage 9: overlay apply — five update actions against the document root.
    let overlay_doc_src = b"overlay: 1.0.0\ninfo: {title: b, version: \"1\"}\nactions:\n".to_vec();
    let overlay_ms = mean_ms(iters, || {
        let doc = load_low(fixture)?;
        let overlay_text = overlay_doc_src.clone();
        let mut actions = String::from(
            "  - target: $.info\n    update:\n      x-a: 1\n  - target: $.info\n    update:\n      x-b: 2\n",
        );
        actions.push_str("  - target: $.paths.*.get\n    update:\n      x-c: true\n");
        actions.push_str("  - target: $.tags\n    update:\n      - name: bench\n  - target: $.servers\n    remove: true\n");
        let mut full = overlay_text;
        full.extend_from_slice(actions.as_bytes());
        let overlay_low = LowDoc::parse(
            Uri::from("mem://bench-overlay.yaml"),
            Source::from_vec(full),
        );
        let overlay = match suspect_overlay::OverlayDoc::parse(&overlay_low) {
            Ok(o) => o,
            Err(_) => return Ok(()),
        };
        match suspect_overlay::apply(&overlay, doc.root()) {
            Ok(applied) => std::hint::black_box(applied.applied_actions),
            Err(_) => 0,
        };
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
        lsp_edit_ms,
        schema_compile_ms,
        jsonpath_ms,
        overlay_ms,
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
            println!("  lsp edit  {:>10.2} ms", report.lsp_edit_ms);
            println!("  schema    {:>10.2} ms", report.schema_compile_ms);
            println!("  jsonpath  {:>10.2} ms", report.jsonpath_ms);
            println!("  overlay   {:>10.2} ms", report.overlay_ms);
        }
        OutputFormat::Json => output::print_json(&report)?,
    }
    Ok(0)
}
