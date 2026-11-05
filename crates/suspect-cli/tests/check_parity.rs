//! `suspect check` and the language server report the same findings.
//!
//! Both run `suspect_check::check_document` and `apply_policy`; this test
//! holds them to it on the same fixtures: every `check --format json`
//! finding and every published diagnostic, mapped to `(code, severity,
//! start, end)` with LSP positions (0-based line, UTF-16 column).

#[allow(dead_code)]
mod support {
    pub mod editor;
}

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use support::editor::{Editor, editor_capabilities};

type Key = (String, String, (u64, u64), (u64, u64));

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fixtures, copied under `root` beside one settings file that shows
/// hints and overrides a lint and a validate code.
fn fixtures(root: &Path) -> Vec<PathBuf> {
    let repo = repo();
    let mut out = Vec::new();
    let mut copy = |from: PathBuf, name: &str| {
        let dest = root.join(name);
        std::fs::copy(&from, &dest).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        out.push(dest);
    };
    copy(repo.join("docs/demo/petstore.yaml"), "petstore.yaml");
    for name in [
        "actions.overlay.yaml",
        "broken.arazzo.yaml",
        "broken.load.yaml",
        "broken.swagger.yaml",
        "empty.overlay.yaml",
    ] {
        copy(
            repo.join("crates/suspect-check/tests/fixtures").join(name),
            name,
        );
    }
    let mut write = |name: &str, text: &str| {
        let path = root.join(name);
        std::fs::write(&path, text).unwrap();
        out.push(path);
    };
    write(
        "markdown.yaml",
        "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\n  description: |\n    # One\n\n    # Two\n\n    See https://example.com/docs\npaths: {}\n",
    );
    write(
        "syntax.yaml",
        "openapi: 3.1.0\ninfo:\n  title: \"unclosed\n  version: '1'\npaths: {}\n",
    );
    write(
        "suppressed.yaml",
        "openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths:\n  /a:\n    # suspect-disable operation-operationId\n    get:\n      responses: {'200': {description: ok}}\n  /b:\n    get:\n      x-suspect-disable: [nope]\n      responses: {'200': {description: ok}}\n",
    );
    write(
        "api.json",
        "{\"openapi\": \"3.1.0\", \"info\": {\"title\": \"T\", \"version\": \"1\"}, \"paths\": {\"/a/{id}\": {\"get\": {\"responses\": {\"200\": {\"description\": \"ok\"}}}}}}\n",
    );
    std::fs::write(
        root.join(".suspect.yaml"),
        "lint:\n  min_severity: hint\n  rules:\n    info-license: off\n    oas-operation-no-error-response: warning\n",
    )
    .unwrap();
    out
}

/// LSP position of byte `offset` in `text`.
fn position(text: &str, offset: usize) -> (u64, u64) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() as u64;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before[start..].encode_utf16().count() as u64;
    (line, col)
}

fn cli(root: &Path, path: &Path) -> Vec<Key> {
    let out = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args(["check", "--format", "json"])
        .arg(path)
        .env_remove("SUSPECT_LINT_MIN_SEVERITY")
        .env_remove("SUSPECT_LINT_RULESET")
        .env_remove("SUSPECT_STRICT_FORMAT")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    let text = std::fs::read_to_string(path).unwrap();
    let mut keys: Vec<Key> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let (start, end) = match f.get("range") {
                Some(range) => (
                    position(&text, range["start"].as_u64().unwrap() as usize),
                    position(&text, range["end"].as_u64().unwrap() as usize),
                ),
                None => ((0, 0), (0, 0)),
            };
            (
                f["code"].as_str().unwrap().to_owned(),
                f["severity"].as_str().unwrap().to_owned(),
                start,
                end,
            )
        })
        .collect();
    keys.sort();
    keys
}

fn lsp(editor: &Editor, path: &Path) -> Vec<Key> {
    let uri = editor.open(path, &std::fs::read_to_string(path).unwrap());
    let report = editor
        .request(
            "textDocument/diagnostic",
            serde_json::json!({"textDocument": {"uri": uri}}),
        )
        .expect("document/diagnostic");
    let at = |p: &Value| {
        (
            p["line"].as_u64().unwrap(),
            p["character"].as_u64().unwrap(),
        )
    };
    let mut keys: Vec<Key> = report["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|d| {
            let severity = match d["severity"].as_u64() {
                Some(1) => "error",
                Some(2) => "warning",
                Some(3) => "info",
                _ => "hint",
            };
            (
                d["code"].as_str().unwrap_or_default().to_owned(),
                severity.to_owned(),
                at(&d["range"]["start"]),
                at(&d["range"]["end"]),
            )
        })
        .collect();
    keys.sort();
    keys
}

#[test]
fn check_and_the_editor_report_the_same_findings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let paths = fixtures(&root);
    let editor = Editor::start(&root, editor_capabilities());
    let mut compared = 0;
    for path in &paths {
        let from_cli = cli(&root, path);
        let from_lsp = lsp(&editor, path);
        assert_eq!(
            from_cli,
            from_lsp,
            "{}: CLI (left) and editor (right) disagree",
            path.display()
        );
        compared += from_cli.len();
    }
    // The fixtures are not trivially clean.
    assert!(compared > 40, "only {compared} findings compared");
}
