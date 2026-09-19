//! Generated incoming receipt helpers for the Ruby gem: emission shape, plan
//! carriage, and native behavioral verification of the emitted decoders and
//! constructors over fake webhook deliveries. Static runtime files are never
//! modified; contracts without a declared webhook or callback emit no new
//! bytes at all, and the direct operation calls stay unchanged.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    ruby_sdk,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// Two declared webhooks (one with a required header and a body-less 204
/// reply, one schema-free body with declared reply headers), a declared-body
/// webhook, and one runtime-expression callback on an outgoing operation.
fn incoming_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
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
                    "responses": {"200": {
                        "description": "ok",
                        "headers": {
                            "x-trace-id": {"description": "echoed trace", "schema": {"type": "string"}},
                            "x-request-id": {"required": true, "description": "correlation", "schema": {"type": "string"}}
                        },
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}
                    }}
                }
            },
            "note": {
                "post": {
                    "operationId": "onNote",
                    "requestBody": {"content": {"application/json": {}}},
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    })
}

/// The same shape with the webhooks removed and the callback detached: the
/// receipt-less control.
fn control_document() -> Value {
    let mut document = incoming_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    document["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}}}
    });
    document
}

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.incoming.test/ruby-incoming.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
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

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::RubyHttp,
            package_name: "incoming-sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn content<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate(incoming_document());
    let incoming = content(&files, "ruby/lib/incoming_sdk/incoming.rb");
    for expected in [
        // The module is registered under the generated root, beside OAuth.
        "module Incoming",
        // Frozen route constants: the declared method is what the provider
        // sends, the path is carried verbatim.
        "NEW_ISSUE_WEBHOOK_ROUTE = { method: \"POST\", path: \"newIssue\", expression: false }.freeze",
        "SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE = { method: \"POST\", path: \"{$request.body\\#/callbackUrl}\", expression: true }.freeze",
        "PING_WEBHOOK_ROUTE = { method: \"POST\", path: \"ping\", expression: false }.freeze",
        // The decoder presence-checks the declared required header and decodes
        // through the package's own codec machinery.
        "def decode_new_issue_webhook(headers, body)",
        "unless header_value(headers, \"x-signature\")",
        "kind: :'incoming-request'",
        "Codecs::SchemaWebhooksNewIssuePostRequestBodyContentApplicationJson.decode_json(body)",
        // The schema-free body decodes as parsed JSON.
        "def decode_note_webhook(headers, body)",
        "Json.parse(body)",
        // The declared 204 reply: pinned status, no body, no result argument.
        "def construct_new_issue_response",
        "[204, {}.freeze, '']",
        // The declared 200 reply encodes through the response codec and
        // applies the declared reply headers with required-presence checks.
        "def construct_ping_response(result, typed_headers: nil)",
        "Codecs::SchemaWebhooksPingPostResponsesSchema200ContentApplicationJson.encode_json(result)",
        "headers[\"x-trace-id\"] = value if value",
        "unless value",
        "the declared required reply header \"x-request-id\" is absent",
        "def construct_note_response",
        // The callback receipt decodes through its own declared schema.
        "def decode_subscribe_on_event_webhook(headers, body)",
        "Codecs::SchemaPathsSubscribePostCallbacksOnEventRequestBodyCallbackUrlPostRequestBodyContentApplicationJson.decode_json(body)",
        // The case-insensitive declared header reader is the shared helper.
        "def header_value(headers, name)",
        "key.casecmp(name).zero?",
    ] {
        assert!(
            incoming.contains(expected),
            "incoming.rb is missing:\n{expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The RBS signatures cover the emitted module.
    let signatures = content(&files, "ruby/sig/incoming_sdk.rbs");
    for expected in [
        "module Incoming",
        "NEW_ISSUE_WEBHOOK_ROUTE: {method: String, path: String, expression: bool}",
        "def self.decode_new_issue_webhook: (Hash[String, String] headers, String body) -> Types::schema_webhooks_new_issue_post_request_body_content_application_json",
        "def self.construct_new_issue_response: () -> [Integer, Hash[String, String], String]",
        "def self.construct_ping_response: (Types::schema_webhooks_ping_post_responses_schema200_content_application_json result, ?typed_headers: Hash[String, String]?) -> [Integer, Hash[String, String], String]",
        "def self.decode_subscribe_on_event_webhook: (Hash[String, String] headers, String body) -> Types::schema_paths_subscribe_post_callbacks_on_event_request_body_call_",
    ] {
        assert!(
            signatures.contains(expected),
            "signatures are missing:\n{expected}\n--- emitted: ---\n{signatures}"
        );
    }

    // The entry requires the module exactly when the file is emitted, after
    // the runtime it integrates with.
    let entry = content(&files, "ruby/lib/incoming_sdk.rb");
    assert!(
        entry.contains("require_relative \"incoming_sdk/http\""),
        "{entry}"
    );
    assert!(
        entry.contains("require_relative \"incoming_sdk/incoming\""),
        "{entry}"
    );
    assert!(
        entry
            .find("require_relative \"incoming_sdk/incoming\"")
            .unwrap()
            > entry
                .find("require_relative \"incoming_sdk/http\"")
                .unwrap(),
        "incoming.rb must be required after http.rb"
    );

    // The direct calls stay exported and unchanged.
    assert!(content(&files, "ruby/lib/incoming_sdk/client.rb").contains("def list_widgets("));
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn receipt_less_contracts_emit_no_new_bytes() {
    // The receipt-less control — with and without an empty webhooks map —
    // emits no incoming artifacts and stays byte-identical.
    let control = generate(control_document());
    let mut emptied = incoming_document();
    emptied["webhooks"] = json!({});
    emptied["paths"]["/subscribe"] = control_document()["paths"]["/subscribe"].clone();
    let emptied = generate(emptied);
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("incoming_sdk/incoming.rb"))
    );
    assert!(
        !content(&control, "ruby/lib/incoming_sdk.rb").contains("incoming_sdk/incoming"),
        "the entry must not require an unemitted module"
    );
    assert!(
        !content(&control, "ruby/sig/incoming_sdk.rbs")
            .contains("module Incoming\n    SOURCE: String")
    );
    // With or without an empty webhooks map, the receipt-less gem keeps the
    // exact same artifact set (the two documents differ only in spans, so the
    // artifact set — not the serialized spans — is the invariant).
    assert_eq!(control.len(), emptied.len());
    for (left, right) in control.iter().zip(emptied.iter()) {
        assert_eq!(left.path, right.path);
    }
    // The declared receipts change only the emitted package they belong to:
    // the receipt module is the one new file, and the direct calls stay.
    let receipts = generate(incoming_document());
    assert_eq!(receipts.len(), control.len() + 1);
    let added = receipts
        .iter()
        .find(|file| !control.iter().any(|candidate| candidate.path == file.path))
        .map(|file| file.path.clone())
        .expect("the receipt module is the only new file");
    assert_eq!(added, "ruby/lib/incoming_sdk/incoming.rb");

    // The plan carries the compiled receipts only when they are declared.
    let plan = {
        let contract = contract_with_document(incoming_document());
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        ruby_sdk::plan_sdk(contract, &selected, ruby_sdk::RubyConfig::default())
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
    };
    let emission = plan.incoming().expect("declared receipts are carried");
    assert_eq!(emission.receipts.len(), 4);
    let new_issue = emission
        .receipts
        .iter()
        .find(|receipt| receipt.name == "newIssue")
        .unwrap();
    assert_eq!(new_issue.decode, "decode_new_issue_webhook");
    assert_eq!(
        new_issue.construct.as_deref(),
        Some("construct_new_issue_response")
    );
    assert_eq!(new_issue.route_const, "NEW_ISSUE_WEBHOOK_ROUTE");
    assert_eq!(new_issue.method, "POST");
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    assert!(format!("{:?}", new_issue.payload).starts_with("Json"));
    let ping = emission
        .receipts
        .iter()
        .find(|receipt| receipt.name == "ping")
        .unwrap();
    let response = ping.response.as_ref().expect("the declared 200 reply");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers,
        vec!["x-request-id".to_owned(), "x-trace-id".to_owned()]
    );
    assert_eq!(response.required_headers, vec!["x-request-id".to_owned()]);
    let callback = emission
        .receipts
        .iter()
        .find(|receipt| receipt.name == "subscribe.onEvent")
        .unwrap();
    assert_eq!(callback.kind, "callback");
    assert!(callback.expression);
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    let control_plan = {
        let contract = contract_with_document(control_document());
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        ruby_sdk::plan_sdk(contract, &selected, ruby_sdk::RubyConfig::default())
            .unwrap_or_else(|errors| panic!("{errors:#?}"))
    };
    assert!(control_plan.incoming().is_none());
}

const BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'incoming_sdk'

# Route constants are the registration hints; the declared method is what the
# provider sends.
raise 'route changed' unless IncomingSdk::Incoming::NEW_ISSUE_WEBHOOK_ROUTE ==
  { method: 'POST', path: 'newIssue', expression: false }
raise 'expression lost' unless IncomingSdk::Incoming::SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE ==
  { method: 'POST', path: '{$request.body#/callbackUrl}', expression: true }

# A valid fake webhook POST decodes into the declared model, regardless of
# header-name casing.
payload = IncomingSdk::Incoming.decode_new_issue_webhook({ 'X-Signature' => 'abc' }, '{"id":"i1","title":"t"}')
raise 'decode changed' unless payload.id == 'i1' && payload.title == 't'

# A missing required header is the typed incoming-request failure.
begin
  IncomingSdk::Incoming.decode_new_issue_webhook({}, '{"id":"i1","title":"t"}')
  raise 'a missing header must fail'
rescue IncomingSdk::SdkError => error
  raise 'wrong failure kind' unless error.kind == :'incoming-request'
end

# An invalid payload fails through the package's own codec machinery.
begin
  IncomingSdk::Incoming.decode_new_issue_webhook({ 'x-signature' => 'abc' }, '{"id":"i1"}')
  raise 'an invalid payload must fail'
rescue IncomingSdk::SdkError
  nil
end

# A required declared body refuses an empty delivery.
begin
  IncomingSdk::Incoming.decode_new_issue_webhook({ 'x-signature' => 'abc' }, '')
  raise 'an empty delivery must fail'
rescue IncomingSdk::SdkError => error
  raise 'wrong failure kind' unless error.kind == :'incoming-request'
end

# The declared 204 reply constructs with the pinned status and no body.
raise 'reply changed' unless IncomingSdk::Incoming.construct_new_issue_response == [204, {}, '']
# A body-less receipt decodes to nil: the decoder validates its declared
# headers and nothing else.
raise 'body-less decode changed' unless IncomingSdk::Incoming.decode_ping_webhook({}, '') .nil?

# The declared 200 reply encodes its body through the response codec and
# applies the declared reply headers with required-presence checks only.
pong = IncomingSdk::Codecs::Pong.decode_json('{"pong":true}')
raise 'schema-free probe changed' unless pong.pong == true
begin
  IncomingSdk::Incoming.construct_ping_response(pong)
  raise 'a required reply header must be checked'
rescue IncomingSdk::SdkError => error
  raise 'wrong reply failure' unless error.kind == :'incoming-request'
end
reply = IncomingSdk::Incoming.construct_ping_response(pong, typed_headers: { 'X-Trace-Id' => 't9', 'x-request-id' => 'r1' })
raise 'reply status changed' unless reply[0] == 200
raise 'reply headers changed' unless reply[1] == { 'x-trace-id' => 't9', 'x-request-id' => 'r1' }
raise 'reply body changed' unless reply[2] == '{"pong":true}'

# The schema-free body decodes as parsed JSON and the body-less reply
# constructs without a result argument.
note = IncomingSdk::Incoming.decode_note_webhook({}, '{"any":{"shape":true}}')
raise 'schema-free decode changed' unless note == { 'any' => { 'shape' => true } }
raise 'note reply changed' unless IncomingSdk::Incoming.construct_note_response == [200, {}.freeze, '']

# The callback receipt decodes through its own declared schema and carries its
# route expression verbatim.
event = IncomingSdk::Incoming.decode_subscribe_on_event_webhook({}, '{"kind":"open"}')
raise 'callback decode changed' unless event.kind == 'open'

puts 'incoming behavior verified'
"#;

fn ruby_home() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}

/// The gem requires Ruby >= 3.3; older interpreters cannot even parse the
/// emitted runtime syntax, so discovery refuses them.
fn ruby() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_RUBY_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let ruby = ruby_home().join("bin/ruby");
    if !ruby.is_file() {
        return None;
    }
    let output = Command::new(&ruby).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let minor = text
        .strip_prefix("ruby ")
        .and_then(|rest| rest.split('.').nth(1))
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 3).then_some(ruby)
}

fn checked(command: &mut Command, root: &std::path::Path, label: &str) {
    let output = command.output().unwrap();
    std::fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    std::fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_receipts_decode_deliveries_and_construct_replies() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_incoming: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(incoming_document());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "syntax",
            );
        }
    }

    std::fs::write(root.path().join("behavior.rb"), BEHAVIOR).unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
    eprintln!("ruby_incoming: native Ruby behavioral gate passed");
}
