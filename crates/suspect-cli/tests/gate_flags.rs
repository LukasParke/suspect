//! `--check` gates: `diff structural`, `diff routes` and `overlay explain` keep exiting 0
//! by default and exit 1 under `--check` when the result is not clean.

use std::path::{Path, PathBuf};
use std::process::Output;

fn run(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args(args)
        .output()
        .unwrap()
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

const SPEC: &str = r#"openapi: 3.1.0
info: {title: Pets, version: '1'}
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200': {description: ok}
"#;

#[test]
fn diff_check_exits_1_only_when_documents_differ() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.yaml", SPEC);
    let b = write(
        dir.path(),
        "b.yaml",
        &SPEC.replace("version: '1'", "version: '2'"),
    );
    assert_eq!(
        run(&["diff", "structural", s(&a), s(&b)]).status.code(),
        Some(0)
    );
    assert_eq!(
        run(&["diff", "structural", s(&a), s(&b), "--check"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        run(&["diff", "structural", s(&a), s(&a), "--check"])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn reverse_check_exits_1_on_undocumented_routes() {
    let dir = tempfile::tempdir().unwrap();
    let spec = write(dir.path(), "openapi.yaml", SPEC);
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    write(
        &src,
        "app.ts",
        "app.get('/pets', listPets);\napp.post('/owners', createOwner);\n",
    );
    assert_eq!(
        run(&["diff", "routes", s(&spec), s(&src)]).status.code(),
        Some(0)
    );
    let gated = run(&[
        "diff",
        "routes",
        s(&spec),
        s(&src),
        "--check",
        "--format",
        "json",
    ]);
    assert_eq!(gated.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&gated.stdout).unwrap();
    assert!(!report["undocumented"].as_array().unwrap().is_empty());

    write(&src, "app.ts", "app.get('/pets', listPets);\n");
    assert_eq!(
        run(&["diff", "routes", s(&spec), s(&src), "--check"])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn overlay_explain_check_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let target = write(dir.path(), "target.yaml", SPEC);
    let overlay = write(
        dir.path(),
        "overlay.yaml",
        r#"overlay: 1.0.0
info: {title: Polish, version: '1'}
actions:
  - target: '$.paths["/pets"].get'
    update: {summary: Lists every pet}
  - target: '$.paths["/missing"]'
    update: {summary: never matches}
"#,
    );
    let plain = run(&["overlay", "explain", s(&overlay), s(&target)]);
    assert_eq!(plain.status.code(), Some(0));

    let gated = run(&[
        "overlay",
        "explain",
        s(&overlay),
        s(&target),
        "--check",
        "--format",
        "json",
    ]);
    assert_eq!(gated.status.code(), Some(1));
    let steps: serde_json::Value = serde_json::from_slice(&gated.stdout).unwrap();
    let steps = steps.as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["matches"], 1);
    assert_eq!(steps[0]["kind"], "update");
    assert_eq!(steps[1]["matches"], 0);
    let stderr = String::from_utf8_lossy(&gated.stderr);
    assert!(stderr.contains("action #1 matched nothing"), "{stderr}");
}

#[test]
fn gen_check_writes_nothing_and_diff_needs_check() {
    let dir = tempfile::tempdir().unwrap();
    let spec = write(dir.path(), "openapi.yaml", SPEC);
    let out = dir.path().join("out");
    let check = run(&[
        "gen",
        "template",
        s(&spec),
        "--preset",
        "docs-md",
        "-o",
        s(&out),
        "--check",
    ]);
    assert_eq!(check.status.code(), Some(1), "missing output drifts");
    assert!(!out.exists(), "--check writes nothing");
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        !stdout.contains("--- "),
        "no diffs without --diff: {stdout}"
    );

    // `--diff` adds unified diffs to `--check`; alone it is a usage error.
    let diff = run(&[
        "gen",
        "template",
        s(&spec),
        "--preset",
        "docs-md",
        "--output",
        s(&out),
        "--diff",
    ]);
    assert_eq!(diff.status.code(), Some(2));
    let diff = run(&[
        "gen",
        "template",
        s(&spec),
        "--preset",
        "docs-md",
        "--output",
        s(&out),
        "--check",
        "--diff",
    ]);
    assert_eq!(diff.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&diff.stdout).contains("--- "));
    assert!(!out.exists());

    assert_eq!(
        run(&[
            "gen",
            "template",
            s(&spec),
            "--preset",
            "docs-md",
            "-o",
            s(&out)
        ])
        .status
        .code(),
        Some(0)
    );
    assert_eq!(
        run(&[
            "gen",
            "template",
            s(&spec),
            "--preset",
            "docs-md",
            "-o",
            s(&out),
            "--check"
        ])
        .status
        .code(),
        Some(0),
        "fresh output is clean"
    );
}
