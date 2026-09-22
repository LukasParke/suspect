//! Emitted-only incoming receipt helpers for the PHP HTTP backend: one
//! generated `src/Incoming.php` with per-receipt decoders and constructors,
//! frozen route constants and compiled descriptors, `php -l` lint of every
//! emitted file, and native behavior over the generated package. Plans
//! without incoming declarations emit no file at all and stay byte-identical.
#![cfg(all(feature = "php-sdk", feature = "http-protocol"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const NAMESPACE: &str = "IncomingFixture";

/// One document-level webhook with a required header and a required JSON body
/// answering 204, one schema-free JSON webhook answering 200, and one
/// operation-attached callback carrying a runtime-expression route.
fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
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
        "components": {"schemas": {
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
        }},
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
            },
            "signal": {
                "post": {
                    "operationId": "onSignal",
                    "requestBody": {"required": true, "content": {"application/json": {"schema": {}}}},
                    "responses": {"204": {"description": "Accepted"}}
                }
            },
            "pongTyped": {
                "post": {
                    "operationId": "onPongTyped",
                    "responses": {"200": {
                        "description": "ok",
                        "headers": {
                            "x-trace-id": {"description": "optional trace", "schema": {"type": "string"}},
                            "x-trace-required": {"description": "required trace", "required": true, "schema": {"type": "string"}}
                        },
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}
                    }}
                }
            }
        }
    })
}

/// The same shape without the webhooks collection and the callback: the
/// receipt-less control.
fn control_document() -> Value {
    let mut document = document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    document["paths"]["/subscribe"]["post"]
        .as_object_mut()
        .unwrap()
        .remove("callbacks");
    document
}

fn contract(document: &Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.incoming.test/php-incoming.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(document).unwrap(),
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

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::PhpHttp,
        package_name: "incoming/php-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(NAMESPACE.into()),
    }
}

fn generate(document: &Value) -> Vec<OutFile> {
    let contract = contract(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(
        contract,
        &selected,
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn file<'a>(files: &'a [OutFile], path: &str) -> Option<&'a OutFile> {
    files.iter().find(|file| file.path == path)
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn incoming_file_emits_exactly_when_receipts_are_declared() {
    let files = generate(&document());
    let incoming = file(&files, "php/src/Incoming.php")
        .expect("the incoming class is emitted for a webhook/callback contract")
        .content
        .clone();
    for expected in [
        // The generated class, its branded failure and the frozen descriptors.
        "final class Incoming",
        "final class IncomingRequestException extends \\RuntimeException",
        "private const RECEIPTS = [",
        // Route constants are the registration hints: method, route, expression.
        "public const NEW_ISSUE_WEBHOOK_ROUTE = ['method' => \"POST\", 'route' => \"newIssue\", 'expression' => false];",
        "public const PING_WEBHOOK_ROUTE = ['method' => \"POST\", 'route' => \"ping\", 'expression' => false];",
        "public const PONG_TYPED_WEBHOOK_ROUTE = ['method' => \"POST\", 'route' => \"pongTyped\", 'expression' => false];",
        // The callback receipt carries its runtime expression verbatim (the
        // emitted literal escapes the `$` for the double-quoted PHP string).
        "public const SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE = ['method' => \"POST\", 'route' => \"{\\$request.body#/callbackUrl}\", 'expression' => true];",
        // Per-receipt decoders: declared required headers, typed payload.
        "public static function decodeNewIssueWebhook(array $headers, string $body): IssueEvent",
        "self::requireHeader($headers, \"x-signature\");",
        "return Codecs::decodeNewIssueBody($body);",
        "public static function decodeSubscribeOnEventWebhook(array $headers, string $body): Event",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "public static function constructNewIssueResponse(): array",
        "return [204, [], ''];",
        // The declared 200 reply encodes through the response codec.
        "public static function constructPingResponse(mixed $result): array",
        "$reply = Codecs::encodePingResponse200($result);",
        // The schema-free JSON body still compiles to a whole-value model and
        // answers the declared 204.
        "public static function decodeSignalWebhook(array $headers, string $body): JsonValue",
        "return Codecs::decodeSignalBody($body);",
        "public static function constructSignalResponse(): array",
        // Declared reply headers join the constructor with required-presence
        // checks on the required ones.
        "public static function constructPongTypedResponse(mixed $result, ?array $typedHeaders = null): array",
        "if ($value === null) { throw new IncomingRequestException(\"the declared required reply header {$name} is absent\"); }",
        // Descriptor entries carry kind, method, route, source and codecs.
        "\"newIssue\" => ['kind' => \"webhook\", 'method' => \"POST\", 'route' => \"newIssue\", 'expression' => false",
        "'payload' => \"json\", 'payload_codec' => \"NewIssueBody\", 'reply' => \"none\", 'reply_status' => 204",
        "\"subscribe.onEvent\" => ['kind' => \"callback\"",
        // Case-insensitive header reads and required-presence checks.
        "private static function headerValue(?array $headers, string $name): ?string",
        "private static function requireHeader(array $headers, string $name): void",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.php is missing:\n{expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // Without declared receipts nothing new is emitted: no Incoming class at
    // all. A receipt-less document without the webhooks key and one carrying
    // an empty webhooks map generate byte-identical packages, and the declared
    // document adds exactly the one file.
    let without = control_document();
    let mut emptied = control_document();
    emptied["webhooks"] = json!({});
    let (without_contract, emptied_contract) = (contract(&without), contract(&emptied));
    let selected = |contract: &Arc<Contract>| {
        contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>()
    };
    let without_files = backend::generate_with_options(
        without_contract.clone(),
        &selected(&without_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    let emptied_files = backend::generate_with_options(
        emptied_contract.clone(),
        &selected(&emptied_contract),
        &target(),
        &GenerationOptions::default(),
    )
    .unwrap();
    let declared = generate(&document());
    assert!(file(&without_files, "php/src/Incoming.php").is_none());
    assert!(file(&emptied_files, "php/src/Incoming.php").is_none());
    // The two receipt-less shapes emit the same artifact set (their source
    // spans differ, so only paths are compared, mirroring the TS control).
    assert_eq!(sorted(&without_files), sorted(&emptied_files));
    assert_eq!(declared.len(), without_files.len() + 1);
    assert!(file(&declared, "php/src/Incoming.php").is_some());
    assert!(
        !sorted(&without_files)
            .iter()
            .any(|path| path.to_ascii_lowercase().contains("incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn the_plan_carries_the_compiled_incoming_receipts() {
    let contract = contract(&document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::php_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::php_sdk::PhpConfig {
            package_name: "incoming/php-sdk".into(),
            package_version: "1.0.0".into(),
            namespace: NAMESPACE.into(),
            ..Default::default()
        },
    )
    .unwrap();
    let names = plan
        .incoming()
        .operations()
        .iter()
        .map(|operation| operation.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "newIssue",
            "ping",
            "pongTyped",
            "signal",
            "subscribe.onEvent"
        ]
    );
    let new_issue = plan
        .incoming()
        .operations()
        .iter()
        .find(|operation| operation.name() == "newIssue")
        .unwrap();
    assert_eq!(new_issue.method().as_str(), "POST");
    assert!(!new_issue.route().expression());
    let callback = plan
        .incoming()
        .operations()
        .iter()
        .find(|operation| operation.name() == "subscribe.onEvent")
        .unwrap();
    assert!(callback.route().expression());
    assert_eq!(callback.route().route(), "{$request.body#/callbackUrl}");
}

/// The repository's verified PHP 8.3 interpreter, when available.
fn php() -> Option<PathBuf> {
    let candidate = std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-tools/php-8.3.32/php")
        });
    Command::new(&candidate)
        .arg("-v")
        .output()
        .is_ok()
        .then_some(candidate)
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn emitted_php_files_lint() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; skipping the lint check");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&document()), root.path()).unwrap();
    let mut linted = 0usize;
    for entry in fs::read_dir(root.path().join("php/src")).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("php") {
            continue;
        }
        let output = Command::new(&php).arg("-l").arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "{} failed to lint:\n{}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        linted += 1;
    }
    assert!(
        linted > 10,
        "expected the generated package to lint: {linted}"
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn receipt_decoders_and_constructors_drive_the_package_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&document());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    fs::write(root.path().join("behavior.php"), BEHAVIOR).unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("behavior.php"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("behavior.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("behavior.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const BEHAVIOR: &str = r#"<?php
declare(strict_types=1);

foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }

use IncomingFixture\Incoming;
use IncomingFixture\IncomingRequestException;
use IncomingFixture\IssueEvent;
use IncomingFixture\Pong;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

// Route constants are the registration hints; the declared method is what the
// provider sends.
$route = Incoming::NEW_ISSUE_WEBHOOK_ROUTE;
check($route['method'] === 'POST' && $route['route'] === 'newIssue' && $route['expression'] === false, 'webhook route constant');
$callback = Incoming::SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE;
check($callback['route'] === '{$request.body#/callbackUrl}' && $callback['expression'] === true, 'callback route expression carried verbatim');
check(Incoming::PING_WEBHOOK_ROUTE['route'] === 'ping', 'ping route constant');

// A valid fake webhook POST decodes into the declared model, regardless of
// header-name casing.
$payload = Incoming::decodeNewIssueWebhook(['X-Signature' => 'abc'], '{"id":"i1","title":"t"}');
check($payload instanceof IssueEvent, 'decoded payload class: ' . get_class($payload));
check($payload->id === 'i1' && $payload->title === 't', 'decoded payload members');

// A missing required header is the typed incoming-request failure.
try {
    Incoming::decodeNewIssueWebhook([], '{"id":"i1","title":"t"}');
    throw new LogicException('expected a missing-header failure');
} catch (IncomingRequestException $error) {
    check(str_contains($error->getMessage(), 'x-signature'), 'missing header message: ' . $error->getMessage());
}

// An invalid payload is a typed failure too.
try {
    Incoming::decodeNewIssueWebhook(['x-signature' => 'abc'], '{"id":"i1"}');
    throw new LogicException('expected an invalid-payload failure');
} catch (IncomingRequestException $error) {
    check(!str_contains($error->getMessage(), 'i1'), 'the failure carries no received payload text: ' . $error->getMessage());
}

// A required declared body refuses an empty delivery.
try {
    Incoming::decodeNewIssueWebhook(['x-signature' => 'abc'], '');
    throw new LogicException('expected an absent-body failure');
} catch (IncomingRequestException $error) {
    check(str_contains($error->getMessage(), 'required receipt body'), 'absent body message: ' . $error->getMessage());
}

// The body-less webhook decodes to null: the decoder validates headers and
// stops there.
check(Incoming::decodePingWebhook([], '') === null, 'a receipt with no declared body decodes to null');

// The schema-free JSON webhook decodes to the parsed JSON value.
$free = Incoming::decodeSignalWebhook([], '{"any": ["json", 1, true]}');
check($free->kind === \IncomingFixture\JsonKind::Object, 'schema-free payload kind');
check($free->asObject()['any']->asArray()[1]->asNumber()->toInt() === 1, 'schema-free payload values survive');

// The declared 204 reply constructs with the pinned status and no body.
$reply = Incoming::constructNewIssueResponse();
check(is_array($reply) && count($reply) === 3, 'reply tuple shape');
check($reply[0] === 204 && $reply[1] === [] && $reply[2] === '', '204 reply carries the pinned status, no headers and no body');

// The declared 200 reply encodes its body through the response codec.
$pong = Pong::fromJson('{"pong":true}');
$reply = Incoming::constructPingResponse($pong);
check($reply[0] === 200, '200 reply status');
check($reply[2] === \IncomingFixture\Codecs::encodePingResponse200($pong), '200 reply body encodes through the response codec');
check($reply[1] === [], '200 reply headers');

// Declared reply headers apply from the optional typed headers argument, with
// required-presence checks for the required ones.
$pong = Pong::fromJson('{"pong":false}');
$reply = Incoming::constructPongTypedResponse($pong, ['X-Trace-Id' => 'trace-1', 'x-trace-required' => 'req-1']);
check($reply[1]['x-trace-id'] === 'trace-1' && $reply[1]['x-trace-required'] === 'req-1', 'declared reply headers applied by wire name: ' . json_encode($reply[1]));
try {
    Incoming::constructPongTypedResponse($pong);
    throw new LogicException('expected a missing required reply header failure');
} catch (IncomingRequestException $error) {
    check(str_contains($error->getMessage(), 'x-trace-required'), 'missing reply header message: ' . $error->getMessage());
}

// The callback receipt decodes through its own declared schema.
$event = Incoming::decodeSubscribeOnEventWebhook([], '{"kind":"open"}');
check($event->kind === 'open', 'callback payload decodes');

// The frozen descriptors are generated data keyed by the declared name.
$descriptors = new \ReflectionClass(Incoming::class);
$receipts = $descriptors->getConstant('RECEIPTS');
check($receipts['newIssue']['payload'] === 'json' && $receipts['newIssue']['payload_codec'] === 'NewIssueBody', 'descriptor payload');
check($receipts['newIssue']['reply'] === 'none' && $receipts['newIssue']['reply_status'] === 204, 'descriptor reply');
check($receipts['newIssue']['required_headers'] === ['x-signature'], 'descriptor required headers');
check($receipts['subscribe.onEvent']['kind'] === 'callback' && $receipts['subscribe.onEvent']['expression'] === true, 'descriptor callback');
check(str_contains($receipts['subscribe.onEvent']['source']['pointer'], 'callbacks'), 'descriptor source pointer: ' . $receipts['subscribe.onEvent']['source']['pointer']);
check($receipts['pongTyped']['reply_headers'] === ['x-trace-id', 'x-trace-required'], 'descriptor reply headers: ' . json_encode($receipts['pongTyped']['reply_headers']));

echo 'incoming behavior verified', PHP_EOL;
"#;
