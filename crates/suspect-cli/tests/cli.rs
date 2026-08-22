//! End-to-end tests for the `suspect` CLI commands, exercised through the
//! library functions that `main.rs` dispatches to.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use suspect_cli::commands::check::check_file;
use suspect_cli::commands::lint::lint_findings;
use suspect_cli::commands::overlay::{OverlayCmd, apply_docs};
use suspect_cli::commands::stats::stats_of;
use suspect_cli::{DocFormat, OutputFormat, Severity, Strategy, bundle, diff};

fn ws_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    ws_root().join("fixtures").join(name)
}

/// Writes a small deterministic OpenAPI 3.1 spec into a temp dir so tests do
/// not depend on the gitignored generated fixtures.
fn write_inline_spec(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\npaths:\n  /pets:\n    get:\n      responses:\n        \"200\":\n          description: ok\n          content:\n            application/json:\n              schema:\n                $ref: \'#/components/schemas/Pet\'\ncomponents:\n  schemas:\n    Pet:\n      type: object\n",
    )
    .unwrap();
    path
}

/// Circular two-schema spec (A -> B -> A) for inline-bundle termination.
fn write_circular_spec(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        "openapi: 3.1.0\ninfo:\n  title: c\n  version: \"1\"\npaths: {}\ncomponents:\n  schemas:\n    A:\n      $ref: \'#/components/schemas/B\'\n    B:\n      properties:\n        back:\n          $ref: \'#/components/schemas/A\'\n",
    )
    .unwrap();
    path
}

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// Unique temp directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("suspect-cli-{}-{}-{tag}", std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(name);
        let mut f = std::fs::File::create(&path).expect("create file");
        f.write_all(content.as_bytes()).expect("write file");
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- check -----------------------------------------------------------

#[test]
fn check_reports_family_and_clean_syntax() {
    let tmp = TempDir::new("check");
    let input = write_inline_spec(&tmp.0, "spec.yaml");
    let report = check_file(&input);
    assert_eq!(report.family, "Oas31");
    assert_eq!(report.syntax_errors, 0);
    assert!(
        report.findings.is_empty(),
        "unexpected findings: {:?}",
        report.findings
    );
    assert!(report.ref_edges > 0);
}

#[test]
fn check_counts_syntax_errors_on_broken_file() {
    let tmp = TempDir::new("broken");
    let path = tmp.write("broken.yaml", "openapi: 3.1.0\npaths: [1, 2\n  bad: : :\n");
    let report = check_file(&path);
    assert!(report.syntax_errors > 0, "expected syntax errors");
    assert!(report.findings.iter().any(|f| f.code == "syntax-error"));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.severity == Severity::Error)
    );
}

#[test]
fn check_reports_unreadable_file_as_error_finding() {
    let report = check_file(Path::new("/nonexistent/suspect-cli-missing.yaml"));
    assert!(report.findings.iter().any(|f| f.code == "io-error"));
}

// ---- lint ------------------------------------------------------------

fn tiny_spec_with_findings(dir: &TempDir) -> PathBuf {
    dir.write(
        "spec.yaml",
        "openapi: \"3.0.0\"\n\
         info:\n  title: t\n  version: \"1\"\n\
         paths:\n\
         \x20 /ping:\n\
         \x20   get:\n\
         \x20     summary: ping\n\
         \x20     responses:\n\
         \x20       '404':\n         description: nope\n",
    )
}

#[test]
fn lint_flags_missing_operation_id() {
    let tmp = TempDir::new("lint");
    let spec = tiny_spec_with_findings(&tmp);
    let findings = lint_findings(&[spec], None, Severity::Hint).expect("lint runs");
    assert!(
        findings.iter().any(|f| f.code == "operation-operationId"),
        "expected operationId finding, got {:?}",
        findings
            .iter()
            .map(|f| (&f.code, f.severity))
            .collect::<Vec<_>>()
    );
    // JSON shape: every finding serializes with all documented fields.
    for f in &findings {
        let v = serde_json::to_value(f).expect("serialize");
        for key in ["file", "severity", "code", "message", "line", "col"] {
            assert!(v.get(key).is_some(), "missing JSON field {key}");
        }
    }
}

#[test]
fn lint_min_severity_filters() {
    let tmp = TempDir::new("lintmin");
    let spec = tiny_spec_with_findings(&tmp);
    let all = lint_findings(std::slice::from_ref(&spec), None, Severity::Hint).expect("lint runs");
    assert!(!all.is_empty());
    let errors_only = lint_findings(&[spec], None, Severity::Error).expect("lint runs");
    assert!(errors_only.iter().all(|f| f.severity == Severity::Error));
    assert!(errors_only.len() <= all.len());
    // Every error in `all` survives in `errors_only`.
    for f in all.iter().filter(|f| f.severity == Severity::Error) {
        assert!(
            errors_only
                .iter()
                .any(|g| g.code == f.code && g.line == f.line),
            "error finding {f:?} lost by filter"
        );
    }
}

#[test]
fn lint_exit_codes_clean_vs_findings() {
    let tmp = TempDir::new("lintexit");
    let clean = tmp.write(
        "clean.yaml",
        "openapi: \"3.0.0\"\n\
         info:\n  title: t\n  version: \"1\"\n\
         paths:\n\
         \x20 /ping:\n\
         \x20   get:\n\
         \x20     operationId: getPing\n\
         \x20     responses:\n\
         \x20       '200':\n         description: ok\n",
    );
    let dirty = tiny_spec_with_findings(&tmp);
    let code_clean =
        suspect_cli::commands::lint::lint(&[clean], None, Severity::Error, OutputFormat::Text)
            .expect("lint runs");
    assert_eq!(code_clean, 0, "clean spec must exit 0");
    let code_dirty =
        suspect_cli::commands::lint::lint(&[dirty], None, Severity::Error, OutputFormat::Text)
            .expect("lint runs");
    assert_eq!(code_dirty, 1, "spec with Error findings must exit 1");
}

// ---- overlay ---------------------------------------------------------

#[test]
fn overlay_apply_writes_output_and_reports_unmatched() {
    let tmp = TempDir::new("overlay");
    let target = tmp.write(
        "target.yaml",
        "openapi: 3.1.0\ninfo:\n  title: original\n  version: \"1\"\npaths: {}\n",
    );
    let overlay = tmp.write(
        "overlay.yaml",
        "overlay: 1.0.0\ninfo:\n  title: o\n  version: \"1\"\n\
         actions:\n\
         \x20 - target: $.info\n\
         \x20   update:\n\
         \x20     title: patched\n\
         \x20 - target: $.does.not.exist\n\
         \x20   update: x\n",
    );
    let applied = apply_docs(&overlay, &target).expect("apply works");
    assert_eq!(applied.applied_actions, 1);
    assert_eq!(
        applied.output.get("info").and_then(|i| i.get("title")),
        Some(&suspect_overlay::Value::Str("patched".into()))
    );

    // The command writes the output file end-to-end.
    let out = tmp.path("out.json");
    let code = suspect_cli::commands::overlay::run(OverlayCmd::Apply {
        overlay,
        target,
        output: Some(out.clone()),
    })
    .expect("run works");
    assert_eq!(code, 0);
    let text = std::fs::read_to_string(&out).expect("output written");
    assert!(
        text.contains("patched"),
        "output missing applied key: {text}"
    );
}

// ---- fmt -------------------------------------------------------------

#[test]
fn fmt_yaml_round_trips() {
    let tmp = TempDir::new("fmt");
    let input = tmp.write(
        "in.yaml",
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: \"1\"\npaths: {}\n",
    );
    let out = tmp.path("out.yaml");
    let code = suspect_cli::commands::fmt::fmt(&input, Some(&out), false, false).expect("fmt");
    assert_eq!(code, 0);
    let reparsed = suspect_cli::load_doc(&out).expect("output parses");
    assert!(reparsed.syntax_errors().is_empty());
    assert_eq!(reparsed.sniff_family(), suspect_low::SpecFamily::Oas31);
}

#[test]
fn fmt_json_in_json_out() {
    let tmp = TempDir::new("fmtjson");
    let input = tmp.write(
        "in.json",
        r#"{"openapi": "3.1.0", "info": {"title": "t", "version": "1"}, "paths": {}}"#,
    );
    let out = tmp.path("out.json");
    let code = suspect_cli::commands::fmt::fmt(&input, Some(&out), true, false).expect("fmt");
    assert_eq!(code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid json");
    assert_eq!(value["openapi"], "3.1.0");
    assert_eq!(value["info"]["title"], "t");
}

// ---- stats -----------------------------------------------------------

#[test]
fn stats_matches_hand_written_spec() {
    let tmp = TempDir::new("stats");
    let spec = tmp.write(
        "small.yaml",
        "openapi: 3.0.0\n\
         info:\n  title: t\n  version: \"1\"\n\
         tags:\n  - name: a\n\
         paths:\n\
         \x20 /a:\n\
         \x20   get:\n     responses:\n       '200':\n         description: ok\n\
         \x20   post:\n     responses:\n       '201':\n         description: made\n\
         \x20 /b:\n\
         \x20   get:\n     responses:\n       '200':\n         description: ok\n\
         components:\n\
         \x20 schemas:\n    S:\n      type: object\n\
         \x20 parameters:\n    P:\n      name: p\n      in: query\n      schema:\n        type: string\n\
         \x20 securitySchemes:\n    k:\n      type: apiKey\n",
    );
    let report = stats_of(&spec).expect("stats");
    assert_eq!(report.family, "Oas30");
    assert_eq!(report.paths, 2);
    assert_eq!(report.operations, 3);
    assert_eq!(report.schemas, 1);
    assert_eq!(report.parameters, 1);
    assert_eq!(report.security_schemes, 1);
    assert_eq!(report.tags, 1);
    assert_eq!(report.workflows, 0);
    assert_eq!(report.actions, 0);
    assert!(report.size_bytes > 0);
    assert!(report.parse_ms >= 0.0);
}

// ---- bundle ----------------------------------------------------------

#[test]
fn bundle_keep_is_byte_identical_passthrough() {
    let tmpsrc = TempDir::new("keep-src");
    let input = write_inline_spec(&tmpsrc.0, "spec.yaml");
    let tmp = TempDir::new("keep");
    let out = tmp.path("bundled.yaml");
    let code = bundle::bundle(&input, Some(&out), Strategy::Keep, None).expect("bundle");
    assert_eq!(code, 0, "fixture refs must resolve cleanly");
    let original = std::fs::read(&input).expect("read input");
    let emitted = std::fs::read(&out).expect("read output");
    assert_eq!(original, emitted, "keep must be byte-identical");
}

#[test]
fn bundle_inline_terminates_on_circular_fixture() {
    let tmpsrc = TempDir::new("inline-src");
    let input = write_circular_spec(&tmpsrc.0, "circular.yaml");
    let tmp = TempDir::new("inline");
    let out = tmp.path("bundled.json");
    let code = bundle::bundle(&input, Some(&out), Strategy::Inline, Some(DocFormat::Json))
        .expect("bundle inline");
    assert_eq!(code, 0);

    let text = std::fs::read_to_string(&out).expect("read bundled output");
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

    // Every surviving $ref must carry the cyclic marker.
    fn check_refs(v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                if map.contains_key("$ref") {
                    assert_eq!(
                        map.get("x-suspect-cyclic"),
                        Some(&serde_json::Value::Bool(true)),
                        "$ref without cyclic marker: {map:?}"
                    );
                }
                for (_, child) in map {
                    check_refs(child);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    check_refs(item);
                }
            }
            _ => {}
        }
    }
    check_refs(&value);
}

#[test]
fn bundle_inline_resolves_acyclic_refs_and_marks_cycles() {
    let tmp = TempDir::new("inlinesmall");
    let input = tmp.write(
        "spec.yaml",
        "openapi: 3.1.0\n\
         info:\n  title: t\n  version: \"1\"\n\
         paths: {}\n\
         components:\n\
         \x20 schemas:\n\
         \x20   Plain:\n     type: object\n     properties:\n       a:\n         type: string\n\
         \x20   Holder:\n     type: object\n     properties:\n       p:\n         $ref: '#/components/schemas/Plain'\n\
         \x20   Loop:\n     type: object\n     properties:\n       me:\n         $ref: '#/components/schemas/Loop'\n",
    );
    let out = tmp.path("bundled.json");
    let code = bundle::bundle(&input, Some(&out), Strategy::Inline, Some(DocFormat::Json))
        .expect("bundle inline");
    assert_eq!(code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("json");

    // Acyclic ref resolved into a deep copy equal to the source schema.
    let plain_source = &value["components"]["schemas"]["Plain"];
    let holder_p = &value["components"]["schemas"]["Holder"]["properties"]["p"];
    assert_eq!(
        holder_p, plain_source,
        "resolved inline schema must equal source schema"
    );
    assert!(holder_p.get("$ref").is_none());

    // Cyclic ref kept as marked marker.
    let loop_me = &value["components"]["schemas"]["Loop"]["properties"]["me"];
    assert_eq!(loop_me["$ref"], "#/components/schemas/Loop");
    assert_eq!(loop_me["x-suspect-cyclic"], serde_json::Value::Bool(true));
}

// ---- diff ------------------------------------------------------------

#[test]
fn diff_reports_added_removed_changed() {
    use suspect_overlay::Value;
    let a = Value::Object(vec![
        ("x".into(), Value::Int(1)),
        ("y".into(), Value::Object(vec![("z".into(), Value::Int(2))])),
        (
            "list".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        ),
        ("gone".into(), Value::Str("bye".into())),
    ]);
    let b = Value::Object(vec![
        ("x".into(), Value::Int(2)),
        ("y".into(), Value::Object(vec![("z".into(), Value::Int(2))])),
        (
            "list".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
        ),
        ("new".into(), Value::Int(4)),
    ]);
    let mut report = diff::DiffReport::default();
    diff::diff_values("", &a, &b, &mut report);
    assert_eq!(
        report.added,
        vec!["/list/2".to_string(), "/new".to_string()]
    );
    assert_eq!(report.removed, vec!["/gone".to_string()]);
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].path, "/x");
    assert_eq!(report.changed[0].from, "1");
    assert_eq!(report.changed[0].to, "2");
}
#[test]
fn diff_identical_documents_is_empty() {
    let path = ws_root().join("corpus").join("petstore-expanded.yaml");
    let fallback_dir = std::env::temp_dir().join("suspect-cli-diff");
    let _ = std::fs::create_dir_all(&fallback_dir);
    let path = if path.exists() {
        path
    } else {
        write_inline_spec(&fallback_dir, "diff-spec.yaml")
    };

    let doc_a = suspect_cli::load_doc(&path).expect("parse a");
    let doc_b = suspect_cli::load_doc(&path).expect("parse b");
    let va = suspect_overlay::Value::from_node(doc_a.root());
    let vb = suspect_overlay::Value::from_node(doc_b.root());
    let mut report = diff::DiffReport::default();
    diff::diff_values("", &va, &vb, &mut report);
    assert!(
        report.is_empty(),
        "identical files must diff empty: {report:?}"
    );
}
