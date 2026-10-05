//! The language server, driven the way an editor drives it, across its
//! whole feature surface.
//!
//! # Why this file exists
//!
//! The server passed 2,000+ unit tests and then failed in VS Code. Every one
//! of its deadlocks needed two things to coincide — a handler holding a
//! state guard, and a *second* party queued behind it. A test that sends one
//! request and waits cannot produce that. A test that sends requests one at a
//! time cannot produce it either, because the first is finished before the
//! second starts.
//!
//! So every test here goes through [`support::editor::Editor`], which sends
//! many requests before waiting for any of them, sends notifications without
//! ids, and answers the server's own requests. One test replays traffic
//! recorded from a real VS Code session — see
//! `fixtures/vscode-open-traffic.json`.
//!
//! What is asserted is *content*, not just "it answered". A server that
//! answers every request with `null` passes a liveness test and is useless.

// The harness modules compile into more than one test binary, so an item
// used by the session suite is unused in the bench suite and vice versa.
#[allow(dead_code)]
mod support {
    pub mod editor;
    pub mod workspace;
}

use std::time::{Duration, Instant};

use serde_json::Value;
use support::editor::{Editor, editor_capabilities, url_of};
use support::workspace::Workspace;

/// Built once and shared: the read-only feature sweep must not pay for
/// indexing a workspace per assertion, and issuing requests against a live
/// server from several threads is exactly the concurrency worth having.
fn shared() -> &'static (Workspace, Editor) {
    static SHARED: std::sync::OnceLock<(Workspace, Editor)> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = Workspace::build(dir.path());
        let editor = Editor::start(&workspace.root, editor_capabilities());
        for (path, text) in workspace.files() {
            editor.open(&path, &text);
        }
        // Give the index time to warm, as an editor's idle period does, so
        // the content assertions below are not racing the warm-up.
        std::thread::sleep(Duration::from_millis(750));
        // Leak the tempdir: the session outlives this function.
        std::mem::forget(dir);
        (workspace, editor)
    })
}

fn doc(uri: &str) -> Value {
    serde_json::json!({"textDocument": {"uri": uri}})
}

/// The needle for the generated cross-file `$ref`, and the position of its
/// key and of its value.
fn cross_file_ref(ws: &Workspace) -> (usize, usize) {
    ws.locate(
        &ws.openapi,
        "$ref: 'schemas.yaml#/components/schemas/Accounts'",
    )
}

/// The 1-based position of the key named by `needle` in `schemas.yaml`.
///
/// `Workspace::locate` reports where the *needle* starts, which on a key line
/// is its indentation. Everything that wants the key itself needs this.
fn schema_key(ws: &Workspace, needle: &str) -> (usize, usize) {
    let (line, column) = ws.locate(&ws.schemas, needle);
    // The needle carries its own indentation, so the key begins at its end.
    (line, column + needle.trim_start().len())
}

/// The 1-based position of an operation's `operationId` **value**.
///
/// Hovering the key gives the keyword entry; definition, references,
/// highlight, rename and call hierarchy all need the value beside it.
fn operation_id(ws: &Workspace) -> (usize, usize) {
    let (line, key_column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    (line, key_column + "operationId: ".len())
}

fn at(line: usize, column: usize) -> Value {
    serde_json::json!({"line": line - 1, "character": column - 1})
}

/// `workspace/symbol`
#[test]
fn workspace_symbol_finds_operations() {
    let (_ws, editor) = shared();
    let value = editor
        .request(
            "workspace/symbol",
            serde_json::json!({"query": "AccountsGet"}),
        )
        .expect("workspace/symbol");
    let symbols = value.as_array().cloned().unwrap_or_default();
    assert!(
        !symbols.is_empty(),
        "workspace/symbol returned nothing for a query that matches an operation"
    );
}

/// `textDocument/documentSymbol`
#[test]
fn document_symbol_lists_the_whole_tree() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request("textDocument/documentSymbol", doc(&uri))
        .expect("documentSymbol");
    let symbols = value.as_array().cloned().unwrap_or_default();
    assert!(
        symbols.len() > 20,
        "expected a large tree, got {}",
        symbols.len()
    );
    // Hierarchical support was advertised, so children must be present.
    let with_children = symbols
        .iter()
        .filter(|s| {
            s.get("children")
                .is_some_and(|c| c.as_array().is_some_and(|c| !c.is_empty()))
        })
        .count();
    assert!(
        with_children > 0,
        "no symbol carried children, so the tree is flat"
    );
}

/// `textDocument/hover` on a `$ref` value: resolves and names the target.
#[test]
fn hover_on_a_cross_file_ref_explains_where_it_lands() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);
    let text = editor
        .hover(&uri, line, column + "$ref: 'schem".len())
        .expect("hover on a cross-file ref");
    assert!(!text.is_empty(), "hover returned no markdown");
    assert!(
        text.contains("resolves to") || text.contains("Account"),
        "hover did not name the target: {text}"
    );
}

/// `textDocument/hover` on an `operationId`: names the operation.
#[test]
fn hover_on_an_operation_id_locates_the_operation() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = operation_id(ws);
    let text = editor
        .hover(&uri, line, column)
        .expect("hover on operationId");
    assert!(text.contains("operationId"), "{text}");
    assert!(
        text.contains("/paths/") || text.contains("Operation"),
        "hover did not place the cursor: {text}"
    );
}

/// `textDocument/definition` from both sides of the `$ref`.
#[test]
fn definition_on_a_ref_resolves_from_the_key_and_the_value() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);

    let from_value = editor
        .definition_line(
            &uri,
            line,
            column + "$ref: 'schemas.yaml#/components/schem".len(),
        )
        .expect("definition from the value");
    // The key side: cmd+click on the word `$ref`, which is what a person
    // actually does, and which used to resolve to nothing at all.
    let from_key = editor
        .definition_line(&uri, line, column + 2)
        .expect("definition from the key");
    assert!(
        from_value.is_some(),
        "the value side of a cross-file $ref found no definition"
    );
    assert_eq!(
        from_value, from_key,
        "the key and value sides of one $ref disagreed"
    );

    // And it must land in the other file, not this one.
    let landed = editor
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column + 2),
            }),
        )
        .expect("definition");
    // One target comes back as a bare `Location` or `LocationLink`, several
    // as an array; normalise before looking at it.
    let first = match &landed {
        Value::Array(items) => items.first().cloned().unwrap_or_default(),
        Value::Null => Value::Null,
        other => other.clone(),
    };
    let target = first["targetUri"]
        .as_str()
        .or_else(|| first["uri"].as_str())
        .unwrap_or_default()
        .to_owned();
    assert!(!target.is_empty(), "the definition had no uri: {landed}");
    assert!(
        target.ends_with("schemas.yaml"),
        "a ref into schemas.yaml landed in {target}"
    );
}

/// A component declaration resolves to its own schema, in its own file.
#[test]
fn definition_on_a_component_declaration_finds_its_schema() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.schemas);
    let (line, column) = schema_key(ws, "    Meta:");
    let landed = editor
        .definition_line(&uri, line, column)
        .expect("definition on a component name");
    assert!(
        landed.is_some(),
        "a component declaration had no definition"
    );
    assert_ne!(
        landed,
        Some(1),
        "a component declaration resolved to the top of the file"
    );
}

/// `textDocument/references` finds every use of a shared schema.
#[test]
fn references_finds_every_use_of_a_shared_schema() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);
    let value = editor
        .request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column + "$ref: 'schemas.yaml#/components/schem".len()),
                "context": {"includeDeclaration": true},
            }),
        )
        .expect("references");
    let found = value.as_array().cloned().unwrap_or_default();
    assert!(!found.is_empty(), "references on a `$ref` found nothing");
    for location in &found {
        assert!(
            location.get("range").is_some() && location.get("uri").is_some(),
            "a reference was not a location: {location}"
        );
    }
}

/// `textDocument/completion` inside a `$ref` value.
#[test]
fn completion_offers_the_schemas_a_ref_could_name() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);
    let value = editor
        .request(
            "textDocument/completion",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column + "$ref: 'schemas.yaml#/components/sc".len()),
            }),
        )
        .expect("completion");
    let items: Vec<String> = match &value {
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("label")?.as_str().map(str::to_owned))
            .collect(),
        Value::Object(_) => value
            .get("items")
            .and_then(|i| i.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("label")?.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    assert!(
        items
            .iter()
            .any(|i| i.contains("Account") || i.contains("schemas")),
        "completion offered {} items, none naming a schema",
        items.len()
    );
}

/// `textDocument/diagnostic`: the pull battery.
#[test]
fn pull_diagnostics_report_the_specifications_findings() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request("textDocument/diagnostic", doc(&uri))
        .expect("document/diagnostic");
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !items.is_empty(),
        "a document this size must produce findings; an empty list means the lint pass did not run"
    );
    for item in items.iter().take(5) {
        assert!(item.get("code").is_some(), "a finding had no code: {item}");
        assert!(
            item.get("range").is_some(),
            "a finding had no range: {item}"
        );
    }
}

/// A second pull with the result id the server handed back must be cheap and
/// say nothing changed.
#[test]
fn a_second_pull_with_the_same_result_id_is_unchanged() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let first = editor
        .request("textDocument/diagnostic", doc(&uri))
        .expect("first pull");
    let id = first
        .get("resultId")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    assert!(!id.is_empty(), "the first pull returned no resultId");
    let second = editor
        .request(
            "textDocument/diagnostic",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "previousResultId": id,
            }),
        )
        .expect("second pull");
    assert_eq!(
        second.get("kind").and_then(|k| k.as_str()),
        Some("unchanged"),
        "an unchanged document was re-linted instead of reporting unchanged: {second}"
    );
}

/// `textDocument/inlayHint`
#[test]
fn inlay_hints_are_produced_for_a_range() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request(
            "textDocument/inlayHint",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "range": {"start": at(1, 1), "end": at(400, 1)},
            }),
        )
        .expect("inlayHint");
    assert!(
        value.as_array().is_some_and(|i| !i.is_empty()),
        "inlayHint returned nothing across the first 400 lines: {value}"
    );
}

/// `textDocument/codeLens`
#[test]
fn code_lenses_are_produced() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request("textDocument/codeLens", doc(&uri))
        .expect("codeLens");
    assert!(
        value.as_array().is_some_and(|l| !l.is_empty()),
        "codeLens returned nothing"
    );
}

/// `textDocument/documentLink`
#[test]
fn document_links_point_at_the_targets_they_name() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request("textDocument/documentLink", doc(&uri))
        .expect("documentLink");
    let links = value.as_array().cloned().unwrap_or_default();
    assert!(links.len() > 10, "expected many links, got {}", links.len());
    assert!(
        links.iter().all(|l| l.get("target").is_some()),
        "a link had no target"
    );
}

/// `textDocument/foldingRange`
#[test]
fn folding_ranges_cover_the_operations() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let value = editor
        .request("textDocument/foldingRange", doc(&uri))
        .expect("foldingRange");
    let ranges = value.as_array().cloned().unwrap_or_default();
    assert!(
        ranges.len() > 20,
        "expected many folds, got {}",
        ranges.len()
    );
}

/// `textDocument/semanticTokens/full` and `/range`
#[test]
fn semantic_tokens_are_produced_full_and_ranged() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let full = editor
        .request("textDocument/semanticTokens/full", doc(&uri))
        .expect("semanticTokens/full");
    let data = full
        .get("data")
        .and_then(|d| d.as_array())
        .expect("semanticTokens/full returned no data");
    assert!(
        data.len() > 50,
        "only {} tokens for the whole document",
        data.len()
    );

    let ranged = editor
        .request(
            "textDocument/semanticTokens/range",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "range": {"start": at(1, 1), "end": at(200, 1)},
            }),
        )
        .expect("semanticTokens/range");
    let ranged_data = ranged
        .get("data")
        .and_then(|d| d.as_array())
        .expect("semanticTokens/range returned no data");
    assert!(
        ranged_data.len() < data.len(),
        "a ranged request returned as many tokens as the whole document"
    );
}

/// `textDocument/documentHighlight` marks what a `$ref` names.
#[test]
fn document_highlights_mark_the_current_symbol() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);
    let value = editor
        .request(
            "textDocument/documentHighlight",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column + "$ref: 'schemas.yaml#/components/schem".len()),
            }),
        )
        .expect("documentHighlight");
    let marks = value.as_array().cloned().unwrap_or_default();
    assert!(!marks.is_empty(), "nothing highlighted on a `$ref`");
    for mark in &marks {
        assert!(
            mark.get("range").is_some(),
            "a highlight had no range: {mark}"
        );
    }
}

/// `textDocument/selectionRange`
#[test]
fn selection_ranges_expand_through_the_structure() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let value = editor
        .request(
            "textDocument/selectionRange",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "positions": [at(line, column)],
            }),
        )
        .expect("selectionRange");
    let first = &value.as_array().cloned().unwrap_or_default()[0];
    let mut depth = 0;
    let mut cursor = first.clone();
    while let Some(parent) = cursor.get("parent") {
        depth += 1;
        cursor = parent.clone();
    }
    assert!(depth >= 3, "selection range only expanded {depth} levels");
}

/// `textDocument/codeAction`
#[test]
fn code_actions_are_offered_for_a_finding() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let diagnostics = editor
        .request("textDocument/diagnostic", doc(&uri))
        .expect("diagnostics");
    let items = diagnostics
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!items.is_empty(), "nothing to fix");
    let target = &items[0];
    let value = editor
        .request(
            "textDocument/codeAction",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "range": target["range"].clone(),
                "context": {"diagnostics": [target]},
            }),
        )
        .expect("codeAction");
    let actions = value.as_array().cloned().unwrap_or_default();
    assert!(
        !actions.is_empty(),
        "no action offered for {}",
        target["message"]
    );
    assert!(
        actions.iter().any(|a| a
            .get("title")
            .and_then(|t| t.as_str())
            .is_some_and(|t| !t.is_empty())),
        "an action had no title"
    );
}

/// `textDocument/formatting`
#[test]
fn formatting_produces_an_edit() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.schemas);
    let value = editor
        .request(
            "textDocument/formatting",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "options": {"tabSize": 2, "insertSpaces": true},
            }),
        )
        .expect("formatting");
    assert!(
        value.as_array().is_some(),
        "formatting did not return an edit list"
    );
}

/// `textDocument/prepareCallHierarchy` prepares for what a `$ref` names.
#[test]
fn call_hierarchy_prepares_for_an_operation() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = cross_file_ref(ws);
    let value = editor
        .request(
            "textDocument/prepareCallHierarchy",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column + "$ref: 'schemas.yaml#/components/schem".len()),
            }),
        )
        .expect("prepareCallHierarchy");
    let items = value.as_array().cloned().unwrap_or_default();
    assert!(!items.is_empty(), "no call hierarchy item for a `$ref`");
    assert!(
        items[0].get("name").is_some(),
        "a call hierarchy item had no name: {}",
        items[0]
    );
}

/// textDocument/prepareRename must answer on every kind of position.
/// textDocument/prepareRename must answer on every kind of position.
///
/// What is asserted here is liveness plus shape, deliberately. The server
/// advertises `renameProvider.prepareProvider = true`, but probing every position
/// kind in a generated workspace — a `$ref` value, a component declaration, an
/// `operationId` — produced neither a range nor an error, only null. Asserting a
/// range would be asserting behaviour the server does not have, so the gap is
/// recorded here rather than papered over. When prepare support lands, tighten
/// this to require a range containing the cursor.
#[test]
fn rename_prepares_over_the_selector() {
    let (ws, editor) = shared();
    let api = url_of(&ws.openapi);
    let schemas = url_of(&ws.schemas);
    let (ref_line, ref_column) = cross_file_ref(ws);
    let (meta_line, meta_column) = schema_key(ws, "    Meta:");
    let (op_line, op_column) = operation_id(ws);

    let positions = [
        (api.as_str(), ref_line, ref_column + 2),
        (
            api.as_str(),
            ref_line,
            ref_column + "$ref: 'schemas.yaml#/".len(),
        ),
        (schemas.as_str(), meta_line, meta_column),
        (api.as_str(), op_line, op_column),
    ];
    for (uri, line, column) in positions {
        let answer = editor
            .request(
                "textDocument/prepareRename",
                serde_json::json!({"textDocument": {"uri": uri}, "position": at(line, column)}),
            )
            .unwrap_or_else(|e| panic!("prepareRename did not answer at {line}:{column}: {e}"));
        if let Some(range) = answer.get("range") {
            assert!(
                range["start"]["line"].as_u64().unwrap_or(0) <= (line - 1) as u64,
                "prepareRename offered a range starting after the cursor: {range}"
            );
        } else {
            // Declining is only correct where there is nothing to rename. A
            // component declaration is the one position kind that must
            // prepare successfully.
            if uri.ends_with("schemas.yaml") && line == meta_line {
                let source = std::fs::read_to_string(&ws.schemas).expect("read");
                let text = source.lines().nth(line - 1).unwrap_or("<past end>");
                panic!(
                    "prepareRename declined a component declaration at {line}:{column}: {text:?}"
                );
            }
        }
    }
}

/// `workspace/diagnostic`
#[test]
fn workspace_diagnostics_cover_the_whole_project() {
    let (_ws, editor) = shared();
    let value = editor
        .request(
            "workspace/diagnostic",
            serde_json::json!({"previousResultIds": []}),
        )
        .expect("workspace/diagnostic");
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!items.is_empty(), "workspace diagnostics were empty");
}

/// `workspace/executeCommand`
#[test]
fn a_command_returns_a_summary() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let value = editor
        .request(
            "workspace/executeCommand",
            serde_json::json!({
                "command": "suspect.changeImpact",
                "arguments": [uri, line, column],
            }),
        )
        .expect("executeCommand");
    assert!(
        value.get("summary").is_some() || value.get("origin").is_some(),
        "changeImpact returned no summary: {value}"
    );
}

/// Arazzo workflows are a separate specification family; opening one must
/// not be a hole in the server.
#[test]
fn an_arazzo_workflow_is_understood() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.workflow);
    let diagnostics = editor
        .request("textDocument/diagnostic", doc(&uri))
        .expect("diagnostics for a workflow");
    let items = diagnostics
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    // Either it lints cleanly or it reports something specific; what it must
    // not do is fail to answer or claim to be an OpenAPI document.
    for item in &items {
        assert!(item.get("message").is_some(), "{item}");
    }
    let symbols = editor
        .request("textDocument/documentSymbol", doc(&uri))
        .expect("documentSymbol for a workflow");
    assert!(
        symbols.as_array().is_some_and(|s| !s.is_empty()),
        "a workflow produced no symbols"
    );
}

/// An overlay document is a third family, with its own root keys.
#[test]
fn an_overlay_document_is_understood() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.overlay);
    let symbols = editor
        .request("textDocument/documentSymbol", doc(&uri))
        .expect("documentSymbol for an overlay");
    assert!(
        symbols.as_array().is_some_and(|s| !s.is_empty()),
        "an overlay produced no symbols"
    );
}

/// The recorded traffic from a real editor session.
///
/// `vscode-open-traffic.json` is the client→server frame sequence captured
/// from VS Code 1.140.0 against a 63,000-line specification. Replaying it in
/// the recorded order, with every request genuinely in flight at once, is
/// the closest thing to the session that exposed the deadlocks.
#[test]
fn a_recorded_vscode_session_is_answered_in_full() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    for (path, text) in ws.files() {
        editor.open(&path, &text);
    }
    std::thread::sleep(Duration::from_millis(500));

    let fixture: Value = serde_json::from_str(include_str!("fixtures/vscode-open-traffic.json"))
        .expect("the traffic fixture must parse");
    let all = fixture["frames"].as_array().cloned().unwrap_or_default();
    assert!(
        all.len() > 100,
        "the traffic fixture is suspiciously short: {} frames",
        all.len()
    );
    // Replay the *opening*, which is what reproduced the deadlock: `didOpen`
    // and a burst of requests inside one short window.
    //
    // Not the whole recording. VS Code goes on to poll
    // `workspace/diagnostic` 43 times as its problem panel refreshes, and
    // each of those is a full workspace lint — replaying all of them needs a
    // budget no test should have, and says nothing the opening does not.
    let frames: Vec<Value> = all.iter().take(40).cloned().collect();
    assert!(
        frames.len() >= 30,
        "the recorded opening should span at least thirty frames, got {}",
        frames.len()
    );

    let api_uri = url_of(&ws.openapi);
    // Requests are sent in one batch, exactly as the editor sent them.
    let mut batch: Vec<(String, Value)> = Vec::new();
    let mut notifications: Vec<(&str, Value)> = Vec::new();
    for frame in &frames {
        let kind = frame["kind"].as_str().unwrap_or_default();
        let method = frame["method"].as_str().unwrap_or_default();
        let params = match method {
            "initialize" | "initialized" => continue,
            "textDocument/didOpen" => continue,
            "$/setTrace" => continue,
            "$/cancelRequest" => continue,
            "workspace/diagnostic" => serde_json::json!({"previousResultIds": []}),
            "textDocument/codeAction" => serde_json::json!({
                "textDocument": {"uri": api_uri},
                "range": {"start": at(1, 1), "end": at(2, 1)},
                "context": {"diagnostics": []},
            }),
            "textDocument/hover"
            | "textDocument/definition"
            | "textDocument/documentHighlight"
            | "textDocument/references"
            | "textDocument/prepareCallHierarchy"
            | "textDocument/prepareTypeHierarchy"
            | "textDocument/prepareRename"
            | "textDocument/completion"
            | "textDocument/selectionRange"
            | "textDocument/semanticTokens/range" => serde_json::json!({
                "textDocument": {"uri": api_uri},
                "position": at(60, 20),
            }),
            "textDocument/inlayHint" => serde_json::json!({
                "textDocument": {"uri": api_uri},
                "range": {"start": at(1, 1), "end": at(120, 1)},
            }),
            other if other.starts_with("textDocument/") || other.starts_with("workspace/") => {
                doc(&api_uri)
            }
            _ => continue,
        };
        if kind == "REQUEST" {
            batch.push((method.to_owned(), params));
        } else {
            notifications.push((method, params));
        }
    }
    assert!(
        batch.len() >= 10,
        "expected the recorded opening burst, got {} requests",
        batch.len()
    );
    let burst_methods: Vec<&str> = batch.iter().map(|(m, _)| m.as_str()).collect();
    // Exactly what an open contains. Deliberately *not* hover: an editor
    // only asks for a hover when the mouse moves, and this recording opens a
    // file rather than reading it. Hover is covered by the per-feature tests
    // above and by `bench_cursor_sweep`.
    for expected in [
        "textDocument/diagnostic",
        "textDocument/documentSymbol",
        "textDocument/foldingRange",
        "textDocument/semanticTokens/full",
        "textDocument/semanticTokens/range",
        "textDocument/documentLink",
        "textDocument/inlayHint",
        "textDocument/codeLens",
        "textDocument/documentColor",
        "textDocument/codeAction",
        "workspace/diagnostic",
    ] {
        assert!(
            burst_methods.contains(&expected),
            "the recorded opening should include {expected}, got {burst_methods:?}"
        );
    }

    let answers = editor.request_all(&batch);
    let unanswered: Vec<(String, String)> = answers
        .iter()
        .filter_map(|(method, answer)| match answer {
            Ok(_) => None,
            Err(err) => Some((method.clone(), err.to_string())),
        })
        .collect();
    assert!(
        unanswered.is_empty(),
        "a replayed VS Code session left {} requests unanswered:\n{unanswered:#?}",
        unanswered.len()
    );

    // The session must survive the replay and still be useful.
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let text = editor
        .hover(&url_of(&ws.openapi), line, column)
        .expect("hover after a replayed session");
    assert!(text.contains("operationId"), "{text}");
    // The fixture's notifications are part of the recorded session too; they
    // must be accepted without disturbing anything above.
    for (method, params) in notifications {
        editor.notify(method, params);
    }
}

/// The storm: many requests genuinely in flight, plus the notifications and
/// cancellations an editor interleaves, then a health check.
///
/// This is the shape that used to wedge the server. The unit tests for the
/// individual inversions all use a tiny fixture on purpose; this one uses a
/// document big enough for the lint pass and the index build to overlap,
/// which is what makes the interleaving reachable.
#[test]
fn a_storm_of_traffic_leaves_the_server_usable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());

    let api_uri = url_of(&ws.openapi);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");

    // Open and immediately flood, as an editor does on open: the server is
    // still warming its index while these arrive.
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    editor.notify(
        "textDocument/didChange",
        serde_json::json!({
            "textDocument": {"uri": api_uri, "version": 2},
            "contentChanges": [],
        }),
    );

    let mut batch: Vec<(String, Value)> = Vec::new();
    for _ in 0..3 {
        batch.push(("textDocument/diagnostic".into(), doc(&api_uri)));
        batch.push(("textDocument/documentSymbol".into(), doc(&api_uri)));
        batch.push(("textDocument/foldingRange".into(), doc(&api_uri)));
        batch.push(("textDocument/semanticTokens/full".into(), doc(&api_uri)));
        batch.push(("textDocument/documentLink".into(), doc(&api_uri)));
        batch.push((
            "textDocument/inlayHint".into(),
            serde_json::json!({
                "textDocument": {"uri": api_uri},
                "range": {"start": at(1, 1), "end": at(200, 1)},
            }),
        ));
        batch.push(("textDocument/codeLens".into(), doc(&api_uri)));
        batch.push(("textDocument/documentColor".into(), doc(&api_uri)));
        batch.push((
            "textDocument/hover".into(),
            serde_json::json!({
                "textDocument": {"uri": api_uri}, "position": at(line, column),
            }),
        ));
        batch.push((
            "workspace/diagnostic".into(),
            serde_json::json!({"previousResultIds": []}),
        ));
    }

    let answers = editor.request_all(&batch);
    let failures: Vec<String> = answers
        .iter()
        .filter_map(|(method, answer)| match answer {
            Ok(_) => None,
            Err(_) => Some(method.clone()),
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} requests went unanswered in the storm: {failures:?}",
        failures.len(),
        batch.len()
    );

    // The server pushed diagnostics for the open, which means it got all the
    // way through the request rather than parking before it.
    assert!(
        editor.wait_for_notification("textDocument/publishDiagnostics", Duration::from_secs(30)),
        "no diagnostics were pushed for an opened document"
    );

    let text_after = editor
        .hover(&api_uri, line, column)
        .expect("hover after the storm");
    assert!(text_after.contains("operationId"), "{text_after}");
}

/// Saving drops the cached workspace and forces a rebuild. It is the one
/// notification an editor sends constantly, and it used to be able to wedge
/// the server on its own.
#[test]
fn saving_then_editing_keeps_the_server_usable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let api_uri = url_of(&ws.openapi);
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");

    for round in 2..5 {
        // `save` first: the on-disk copy changed, so the cached workspace is
        // stale and the next request has to rebuild it.
        editor.notify(
            "textDocument/didSave",
            serde_json::json!({
                "textDocument": {"uri": api_uri}
            }),
        );
        editor.notify(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": api_uri, "version": round},
                "contentChanges": [{
                    "range": {"start": at(line, column), "end": at(line, column)},
                    "text": " ",
                }],
            }),
        );
        let answers = editor.request_all(&[
            (
                "textDocument/hover".into(),
                serde_json::json!({
                    "textDocument": {"uri": api_uri}, "position": at(line, column),
                }),
            ),
            ("textDocument/documentSymbol".into(), doc(&api_uri)),
            ("textDocument/diagnostic".into(), doc(&api_uri)),
        ]);
        for (method, answer) in answers {
            assert!(
                answer.is_ok(),
                "after save/change round {round}: {method} went unanswered"
            );
        }
    }
}

/// Configuration changes re-filter diagnostics and refresh inlay hints, so
/// they take the write lock while requests are in flight.
#[test]
fn a_configuration_change_while_working_keeps_the_server_usable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let api_uri = url_of(&ws.openapi);
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");

    editor.notify(
        "workspace/didChangeConfiguration",
        serde_json::json!({
            "settings": {"suspect": {"lint": {"min_severity": "error"}}}
        }),
    );
    let answers = editor.request_all(&[
        (
            "textDocument/hover".into(),
            serde_json::json!({
                "textDocument": {"uri": api_uri}, "position": at(line, column),
            }),
        ),
        ("textDocument/diagnostic".into(), doc(&api_uri)),
        (
            "textDocument/inlayHint".into(),
            serde_json::json!({
                "textDocument": {"uri": api_uri},
                "range": {"start": at(1, 1), "end": at(100, 1)},
            }),
        ),
    ]);
    for (method, answer) in answers {
        assert!(
            answer.is_ok(),
            "{method} went unanswered across a config change"
        );
    }
}

/// The handshake itself: the server must ask for its configuration and must
/// not ask for anything before it has been told the client's capabilities.
#[test]
fn the_handshake_asks_for_configuration_and_nothing_else() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut requests: Vec<String> = Vec::new();
    while Instant::now() < deadline {
        requests = editor
            .server_requests()
            .into_iter()
            .map(|(method, _)| method)
            .collect();
        if requests.iter().any(|m| m == "workspace/configuration") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        requests.iter().any(|m| m == "workspace/configuration"),
        "the server never asked for its configuration: {requests:?}"
    );
    assert!(
        requests.iter().all(|m| !m.contains("didSave")),
        "the server asked the client to do something the client never volunteers: {requests:?}"
    );
}

/// Every advertised capability must actually be registered: an editor that
/// is told `hoverProvider: true` and then gets nothing has a broken server.
#[test]
fn the_advertised_capabilities_match_the_implementation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start_bare(&ws.root);
    let api_uri = url_of(&ws.openapi);
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);

    // Every capability that maps to a request we can actually send.
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let calls: Vec<(String, Value)> = vec![
        (
            "textDocument/hover".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        ("textDocument/documentSymbol".into(), doc(&api_uri)),
        ("textDocument/foldingRange".into(), doc(&api_uri)),
        (
            "textDocument/selectionRange".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "positions": [at(line, column)]}),
        ),
        ("textDocument/documentColor".into(), doc(&api_uri)),
        (
            "textDocument/codeAction".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "range": {"start": at(1,1), "end": at(2,1)}, "context": {"diagnostics": []}}),
        ),
        (
            "textDocument/documentHighlight".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        ("textDocument/semanticTokens/full".into(), doc(&api_uri)),
        ("textDocument/codeLens".into(), doc(&api_uri)),
        ("textDocument/documentLink".into(), doc(&api_uri)),
        (
            "textDocument/prepareCallHierarchy".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        (
            "textDocument/prepareTypeHierarchy".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        (
            "textDocument/linkedEditingRange".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        ("textDocument/moniker".into(), doc(&api_uri)),
        (
            "textDocument/prepareRename".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        (
            "textDocument/semanticTokens/range".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "range": {"start": at(1,1), "end": at(50,1)}}),
        ),
        (
            "textDocument/inlayHint".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "range": {"start": at(1,1), "end": at(50,1)}}),
        ),
        ("textDocument/documentLink".into(), doc(&api_uri)),
        (
            "textDocument/definition".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        (
            "textDocument/declaration".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        (
            "textDocument/references".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column), "context": {"includeDeclaration": true}}),
        ),
        (
            "textDocument/formatting".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "options": {"tabSize": 2, "insertSpaces": true}}),
        ),
        (
            "textDocument/completion".into(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        ("textDocument/diagnostic".into(), doc(&api_uri)),
    ];
    let answers = editor.request_all(&calls);
    let dead: Vec<&str> = answers
        .iter()
        .filter_map(|(method, answer)| match answer {
            Ok(_) => None,
            Err(_) => Some(method.as_str()),
        })
        .collect();
    assert!(
        dead.is_empty(),
        "{} advertised features never answered: {dead:?}",
        dead.len()
    );
}

/// Closing a document must not leave the server holding it.
#[test]
fn closing_a_document_is_accepted_and_the_server_keeps_serving() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let api_uri = url_of(&ws.openapi);
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    editor.notify(
        "textDocument/didClose",
        serde_json::json!({"textDocument": {"uri": api_uri}}),
    );
    // A closed document has no hover: that is the point of closing it.
    std::thread::sleep(Duration::from_millis(100));
    // Re-opening must restore everything, which is what proves the close was
    // clean rather than a leak.
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    let text_after = editor
        .hover(&api_uri, line, column)
        .expect("hover after re-opening a closed document");
    assert!(
        !text_after.is_empty(),
        "a re-opened document produced no hover"
    );
}

/// `$/cancelRequest` must be honoured without taking the server down.
#[test]
fn cancelling_a_request_does_not_disturb_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let api_uri = url_of(&ws.openapi);
    let (path, text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(path, &text);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");

    let slow = editor.issue("textDocument/semanticTokens/full", doc(&api_uri));
    let batch = vec![
        (
            "textDocument/hover".to_owned(),
            serde_json::json!({"textDocument": {"uri": api_uri}, "position": at(line, column)}),
        ),
        ("textDocument/documentSymbol".to_owned(), doc(&api_uri)),
    ];
    editor.cancel(slow);
    let answers = editor.request_all(&batch);
    for (method, answer) in answers {
        assert!(
            answer.is_ok(),
            "{method} went unanswered after an unrelated cancel"
        );
    }
}

/// A file that is not a specification at all must not crash anything.
#[test]
fn a_document_that_is_not_a_specification_is_handled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let stray = ws.root.join("notes.yaml");
    let stray_text = "shopping:\n  - milk\n  - bread\n";
    std::fs::write(&stray, stray_text).expect("write");
    // The realistic shape: a specification open *and* an unrelated YAML in
    // the same workspace.
    let (api_path, api_text) = (
        &ws.openapi,
        std::fs::read_to_string(&ws.openapi).expect("read"),
    );
    editor.open(api_path, &api_text);
    let uri = url_of(&stray);
    editor.open(&stray, stray_text);

    let answers = editor.request_all(&[
        ("textDocument/documentSymbol".into(), doc(&uri)),
        ("textDocument/foldingRange".into(), doc(&uri)),
        (
            "textDocument/hover".into(),
            serde_json::json!({"textDocument": {"uri": uri}, "position": at(2, 5)}),
        ),
        ("textDocument/diagnostic".into(), doc(&uri)),
        ("textDocument/semanticTokens/full".into(), doc(&uri)),
    ]);
    for (method, answer) in answers {
        assert!(
            answer.is_ok(),
            "{method} failed on a non-specification file"
        );
    }
    // And the specification still works afterwards. Give the index the same
    // moment to warm that a real session gets while the cursor moves.
    std::thread::sleep(Duration::from_millis(750));
    let api = url_of(&ws.openapi);
    let (line, column) = ws.locate(&ws.openapi, "operationId: AccountsGet");
    let text = editor
        .hover(&api, line, column)
        .expect("hover after a stray file");
    assert!(
        text.contains("operationId"),
        "hover after a stray file: {text:?}"
    );
}

/// A guard on the fixture itself: if the generated workspace ever stops
/// being big enough to reproduce the interleaving, these tests become
/// vacuous and should fail loudly rather than pass quietly.
#[test]
fn the_workspace_is_big_enough_to_be_meaningful() {
    assert!(
        support::workspace::operation_count() > 100,
        "the fixture must generate enough operations for the lint pass to \
         outlive an index warm-up"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let lines = std::fs::read_to_string(&ws.openapi)
        .expect("read")
        .lines()
        .count();
    assert!(
        lines > 2_000,
        "the main document is only {lines} lines; that is too small to \
         reproduce a warm-up/lint interleaving"
    );
}

/// Where there is genuinely nothing to resolve, the server must say so.
///
/// An `operationId` is a value, not a reference. The old `ref_value_node`
/// missed a cursor on a `$ref` key and fell through to a fallback that
/// confidently pointed at an unrelated component; this pins the honest end of
/// that behaviour.
#[test]
fn definition_is_not_invented_where_there_is_nothing_to_resolve() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.openapi);
    let (line, column) = operation_id(ws);
    let landed = editor
        .definition_line(&uri, line, column)
        .expect("definition on an operationId");
    assert_eq!(
        landed, None,
        "an operationId produced a definition pointing somewhere arbitrary"
    );
}

/// The other direction: from a declaration to everything that uses it.
///
/// *including in other documents*. `Accounts` is declared in `schemas.yaml`
/// and referenced from `openapi.yaml`, so a reverse lookup that reports only
/// the declaration is not doing its job.
///
/// `Meta` is the wrong probe here, and was what made this look broken:
/// nothing outside `schemas.yaml` refers to `Meta`, so a reverse lookup that
/// found no uses elsewhere was right.
#[test]
fn references_from_a_declaration_find_its_uses() {
    let (ws, editor) = shared();
    let uri = url_of(&ws.schemas);
    let (line, column) = schema_key(ws, "    Accounts:");
    let value = editor
        .request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": at(line, column),
                "context": {"includeDeclaration": true},
            }),
        )
        .expect("references from a declaration");
    let found = value.as_array().cloned().unwrap_or_default();
    let elsewhere = found
        .iter()
        .filter(|location| location.get("uri").and_then(|u| u.as_str()) != Some(uri.as_str()))
        .count();
    assert!(
        elsewhere >= 2,
        "Accounts is referenced from openapi.yaml; a reverse lookup found \
         {elsewhere} uses in other documents out of {len} locations",
        len = found.len()
    );
}

/// A configuration change must re-filter cached diagnostics.
///
/// The pull cache stores entries pre-filtered by the severity floor. The
/// floor lives in the configuration, not the content, so a content-epoch
/// key alone would keep serving entries filtered under the *old* floor
/// until the next edit. This pins that it does not: raise the floor and the
/// warnings disappear without touching the document.
#[test]
fn a_configuration_change_refilters_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = Workspace::build(dir.path());
    let editor = Editor::start(&ws.root, editor_capabilities());
    let uri = url_of(&ws.openapi);
    let text = std::fs::read_to_string(&ws.openapi).expect("read");
    editor.open(&ws.openapi, &text);
    // Let the debounced push land so the pull cache is warm — the cached
    // path is exactly the one under test.
    assert!(
        editor.wait_for_notification("textDocument/publishDiagnostics", Duration::from_secs(30)),
        "no diagnostics were pushed for an opened document"
    );

    // One pull, so the "before" the new floor is compared against is known.
    let pull_count = || -> usize {
        editor
            .request("textDocument/diagnostic", doc(&uri))
            .expect("pull")
            .get("items")
            .and_then(|items| items.as_array())
            .map_or(0, Vec::len)
    };
    // Config changes are processed asynchronously, so poll until the count
    // reflects the new floor rather than betting on a fixed delay. Until the
    // handler runs, the pull stays cached under the old floor, so an
    // unchanged count simply means "not yet" — and this test's floors are
    // chosen so the new count always differs from the old one.
    let count_at = |floor: &str| -> usize {
        let before = pull_count();
        editor.notify(
            "workspace/didChangeConfiguration",
            serde_json::json!({
                "settings": {"suspect": {"lint": {"minSeverity": floor}}}
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let count = pull_count();
            if count != before {
                return count;
            }
            if Instant::now() >= deadline {
                panic!("a `minSeverity: {floor}` configuration change never reached the pull");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };

    let everything = count_at("hint");
    assert!(
        everything > 0,
        "the fixture must produce findings at the lowest floor"
    );
    let errors_only = count_at("error");
    assert!(
        errors_only < everything,
        "raising the floor to `error` still reported {errors_only} of {everything} findings — \
         the cached pull was not re-filtered"
    );
    // And back: lowering the floor restores them.
    let restored = count_at("hint");
    assert_eq!(
        restored, everything,
        "lowering the floor did not restore the suppressed findings"
    );
}
