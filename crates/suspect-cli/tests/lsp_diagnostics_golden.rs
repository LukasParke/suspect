//! Golden diagnostics from the language server: every document of the
//! `lsp_session.rs` workspace plus a few broken fixtures, pulled with
//! `textDocument/diagnostic` the way an editor pulls them, one line per
//! finding.
//!
//! The diagnostic battery runs through `suspect_check` (P1 step 3); this
//! file was recorded before that switch, so any change in what the editor
//! shows fails here. Regenerate after an intended change with:
//!
//! ```text
//! SUSPECT_BLESS=1 cargo test -p suspect-cli --test lsp_diagnostics_golden
//! ```

#[allow(dead_code)]
mod support {
    pub mod editor;
    pub mod workspace;
}

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::editor::{Editor, editor_capabilities, url_of};
use support::workspace::Workspace;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Extra documents beside the generated workspace: the demo petstore, and
/// under `broken/` (with its own settings file) the broken Swagger,
/// Arazzo, Overlay and load-profile fixtures of the registry corpus, a
/// markdown description and a syntax error.
fn extras(root: &Path) -> Vec<PathBuf> {
    let repo = repo_root();
    let mut out = Vec::new();
    let copy = |from: PathBuf, to: &str, out: &mut Vec<PathBuf>| {
        let dest = root.join(to);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(&from, &dest).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        out.push(dest);
    };
    copy(
        repo.join("docs/demo/petstore.yaml"),
        "petstore.yaml",
        &mut out,
    );
    for name in [
        "actions.overlay.yaml",
        "broken.arazzo.yaml",
        "broken.load.yaml",
        "broken.swagger.yaml",
        "empty.overlay.yaml",
    ] {
        copy(
            repo.join("crates/suspect-check/tests/fixtures").join(name),
            &format!("broken/{name}"),
            &mut out,
        );
    }
    // The workspace's own `.suspect.yaml` sets a warning floor; this one
    // shows hints and exercises per-code overrides on lint and validate
    // codes.
    let config = root.join("broken/.suspect.yaml");
    std::fs::write(
        &config,
        "lint:\n  min_severity: hint\n  rules:\n    info-license: off\n    swagger-info-field-required: warning\n",
    )
    .unwrap();
    out.push(config);
    let markdown = root.join("broken/markdown.yaml");
    std::fs::write(
        &markdown,
        "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\n  description: |\n    # One\n\n    # Two\n\n    See https://example.com/docs\n\n\n    ```\n    plain\n    ```\npaths: {}\n",
    )
    .unwrap();
    out.push(markdown);
    let syntax = root.join("broken/syntax.yaml");
    std::fs::write(
        &syntax,
        "openapi: 3.1.0\ninfo:\n  title: \"unclosed\n  version: '1'\npaths: {}\n",
    )
    .unwrap();
    out.push(syntax);
    out
}

fn line_for(rel: &str, item: &Value) -> String {
    let range = &item["range"];
    let pos = |p: &Value| {
        format!(
            "{}:{}",
            p["line"].as_u64().unwrap_or(0) + 1,
            p["character"].as_u64().unwrap_or(0) + 1
        )
    };
    let severity = match item["severity"].as_u64() {
        Some(1) => "error",
        Some(2) => "warning",
        Some(3) => "info",
        Some(4) => "hint",
        _ => "none",
    };
    let code = match &item["code"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    format!(
        "{rel}:{}-{} {severity} {} {code}: {}",
        pos(&range["start"]),
        pos(&range["end"]),
        item["source"].as_str().unwrap_or(""),
        item["message"].as_str().unwrap_or("").replace('\n', "\\n"),
    )
}

#[test]
fn the_editor_reports_the_recorded_findings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap();
    let workspace = Workspace::build(&root);
    let mut paths: Vec<PathBuf> = vec![
        workspace.openapi.clone(),
        workspace.schemas.clone(),
        workspace.overlay.clone(),
        workspace.workflow.clone(),
        workspace.config.clone(),
        workspace.manifest.clone(),
    ];
    paths.extend(extras(&root));
    let editor = Editor::start(&root, editor_capabilities());
    let mut lines = Vec::new();
    for path in &paths {
        let text = std::fs::read_to_string(path).unwrap();
        let uri = editor.open(path, &text);
        let report = editor
            .request(
                "textDocument/diagnostic",
                serde_json::json!({"textDocument": {"uri": uri}}),
            )
            .expect("document/diagnostic");
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut found: Vec<String> = report["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|item| line_for(&rel, item))
            .collect();
        found.sort();
        if found.is_empty() {
            lines.push(format!("{rel}: (none)"));
        }
        lines.extend(found);
    }
    // The petstore has two operations without an `operationId`: one
    // finding each, under the canonical code, whatever else changes. The
    // workspace manifest configures codegen, so SDK admission also
    // refuses both (`http-operation-id`), a separate category.
    let missing: Vec<&String> = lines
        .iter()
        .filter(|l| {
            l.starts_with("petstore.yaml:")
                && l.contains("operationId")
                && !l.contains(" http-operation-id:")
        })
        .collect();
    assert_eq!(missing.len(), 2, "{missing:#?}");
    assert!(
        missing
            .iter()
            .all(|l| l.contains(" warning suspect oas-operation-missing-operationId:")),
        "{missing:#?}"
    );
    let root_url = url_of(&root);
    let actual = lines.join("\n").replace(&root_url, "file://ROOT") + "\n";
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/lsp-diagnostics.txt");
    if std::env::var_os("SUSPECT_BLESS").is_some() {
        std::fs::write(&golden, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    assert!(
        actual == expected,
        "the editor's diagnostics differ from {}; regenerate with \
         `SUSPECT_BLESS=1 cargo test -p suspect-cli --test lsp_diagnostics_golden`\n--- actual\n{actual}",
        golden.display()
    );
}
