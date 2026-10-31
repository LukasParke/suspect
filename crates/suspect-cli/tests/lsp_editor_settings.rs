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

/// The `lint.min_severity` row of `suspect/explainConfig`, once it reports
/// `want` (configuration changes apply asynchronously).
fn floor_row(editor: &Editor, want: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = serde_json::Value::Null;
    while Instant::now() < deadline {
        let report = editor
            .request("suspect/explainConfig", serde_json::json!({}))
            .expect("suspect/explainConfig answers");
        last = report["values"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| r["key"] == "lint.min_severity"))
            .cloned()
            .unwrap_or_default();
        if last["value"] == want {
            return report;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("lint.min_severity never became {want}: {last}");
}

#[test]
fn explain_config_shows_the_editor_layer_over_the_settings_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join(".suspect.yaml"), "lint:\n  min_severity: error\n").unwrap();
    let editor = Editor::start(root, editor_capabilities());
    let advertised = &editor.initialize_result()["capabilities"]["experimental"]["suspect"];
    assert_eq!(advertised["requests"]["suspect/explainConfig"], 1);

    let report = floor_row(&editor, "error");
    assert_eq!(report["source"], ".suspect.yaml");
    assert!(report["error"].is_null());

    editor.notify(
        "workspace/didChangeConfiguration",
        serde_json::json!({"settings": {"suspect": {"lint": {"minSeverity": "warning"}}}}),
    );
    let report = floor_row(&editor, "warning");
    let row = report["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "lint.min_severity")
        .unwrap();
    assert_eq!(row["layer"], "editor");
    assert_eq!(row["origin"], "editor suspect.lint.minSeverity");
    let trace: Vec<&serde_json::Value> = row["trace"].as_array().unwrap().iter().collect();
    assert_eq!(trace.len(), 2, "{trace:?}");
    assert!(trace[0][0].as_str().unwrap().ends_with(".suspect.yaml:2"));
    assert_eq!(trace[0][1], "error");
    assert_eq!(trace[1][1], "warning");

    // A rejected value is the report's error, and the file's value stands.
    editor.notify(
        "workspace/didChangeConfiguration",
        serde_json::json!({"settings": {"suspect": {"lint": {"minSeverity": "fatal"}}}}),
    );
    let report = floor_row(&editor, "error");
    assert!(
        report["error"].as_str().unwrap().contains("`fatal`"),
        "{}",
        report["error"]
    );
}

#[test]
fn camel_case_and_snake_case_client_keys_resolve_alike() {
    let dir = tempfile::tempdir().expect("tempdir");
    let editor = Editor::start(dir.path(), editor_capabilities());
    let mut seen = Vec::new();
    for (floor, section) in [
        (
            "info",
            serde_json::json!({"lint": {"minSeverity": "info"}, "ref": {"maxDocs": 7},
                               "editor": {"inlayHints": {"refs": false}}}),
        ),
        (
            "hint",
            serde_json::json!({"lint": {"min_severity": "hint"}, "ref": {"max_docs": 7},
                               "editor": {"inlay_hints": {"refs": false}}}),
        ),
    ] {
        editor.notify(
            "workspace/didChangeConfiguration",
            serde_json::json!({"settings": {"suspect": section}}),
        );
        let report = floor_row(&editor, floor);
        let values: Vec<(String, serde_json::Value, serde_json::Value)> = report["values"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["key"] != "lint.min_severity")
            .map(|r| {
                (
                    r["key"].as_str().unwrap().to_owned(),
                    r["value"].clone(),
                    r["layer"].clone(),
                )
            })
            .collect();
        assert!(values.contains(&("ref.max_docs".into(), 7.into(), "editor".into())));
        assert!(values.contains(&(
            "editor.inlay_hints.refs".into(),
            false.into(),
            "editor".into()
        )));
        seen.push(values);
    }
    assert_eq!(seen[0], seen[1]);
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
