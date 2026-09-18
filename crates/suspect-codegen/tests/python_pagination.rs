//! Generated pagination iteration: emission shape and native behavioral
//! verification of the emitted `iter_*_pages`/`iter_*_items` generators over a
//! stubbed httpx transport. Static runtime files are never modified; the walk
//! lives entirely in the generated `_client.py`.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::json;
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig},
    sdk_defaults::SdkDefaults,
    write_files,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const IMPORT: &str = "pagination_sdk";

fn fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.pagination.test/python-pagination.json").unwrap();
    let response = |schema: serde_json::Value| json!({"200": {"description": "Page", "content": {"application/json": {"schema": schema}}}});
    let document = json!({
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
                "responses": response(json!({
                    "type": "object",
                    "properties": {
                        "data": {"type": "array", "items": {"$ref": "#/components/schemas/Widget"}},
                        "total": {"type": "integer"},
                        "has_more": {"type": "boolean"}
                    }
                }))
            }},
            "/events": {"get": {
                "operationId": "listEvents",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": response(json!({
                    "type": "object",
                    "properties": {
                        "items": {"type": "array", "items": {"type": "string"}},
                        "next_page_token": {"type": "string"}
                    }
                }))
            }}
        },
        "components": {"schemas": {"Widget": {
            "type": "object",
            "properties": {"id": {"type": "string"}}
        }}}
    });
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
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
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}

fn generate(contract: Arc<Contract>, defaults: Option<SdkDefaults>) -> Vec<OutFile> {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "pagination-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    };
    let options = GenerationOptions {
        sdk_defaults: defaults,
        ..Default::default()
    };
    suspect_codegen::backend::generate_with_options(contract, &selected, &target, &options).unwrap()
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == format!("python/src/{IMPORT}/_client.py"))
        .expect("generated client module")
}

#[test]
fn sdk_defaults_emit_pagination_iterators_and_plain_generation_stays_unchanged() {
    let files = generate(fixture(), Some(SdkDefaults::v1()));
    let client = client_file(&files).content.clone();
    for (signature, count) in [
        (
            "def iter_list_widgets_pages(self, **kwargs: Any) -> Iterator[operations.ListWidgetsSuccess]:",
            1,
        ),
        (
            "async def iter_list_widgets_pages(self, **kwargs: Any) -> AsyncIterator[operations.ListWidgetsSuccess]:",
            1,
        ),
        (
            "def iter_list_widgets_items(self, **kwargs: Any) -> Iterator[models.Widget]:",
            1,
        ),
        (
            "async def iter_list_widgets_items(self, **kwargs: Any) -> AsyncIterator[models.Widget]:",
            1,
        ),
        (
            "def iter_list_events_pages(self, **kwargs: Any) -> Iterator[operations.ListEventsSuccess]:",
            1,
        ),
        (
            "async def iter_list_events_pages(self, **kwargs: Any) -> AsyncIterator[operations.ListEventsSuccess]:",
            1,
        ),
        (
            "def iter_list_events_items(self, **kwargs: Any) -> Iterator[str]:",
            1,
        ),
        (
            "async def iter_list_events_items(self, **kwargs: Any) -> AsyncIterator[str]:",
            1,
        ),
    ] {
        assert_eq!(
            client.matches(signature).count(),
            count,
            "signature multiplicity: {signature}"
        );
    }
    // Continuation helpers and the documented caller-override/stall semantics.
    for expected in [
        "def _pagination_field(value: object, name: str) -> object:",
        "def _pagination_count(value: object) -> int:",
        "SdkError('pagination-stalled'",
        "raise SdkError",
        "value == previous",
        "if count == 0:",
        "request['offset'] = offset",
        "request['cursor'] = value",
    ] {
        assert!(client.contains(expected), "missing: {expected}");
    }
    // The collection pointer resolves to declared native attributes.
    assert!(client.contains("_pagination_field(page.data, 'data')"));
    assert!(client.contains("_pagination_field(page.data, 'next_page_token')"));
    assert!(client.contains("_pagination_field(page.data, 'items')"));

    // Without configured defaults nothing new is emitted at all.
    let control = generate(fixture(), None);
    let plain = client_file(&control).content.clone();
    assert!(!plain.contains("iter_list_widgets_pages"));
    assert!(!plain.contains("iter_list_events_pages"));
    assert!(!plain.contains("_pagination_field"));
    assert!(!plain.contains("pagination-stalled"));
    assert_eq!(
        control
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
        files
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
        "pagination emission must not add or remove package files"
    );
}

/// A manual mapping may declare a has-more indicator; the emitted walk must
/// then stop on `false` even when the page still returned items.
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
    let files = generate(fixture(), Some(defaults));
    let client = client_file(&files).content.clone();
    assert_eq!(
        client.matches("is False:").count(),
        2,
        "sync and async walks must both stop on a false has-more indicator"
    );

    let root = tempfile::tempdir().unwrap();
    write_files(&files, root.path()).unwrap();
    let Some(python) = httpx_interpreter() else {
        eprintln!("no interpreter with httpx available; emission assertions only");
        return;
    };
    let script = r#"import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from pagination_sdk import Client


class Stub(httpx.BaseTransport):
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    def handle_request(self, request):
        self.requests.append(request)
        return httpx.Response(200, json=self.pages[min(len(self.requests), len(self.pages)) - 1])


transport = Stub([
    {"data": [{"id": "a"}], "has_more": True},
    {"data": [{"id": "b"}], "has_more": False},
])
with Client(transport=transport) as client:
    pages = list(client.iter_list_widgets_pages(limit=2))
assert len(transport.requests) == 2, len(transport.requests)
assert [item.id for item in pages[0].data.data] == ["a"]
assert [item.id for item in pages[1].data.data] == ["b"]
print("has-more behavior verified")
"#;
    std::fs::write(root.path().join("has_more.py"), script).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("has_more.py"))
            .current_dir(root.path()),
        root.path(),
        "has-more",
    );
}

fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An interpreter able to import httpx: plain `python3` first, then this
/// repository's native Python tools environment, then nothing.
fn httpx_interpreter() -> Option<PathBuf> {
    fn imports_httpx(python: &Path) -> bool {
        Command::new(python)
            .arg("-c")
            .arg("import httpx")
            .output()
            .is_ok_and(|output| output.status.success())
    }
    if imports_httpx(Path::new("python3")) {
        return Some(PathBuf::from("python3"));
    }
    let candidate = std::env::var_os("SUSPECT_PYTHON_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    imports_httpx(&candidate).then_some(candidate)
}

const BEHAVIOR: &str = r#"import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from pagination_sdk import AsyncClient, Client, SdkError


def pairs(request):
    return list(request.url.params.multi_items())


def as_dict(request):
    return dict(pairs(request))


class SyncStub(httpx.BaseTransport):
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    def handle_request(self, request):
        self.requests.append(request)
        index = min(len(self.requests), len(self.pages)) - 1
        return httpx.Response(200, json=self.pages[index])


class AsyncStub(httpx.AsyncBaseTransport):
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    async def handle_async_request(self, request):
        self.requests.append(request)
        index = min(len(self.requests), len(self.pages)) - 1
        return httpx.Response(200, json=self.pages[index])


WIDGET_PAGE_1 = {"data": [{"id": "a"}, {"id": "b"}], "total": 4}
WIDGET_PAGE_MORE = {"data": [{"id": "c"}], "total": 4}
WIDGET_PAGE_END = {"data": [], "total": 4}
CURSOR_PAGE_1 = {"items": ["i1", "i2"], "next_page_token": "c2"}
CURSOR_PAGE_END = {"items": ["i3"]}


def sync_limit_offset_walk():
    transport = SyncStub([WIDGET_PAGE_1, WIDGET_PAGE_END])
    with Client(transport=transport) as client:
        pages = list(client.iter_list_widgets_pages(limit=2, filter="red"))
    assert len(transport.requests) == 2, len(transport.requests)
    assert as_dict(transport.requests[0]) == {"limit": "2", "offset": "0", "filter": "red"}, as_dict(transport.requests[0])
    assert as_dict(transport.requests[1]) == {"limit": "2", "offset": "2", "filter": "red"}, as_dict(transport.requests[1])
    assert len(pages) == 2
    assert [item.id for item in pages[0].data.data] == ["a", "b"]


def sync_caller_offset_is_honored_then_replaced():
    transport = SyncStub([WIDGET_PAGE_1, WIDGET_PAGE_END])
    with Client(transport=transport) as client:
        list(client.iter_list_widgets_pages(limit=2, offset=10))
    assert as_dict(transport.requests[0])["offset"] == "10"
    assert as_dict(transport.requests[1])["offset"] == "12"


def sync_items_flatten_across_pages():
    transport = SyncStub([WIDGET_PAGE_1, WIDGET_PAGE_MORE, WIDGET_PAGE_END])
    with Client(transport=transport) as client:
        items = [item.id for item in client.iter_list_widgets_items(limit=2)]
    assert items == ["a", "b", "c"], items
    assert len(transport.requests) == 3, len(transport.requests)


def sync_early_break_makes_no_third_request():
    transport = SyncStub([WIDGET_PAGE_1, WIDGET_PAGE_MORE, {"data": [{"id": "e"}], "total": 4}])
    with Client(transport=transport) as client:
        iterator = client.iter_list_widgets_pages(limit=2)
        first = next(iterator)
        assert first.data.data[0].id == "a"
        iterator.close()
        assert len(transport.requests) == 1, len(transport.requests)
    collected = []
    transport = SyncStub([WIDGET_PAGE_1, WIDGET_PAGE_MORE, {"data": [{"id": "e"}], "total": 4}])
    with Client(transport=transport) as client:
        for item in client.iter_list_widgets_items(limit=2):
            collected.append(item.id)
            if len(collected) == 2:
                break
    assert collected == ["a", "b"], collected
    assert len(transport.requests) == 1, len(transport.requests)


def sync_cursor_walk_and_stop():
    transport = SyncStub([CURSOR_PAGE_1, CURSOR_PAGE_END])
    with Client(transport=transport) as client:
        walked = list(client.iter_list_events_pages())
    assert len(transport.requests) == 2, len(transport.requests)
    assert "cursor" not in as_dict(transport.requests[0]), as_dict(transport.requests[0])
    assert as_dict(transport.requests[1]) == {"cursor": "c2"}, as_dict(transport.requests[1])
    assert [page.data.items for page in walked] == [["i1", "i2"], ["i3"]]
    transport = SyncStub([CURSOR_PAGE_1, CURSOR_PAGE_END])
    with Client(transport=transport) as client:
        items = list(client.iter_list_events_items())
    assert items == ["i1", "i2", "i3"], items


def sync_caller_cursor_is_honored_then_replaced():
    transport = SyncStub([{"items": ["x"], "next_page_token": "c9"}, {"items": ["y"]}])
    with Client(transport=transport) as client:
        list(client.iter_list_events_pages(cursor="c1"))
    assert as_dict(transport.requests[0]) == {"cursor": "c1"}, as_dict(transport.requests[0])
    assert as_dict(transport.requests[1]) == {"cursor": "c9"}, as_dict(transport.requests[1])


def sync_repeated_continuation_raises_stalled():
    transport = SyncStub([{"items": ["x"], "next_page_token": "c"}])
    with Client(transport=transport) as client:
        try:
            list(client.iter_list_events_pages())
        except SdkError as error:
            assert error.kind == "pagination-stalled", error.kind
        else:
            raise AssertionError("expected a pagination-stalled failure")
    assert len(transport.requests) == 2, len(transport.requests)


def sync_empty_cursor_token_stops():
    transport = SyncStub([{"items": ["x"], "next_page_token": ""}])
    with Client(transport=transport) as client:
        assert len(list(client.iter_list_events_pages())) == 1
    assert len(transport.requests) == 1, len(transport.requests)


async def async_checks():
    transport = AsyncStub([WIDGET_PAGE_1, WIDGET_PAGE_END])
    async with AsyncClient(transport=transport) as client:
        walked = []
        async for page in client.iter_list_widgets_pages(limit=2, filter="red"):
            walked.append(page)
    assert len(walked) == 2
    assert len(transport.requests) == 2, len(transport.requests)
    assert as_dict(transport.requests[0])["offset"] == "0"
    assert as_dict(transport.requests[1])["offset"] == "2"
    assert [item.id for item in walked[0].data.data] == ["a", "b"]

    transport = AsyncStub([CURSOR_PAGE_1, CURSOR_PAGE_END])
    async with AsyncClient(transport=transport) as client:
        items = [item async for item in client.iter_list_events_items()]
    assert items == ["i1", "i2", "i3"], items
    assert len(transport.requests) == 2, len(transport.requests)
    assert as_dict(transport.requests[1]) == {"cursor": "c2"}, as_dict(transport.requests[1])

    transport = AsyncStub([WIDGET_PAGE_1, WIDGET_PAGE_MORE, {"data": [{"id": "e"}], "total": 4}])
    async with AsyncClient(transport=transport) as client:
        iterator = client.iter_list_widgets_pages(limit=2)
        first = await iterator.__anext__()
        await iterator.aclose()
    assert first.data.data[0].id == "a"
    assert len(transport.requests) == 1, len(transport.requests)


sync_limit_offset_walk()
sync_caller_offset_is_honored_then_replaced()
sync_items_flatten_across_pages()
sync_early_break_makes_no_third_request()
sync_cursor_walk_and_stop()
sync_caller_cursor_is_honored_then_replaced()
sync_repeated_continuation_raises_stalled()
sync_empty_cursor_token_stops()
asyncio.run(async_checks())
print("pagination behavior verified")
"#;

#[test]
fn native_pagination_walks_stops_and_stalls_over_a_stubbed_transport() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(fixture(), Some(SdkDefaults::v1()));
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

    let Some(python) = httpx_interpreter() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(root.path().join("behavior.py"), BEHAVIOR).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("behavior.py"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
}

#[test]
fn page_size_fallback_supplies_the_first_page_limit_only() {
    let defaults: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "pagination": {"mode": "auto", "page_size": 5}
    }))
    .unwrap();
    let files = generate(fixture(), Some(defaults));
    let client = client_file(&files).content.clone();
    // The documented fallback fills the absent limit on page 1 only.
    assert!(
        client.contains("request['limit'] = 5"),
        "the walk lacks the documented page-size fallback:\n{}",
        client
    );

    let root = tempfile::tempdir().unwrap();
    write_files(&files, root.path()).unwrap();
    let Some(python) = httpx_interpreter() else {
        eprintln!("no interpreter with httpx available; emission assertions only");
        return;
    };
    let script = r#"import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from pagination_sdk import Client


class Stub(httpx.BaseTransport):
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    def handle_request(self, request):
        self.requests.append(request)
        index = min(len(self.requests), len(self.pages)) - 1
        return httpx.Response(200, json=self.pages[index])


def pairs(request):
    return list(request.url.params.multi_items())


def as_dict(request):
    return dict(pairs(request))


def fallback_supplies_page_one_limit():
    transport = Stub([
        {"data": [{"id": str(i)} for i in range(5)], "total": 8},
        {"data": [{"id": "f"}, {"id": "g"}, {"id": "h"}], "total": 8},
        {"data": [], "total": 8},
    ])
    with Client(transport=transport) as client:
        pages = list(client.iter_list_widgets_pages(offset=0, filter="active"))
    assert len(transport.requests) == 3, len(transport.requests)
    assert as_dict(transport.requests[0]) == {"limit": "5", "offset": "0", "filter": "active"}, as_dict(transport.requests[0])
    assert as_dict(transport.requests[1]) == {"limit": "5", "offset": "5", "filter": "active"}, as_dict(transport.requests[1])
    assert as_dict(transport.requests[2]) == {"limit": "5", "offset": "8", "filter": "active"}, as_dict(transport.requests[2])
    assert len(pages) == 3


def explicit_limit_wins():
    transport = Stub([
        {"data": [{"id": "a"}, {"id": "b"}], "total": 4},
        {"data": [], "total": 4},
    ])
    with Client(transport=transport) as client:
        list(client.iter_list_widgets_pages(limit=2))
    assert as_dict(transport.requests[0]) == {"limit": "2", "offset": "0"}, as_dict(transport.requests[0])
    assert as_dict(transport.requests[1]) == {"limit": "2", "offset": "2"}, as_dict(transport.requests[1])


fallback_supplies_page_one_limit()
explicit_limit_wins()
print("page-size fallback verified")
"#;
    std::fs::write(root.path().join("page_size.py"), script).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("page_size.py"))
            .current_dir(root.path()),
        root.path(),
        "page-size",
    );
}
