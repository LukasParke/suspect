//! Emitted-only pagination for the PHP HTTP backend: per-operation page/item
//! generator methods plus a next-page arguments builder on the generated
//! `Client`, `php -l` lint of every emitted file, and native behavior over a
//! stubbed transport. Static runtime files are never modified; the walks live
//! entirely inside the generated `Client.php`, and no-policy generation stays
//! byte-identical.
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
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const NAMESPACE: &str = "PaginationFixture";

fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Pagination", "version": "1"},
        "servers": [{"url": "https://api.pagination.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer"}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "data": {"type": "array", "items": {"$ref": "#/components/schemas/Widget"}},
                        "total": {"type": "integer"},
                        "has_more": {"type": "boolean"}
                    }
                }}}}}
            }},
            "/events": {"get": {
                "operationId": "listEvents",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "items": {"type": "array", "items": {"type": "string"}},
                        "next_page_token": {"type": "string"}
                    }
                }}}}}
            }},
            "/keys": {"get": {
                "operationId": "scanKeys",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {"type": "object", "properties": {"next_page_token": {"type": "string"}}}}}}}
            }},
        },
        "components": {"schemas": {"Widget": {
            "type": "object",
            "properties": {"id": {"type": "string"}}
        }}}
    })
}

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/php-pagination.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document()).unwrap(),
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
        package_name: "pagination/php-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(NAMESPACE.into()),
    }
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "php/src/Client.php")
        .expect("generated client")
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn pagination_methods_emit_only_under_sdk_defaults() {
    let files = generate(&configured());
    let client = client_file(&files).content.clone();
    for expected in [
        // Page/item generators for both walks.
        "public function listWidgetsPages(ListWidgetsInput $input = new ListWidgetsInput(), ?RequestOptions $options = null): \\Generator",
        "public function listWidgetsItems(ListWidgetsInput $input = new ListWidgetsInput(), ?RequestOptions $options = null): \\Generator",
        "public function listEventsPages(ListEventsInput $input = new ListEventsInput(), ?RequestOptions $options = null): \\Generator",
        "public function listEventsItems(ListEventsInput $input = new ListEventsInput(), ?RequestOptions $options = null): \\Generator",
        // Next-page arguments builder.
        "public function listWidgetsNextPage(ListWidgetsInput $input = new ListWidgetsInput(), ?RequestOptions $options = null): ?array",
        "public function listEventsNextPage(ListEventsInput $input = new ListEventsInput(), ?RequestOptions $options = null): ?array",
        // Pointer readers over generated property chains.
        "private static function listWidgetsPageItems(",
        "private static function listWidgetsPageInput(",
        "private static function listWidgetsPageArguments(",
        "private static function listEventsPageItems(",
        "private static function listEventsPageCursor(",
        // Traversal semantics.
        "$page = $this->listWidgets($input, $options);",
        "$page = $this->listEvents($input, $options);",
        "JsonNumber::fromInt($offset)",
        "$input = self::listWidgetsPageInput($input, JsonNumber::fromInt($offset));",
        "if ($value === null || $value === '') { return; }",
        "if ($value === $previous) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }",
        "$offset += $count;",
        "yield from $items;",
        "return ['limit' => $input->limit, 'offset' => $offset, 'filter' => $input->filter];",
    ] {
        assert!(
            client.contains(expected),
            "Client.php is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }

    // Without configured defaults nothing new is emitted at all; the emitted
    // package keeps exactly the same file set.
    let control = generate(&GenerationOptions::default());
    let plain = client_file(&control).content.clone();
    assert!(!plain.contains("listWidgetsPages"));
    assert!(!plain.contains("listEventsPageCursor"));
    assert!(!plain.contains("pagination-stalled"));
    assert_eq!(
        sorted(&control),
        sorted(&files),
        "pagination emission must not add or remove package files"
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({"version": "v1", "pagination": "off"})).unwrap(),
        ),
        ..GenerationOptions::default()
    };
    let disabled = generate(&off);
    let control = generate(&GenerationOptions::default());
    assert_eq!(sorted(&disabled), sorted(&control));
    for file in &control {
        let emitted = disabled
            .iter()
            .find(|candidate| candidate.path == file.path)
            .expect("same file set");
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
}

/// A manual mapping may declare a has-more indicator; the emitted walk must
/// then stop on `false` even when the page still returned items.
#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn manual_has_more_mapping_emits_and_honors_the_false_stop_rule() {
    let defaults: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "pagination": {
            "operations": {
                "listWidgets": {
                    "pattern": "limit-offset",
                    "request": {"limit": "limit", "offset": "offset"},
                    "response": {"items": "/data", "has-more": "/has_more"},
                    "initial_offset": 0,
                    "advance": "items-returned"
                }
            }
        }
    }))
    .unwrap();
    let files = generate(&GenerationOptions {
        sdk_defaults: Some(defaults),
        ..GenerationOptions::default()
    });
    let client = client_file(&files).content.clone();
    assert_eq!(
        client.matches("=== false").count(),
        2,
        "page and next-page walks must stop on a false has-more indicator; the item walk inherits it:\n{client}"
    );
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let script = r#"<?php
declare(strict_types=1);
foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }
use PaginationFixture\Client;
use PaginationFixture\Credentials;
use PaginationFixture\ClientOptions;
use PaginationFixture\HttpResponse;
use PaginationFixture\HttpRequest;
use PaginationFixture\JsonNumber;
use PaginationFixture\ListWidgetsInput;
use PaginationFixture\Transport;
final class HasMoreStub implements Transport
{
    public array $requests = [];
    public function __construct(private array $pages) {}
    public function send(HttpRequest $request): HttpResponse
    {
        $index = count($this->requests);
        $this->requests[] = $request;
        return new HttpResponse(200, ['content-type' => 'application/json'], json_encode($this->pages[$index], JSON_THROW_ON_ERROR));
    }
}
$transport = new HasMoreStub([
    ['data' => [['id' => 'a']], 'has_more' => true],
    ['data' => [['id' => 'b']], 'has_more' => false],
]);
$client = new Client(new Credentials([]), $transport, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$pages = iterator_to_array($client->listWidgetsPages(new ListWidgetsInput(limit: JsonNumber::fromInt(5))));
assert(count($transport->requests) === 2, 'has-more false stops the walk: ' . count($transport->requests));
assert(count($pages) === 2, 'two pages: ' . count($pages));
$itemsTransport = new HasMoreStub([
    ['data' => [['id' => 'a']], 'has_more' => true],
    ['data' => [['id' => 'b']], 'has_more' => false],
    ['data' => [['id' => 'never']], 'has_more' => false],
]);
$itemsClient = new Client(new Credentials([]), $itemsTransport, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$items = iterator_to_array($itemsClient->listWidgetsItems(new ListWidgetsInput(limit: JsonNumber::fromInt(5))), false);
assert(count($itemsTransport->requests) === 2, 'item walk stops at the flag: ' . count($itemsTransport->requests));
assert(count($items) === 2, 'items stop at the flag: ' . count($items));
echo 'has-more behavior verified', PHP_EOL;
"#;
    fs::write(root.path().join("has_more.php"), script).unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("has_more.php"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("has_more.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("has_more.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A detected cursor walk without a recognized items pointer (scanKeys) still
/// emits the page walk and the next-page builder; only the flattening
/// generator is omitted.
#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn cursor_without_items_emits_the_page_walk_only() {
    let files = generate(&configured());
    let client = client_file(&files).content.clone();
    assert!(
        client.contains("public function scanKeysPages(ScanKeysInput $input = new ScanKeysInput(), ?RequestOptions $options = null): \\Generator"),
        "{client}"
    );
    assert!(
        client.contains("public function scanKeysNextPage(ScanKeysInput $input = new ScanKeysInput(), ?RequestOptions $options = null): ?array"),
        "{client}"
    );
    assert!(!client.contains("scanKeysItems"), "{client}");
    assert!(!client.contains("scanKeysPageItems"), "{client}");
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
    suspect_codegen::write_files(&generate(&configured()), root.path()).unwrap();
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
fn pagination_walks_drive_stubbed_pages_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&configured());
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

use PaginationFixture\Client;
use PaginationFixture\Credentials;
use PaginationFixture\ClientOptions;
use PaginationFixture\HttpResponse;
use PaginationFixture\HttpRequest;
use PaginationFixture\JsonNumber;
use PaginationFixture\ListWidgetsInput;
use PaginationFixture\ListEventsInput;
use PaginationFixture\SdkError;
use PaginationFixture\Transport;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

/** @param list<array<string,mixed>> $pages */
final class Stub implements Transport
{
    /** @var list<HttpRequest> */
    public array $requests = [];
    /** @param list<array<string,mixed>> $pages */
    public function __construct(private array $pages) {}
    public function send(HttpRequest $request): HttpResponse
    {
        $index = count($this->requests);
        $this->requests[] = $request;
        if ($index >= count($this->pages)) { throw new LogicException('unexpected extra request: ' . $request->url); }
        return new HttpResponse(200, ['content-type' => 'application/json'], json_encode($this->pages[$index], JSON_THROW_ON_ERROR));
    }
    /** @return list<string> */
    public function queries(): array
    {
        $queries = [];
        foreach ($this->requests as $request) { $queries[] = (string) parse_url($request->url, PHP_URL_QUERY); }
        return $queries;
    }
}

$widgets = new Stub([
    ['data' => [['id' => 'a'], ['id' => 'b']], 'total' => 9],
    ['data' => [['id' => 'c']], 'total' => 9],
    ['data' => [], 'total' => 9],
]);
$client = new Client(new Credentials([]), $widgets, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));

// Limit/offset walk: the first page is the direct call's result, later requests
// change only the offset, and the filter argument is preserved exactly.
$pages = iterator_to_array($client->listWidgetsPages(new ListWidgetsInput(limit: JsonNumber::fromInt(2), filter: 'red')));
check(count($pages) === 3, 'three pages walked: ' . count($pages));
check($pages[0]->body->data[1]->id === 'b', 'first page is the direct result');
check($pages[2]->body->data === [], 'zero-item page ends the walk');
$queries = $widgets->queries();
check(str_contains($queries[0], 'limit=2') && str_contains($queries[0], 'offset=0') && str_contains($queries[0], 'filter=red'), 'first page query: ' . $queries[0]);
check(str_contains($queries[1], 'offset=2') && str_contains($queries[1], 'filter=red'), 'second page query: ' . $queries[1]);
check(str_contains($queries[2], 'offset=3'), 'third page query: ' . $queries[2]);

// A caller-supplied offset wins for page 1 and is replaced afterwards.
$widgets = new Stub([
    ['data' => [['id' => 'a'], ['id' => 'b']], 'total' => 9],
    ['data' => [], 'total' => 9],
]);
$client = new Client(new Credentials([]), $widgets, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
iterator_to_array($client->listWidgetsPages(new ListWidgetsInput(limit: JsonNumber::fromInt(2), offset: JsonNumber::fromInt(10))));
check(str_contains($widgets->queries()[0], 'offset=10'), 'caller offset honored: ' . $widgets->queries()[0]);
check(str_contains($widgets->queries()[1], 'offset=12'), 'offset advances from the caller value: ' . $widgets->queries()[1]);

// Early break never starts the next request.
$widgets = new Stub([
    ['data' => [['id' => 'a']], 'total' => 9],
    ['data' => [['id' => 'b']], 'total' => 9],
]);
$client = new Client(new Credentials([]), $widgets, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$seen = [];
foreach ($client->listWidgetsPages(new ListWidgetsInput(limit: JsonNumber::fromInt(1))) as $page) {
    $seen[] = $page;
    break;
}
check(count($seen) === 1 && count($widgets->requests) === 1, 'early break issues no second request');
$generator = $client->listWidgetsPages(new ListWidgetsInput(limit: JsonNumber::fromInt(1)));
$generator->current();
unset($generator);
check(count($widgets->requests) === 2, 'abandoning a generator never fetches another page');

// Cursor walk: the cursor is absent on page 1, resolved from the page afterwards.
$events = new Stub([
    ['items' => ['i1', 'i2'], 'next_page_token' => 'c2'],
    ['items' => ['i3']],
]);
$client = new Client(new Credentials([]), $events, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$walked = iterator_to_array($client->listEventsPages());
check(count($walked) === 2, 'cursor walk pages: ' . count($walked));
check($walked[1]->body->items[0] === 'i3', 'cursor page items');
$queries = $events->queries();
check(!str_contains($queries[0], 'cursor='), 'first cursor page omits the cursor: ' . $queries[0]);
check(str_contains($queries[1], 'cursor=c2'), 'second cursor page: ' . $queries[1]);

// A caller-supplied cursor wins for page 1.
$events = new Stub([
    ['items' => ['x'], 'next_page_token' => 'c9'],
    ['items' => ['y']],
]);
$client = new Client(new Credentials([]), $events, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
iterator_to_array($client->listEventsPages(new ListEventsInput(cursor: 'c1')));
check(str_contains($events->queries()[0], 'cursor=c1'), 'caller cursor honored');
check(str_contains($events->queries()[1], 'cursor=c9'), 'caller cursor replaced: ' . $events->queries()[1]);

// An empty cursor token stops the walk.
$events = new Stub([['items' => ['x'], 'next_page_token' => ''], ['items' => ['loop']]]);
$client = new Client(new Credentials([]), $events, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
check(count(iterator_to_array($client->listEventsPages())) === 1, 'empty cursor stops');
check(count($events->requests) === 1, 'empty cursor issues one request');

// A repeated identical continuation value throws the typed pagination error.
$events = new Stub([
    ['items' => ['x'], 'next_page_token' => 'again'],
    ['items' => ['x'], 'next_page_token' => 'again'],
    ['items' => ['x'], 'next_page_token' => 'again'],
]);
$client = new Client(new Credentials([]), $events, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
try {
    iterator_to_array($client->listEventsPages());
    throw new LogicException('expected a pagination-stalled failure');
} catch (SdkError $error) {
    check($error->kind === 'pagination-stalled', 'stall kind: ' . $error->kind);
}
check(count($events->requests) === 2, 'the walk stops at the repeated continuation: ' . count($events->requests));

// NextPage fetches one page and returns the rebuilt constructor arguments.
$widgets = new Stub([
    ['data' => [['id' => 'a'], ['id' => 'b']], 'total' => 9],
    ['data' => [['id' => 'c']], 'total' => 9],
    ['data' => [], 'total' => 9],
]);
$client = new Client(new Credentials([]), $widgets, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$input = new ListWidgetsInput(limit: JsonNumber::fromInt(2), filter: 'red');
$followed = 0;
while (true) {
    $next = $client->listWidgetsNextPage($input);
    if ($next === null) { break; }
    $input = new ListWidgetsInput(...$next);
    $followed += 1;
}
check($followed === 2, 'manual walk follows: ' . $followed);
check($input->filter === 'red', 'manual walk preserves the filter');
check($input->offset !== null && $input->offset->toDecimalString() === '3', 'manual walk offset: ' . $input->offset?->toDecimalString());
check(count($widgets->requests) === 3, 'manual walk requests: ' . count($widgets->requests));

// A cursor walk without a mapped items pointer still walks pages.
$keys = new Stub([
    ['next_page_token' => 'k2'],
    ['next_page_token' => 'k9'],
    new stdClass(),
]);
$client = new Client(new Credentials([]), $keys, new ClientOptions(serverUrl: 'https://api.pagination.test/v1'));
$walked = iterator_to_array($client->scanKeysPages());
check(count($walked) === 3 && count($keys->requests) === 3, 'items-less cursor walk: ' . count($walked) . '/' . count($keys->requests));
check(!method_exists($client, 'scanKeysItems'), 'no item walk without a mapped items pointer');

echo 'pagination behavior verified', PHP_EOL;
"#;
