//! End-to-end tests for the `suspect` CLI commands, exercised through the
//! library functions that `main.rs` dispatches to.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use suspect_cli::commands::check;
use suspect_cli::commands::overlay::{OverlayCmd, apply_docs};
use suspect_cli::commands::stats::stats_of;
use suspect_cli::{DocFormat, FindingsFormat, Strategy, bundle, diff};
use suspect_diag::Severity as DiagSeverity;

fn ws_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

// ---- stats census ----------------------------------------------------

#[test]
fn stats_reports_family_census_and_clean_syntax() {
    let tmp = TempDir::new("census");
    let input = write_inline_spec(&tmp.0, "spec.yaml");
    let report = stats_of(&input).expect("stats");
    assert_eq!(report.family, "Oas31");
    assert_eq!(report.syntax_errors, 0);
    assert!(report.ref_edges > 0);
    assert_eq!(report.workspace_docs, 1);
}

#[test]
fn stats_counts_syntax_errors_on_broken_file() {
    let tmp = TempDir::new("broken");
    let path = tmp.write("broken.yaml", "openapi: 3.1.0\npaths: [1, 2\n  bad: : :\n");
    let report = stats_of(&path).expect("stats");
    assert!(report.syntax_errors > 0, "expected syntax errors");
}

// ---- lint (the check pass with the lint filter) ------------------------

/// `(code, severity)` of every `suspect lint` finding on `spec`, under
/// `config` as the directory's `.suspect.yaml`.
fn lint_codes(dir: &TempDir, spec: &Path, config: &str) -> Vec<(String, DiagSeverity)> {
    std::fs::write(dir.path(".suspect.yaml"), config).unwrap();
    let resolved = suspect_config::resolve(&dir.0).expect("valid settings");
    let options = check::Options::new(&resolved, Some(check::LINT), None, None);
    check::run(
        std::slice::from_ref(&spec.to_path_buf()),
        &resolved,
        &options,
    )
    .expect("lint runs")
    .findings
    .iter()
    .map(|f| (f.code.to_string(), f.severity))
    .collect()
}

fn has(findings: &[(String, DiagSeverity)], code: &str) -> Option<DiagSeverity> {
    findings.iter().find(|(c, _)| c == code).map(|(_, s)| *s)
}

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
fn lint_flags_missing_operation_id_under_the_canonical_code() {
    let tmp = TempDir::new("lint");
    let spec = tiny_spec_with_findings(&tmp);
    let findings = lint_codes(&tmp, &spec, "{}\n");
    // `operation-operationId` is an alias: one finding, canonical code and
    // severity.
    assert_eq!(
        has(&findings, "oas-operation-missing-operationId"),
        Some(DiagSeverity::Warning),
        "{findings:?}"
    );
    assert!(has(&findings, "operation-operationId").is_none());
}

#[test]
fn lint_min_severity_filters() {
    let tmp = TempDir::new("lintmin");
    let spec = tiny_spec_with_findings(&tmp);
    let all = lint_codes(&tmp, &spec, "{}\n");
    assert!(!all.is_empty());
    let warnings = lint_codes(&tmp, &spec, "lint:\n  min_severity: warning\n");
    assert!(warnings.iter().all(|(_, s)| *s >= DiagSeverity::Warning));
    assert!(warnings.len() < all.len());
    for f in all.iter().filter(|(_, s)| *s >= DiagSeverity::Warning) {
        assert!(warnings.contains(f), "{f:?} lost by the floor");
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
    // An undeclared path parameter is a lint error (`path-params`).
    let dirty = tmp.write(
        "dirty.yaml",
        "openapi: \"3.0.0\"\n\
         info:\n  title: t\n  version: \"1\"\n\
         paths:\n\
         \x20 /ping/{id}:\n\
         \x20   get:\n\
         \x20     operationId: getPing\n\
         \x20     responses:\n\
         \x20       '200':\n         description: ok\n",
    );
    let lint = |path: &Path| {
        suspect_cli::commands::check::command(&check::Request {
            paths: &[path.to_path_buf()],
            flags: suspect_config::Layer::flags(),
            checks: Some(check::LINT.to_vec()),
            rules: None,
            sdk: None,
            write_baseline: None,
            format: FindingsFormat::Text,
        })
        .expect("lint runs")
    };
    std::fs::write(tmp.path(".suspect.yaml"), "lint:\n  min_severity: error\n").unwrap();
    assert_eq!(lint(&clean), 0, "clean spec must exit 0");
    assert_eq!(lint(&dirty), 1, "spec with error findings must exit 1");
}

/// A spec with one design-class finding (plaintext server URL — truthful
/// documentation of an API the documenter does not own) and one
/// document-class finding (missing operationId).
fn design_and_document_spec(dir: &TempDir) -> PathBuf {
    dir.write(
        "design.yaml",
        "openapi: \"3.0.0\"\n\
         info:\n  title: t\n  version: \"1\"\n\
         servers:\n\
         \x20 - url: http://media.example.local:32400\n\
         paths:\n\
         \x20 /ping:\n\
         \x20   get:\n\
         \x20     responses:\n\
         \x20       '200':\n         description: ok\n",
    )
}

#[test]
fn lint_design_policy_separates_api_design_from_document_accuracy() {
    let tmp = TempDir::new("lintdesign");
    let spec = design_and_document_spec(&tmp);
    let design = "security-server-https-only";
    let document = "oas-operation-missing-operationId";

    // Default: both classes are reported.
    let all = lint_codes(&tmp, &spec, "{}\n");
    assert!(has(&all, design).is_some(), "{all:?}");
    assert!(has(&all, document).is_some());

    // `design: off` drops the whole design class; accuracy survives.
    let kept = lint_codes(&tmp, &spec, "lint:\n  design: off\n");
    assert!(has(&kept, design).is_none(), "{kept:?}");
    assert!(has(&kept, document).is_some());

    // `design: info` downgrades instead of dropping — visible, not gating.
    let downgraded = lint_codes(&tmp, &spec, "lint:\n  design: info\n");
    assert_eq!(has(&downgraded, design), Some(DiagSeverity::Info));

    // A per-rule override wins over the class.
    let mixed = lint_codes(
        &tmp,
        &spec,
        "lint:\n  design: off\n  rules:\n    security-server-https-only: info\n",
    );
    assert_eq!(has(&mixed, design), Some(DiagSeverity::Info));

    // Per-rule `off` drops a single document rule, through its alias too.
    let dropped = lint_codes(
        &tmp,
        &spec,
        "lint:\n  rules:\n    operation-operationId: off\n",
    );
    assert!(has(&dropped, document).is_none(), "{dropped:?}");
    assert!(has(&dropped, design).is_some());

    // The editor's severity vocabulary is accepted, and an unrecognized
    // name is loud, not silently absent.
    let warning = lint_codes(&tmp, &spec, "lint:\n  design: warn\n");
    assert_eq!(has(&warning, design), Some(DiagSeverity::Warning));
    std::fs::write(tmp.path(".suspect.yaml"), "lint:\n  design: loud\n").unwrap();
    assert!(suspect_config::resolve(&tmp.0).is_err());
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
        out: suspect_cli::DocOut {
            output: Some(out.clone()),
            to: None,
        },
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
