//! Emitted-only pagination for the TypeScript HTTP backend: descriptor map,
//! per-operation page/item/next-page exports, strict compilation, and native
//! node behavior against a stubbed fetch.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig, generate_with_options};
use suspect_codegen::sdk_defaults::SdkDefaults;
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

/// One limit/offset list operation and one cursor list operation.
fn pagination_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Pagination", "version": "1"},
        "servers": [{"url": "https://api.pagination.test/v1"}],
        "security": [{"apiKey": []}],
        "components": {"securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}}},
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
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}}
            },
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
                    },
                    "required": ["items"]
                }}}}}}
            }
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@pagination/fixture".into(),
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

fn generate(document: Value, options: &GenerationOptions) -> Vec<suspect_codegen::OutFile> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn operations_source(files: &[suspect_codegen::OutFile]) -> String {
    files
        .iter()
        .find(|file| file.path == "typescript/operations.ts")
        .unwrap()
        .content
        .clone()
}

fn pagination_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn pagination_exports_and_descriptors_emit_only_under_sdk_defaults() {
    let files = generate(pagination_document(), &pagination_options());
    let pagination = files
        .iter()
        .find(|file| file.path == "typescript/pagination.ts")
        .expect("pagination module emitted under sdk defaults");
    let operations = operations_source(&files);

    for expected in [
        "export function listWidgetsPages(",
        "export function listWidgetsItems(",
        "export function listWidgetsNextPage(",
        "export type ListWidgetsItem = ",
        "paginationDescriptors.listWidgets",
    ] {
        assert!(
            operations.contains(expected),
            "operations.ts lacks {expected}"
        );
    }
    for expected in [
        "paginationDescriptors = /* @__PURE__ */ Object.freeze(",
        "pattern: \"limit-offset\"",
        "pattern: \"cursor\"",
        "request: /* @__PURE__ */ Object.freeze({limit:\"limit\",offset:\"offset\",} as const)",
        "response: /* @__PURE__ */ Object.freeze({items:\"/data\",total:\"/total\",} as const)",
        "response: /* @__PURE__ */ Object.freeze({items:\"/items\",nextCursor:\"/next_page_token\",} as const)",
        "initialOffset: 0",
        "initialOffset: null",
        "advance: \"items-returned\"",
        "advance: \"next-offset\"",
    ] {
        assert!(
            pagination.content.contains(expected),
            "pagination.ts lacks {expected}"
        );
    }

    // Without SDK defaults nothing new is emitted at all.
    let plain = generate(pagination_document(), &GenerationOptions::default());
    assert!(
        !plain
            .iter()
            .any(|file| file.path == "typescript/pagination.ts")
    );
    let operations = operations_source(&plain);
    assert!(!operations.contains("paginationDescriptors"));
    assert!(!operations.contains("listWidgetsPages"));
    assert!(!operations.contains("listWidgetsItems"));
    assert!(!operations.contains("listWidgetsNextPage"));
}

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn generated_pagination_compiles_strictly_with_the_package() {
    if !tool_available("tsc") {
        eprintln!("tsc is not on PATH; skipping the strict pagination compile check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(pagination_document(), &pagination_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "operations.ts",
            "pagination.ts",
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

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn pagination_walkers_drive_stubbed_pages_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the behavioral pagination check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(pagination_document(), &pagination_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
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
  listWidgetsPages,
  listWidgetsItems,
  listWidgetsNextPage,
  listEventsItems,
} from './dist/operations.js';

let queue = [];
let requests = [];
globalThis.fetch = async (input, init) => {
  const url = String(input instanceof Request ? input.url : input);
  if (init?.signal?.aborted) throw init.signal.reason ?? new Error('aborted');
  requests.push(url);
  const page = queue.shift();
  assert.ok(page, 'unexpected extra request: ' + url);
  return new Response(JSON.stringify(page), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
};
const client = { auth: { apiKey: 'fixture-secret' } };
const query = (url) => Object.fromEntries(new URL(url).searchParams);

// Two-page limit/offset walk: exactly 2 requests, offset=0 then offset=2,
// with the filter parameter preserved on every page.
{
  queue = [
    { data: ['a', 'b'], total: 4 },
    { data: ['c', 'd'], total: 4 },
  ];
  requests = [];
  const pages = [];
  for await (const page of listWidgetsPages(client, { limit: 2, offset: 0, filter: 'active' })) {
    pages.push(page);
    if (pages.length === 2) break;
  }
  assert.equal(pages.length, 2);
  assert.deepEqual(pages[0].data.data, ['a', 'b']);
  assert.deepEqual(pages[1].data.data, ['c', 'd']);
  assert.equal(requests.length, 2, 'breaking after two pages never starts a third request');
  assert.deepEqual(query(requests[0]), { limit: '2', offset: '0', filter: 'active' });
  assert.deepEqual(query(requests[1]), { limit: '2', offset: '2', filter: 'active' });
}

// Full items walk: the empty third page stops the walk.
{
  queue = [
    { data: ['a', 'b'], total: 4 },
    { data: ['c', 'd'], total: 4 },
    { data: [], total: 4 },
  ];
  requests = [];
  const items = [];
  for await (const item of listWidgetsItems(client, { limit: 2, offset: 0, filter: 'active' })) {
    items.push(item);
  }
  assert.deepEqual(items, ['a', 'b', 'c', 'd']);
  assert.equal(requests.length, 3);
  assert.deepEqual(query(requests[2]), { limit: '2', offset: '4', filter: 'active' });
}

// Cursor walk: cursor=c2 on the second request; stops when next_page_token is
// missing from the page.
{
  queue = [
    { items: ['x', 'y'], next_page_token: 'c2' },
    { items: ['z'] },
  ];
  requests = [];
  const events = [];
  for await (const item of listEventsItems(client, {})) {
    events.push(item);
  }
  assert.deepEqual(events, ['x', 'y', 'z']);
  assert.equal(requests.length, 2);
  assert.equal(query(requests[0]).cursor, undefined);
  assert.equal(query(requests[1]).cursor, 'c2');
}

// Early break after the first item issues no second request.
{
  queue = [
    { items: ['x', 'y'], next_page_token: 'c2' },
    { items: ['z'] },
  ];
  requests = [];
  const early = [];
  for await (const item of listEventsItems(client, {})) {
    early.push(item);
    break;
  }
  assert.deepEqual(early, ['x']);
  assert.equal(requests.length, 1, 'early break never fires the next page request');
}

// Call options propagate: a pre-aborted signal cancels the first page.
{
  queue = [];
  requests = [];
  const controller = new AbortController();
  controller.abort(new Error('stop'));
  await assert.rejects(
    async () => {
      for await (const item of listEventsItems(client, {}, { signal: controller.signal })) void item;
    },
    (error) => error instanceof Error,
  );
  assert.equal(requests.length, 0, 'an aborted call issues no request');
}

// Manual driving through NextPage: fetches the page described by the input and
// returns the rebuilt input for the following page, or null at the stop rule.
{
  queue = [
    { data: ['a', 'b'], total: 4 },
    { data: ['c', 'd'], total: 4 },
    { data: [], total: 4 },
  ];
  requests = [];
  let input = { limit: 2, offset: 0, filter: 'active' };
  let followed = 0;
  for (;;) {
    const next = await listWidgetsNextPage(client, input);
    if (next === null) break;
    input = next;
    followed += 1;
  }
  assert.equal(followed, 2);
  assert.equal(input.offset, 4);
  assert.equal(input.filter, 'active');
  assert.equal(requests.length, 3);
}

// A repeated identical continuation value throws the branded pagination error
// instead of looping forever.
{
  queue = [
    { items: ['x'], next_page_token: 'again' },
    { items: ['x'], next_page_token: 'again' },
    { items: ['x'], next_page_token: 'again' },
  ];
  requests = [];
  await assert.rejects(
    async () => {
      for await (const item of listEventsItems(client, { cursor: 'start' })) void item;
    },
    (error) => error instanceof Error && error.name === 'PaginationError',
  );
  assert.equal(requests.length, 2, 'the walk stops at the repeated continuation value');
}
"#;

/// Generation options carrying the documented SDK fallback page size.
fn page_size_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version": "v1",
                "pagination": {"mode": "auto", "page_size": 5}
            }))
            .unwrap(),
        ),
        ..GenerationOptions::default()
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn page_size_fallback_supplies_the_first_page_limit_only() {
    let files = generate(pagination_document(), &page_size_options());
    let pagination = files
        .iter()
        .find(|file| file.path == "typescript/pagination.ts")
        .expect("pagination module emitted under sdk defaults");
    // The descriptor carries the fallback page size for the limit-bearing
    // walk only.
    assert!(
        pagination.content.contains("initialLimit: 5"),
        "pagination.ts lacks the configured fallback page size"
    );
    assert!(
        pagination.content.contains("initialLimit: null"),
        "the cursor walk without a limit role carries no fallback"
    );

    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the page_size behavioral check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), PAGE_SIZE_DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
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

const PAGE_SIZE_DRIVER: &str = r#"
import assert from 'node:assert/strict';
import { listWidgetsPages } from './dist/operations.js';

let queue = [];
let requests = [];
globalThis.fetch = async (input, init) => {
  const url = String(input instanceof Request ? input.url : input);
  if (init?.signal?.aborted) throw init.signal.reason ?? new Error('aborted');
  requests.push(url);
  const page = queue.shift();
  assert.ok(page, 'unexpected extra request: ' + url);
  return new Response(JSON.stringify(page), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
};
const client = { auth: { apiKey: 'fixture-secret' } };
const query = (url) => Object.fromEntries(new URL(url).searchParams);

// The caller omits the limit, so the first request carries the documented SDK
// page size (5) and later pages keep it; other members are preserved.
{
  queue = [
    { data: ['a', 'b', 'c', 'd', 'e'], total: 8 },
    { data: ['f', 'g', 'h'], total: 8 },
  ];
  requests = [];
  const pages = [];
  for await (const page of listWidgetsPages(client, { offset: 0, filter: 'active' })) {
    pages.push(page);
    if (pages.length === 2) break;
  }
  assert.equal(pages.length, 2);
  assert.equal(requests.length, 2, 'breaking after two pages never starts a third request');
  assert.deepEqual(query(requests[0]), { limit: '5', offset: '0', filter: 'active' });
  assert.deepEqual(query(requests[1]), { limit: '5', offset: '5', filter: 'active' });
}

// An explicitly passed limit wins on page 1 and is kept afterwards.
{
  queue = [
    { data: ['a', 'b'], total: 4 },
    { data: ['c', 'd'], total: 4 },
  ];
  requests = [];
  const pages = [];
  for await (const page of listWidgetsPages(client, { limit: 2, offset: 0 })) {
    pages.push(page);
    if (pages.length === 2) break;
  }
  assert.equal(pages.length, 2);
  assert.deepEqual(query(requests[0]), { limit: '2', offset: '0' });
  assert.deepEqual(query(requests[1]), { limit: '2', offset: '2' });
}
"#;
