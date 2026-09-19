//! Emitted-only incoming receipt helpers for the Rust HTTP backend:
//! `rust/src/incoming.rs`, model-only compilation of the emitted package, and
//! native decode/construct behavior against a compiled consumer. Plans without
//! incoming declarations emit no module and no bytes at all, and the retained
//! v1/v2 planning APIs never gain receipts.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
    rust_http::{self, IncomingPayload},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.incoming.test/rust-incoming.json";

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

/// Two contracts over one document URI, so their emitted bytes are comparable.
fn contracts_pair(first: Value, second: Value) -> (Arc<Contract>, Arc<Contract>) {
    let entry = Uri::parse(ENTRY).unwrap();
    let build = |document: &Value| {
        let provider = Arc::new(
            DocumentProvider::new([ProvidedDocument::new(
                entry.clone(),
                entry.clone(),
                serde_json::to_vec_pretty(&document).unwrap(),
            )
            .unwrap()])
            .unwrap(),
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .allowed_documents(provider.logical_uris())
                .document_provider(provider)
                .build()
                .unwrap(),
        );
        Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
    };
    (build(&first), build(&second))
}

/// One webhook with a required header, a required JSON body and a declared
/// body-less 204 reply; one webhook with a JSON 200 reply carrying a required
/// declared reply header; one schema-free JSON webhook; and one callback
/// receipt with a runtime-expression route.
fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {"schemas": {
            "IssueEvent": {"type": "object", "properties": {"id": {"type": "string"}, "title": {"type": "string"}}, "required": ["id", "title"]},
            "Event": {"type": "object", "properties": {"kind": {"type": "string"}}, "required": ["kind"]},
            "Pong": {"type": "object", "properties": {"pong": {"type": "boolean"}}, "required": ["pong"]}
        }},
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok"}},
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
            "newIssue": {"post": {
                "operationId": "onNewIssue",
                "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                "responses": {"204": {"description": "Accepted"}}
            }},
            "ping": {"post": {
                "operationId": "onPing",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}, "headers": {"x-trace": {"description": "trace", "required": true, "schema": {"type": "string"}}}}}
            }},
            "echo": {"post": {
                "operationId": "onEcho",
                "requestBody": {"required": true, "content": {"application/json": {}}},
                "responses": {"200": {"description": "ok", "content": {"application/json": {}}}}
            }}
        }
    })
}

/// The control document: the same paths without any webhooks, callbacks or
/// receipts, so nothing new may be emitted for it.
fn control_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {"schemas": {}},
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok"}}
            }}
        }
    })
}

fn selection(contract: &Contract) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::RustHttp,
        package_name: "incoming-sdk-rust".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = selection(&contract);
    backend::generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn by_path(files: &[OutFile]) -> std::collections::BTreeMap<&str, &str> {
    files
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect()
}

#[test]
fn the_v3_path_compiles_receipts_and_the_v1_v2_paths_stay_empty() {
    let contract = contract_with_document(document());
    let selected = selection(&contract);
    let plan = rust_http::plan_http_v3(
        contract.clone(),
        &selected,
        rust_http::HttpConfig::default(),
    )
    .unwrap();
    let incoming = plan
        .incoming()
        .expect("the v3 path compiles the incoming plan");
    assert_eq!(incoming.operations().len(), 4);
    let receipts = plan.incoming_receipts();
    assert_eq!(receipts.len(), 4);
    let new_issue = receipts
        .iter()
        .find(|receipt| receipt.name == "newIssue")
        .expect("the declared webhook compiles");
    assert_eq!(new_issue.kind, "webhook");
    assert_eq!(new_issue.decode, "decode_new_issue_webhook");
    assert_eq!(
        new_issue.construct.as_deref(),
        Some("construct_new_issue_response")
    );
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    let IncomingPayload::Json { model, .. } = &new_issue.payload else {
        panic!("the declared JSON body compiles to a model codec");
    };
    assert_eq!(model, "OnNewIssueRequest");
    let ping = receipts
        .iter()
        .find(|receipt| receipt.name == "ping")
        .expect("the ping webhook compiles");
    assert!(matches!(ping.payload, IncomingPayload::None));
    let response = ping.response.as_ref().expect("an exact 2xx reply exists");
    assert_eq!(response.status, 200);
    assert_eq!(response.headers, vec!["x-trace".to_owned()]);
    assert_eq!(response.required_headers, vec!["x-trace".to_owned()]);
    // The callback compiles with its runtime-expression route verbatim.
    let callback = receipts
        .iter()
        .find(|receipt| receipt.name == "subscribe.onEvent")
        .expect("the declared callback compiles");
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.decode, "decode_subscribe_on_event_webhook");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);
    // The retained v1/v2 planning APIs never compile incoming receipts, and
    // their emitted packages gain no artifact at all.
    for planner in [rust_http::plan_http, rust_http::plan_http_v2] {
        let plan = planner(
            contract.clone(),
            &selected,
            rust_http::HttpConfig::default(),
        )
        .unwrap();
        assert!(plan.incoming().is_none());
        assert!(plan.incoming_receipts().is_empty());
        let files = rust_http::emit_http(&plan, &rust_http::PackageConfig::default()).unwrap();
        assert!(!files.iter().any(|file| file.path == "rust/src/incoming.rs"));
        assert!(
            !files
                .iter()
                .find(|file| file.path == "rust/src/lib.rs")
                .unwrap()
                .content
                .contains("pub mod incoming")
        );
    }
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let generated = generate_document(document());
    let files = by_path(&generated);
    let module = files
        .get("rust/src/incoming.rs")
        .expect("the incoming module is emitted for a webhook/callback contract");
    for expected in [
        "pub struct IncomingRoute",
        "pub struct IncomingDescriptor",
        "pub static RECEIPTS: &[IncomingDescriptor] = &[",
        // The newIssue webhook: route constants carry the declared method/path
        // verbatim, and the descriptor freezes the compiled receipt data.
        "pub static NEW_ISSUE_ROUTE: IncomingRoute = IncomingRoute {\n    method: \"POST\",\n    route: \"newIssue\",\n    expression: false,\n};",
        "pub static NEW_ISSUE_DESCRIPTOR: IncomingDescriptor = IncomingDescriptor {\n    kind: \"webhook\",\n    method: \"POST\",\n    route: \"newIssue\",\n    expression: false,",
        "required_headers: &[\"x-signature\"],",
        "payload: \"json\",\n    payload_codec: std::option::Option::Some(\"OnNewIssueRequest\"),",
        "reply: \"none\",\n    reply_status: std::option::Option::Some(204),",
        // The decoder: presence-checked headers and the operation's own codec.
        "pub fn decode_new_issue_webhook(\n    headers: &[(String, String)],\n    body: &[u8],\n) -> std::result::Result<crate::models::OnNewIssueRequest, crate::http::SdkError> {",
        "if header_value(headers, \"x-signature\").is_none() {",
        "\"required receipt header x-signature is absent\",",
        "crate::codecs::OnNewIssueRequestCodec::decode_bytes(body)",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "pub fn construct_new_issue_response() -> std::result::Result<(u16, std::vec::Vec<(String, String)>, std::vec::Vec<u8>), crate::http::SdkError> {\n    std::result::Result::Ok((204, std::vec::Vec::new(), std::vec::Vec::new()))\n}",
        // The 200 reply encodes through the response codec and presence-checks
        // its required declared reply header.
        "pub fn construct_ping_response(\n    result: &crate::models::OnPingResponse200,\n    typed_headers: &[(String, String)]\n) -> std::result::Result<(u16, std::vec::Vec<(String, String)>, std::vec::Vec<u8>), crate::http::SdkError> {",
        "crate::codecs::OnPingResponse200Codec::encode(result)",
        "format!(\"the declared required reply header {name} is absent\"),",
        "reply_headers: &[\"x-trace\"],",
        // The callback receipt carries its runtime expression verbatim.
        "route: \"{$request.body#/callbackUrl}\",\n    expression: true,",
        "pub fn decode_subscribe_on_event_webhook(",
        "crate::codecs::EventReceivedRequestCodec::decode_bytes(body)",
        // Schema-free JSON goes through the exact JSON runtime.
        "pub fn decode_echo_webhook(\n    headers: &[(String, String)],\n    body: &[u8],\n) -> std::result::Result<crate::JsonValue, crate::http::SdkError> {",
        "crate::parse_json_bytes(body, JSON_LIMITS)",
        "pub fn construct_echo_response(\n    result: &crate::JsonValue\n) -> std::result::Result<(u16, std::vec::Vec<(String, String)>, std::vec::Vec<u8>), crate::http::SdkError> {",
        "crate::stringify_json(result, JSON_LIMITS)",
    ] {
        assert!(
            module.contains(expected),
            "incoming.rs lacks {expected}\n{module}"
        );
    }
    // The emitted lib.rs gains exactly the module declaration.
    let lib = files.get("rust/src/lib.rs").unwrap();
    assert!(lib.contains("#[cfg(feature=\"http\")]\npub mod incoming;\n"));
    // Nothing else in the emitted package leaks incoming helpers.
    for (path, content) in &files {
        if *path == "rust/src/incoming.rs" || *path == "rust/src/lib.rs" {
            continue;
        }
        assert!(
            !content.contains("decode_new_issue_webhook"),
            "{path} leaked incoming helpers"
        );
        assert!(
            !content.contains("construct_ping_response"),
            "{path} leaked incoming helpers"
        );
    }

    // Without declared receipts nothing new is emitted: no incoming module at
    // all, with or without an empty webhooks map.
    let generated_control = generate_document(control_document());
    let control = by_path(&generated_control);
    assert!(!control.contains_key("rust/src/incoming.rs"));
    assert!(
        !control
            .get("rust/src/lib.rs")
            .unwrap()
            .contains("pub mod incoming")
    );
    assert!(
        !control.keys().any(|path| path.contains("incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );
    let mut emptied = control_document();
    emptied["webhooks"] = json!({});
    let (control_contract, emptied_contract) = contracts_pair(control_document(), emptied);
    let control = backend::generate_with_options(
        control_contract.clone(),
        &selection(&control_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    let emptied = backend::generate_with_options(
        emptied_contract.clone(),
        &selection(&emptied_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    // The empty webhooks map changes nothing but the manifest's source spans
    // (the declaration key itself shifts every later span), so the artifact
    // sets match exactly and every span-free artifact is byte-identical.
    assert_eq!(control.len(), emptied.len());
    for (first, second) in control.iter().zip(&emptied) {
        assert_eq!(first.path, second.path);
        if first.path == "rust/http-manifest.json" {
            continue;
        }
        assert_eq!(
            first.content, second.content,
            "a receipt-less contract must emit byte-identical artifacts"
        );
    }
    assert!(!control.iter().any(|file| file.path.contains("incoming")));
}

#[test]
fn a_broken_incoming_declaration_refuses_the_v3_plan() {
    let mut broken = document();
    broken["webhooks"]["newIssue"]["post"]["requestBody"] =
        json!({"required": true, "content": {}});
    let contract = contract_with_document(broken);
    let selected = selection(&contract);
    let error = rust_http::plan_http_v3(contract, &selected, rust_http::HttpConfig::default())
        .expect_err("a broken body declaration refuses");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "http-content-empty"),
        "expected the shared content refusal: {error:?}"
    );
}

#[test]
fn receipts_beyond_json_refuse_with_source_linked_diagnostics() {
    // A text/plain receipt body has no v1 Rust receipt decode: the plan is
    // refused with the shared source-linked diagnostic instead of skipping.
    let mut text = document();
    text["webhooks"]["newIssue"]["post"]["requestBody"]["content"] =
        json!({"text/plain": {"schema": {"type": "string"}}});
    let contract = contract_with_document(text);
    let selected = selection(&contract);
    let error = rust_http::plan_http_v3(contract, &selected, rust_http::HttpConfig::default())
        .expect_err("a text receipt is refused");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "sdk-incoming-receipt-unrepresentable"),
        "expected the receipt refusal: {error:?}"
    );
    // A binary reply has no v1 Rust constructor either.
    let mut binary = document();
    binary["webhooks"]["ping"]["post"]["responses"]["200"]["content"] =
        json!({"application/octet-stream": {}});
    let contract = contract_with_document(binary);
    let selected = selection(&contract);
    let error = rust_http::plan_http_v3(contract, &selected, rust_http::HttpConfig::default())
        .expect_err("a binary reply is refused");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "sdk-incoming-response-unrepresentable"),
        "expected the reply refusal: {error:?}"
    );
}

/// Per-user cargo target directory, so concurrent agents never contend on the
/// shared workspace lock while building emitted packages.
fn cargo_target() -> PathBuf {
    let user = std::env::var_os("USER")
        .or_else(|| std::env::var_os("LOGNAME"))
        .unwrap_or_else(|| format!("uid-{}", std::process::id()).into());
    std::env::temp_dir().join(format!(
        "suspect-rust-incoming-target-{}",
        user.to_string_lossy()
    ))
}

fn cargo(command: &str, manifest: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new("cargo")
        .arg(command)
        .arg("--offline")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(cargo_target())
        .args(args)
        .env_remove("RUST_MIN_STACK")
        .env(
            "RUSTUP_TOOLCHAIN",
            std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").unwrap_or_default(),
        )
        .output()
        .expect("cargo is available");
    let log = format!("{}", String::from_utf8_lossy(&output.stderr));
    (output.status.success(), log)
}

fn registry_unavailable(log: &str) -> bool {
    log.contains("no matching package named")
        || log.contains("failed to download")
        || log.contains("error: failed to select a version")
        || log.contains("network disabled")
        || log.contains("could not download")
}

#[test]
fn emitted_incoming_package_compiles_model_only() {
    let generated = generate_document(document());
    let directory = tempfile::Builder::new()
        .prefix("rust-incoming-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generated, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    // Model-only compilation needs no dependencies at all and must always
    // succeed offline; the incoming module is feature-gated out of it.
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");
    if std::env::var_os("SUSPECT_KEEP_INCOMING").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn decoded_and_constructed_receipts_behave_in_a_compiled_package() {
    let generated = generate_document(document());
    let directory = tempfile::Builder::new()
        .prefix("rust-incoming-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generated, &directory).unwrap();
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"incoming-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"incoming-sdk-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping behavioral incoming execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_INCOMING").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

const CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the emitted incoming receipt helpers.
use sdk::incoming::{
    construct_echo_response, construct_new_issue_response, construct_ping_response,
    construct_subscribe_on_event_response, decode_echo_webhook, decode_new_issue_webhook,
    decode_ping_webhook,
};

#[test]
fn a_valid_webhook_post_decodes_into_the_declared_model() {
    // Header names are matched case-insensitively.
    let payload = decode_new_issue_webhook(
        &[("X-Signature".to_owned(), "abc".to_owned())],
        br#"{"id":"i1","title":"t"}"#,
    )
    .unwrap();
    assert_eq!(payload.id, "i1");
    assert_eq!(payload.title, "t");
}

#[test]
fn a_missing_required_header_is_a_branded_failure() {
    let failure = decode_new_issue_webhook(&[], br#"{"id":"i1","title":"t"}"#).unwrap_err();
    assert_eq!(failure.kind, sdk::http::SdkErrorKind::ResponseDecoding);
    assert!(failure.to_string().contains("x-signature"));
    // The same decode succeeds once the header is present.
    decode_new_issue_webhook(
        &[("x-signature".to_owned(), "abc".to_owned())],
        br#"{"id":"i1","title":"t"}"#,
    )
    .unwrap();
}

#[test]
fn an_invalid_payload_is_a_branded_failure() {
    let failure = decode_new_issue_webhook(
        &[("x-signature".to_owned(), "abc".to_owned())],
        br#"{"id":"i1"}"#,
    )
    .unwrap_err();
    assert_eq!(failure.kind, sdk::http::SdkErrorKind::ResponseDecoding);
    // Malformed JSON fails the same way.
    decode_new_issue_webhook(
        &[("x-signature".to_owned(), "abc".to_owned())],
        b"{nope",
    )
    .unwrap_err();
}

#[test]
fn a_required_declared_body_refuses_an_empty_delivery() {
    let failure =
        decode_new_issue_webhook(&[("x-signature".to_owned(), "abc".to_owned())], b"")
            .unwrap_err();
    assert_eq!(failure.kind, sdk::http::SdkErrorKind::ResponseDecoding);
    assert!(failure.to_string().contains("required receipt body"));
}

#[test]
fn schema_free_json_decodes_and_refuses_invalid_json() {
    let value = decode_echo_webhook(&[], br#"{"any":true}"#).unwrap();
    match value {
        sdk::Nullable::Value(sdk::JsonNonNullValue::Object(fields)) => {
            assert!(fields.contains_key("any"));
        }
        other => panic!("unexpected decoded value: {other:?}"),
    }
    decode_echo_webhook(&[], b"{nope").unwrap_err();
}

#[test]
fn a_body_less_decoder_validates_headers_and_ignores_the_delivery() {
    decode_ping_webhook(&[], b"anything").unwrap();
}

#[test]
fn the_declared_204_reply_constructs_with_the_pinned_status_and_no_body() {
    let (status, headers, body) = construct_new_issue_response().unwrap();
    assert_eq!(status, 204);
    assert!(headers.is_empty());
    assert!(body.is_empty());
}

#[test]
fn the_declared_200_reply_encodes_its_body_and_applies_declared_headers() {
    let pong = sdk::codecs::OnPingResponse200Codec::decode("{\"pong\":true}").unwrap();
    let reply = construct_ping_response(&pong, &[("X-Trace".to_owned(), "t".to_owned())]).unwrap();
    assert_eq!(reply.0, 200);
    assert_eq!(
        reply.1,
        vec![("x-trace".to_owned(), "t".to_owned())],
        "the declared reply header is applied from the caller values"
    );
    assert_eq!(reply.2, sdk::codecs::OnPingResponse200Codec::encode(&pong).unwrap().into_bytes());
    // A missing required reply header is the branded receipt failure.
    let failure = construct_ping_response(&pong, &[]).unwrap_err();
    assert_eq!(failure.kind, sdk::http::SdkErrorKind::ResponseDecoding);
    assert!(failure.to_string().contains("x-trace"));
}

#[test]
fn the_schema_free_reply_writes_its_json_verbatim() {
    let (status, _headers, body) = construct_echo_response(
        &sdk::parse_json("{\"any\":1}", sdk::JsonLimits::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(std::str::from_utf8(&body).unwrap(), "{\"any\":1}");
}

#[test]
fn the_callback_receipt_constructs_its_declared_200_reply() {
    let (status, headers, body) = construct_subscribe_on_event_response().unwrap();
    assert_eq!(status, 200);
    assert!(headers.is_empty());
    assert!(body.is_empty());
}

#[test]
fn route_constants_and_descriptors_carry_the_declaration() {
    let route = sdk::incoming::NEW_ISSUE_ROUTE;
    assert_eq!((route.method, route.route, route.expression), ("POST", "newIssue", false));
    let callback = sdk::incoming::SUBSCRIBE_ON_EVENT_ROUTE;
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);
    let descriptor = sdk::incoming::NEW_ISSUE_DESCRIPTOR;
    assert_eq!(descriptor.kind, "webhook");
    assert_eq!(descriptor.source.pointer, "/webhooks/newIssue/post");
    assert_eq!(descriptor.required_headers, &["x-signature"]);
    assert_eq!(descriptor.payload, "json");
    assert_eq!(descriptor.payload_codec, Some("OnNewIssueRequest"));
    assert_eq!(descriptor.reply, "none");
    assert_eq!(descriptor.reply_status, Some(204));
    assert_eq!(sdk::incoming::RECEIPTS.len(), 4);
    let callback_descriptor = sdk::incoming::SUBSCRIBE_ON_EVENT_DESCRIPTOR;
    assert_eq!(callback_descriptor.kind, "callback");
    assert!(callback_descriptor.expression);
}
"##;
