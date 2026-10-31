//! The editor settings layer as the language server applies it: old
//! client spellings, camelCase and snake_case payloads, and the
//! server's own trace of where each value came from
//! (`suspect/explainConfig`). docs/proposals/P4-editors.md, steps 5-6.

#[allow(dead_code)]
mod support {
    pub mod editor;
    pub mod workspace;
}

use std::time::{Duration, Instant};

use support::editor::{Editor, editor_capabilities};
use support::workspace::Workspace;

/// The `window/logMessage` lines so far.
fn log_lines(editor: &Editor) -> Vec<String> {
    editor
        .notifications()
        .into_iter()
        .filter(|(method, _)| method == "window/logMessage")
        .filter_map(|(_, params)| params["message"].as_str().map(str::to_owned))
        .collect()
}

/// Waits until a log line satisfies `want`, and returns it.
fn wait_for_log(editor: &Editor, want: impl Fn(&str) -> bool) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(line) = log_lines(editor).into_iter().find(|l| want(l)) {
            return Some(line);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn an_old_client_key_applies_and_logs_its_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    editor.notify(
        "workspace/didChangeConfiguration",
        serde_json::json!({"settings": {"suspect": {"testBaseUrl": "http://old.example:1"}}}),
    );
    let line = wait_for_log(&editor, |l| l.contains("suspect.testBaseUrl"));
    let line = line.unwrap_or_else(|| panic!("no log line: {:?}", log_lines(&editor)));
    assert!(line.contains("use `suspect.test.baseUrl`"), "{line}");
}
