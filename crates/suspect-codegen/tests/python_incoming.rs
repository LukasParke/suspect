//! Emitted-only incoming receipt helpers for the Python HTTP backend:
//! `python/src/<pkg>/_incoming.py`, the py_compile gate, and native behavior
//! of a fake webhook POST through the generated decoder. Plans without
//! incoming declarations emit no module at all.

use std::{fs, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    write_files,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const IMPORT: &str = "incoming_sdk";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "security": [{"apiKey": []}],
        "components": {
            "securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"]
                },
                "Event": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}},
                    "required": ["kind"]
                },
                "Pong": {
                    "type": "object",
                    "properties": {"pong": {"type": "boolean"}},
                    "required": ["pong"]
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}},
                "callbacks": {
                    "onEvent": {
                        "{$request.body#/callbackUrl}": {
                            "post": {
                                "operationId": "eventReceived",
                                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Event"}}}},
                                "responses": {"200": {"description": "ok"}}
                            }
                        }
                    }
                }
            }}
        },
        "webhooks": {
            "newIssue": {
                "post": {
                    "operationId": "onNewIssue",
                    "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                    "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                    "responses": {"204": {"description": "Accepted"}}
                }
            },
            "ping": {
                "post": {
                    "operationId": "onPing",
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
                }
            }
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "incoming-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    }
}

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn incoming_file(files: &[OutFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == format!("python/src/{IMPORT}/_incoming.py"))
        .expect("the incoming module is emitted for a webhook/callback contract")
        .content
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate(document());
    let incoming = incoming_file(&files);
    for expected in [
        "def decode_new_issue_webhook(headers: Mapping[str, str], body: bytes | str) -> models.",
        "require_header(headers, \"x-signature\", _NEW_ISSUE_SOURCE)",
        "codecs.OnNewIssueRequestCodec.decode(body_text(body, _NEW_ISSUE_SOURCE))",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "def construct_new_issue_response() -> tuple[int, dict[str, str], bytes]:",
        "return (204, {}, b'')",
        // The 200 reply encodes through the response codec.
        "def construct_ping_response(result: models.OnPingResponse200) -> tuple[int, dict[str, str], bytes]:",
        "codecs.OnPingResponse200Codec.encode(result).encode('utf-8')",
        // The callback receipt carries its runtime expression verbatim.
        "SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE = {'method': 'POST', 'route': '{$request.body#/callbackUrl}', 'expression': True}",
        "def decode_subscribe_on_event_webhook(headers: Mapping[str, str], body: bytes | str) -> models.EventReceivedRequest:",
        // The frozen compiled receipt descriptors.
        "INCOMING_DESCRIPTORS: Mapping[str, Mapping[str, object]] = {",
        "'payload': 'json', 'payload_codec': 'OnNewIssueRequest'",
        "'reply': 'none', 'reply_status': 204",
    ] {
        assert!(
            incoming.contains(expected),
            "_incoming.py lacks {expected}\n{incoming}"
        );
    }
    assert!(incoming.contains(
        "raise SdkError('incoming-request', _NEW_ISSUE_SOURCE, code='incoming-request')"
    ));

    // Without declared receipts nothing new is emitted, with or without an
    // empty webhooks map.
    let mut without = document();
    without
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    without["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}}}
    });
    let mut emptied = document();
    emptied["webhooks"] = json!({});
    emptied["paths"]["/subscribe"] = without["paths"]["/subscribe"].clone();
    let control = generate(without);
    let emptied = generate(emptied);
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("_incoming.py")),
        "a receipt-less contract must emit no incoming module"
    );
    assert!(
        !emptied
            .iter()
            .any(|file| file.path.ends_with("_incoming.py")),
        "an empty webhooks map must emit no incoming module"
    );
}

fn checked(command: &mut Command, cwd: &std::path::Path, stage: &'static str) {
    let output = command.current_dir(cwd).output().unwrap();
    assert!(
        output.status.success(),
        "{stage} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn interpreter_with_httpx() -> Option<String> {
    let python = std::env::var_os("SUSPECT_PYTHON_BIN")
        .unwrap_or_else(|| "python3".into())
        .to_string_lossy()
        .to_string();
    let probe = Command::new(&python)
        .args(["-c", "import httpx"])
        .output()
        .ok()?;
    probe.status.success().then_some(python)
}

const BEHAVIOR: &str = r#"
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

from incoming_sdk import _incoming, model_codecs
from incoming_sdk._types import SdkError


def valid_body():
    return json.dumps({"id": "i1", "title": "t"})


# A valid fake webhook POST decodes into the declared model, regardless of
# header-name casing.
payload = _incoming.decode_new_issue_webhook({"X-Signature": "abc"}, valid_body())
assert payload.id == "i1", payload
assert payload.title == "t", payload

# A missing required header is the branded incoming-request failure.
for headers in ({}, {"x-other": "1"}):
    try:
        _incoming.decode_new_issue_webhook(headers, valid_body())
    except SdkError as error:
        assert error.kind == "incoming-request", error.kind
        assert error.code == "incoming-request", error.code
    else:
        raise AssertionError("the missing required header must fail")

# An invalid payload is a branded failure too.
for body in (json.dumps({"id": "i1"}), ""):
    try:
        _incoming.decode_new_issue_webhook({"x-signature": "abc"}, body)
    except SdkError as error:
        assert error.kind == "incoming-request", error.kind
    else:
        raise AssertionError("the invalid payload must fail")

# The declared 204 reply constructs with the pinned status and no body.
assert _incoming.construct_new_issue_response() == (204, {}, b"")

# The declared 200 reply encodes its body through the response codec.
pong = model_codecs.OnPingResponse200Codec.decode(json.dumps({"pong": True}))
status, headers, body = _incoming.construct_ping_response(pong)
assert status == 200, status
assert headers == {}, headers
assert body == model_codecs.OnPingResponse200Codec.encode(pong).encode("utf-8"), body

# The callback receipt decodes through its own declared schema and carries its
# route expression verbatim.
event = _incoming.decode_subscribe_on_event_webhook({}, json.dumps({"kind": "open"}))
assert event.kind == "open", event
assert _incoming.INCOMING_DESCRIPTORS["subscribe_on_event"]["route"] == "{$request.body#/callbackUrl}"
assert _incoming.INCOMING_DESCRIPTORS["subscribe_on_event"]["expression"] is True
assert _incoming.INCOMING_DESCRIPTORS["new_issue"]["reply_status"] == 204

print("incoming behavior verified")
"#;

#[test]
fn fake_webhook_posts_drive_the_decoder_and_constructor_in_python() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(document());
    write_files(&files, root.path()).unwrap();

    // Every emitted Python file must at least be valid bytecode.
    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile",
            );
        }
    }

    let Some(python) = interpreter_with_httpx() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(root.path().join("behavior.py"), BEHAVIOR).unwrap();
    checked(
        Command::new(&python).arg(root.path().join("behavior.py")),
        root.path(),
        "behavior",
    );
}
