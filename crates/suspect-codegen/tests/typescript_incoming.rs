//! Emitted-only incoming receipt helpers for the TypeScript HTTP backend:
//! `typescript/incoming.ts`, strict compilation, and native node behavior
//! against a fake webhook POST through the generated decoder. Plans without
//! incoming declarations emit no file at all.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig, generate_with_options};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

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

/// Two contracts over one document URI, so their emitted bytes are comparable.
fn contracts_pair(first: Value, second: Value) -> (Arc<Contract>, Arc<Contract>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let build = |document: &Value| {
        std::fs::write(&path, document.to_string()).unwrap();
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
    };
    let first = build(&first);
    let second = build(&second);
    (first, second)
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
        backend: Backend::TypescriptHttp,
        package_name: "@incoming/fixture".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    }
}

fn selected(contract: &Arc<Contract>) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn generate(document: Value) -> Vec<suspect_codegen::OutFile> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn file<'a>(files: &'a [suspect_codegen::OutFile], path: &str) -> Option<&'a str> {
    files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.content.as_str())
}

fn paths(files: &[suspect_codegen::OutFile]) -> Vec<&str> {
    files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>()
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate(document());
    let incoming = file(&files, "typescript/incoming.ts")
        .expect("the incoming module is emitted for a webhook/callback contract");
    for expected in [
        "export type NewIssuePayload = Models.OnNewIssueRequest;",
        "export const NewIssueWebhookRoute: IncomingRoute = /* @__PURE__ */ Object.freeze({ method: \"POST\", route: \"newIssue\", expression: false } as const);",
        "export function decodeNewIssueWebhook(headers: Readonly<Record<string, string>>, body: Uint8Array | string): NewIssuePayload {",
        "requireHeader(headers, \"x-signature\");",
        "Codecs.OnNewIssueRequestCodec.decode",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "export function constructNewIssueResponse(): ConstructedResponse {",
        "return { status: 204, headers: {}, body: '' };",
        // The 200 reply encodes through the response codec.
        "export function constructPingResponse(result: Models.OnPingResponse200): ConstructedResponse {",
        "Codecs.OnPingResponse200Codec.encode(result)",
        // The callback receipt carries its runtime expression verbatim.
        "export const SubscribeOnEventWebhookRoute: IncomingRoute = /* @__PURE__ */ Object.freeze({ method: \"POST\", route: \"{$request.body#/callbackUrl}\", expression: true } as const);",
        "export function decodeSubscribeOnEventWebhook(headers: Readonly<Record<string, string>>, body: Uint8Array | string): SubscribeOnEventPayload {",
        // The branded failure and the frozen compiled descriptors.
        "export class IncomingRequestError extends Error {",
        "export function isIncomingRequestError(",
        "export const incomingDescriptors = /* @__PURE__ */ Object.freeze({",
        "payload: \"json\", payloadCodec: \"OnNewIssueRequest\"",
        "reply: \"none\", replyStatus: 204",
    ] {
        assert!(
            incoming.contains(expected),
            "incoming.ts lacks {expected}\n{incoming}"
        );
    }

    // Without declared receipts nothing new is emitted: no incoming module at
    // all, with or without an empty webhooks map. (Byte identity for documents
    // without receipts is pinned separately by the credential-env fixtures.)
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
    let (control_contract, emptied_contract) = contracts_pair(without, emptied);
    let control = generate_with_options(
        control_contract.clone(),
        &selected(&control_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    let emptied = generate_with_options(
        emptied_contract.clone(),
        &selected(&emptied_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    assert!(file(&control, "typescript/incoming.ts").is_none());
    assert!(file(&emptied, "typescript/incoming.ts").is_none());
    assert!(
        !paths(&control).iter().any(|path| path.contains("incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );
    assert_eq!(paths(&control), paths(&emptied));
}

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

const TSC_ARGS: [&str; 8] = [
    "--strict",
    "--exactOptionalPropertyTypes",
    "--noUncheckedIndexedAccess",
    "--target",
    "ES2022",
    "--module",
    "NodeNext",
    "--moduleResolution",
];

#[test]
fn generated_incoming_module_compiles_strictly_with_the_package() {
    if !tool_available("tsc") {
        eprintln!("tsc is not on PATH; skipping the strict incoming compile check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(document()), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args(TSC_ARGS)
        .args([
            "NodeNext",
            "--skipLibCheck",
            "operations.ts",
            "incoming.ts",
            "--noEmit",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
}

#[test]
fn fake_webhook_posts_drive_the_decoder_and_constructor_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the behavioral incoming check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(document()), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args(TSC_ARGS)
        .args([
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
            "incoming.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("driver.mjs")
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

const DRIVER: &str = r#"
import assert from 'node:assert/strict';
import {
  decodeNewIssueWebhook,
  constructNewIssueResponse,
  NewIssueWebhookRoute,
  decodeSubscribeOnEventWebhook,
  SubscribeOnEventWebhookRoute,
  constructPingResponse,
  PingWebhookRoute,
  isIncomingRequestError,
  incomingDescriptors,
} from './dist/incoming.js';
import * as Codecs from './dist/model-codecs.js';

// Route constants are the registration hints; the declared method is what the
// provider sends.
assert.deepEqual(NewIssueWebhookRoute, { method: 'POST', route: 'newIssue', expression: false });
assert.deepEqual(SubscribeOnEventWebhookRoute, { method: 'POST', route: '{$request.body#/callbackUrl}', expression: true });
assert.deepEqual(PingWebhookRoute, { method: 'POST', route: 'ping', expression: false });

// A valid fake webhook POST decodes into the declared model, regardless of
// header-name casing.
const payload = decodeNewIssueWebhook({ 'X-Signature': 'abc' }, JSON.stringify({ id: 'i1', title: 't' }));
assert.equal(payload.id, 'i1');
assert.equal(payload.title, 't');

// A missing required header is the branded incoming-request failure.
assert.throws(
  () => decodeNewIssueWebhook({}, JSON.stringify({ id: 'i1', title: 't' })),
  (error) => isIncomingRequestError(error) && error.incomingKind === 'incoming-request',
);

// An invalid payload is a branded failure too.
assert.throws(
  () => decodeNewIssueWebhook({ 'x-signature': 'abc' }, JSON.stringify({ id: 'i1' })),
  (error) => isIncomingRequestError(error),
);

// A required declared body refuses an empty delivery.
assert.throws(() => decodeNewIssueWebhook({ 'x-signature': 'abc' }, ''), isIncomingRequestError);

// The declared 204 reply constructs with the pinned status and no body.
assert.deepEqual(constructNewIssueResponse(), { status: 204, headers: {}, body: '' });

// The declared 200 reply encodes its body through the response codec.
const pong = Codecs.OnPingResponse200Codec.decode('{"pong":true}');
const reply = constructPingResponse(pong);
assert.equal(reply.status, 200);
assert.equal(reply.body, Codecs.OnPingResponse200Codec.encode(pong));
assert.deepEqual(reply.headers, {});

// The callback receipt decodes through its own declared schema and carries its
// route expression verbatim.
const event = decodeSubscribeOnEventWebhook({}, JSON.stringify({ kind: 'open' }));
assert.equal(event.kind, 'open');
assert.equal(incomingDescriptors.subscribe_onEvent.route, '{$request.body#/callbackUrl}');
assert.equal(incomingDescriptors.subscribe_onEvent.expression, true);
assert.equal(incomingDescriptors.newIssue.payload, 'json');
assert.equal(incomingDescriptors.newIssue.replyStatus, 204);

console.log('incoming behavior verified');
"#;
