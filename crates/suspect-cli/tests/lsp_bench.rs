//! Benchmarking the language server under editor-shaped load.
//!
//! Each scenario here is a thing an editor actually does, replayed in the
//! order and volume an editor does it. The measurements answer a question a
//! unit test cannot: not "does this request work" but "what does a person
//! experience while it works".
//!
//! Run with `--nocapture` to see the tables. Results are also written to
//! `target/lsp-bench/<scenario>.json` for comparing two runs.
//!
//! # Load shape
//!
//! | scenario | what it models |
//! |---|---|
//! | `open_burst` | the recorded VS Code open, replayed |
//! | `cursor_sweep` | moving the cursor: hover, definition, highlight, completion |
//! | `scroll_flood` | scrolling: ranged tokens, inlay hints, selection ranges |
//! | `symbol_poll` | an open file with outlines visible: symbols, folds, lenses, links |
//! | `edit_churn` | typing and saving: didChange, save, diagnostics re-pull |
//! | `mixed_load` | all of the above interleaved, which is a working editing session |
//!
//! Thresholds are asserted so a regression is a test failure rather than
//! something noticed months later. They are deliberately loose — far above
//! what this fixture measures — because their job is to catch a change in
//! order of magnitude, not to police jitter.

// The harness modules compile into more than one test binary, so an item
// used by the session suite is unused in the bench suite and vice versa.
#[allow(dead_code)]
mod support {
    pub mod bench;
    pub mod editor;
    pub mod workspace;
}

use std::time::{Duration, Instant};

use serde_json::Value;
use support::bench::Bench;
use support::editor::{Editor, Timed, editor_capabilities, url_of};
use support::workspace::Workspace;

fn doc(uri: &str) -> Value {
    serde_json::json!({"textDocument": {"uri": uri}})
}

fn at(line: usize, column: usize) -> Value {
    serde_json::json!({"line": line - 1, "character": column - 1})
}

/// Where the interesting positions are in the generated workspace.
struct Site {
    uri: String,
    operation_id: (usize, usize),
    cross_file_ref: (usize, usize),
    doc_lines: usize,
}

fn site(ws: &Workspace) -> Site {
    let (ol, oc) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let (rl, rc) = ws.locate(
        &ws.openapi,
        "$ref: 'schemas.yaml#/components/schemas/Accounts'",
    );
    Site {
        uri: url_of(&ws.openapi),
        operation_id: (ol, oc + "operationId: ".len()),
        cross_file_ref: (rl, rc + "$ref: 'schemas.yaml#/components/schem".len()),
        doc_lines: std::fs::read_to_string(&ws.openapi)
            .map(|t| t.lines().count())
            .unwrap_or(2_000),
    }
}

/// The directory to measure.
///
/// Defaults to the generated fixture, which keeps the suite hermetic and
/// fast. Point `SUSPECT_BENCH_ROOT` at a real project to measure that
/// instead — which is the only way to see how the numbers behave at the
/// scale where the deadlocks actually appeared:
///
/// ```text
/// SUSPECT_BENCH_ROOT=/Users/luke/github/plex-api-spec \
///   cargo test -p suspect-cli --test lsp_bench -- --nocapture
/// ```
fn bench_root() -> Option<std::path::PathBuf> {
    std::env::var_os("SUSPECT_BENCH_ROOT").map(std::path::PathBuf::from)
}

/// Opens a real project's specification documents, up to a sane cap.
fn real_documents(root: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let mut out: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                if matches!(
                    name.as_deref(),
                    Some("node_modules" | ".git" | ".suspect" | "target")
                ) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let is_spec = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yaml" | "yml" | "json")
            );
            if !is_spec {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                out.push((path, text));
            }
        }
    }
    // Largest first: the specification, not a lockfile.
    out.sort_by_key(|(_, text)| std::cmp::Reverse(text.len()));
    out.truncate(8);
    out
}

/// A session with the workspace open and the index warm, as an editor has it
/// after the first second of looking at a file.
fn warmed() -> (Workspace, Editor, Site) {
    match bench_root() {
        Some(root) => {
            let editor = Editor::start(&root, editor_capabilities());
            for (path, text) in real_documents(&root) {
                editor.open(&path, &text);
            }
            std::thread::sleep(Duration::from_millis(1_500));
            // Positions are found against the biggest document, whatever it
            // turned out to be.
            let biggest = real_documents(&root)
                .into_iter()
                .next()
                .map(|(path, _)| path)
                .unwrap_or_else(|| root.join("openapi.yaml"));
            let ws = Workspace {
                root: root.clone(),
                openapi: biggest,
                schemas: root.join("schemas.yaml"),
                overlay: root.join("overlays/overlay.yaml"),
                workflow: root.join("workflows/health.arazzo.yaml"),
            };
            let site = site_fallback(&ws);
            (ws, editor, site)
        }
        None => {
            let dir = tempfile::tempdir().expect("tempdir");
            let ws = Workspace::build(dir.path());
            let editor = Editor::start(&ws.root, editor_capabilities());
            for (path, text) in ws.files() {
                editor.open(&path, &text);
            }
            std::thread::sleep(Duration::from_millis(900));
            let site = site(&ws);
            std::mem::forget(dir);
            (ws, editor, site)
        }
    }
}

/// Site discovery that tolerates a real project not matching the generator's
/// exact spelling: find the first `operationId` and the first cross-file
/// `$ref`, and fall back to the middle of the document.
fn site_fallback(ws: &Workspace) -> Site {
    let Ok(text) = std::fs::read_to_string(&ws.openapi) else {
        return Site {
            uri: url_of(&ws.openapi),
            operation_id: (20, 10),
            cross_file_ref: (20, 10),
            doc_lines: 2_000,
        };
    };
    let first = |needle: &str| -> (usize, usize) {
        match text.find(needle) {
            Some(offset) => {
                let line = text[..offset].lines().count().max(1);
                let column = offset - text[..offset].rfind('\n').map_or(0, |n| n + 1);
                (line, column + 1)
            }
            None => (20, 10),
        }
    };
    let (ol, oc) = first("operationId:");
    let (rl, rc) = first("$ref:");
    Site {
        uri: url_of(&ws.openapi),
        operation_id: (ol, oc + "operationId: ".len().min(oc).min(13)),
        cross_file_ref: (rl, rc + 20),
        doc_lines: text.lines().count(),
    }
}

/// Records a timed request into the bench.
fn timed(bench: &mut Bench, editor: &Editor, scenario: &'static str, method: &str, params: Value) {
    let (answer, elapsed) = editor.request_timed(method, params);
    let answer = answer.map_err(|e| e.to_string());
    bench.record(scenario, method, elapsed, &answer);
}

fn record(bench: &mut Bench, scenario: &'static str, batch: &[Timed]) {
    bench.record_batch(scenario, batch);
}

/// The recorded VS Code open, as the editor issues it: fifteen requests and a
/// `didOpen` inside one short window.
#[test]
fn bench_open_burst() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    for _ in 0..10 {
        editor.notify(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": site.uri, "version": 1}, "contentChanges": [],
            }),
        );
        let burst = vec![
            ("textDocument/diagnostic".to_owned(), doc(&site.uri)),
            (
                "workspace/diagnostic".to_owned(),
                serde_json::json!({"previousResultIds": []}),
            ),
            ("textDocument/documentSymbol".to_owned(), doc(&site.uri)),
            ("textDocument/foldingRange".to_owned(), doc(&site.uri)),
            (
                "textDocument/semanticTokens/full".to_owned(),
                doc(&site.uri),
            ),
            ("textDocument/documentLink".to_owned(), doc(&site.uri)),
            (
                "textDocument/inlayHint".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri},
                    "range": {"start": at(1, 1), "end": at(400, 1)},
                }),
            ),
            ("textDocument/codeLens".to_owned(), doc(&site.uri)),
            ("textDocument/documentColor".to_owned(), doc(&site.uri)),
            (
                "textDocument/hover".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.operation_id.0, site.operation_id.1),
                }),
            ),
            (
                "textDocument/codeAction".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri},
                    "range": {"start": at(1, 1), "end": at(2, 1)},
                    "context": {"diagnostics": []},
                }),
            ),
        ];
        record(&mut bench, "open_burst", &editor.request_all_timed(&burst));
    }
    finish(&bench, "open_burst", 10);
    assert_healthy(&bench, "open_burst");
    assert_under(&bench, "textDocument/hover", 2_000);
    assert_under(&bench, "textDocument/documentSymbol", 4_000);
}

/// Moving the cursor up and down the document, which is what a person does
/// while reading.
#[test]
fn bench_cursor_sweep() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    // Two passes over the document, stopping every 25 lines: hover plus the
    // navigation an editor requests for a peek.
    let max_stops: usize = std::env::var("SUSPECT_BENCH_STOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    for pass in 0..2 {
        let mut line = 20;
        let mut stops = 0usize;
        while line < site.doc_lines && stops < max_stops {
            stops += 1;
            for column in [3usize, 9, 17] {
                timed(
                    &mut bench,
                    &editor,
                    "cursor_sweep",
                    "textDocument/hover",
                    serde_json::json!({
                        "textDocument": {"uri": site.uri}, "position": at(line, column),
                    }),
                );
                timed(
                    &mut bench,
                    &editor,
                    "cursor_sweep",
                    "textDocument/definition",
                    serde_json::json!({
                        "textDocument": {"uri": site.uri}, "position": at(line, column),
                    }),
                );
                timed(
                    &mut bench,
                    &editor,
                    "cursor_sweep",
                    "textDocument/documentHighlight",
                    serde_json::json!({
                        "textDocument": {"uri": site.uri}, "position": at(line, column),
                    }),
                );
                timed(
                    &mut bench,
                    &editor,
                    "cursor_sweep",
                    "textDocument/prepareRename",
                    serde_json::json!({
                        "textDocument": {"uri": site.uri}, "position": at(line, column),
                    }),
                );
            }
            line += 25;
        }
        editor.notify(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": site.uri, "version": 2 + pass}, "contentChanges": [],
            }),
        );
    }
    finish(&bench, "cursor_sweep", 2);
    assert_healthy(&bench, "cursor_sweep");
    assert_under(&bench, "textDocument/hover", 1_000);
    assert_under(&bench, "textDocument/definition", 2_000);
}

/// Scrolling: the viewport moves, and an editor re-requests ranged tokens,
/// hints and selection ranges for what is now visible.
#[test]
fn bench_scroll_flood() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    let viewport = 60;
    // Bounded: on a 63,000-line document an unbounded sweep is thousands of
    // windows, which is a soak rather than a benchmark and made this test
    // run for the better part of an hour against a real project.
    let max_windows: usize = std::env::var("SUSPECT_BENCH_WINDOWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let mut top = 1;
    let mut windows = 0usize;
    while top + viewport < site.doc_lines && windows < max_windows {
        windows += 1;
        let start = at(top, 1);
        let end = at(top + viewport, 1);
        let batch = vec![
            (
                "textDocument/semanticTokens/range".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "range": {"start": start, "end": end},
                }),
            ),
            (
                "textDocument/inlayHint".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "range": {"start": start, "end": end},
                }),
            ),
            (
                "textDocument/selectionRange".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "positions": [start],
                }),
            ),
            (
                "textDocument/documentHighlight".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": start,
                }),
            ),
        ];
        record(
            &mut bench,
            "scroll_flood",
            &editor.request_all_timed(&batch),
        );
        top += viewport / 2;
    }
    finish(&bench, "scroll_flood", windows);
    assert_healthy(&bench, "scroll_flood");
    assert_under(&bench, "textDocument/semanticTokens/range", 3_000);
    assert_under(&bench, "textDocument/inlayHint", 2_000);
}

/// A file left open with the outline, folds and reference lists visible: the
/// whole-document requests an editor makes once and then keeps alive.
#[test]
fn bench_symbol_poll() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    for _ in 0..20 {
        let batch = vec![
            ("textDocument/documentSymbol".to_owned(), doc(&site.uri)),
            ("textDocument/foldingRange".to_owned(), doc(&site.uri)),
            ("textDocument/codeLens".to_owned(), doc(&site.uri)),
            ("textDocument/documentLink".to_owned(), doc(&site.uri)),
            (
                "textDocument/semanticTokens/full".to_owned(),
                doc(&site.uri),
            ),
            ("textDocument/documentColor".to_owned(), doc(&site.uri)),
        ];
        record(&mut bench, "symbol_poll", &editor.request_all_timed(&batch));
    }
    finish(&bench, "symbol_poll", 20);
    assert_healthy(&bench, "symbol_poll");
    assert_under(&bench, "textDocument/documentLink", 5_000);
    assert_under(&bench, "textDocument/semanticTokens/full", 5_000);
}

/// Typing and saving, which drops the cached workspace and forces a rebuild on
/// the next request — the most expensive thing a user does repeatedly.
#[test]
fn bench_edit_churn() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    for round in 2..12 {
        editor.notify_timed(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": site.uri, "version": round},
                "contentChanges": [{
                    "range": {"start": at(site.operation_id.0, site.operation_id.1),
                              "end": at(site.operation_id.0, site.operation_id.1)},
                    "text": "x",
                }],
            }),
        );
        timed(
            &mut bench,
            &editor,
            "edit_churn",
            "textDocument/diagnostic",
            doc(&site.uri),
        );
        timed(
            &mut bench,
            &editor,
            "edit_churn",
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": site.uri}, "position": at(site.operation_id.0, site.operation_id.1),
            }),
        );
        editor.notify_timed(
            "textDocument/didSave",
            serde_json::json!({
                "textDocument": {"uri": site.uri}
            }),
        );
        let batch = vec![
            ("textDocument/documentSymbol".to_owned(), doc(&site.uri)),
            ("textDocument/diagnostic".to_owned(), doc(&site.uri)),
        ];
        record(&mut bench, "edit_churn", &editor.request_all_timed(&batch));
    }
    finish(&bench, "edit_churn", 10);
    assert_healthy(&bench, "edit_churn");
    // Generous: the full lint pass over a large document is seconds, and
    // this bound is here to catch an order of magnitude, not to police it.
    assert_under(&bench, "textDocument/diagnostic", 60_000);
}

/// Everything at once, which is what an editing session actually looks like.
///
/// This is the scenario with teeth: interleaving whole-document requests,
/// ranged requests, navigation and notifications is the load that wedged the
/// server three separate times.
#[test]
fn bench_mixed_load() {
    let (ws, editor, site) = warmed();
    let mut bench = Bench::new();
    let started = Instant::now();
    let mut round = 0usize;

    let budget = std::env::var("SUSPECT_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20u64);
    while started.elapsed() < Duration::from_secs(budget) {
        round += 1;
        // The editor's whole-document set, in flight together.
        let batch = vec![
            ("textDocument/documentSymbol".to_owned(), doc(&site.uri)),
            ("textDocument/foldingRange".to_owned(), doc(&site.uri)),
            ("textDocument/codeLens".to_owned(), doc(&site.uri)),
            ("textDocument/documentLink".to_owned(), doc(&site.uri)),
            (
                "textDocument/semanticTokens/full".to_owned(),
                doc(&site.uri),
            ),
            (
                "textDocument/inlayHint".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri},
                    "range": {"start": at(1, 1), "end": at(round.min(site.doc_lines), 1)},
                }),
            ),
            (
                "textDocument/hover".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.operation_id.0, site.operation_id.1),
                }),
            ),
            (
                "textDocument/definition".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.cross_file_ref.0, site.cross_file_ref.1),
                }),
            ),
            (
                "textDocument/references".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.cross_file_ref.0, site.cross_file_ref.1),
                    "context": {"includeDeclaration": true},
                }),
            ),
            (
                "textDocument/completion".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.cross_file_ref.0, site.cross_file_ref.1),
                }),
            ),
            ("textDocument/diagnostic".to_owned(), doc(&site.uri)),
            (
                "workspace/diagnostic".to_owned(),
                serde_json::json!({"previousResultIds": []}),
            ),
        ];
        record(&mut bench, "mixed_load", &editor.request_all_timed(&batch));

        // A second document, as a multi-file workspace would have.
        let schemas = url_of(&ws.schemas);
        let other = vec![
            ("textDocument/documentSymbol".to_owned(), doc(&schemas)),
            (
                "textDocument/hover".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": schemas}, "position": at(30, 8),
                }),
            ),
        ];
        record(&mut bench, "mixed_load", &editor.request_all_timed(&other));

        // Notifications interleaved with the traffic above.
        if round.is_multiple_of(3) {
            editor.notify_timed(
                "textDocument/didChange",
                serde_json::json!({
                    "textDocument": {"uri": site.uri, "version": 100 + round},
                    "contentChanges": [],
                }),
            );
        }
        if round.is_multiple_of(5) {
            editor.notify_timed(
                "textDocument/didSave",
                serde_json::json!({
                    "textDocument": {"uri": site.uri}
                }),
            );
        }
        if round.is_multiple_of(7) {
            editor.notify_timed(
                "workspace/didChangeConfiguration",
                serde_json::json!({
                    "settings": {"suspect": {"lint": {"min_severity": "warning"}}}
                }),
            );
        }
    }

    let elapsed = started.elapsed();
    println!(
        "\n  mixed_load: {round} rounds in {:.1}s ({} requests, {:.0} req/s)",
        elapsed.as_secs_f64(),
        bench.count(),
        bench.count() as f64 / elapsed.as_secs_f64().max(0.001)
    );
    finish(&bench, "mixed_load", round);
    assert_healthy(&bench, "mixed_load");
    // Rounds, not samples: at 63,000 lines a single round costs seconds
    // because the lint pass dominates, and a sample-count floor written for
    // the generated fixture would fail for the wrong reason.
    assert!(
        round >= 3,
        "the flood completed only {round} rounds, which is too few to mean anything"
    );
    assert!(
        bench.count() >= round * 12,
        "expected at least twelve measured requests per round, got {} over {round} rounds",
        bench.count()
    );
    assert_under(&bench, "textDocument/hover", 2_000);
    assert_under(&bench, "textDocument/documentSymbol", 5_000);
    assert_under(&bench, "textDocument/documentLink", 5_000);
}

/// A longer soak, off the default test path.
///
/// Run with `cargo test -p suspect-cli --test lsp_bench -- --ignored --nocapture`.
#[test]
#[ignore = "a two-minute flood; run deliberately"]
fn bench_soak() {
    let (_ws, editor, site) = warmed();
    let mut bench = Bench::new();
    let started = Instant::now();
    let mut round = 0usize;
    let soak_seconds = std::env::var("SUSPECT_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120u64);
    while started.elapsed() < Duration::from_secs(soak_seconds) {
        round += 1;
        let batch = vec![
            (
                "textDocument/hover".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri}, "position": at(site.operation_id.0, site.operation_id.1),
                }),
            ),
            ("textDocument/documentSymbol".to_owned(), doc(&site.uri)),
            ("textDocument/diagnostic".to_owned(), doc(&site.uri)),
            (
                "textDocument/inlayHint".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri},
                    "range": {"start": at(1, 1), "end": at(300, 1)},
                }),
            ),
            (
                "textDocument/semanticTokens/range".to_owned(),
                serde_json::json!({
                    "textDocument": {"uri": site.uri},
                    "range": {"start": at(1, 1), "end": at(200, 1)},
                }),
            ),
            (
                "workspace/diagnostic".to_owned(),
                serde_json::json!({"previousResultIds": []}),
            ),
        ];
        record(&mut bench, "soak", &editor.request_all_timed(&batch));
        editor.notify_timed(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": site.uri, "version": 1_000 + round}, "contentChanges": [],
            }),
        );
        if round.is_multiple_of(4) {
            editor.notify_timed(
                "textDocument/didSave",
                serde_json::json!({
                    "textDocument": {"uri": site.uri}
                }),
            );
        }
    }
    println!(
        "\n  soak: {round} rounds in {:.0}s",
        started.elapsed().as_secs_f64()
    );
    finish(&bench, "soak", round);
    assert_healthy(&bench, "soak");
    assert!(
        round >= 10,
        "the soak completed only {round} rounds over {soak_seconds}s"
    );
}

/// Prints the table and writes the JSON.
fn finish(bench: &Bench, scenario: &str, rounds: usize) {
    print!("{}", bench.render());
    let path = std::path::Path::new("target/lsp-bench").join(format!("{scenario}.json"));
    bench.write_json(&path);
    if let Some(slowest) = bench.slowest() {
        println!(
            "  scenario `{scenario}`, {rounds} rounds — slowest request was {} at {:.0}ms",
            slowest.method,
            slowest.elapsed.as_secs_f64() * 1000.0
        );
    }
}

/// No request may go unanswered, at any volume.
fn assert_healthy(bench: &Bench, scenario: &str) {
    let unanswered = bench.unanswered();
    assert!(
        unanswered.is_empty(),
        "{scenario}: {} of {} requests went unanswered, e.g. {}",
        unanswered.len(),
        bench.count(),
        unanswered
            .iter()
            .take(3)
            .map(|s| s.method.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// A method's worst case must stay under `limit`.
fn assert_under(bench: &Bench, method: &str, limit_ms: u64) {
    let by_method = bench.by_method();
    let Some(stats) = by_method.get(method) else {
        return;
    };
    let max = stats.max.as_millis() as u64;
    assert!(
        max < limit_ms,
        "{method} took {max}ms, over the {limit_ms}ms budget (p95 {}ms over {} samples)",
        stats.p95.as_millis(),
        stats.count
    );
}
