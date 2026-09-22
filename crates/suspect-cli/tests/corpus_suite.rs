//! Corpus validation suite over the fetched public example specifications.
//!
//! The corpus is the set of real-world and OAI-official specs that the wider
//! OpenAPI toolchain ecosystem (libopenapi, Spectral, Speakeasy) exercises,
//! joined from their original public homes — see `xtask fetch-corpus` for the
//! pinned upstream sources and their licenses.
//!
//! Tiers:
//! - every corpus spec runs the full pipeline (workspace -> semantic
//!   validation -> contract build) with a no-panic guarantee;
//! - the OAI official conformance examples additionally assert error-free
//!   semantic validation;
//! - a committed snapshot report (`fixtures/corpus-report.json`) makes
//!   per-spec diagnostic counts reviewable in PRs. Count drift is a visible
//!   hint, not a failure; file-set drift is a failure.
//!
//! `corpus/` is gitignored and absent on clean checkouts: fetch it first with
//! `cargo run -p xtask -- fetch-corpus`. Regenerate the snapshot with
//! `SUSPECT_CORPUS_REPORT_WRITE=1 cargo test -p suspect-cli --test corpus_suite`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_ir::contract::Contract;
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::{Severity, validate_entry};

/// The OAI-published conformance examples: these must validate without a
/// single error-severity finding.
const OAI_OFFICIAL: &[&str] = &[
    "petstore.yaml",
    "petstore-expanded.yaml",
    "api-with-examples.yaml",
    "callback-example.yaml",
    "link-example.yaml",
    "uspto.yaml",
];

fn corpus_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if dir.join("petstore-expanded.yaml").exists() {
        Some(dir)
    } else {
        eprintln!(
            "skipping: corpus/ is gitignored and absent — run `cargo run -p xtask -- fetch-corpus`"
        );
        None
    }
}

/// Every YAML/JSON spec currently fetched into the corpus, sorted.
fn corpus_specs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("corpus dir is readable")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            if matches!(ext, "yaml" | "yml" | "json") {
                path.file_name()?.to_str().map(String::from)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    names
}

/// One full pipeline pass over a single spec: workspace, semantic validation,
/// and the source-addressed contract build. Never panics on spec content —
/// malformed documents surface as diagnostics, and families semantic
/// validation does not cover (Swagger 2.0) surface as a recorded refusal.
fn run_pipeline(dir: &Path, name: &str) -> SpecResult {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(dir)
            .build()
            .expect("workspace builds"),
    );
    let session = Session::new(workspace.clone());
    let (errors, warnings, unsupported) = match validate_entry(&session, name) {
        Ok(diags) => (
            diags
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .count(),
            diags
                .iter()
                .filter(|d| d.severity == Severity::Warning)
                .count(),
            false,
        ),
        Err(_) => (0, 0, true),
    };
    let uri = workspace
        .uris()
        .into_iter()
        .find(|uri| uri.as_str().ends_with(name));
    let contract_built = uri.is_some_and(|uri| Contract::from_workspace(&workspace, &uri).is_ok());
    SpecResult {
        name: name.to_string(),
        semantic_errors: errors,
        semantic_warnings: warnings,
        unsupported,
        contract_built,
    }
}

struct SpecResult {
    name: String,
    semantic_errors: usize,
    semantic_warnings: usize,
    /// Semantic validation declines this document's spec family (Swagger 2.0).
    unsupported: bool,
    contract_built: bool,
}

/// The always-on walker check: a tiny local fixture directory (no corpus
/// needed) exercises the same pipeline the corpus tiers use.
#[test]
fn local_fixture_pipeline_produces_located_results_without_panic() {
    let dir = std::env::temp_dir().join(format!(
        "suspect-corpus-suite-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mini-valid.yaml"),
        "openapi: 3.0.0\ninfo: {title: t, version: '1'}\npaths:\n  /p:\n    get:\n      responses:\n        '200':\n          description: ok\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("mini-dangling-ref.yaml"),
        "openapi: 3.0.0\ninfo: {title: t, version: '1'}\npaths:\n  /p:\n    get:\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema:\n                $ref: '#/components/schemas/Missing'\n",
    )
    .unwrap();

    let valid = run_pipeline(&dir, "mini-valid.yaml");
    assert_eq!(valid.semantic_errors, 0, "valid fixture must be error-free");
    assert!(valid.contract_built, "valid fixture must build a contract");

    let dangling = run_pipeline(&dir, "mini-dangling-ref.yaml");
    assert!(
        dangling.semantic_errors > 0,
        "a dangling ref must surface as an error diagnostic, not a panic"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Tier 1: the full pipeline completes on every fetched corpus spec.
#[test]
fn corpus_pipeline_runs_on_every_spec_without_panics() {
    let Some(dir) = corpus_dir() else { return };
    let specs = corpus_specs(&dir);
    assert!(
        !specs.is_empty(),
        "corpus dir exists but holds no specs — re-run fetch-corpus"
    );
    for name in &specs {
        let result = run_pipeline(&dir, name);
        eprintln!(
            "{:<26} errors {:>4}  warnings {:>4}  contract {}{}",
            result.name,
            result.semantic_errors,
            result.semantic_warnings,
            result.contract_built,
            if result.unsupported {
                "  [unsupported family]"
            } else {
                ""
            }
        );
    }
}

/// Tier 2: the OAI-published conformance examples validate error-free.
#[test]
fn oai_official_examples_validate_without_errors() {
    let Some(dir) = corpus_dir() else { return };
    let present: Vec<_> = OAI_OFFICIAL
        .iter()
        .filter(|name| dir.join(name).exists())
        .collect();
    if present.len() < OAI_OFFICIAL.len() - 1 {
        // Tolerate one upstream fetch failure; more means a stale corpus.
        eprintln!(
            "skipping: OAI example set incomplete ({}/6 present)",
            present.len()
        );
        return;
    }
    for name in present {
        let result = run_pipeline(&dir, name);
        assert_eq!(
            result.semantic_errors, 0,
            "{name} is an OAI conformance example and must validate without errors"
        );
    }
}

/// Tier 3: the committed snapshot covers exactly the current corpus file set.
/// Diagnostic-count drift prints a regeneration hint instead of failing, so
/// new rules stay visible without churning the suite.
#[test]
fn corpus_snapshot_report_covers_every_spec() {
    let Some(dir) = corpus_dir() else { return };
    let specs = corpus_specs(&dir);
    let report_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus-report.json");

    if std::env::var("SUSPECT_CORPUS_REPORT_WRITE").is_ok_and(|v| v == "1") {
        let mut entries = serde_json::Map::new();
        for name in &specs {
            let r = run_pipeline(&dir, name);
            entries.insert(
                r.name.clone(),
                serde_json::json!({
                    "semantic_errors": r.semantic_errors,
                    "semantic_warnings": r.semantic_warnings,
                    "unsupported": r.unsupported,
                    "contract_built": r.contract_built,
                }),
            );
        }
        let report = serde_json::json!({
            "format": "suspect.corpus-validation-report.v1",
            "entries": entries,
        });
        std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        std::fs::write(
            &report_path,
            serde_json::to_string_pretty(&report).unwrap() + "\n",
        )
        .unwrap();
        eprintln!("wrote {}", report_path.display());
        return;
    }

    let Ok(raw) = std::fs::read_to_string(&report_path) else {
        panic!(
            "snapshot {} is missing — regenerate with SUSPECT_CORPUS_REPORT_WRITE=1 \
             cargo test -p suspect-cli --test corpus_suite",
            report_path.display()
        );
    };
    let report: serde_json::Value = serde_json::from_str(&raw).expect("snapshot parses");
    let entries = report
        .get("entries")
        .and_then(|e| e.as_object())
        .expect("snapshot carries an entries object");

    let mut snapshot_names: Vec<&str> = entries.keys().map(String::as_str).collect();
    snapshot_names.sort_unstable();
    assert_eq!(
        snapshot_names, specs,
        "snapshot file-set drifted from the corpus — regenerate the report"
    );

    for name in &specs {
        let snapshot = &entries[name];
        let result = run_pipeline(&dir, name);
        let snap_errors = snapshot["semantic_errors"].as_u64().unwrap_or(0) as usize;
        let snap_warnings = snapshot["semantic_warnings"].as_u64().unwrap_or(0) as usize;
        if snap_errors != result.semantic_errors || snap_warnings != result.semantic_warnings {
            eprintln!(
                "hint: {name} now has {}/{} errors/warnings (snapshot {}/{}), \
                 consider regenerating the corpus report",
                result.semantic_errors, result.semantic_warnings, snap_errors, snap_warnings
            );
        }
    }
}
