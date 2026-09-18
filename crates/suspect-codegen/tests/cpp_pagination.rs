//! M3 pagination runtime emission for the C++ backend: generation-time
//! emission shape, the no-policy byte-identity control, and native behavioral
//! verification of the generated pull pagers against a scripted transport.
//! Static runtime files are never modified; every walker lives in the emitted
//! `include/<package>/pagination.hpp` plus conditional `client.hpp` members.

#![cfg(feature = "cpp-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi":"3.1.0",
        "info":{"title":"Pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "parameters":[
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}},
                    {"name":"offset","in":"query","schema":{"type":"integer","minimum":0}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["data","total"],"properties":{
                        "data":{"type":"array","items":{"$ref":"#/components/schemas/Widget"}},
                        "total":{"type":"integer"}
                    }}}}}}}
            },
            "/events":{"get":{
                "operationId":"listEvents",
                "parameters":[
                    {"name":"cursor","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "items":{"type":"array","items":{"type":"string"}},
                        "next_page_token":{"type":"string"}
                    }}}}}}}
            }
        },
        "components":{"schemas":{"Widget":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}}}
    })
}

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
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

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    generate_document(document(), options)
}

fn generate_document(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
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
    let contract = Arc::new(Contract::from_workspace(&workspace, &entry).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::CppHttp,
            package_name: "pagination_cpp".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
        ..Default::default()
    }
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

#[test]
fn configured_emission_adds_only_the_pagination_header() {
    let mut configured = generate(&configured_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    let pagination_path = "cpp/include/pagination_cpp/pagination.hpp";
    assert!(
        !control.iter().any(|file| file.path == pagination_path),
        "no-policy output must not carry pagination walkers"
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if file.path.ends_with("client.hpp") {
            assert_ne!(
                emitted.content, file.content,
                "the paginated client declares the pager-returning methods"
            );
        } else {
            assert_eq!(emitted.content, file.content, "{} changed", file.path);
        }
    }
    let client = configured
        .iter()
        .find(|file| file.path.ends_with("cpp/include/pagination_cpp/client.hpp"))
        .unwrap()
        .content
        .clone();
    for expected in [
        "class ListWidgetsPager;",
        "class ListWidgetsItemsPager;",
        "[[nodiscard]] ListWidgetsPager list_widgets_pages(const ListWidgetsInput& input, CallOptions options = {}) const;",
        "[[nodiscard]] ListWidgetsItemsPager list_widgets_items(const ListWidgetsInput& input, CallOptions options = {}) const;",
        "[[nodiscard]] std::optional<ListWidgetsInput> list_widgets_next_page(const ListWidgetsInput& input, CallOptions options = {}) const;",
    ] {
        assert!(client.contains(expected), "client.hpp lacks {expected}");
    }
    let control_client = control
        .iter()
        .find(|file| file.path.ends_with("cpp/include/pagination_cpp/client.hpp"))
        .unwrap();
    assert!(!control_client.content.contains("list_widgets_pages"));

    let pagination = configured
        .iter()
        .find(|file| file.path == pagination_path)
        .expect("pagination walkers emitted")
        .content
        .clone();
    for expected in [
        // The typed traversal report and the exact pull-pager surface.
        "struct PaginationLoop {",
        "struct ListWidgetsPagerError {",
        "class ListWidgetsPager {",
        "class ListWidgetsItemsPager {",
        "bool next(Presence<ListWidgetsSuccess>& out)",
        "const ListWidgetsSuccess& value() const",
        "const ListWidgetsPagerError& error() const",
        "bool next()",
        // Limit/offset walk: generated accessor chains, the documented initial
        // offset fill, the arithmetic advance and the zero-item stop.
        "inline Presence<const std::vector<::pagination_cpp::Widget>*> list_widgets_page_items(const ListWidgetsStatus200& page)",
        "if (!input_.offset) input_.offset = JsonInteger(0);",
        "std::int64_t base = 0;",
        "input_.offset = JsonInteger(advanced.value());",
        "if (count == 0) return std::nullopt;",
        // Cursor walk: pointer-driven continuation with the repeat guard.
        "inline Presence<std::string> list_events_page_cursor(const ListEventsStatus200& page)",
        "inline Presence<const std::vector<std::string>*> list_events_page_items(const ListEventsStatus200& page)",
        "if (!token || token.value().empty()) return std::nullopt;",
        "PaginationLoop{",
        "the source API returned an identical continuation value; the paginated walk would never terminate",
        "input_.cursor = token.value();",
        // The client method definitions and the next-input builder.
        "inline ListWidgetsPager Client::list_widgets_pages(const ListWidgetsInput& input, CallOptions options) const",
        "inline ListWidgetsItemsPager Client::list_widgets_items(const ListWidgetsInput& input, CallOptions options) const",
        "inline std::optional<ListWidgetsInput> Client::list_widgets_next_page(const ListWidgetsInput& input, CallOptions options) const",
    ] {
        assert!(
            pagination.contains(expected),
            "pagination.hpp is missing:\n{expected}\n--- emitted: ---\n{pagination}"
        );
    }
}

#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({"version":"v1","pagination":"off"})).unwrap(),
        ),
        ..Default::default()
    };
    let mut disabled = generate(&off);
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut disabled);
    sorted(&mut control);
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[test]
fn plan_carries_the_compiled_selection_only_when_configured() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = suspect_codegen::cpp_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::cpp_sdk::SdkConfig {
            name: "pagination_cpp".into(),
            namespace: "pagination_cpp".into(),
            sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    let pagination = configured
        .pagination()
        .expect("configured policy is carried");
    assert!(pagination.emits());
    assert_eq!(pagination.outcome.paginated.len(), 2);
    assert_eq!(pagination.operations.len(), 2);
    let widgets = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "list_widgets")
        .expect("limit/offset operation");
    assert_eq!(
        widgets.advance,
        suspect_codegen::cpp_sdk::Advance::OffsetItemCount
    );
    assert_eq!(widgets.initial_offset, Some(0));
    assert_eq!(widgets.control.field, "offset");
    assert_eq!(
        widgets.control.kind,
        suspect_codegen::cpp_sdk::ControlKind::Integer
    );
    assert!(widgets.control.optional);
    assert_eq!(widgets.pager, "ListWidgetsPager");
    assert_eq!(
        widgets.items_pager.as_deref(),
        Some("ListWidgetsItemsPager")
    );
    assert_eq!(widgets.pages, "list_widgets_pages");
    assert_eq!(widgets.items.as_deref(), Some("list_widgets_items"));
    assert_eq!(widgets.next_page, "list_widgets_next_page");
    assert_eq!(
        widgets.items_accessor.as_ref().map(|a| a.pointer.as_str()),
        Some("/data")
    );
    let events = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "list_events")
        .expect("cursor operation");
    assert_eq!(
        events.advance,
        suspect_codegen::cpp_sdk::Advance::CursorPointer
    );
    assert_eq!(events.initial_offset, None);
    assert_eq!(events.control.field, "cursor");
    assert_eq!(
        events.control.kind,
        suspect_codegen::cpp_sdk::ControlKind::Text
    );
    assert_eq!(events.pager, "ListEventsPager");
    assert!(events.items.is_some());
    let control =
        suspect_codegen::cpp_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
    assert!(control.pagination().is_none());
}

const CONSUMER: &str = r#"// Scripted-transport consumer asserting the generated pagination walkers.
#include <pagination_cpp/sdk.hpp>
#include <pagination_cpp/pagination.hpp>

#include <iostream>
#include <memory>
#include <string>
#include <utility>
#include <vector>

using namespace pagination_cpp;

namespace {

int failures = 0;

void expect(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << "failed: " << message << "\n";
        ++failures;
    }
}

class ScriptedTransport final : public Transport {
public:
    explicit ScriptedTransport(std::vector<std::string> pages) : pages_(std::move(pages)) {}
    Result<HttpResponse, TransportError> send(const HttpRequest& request, const TransportOptions&) const override {
        const auto marker = request.url.find('?');
        queries_.push_back(marker == std::string::npos ? std::string() : request.url.substr(marker + 1));
        const auto index = queries_.size() - 1;
        if (index >= pages_.size()) {
            TransportError error;
            error.kind = TransportError::Kind::Protocol;
            error.message = "unexpected request " + std::to_string(index);
            return Result<HttpResponse, TransportError>::failure(std::move(error));
        }
        HttpResponse response;
        response.status = 200;
        response.headers = Headers{{"Content-Type", "application/json"}};
        response.body = pages_[index];
        return Result<HttpResponse, TransportError>::success(std::move(response));
    }
    const std::vector<std::string>& queries() const { return queries_; }

private:
    std::vector<std::string> pages_;
    mutable std::vector<std::string> queries_;
};

// Two-page limit/offset walk: exactly 2 requests, offset 0 then 2, with the
// filter preserved and the caller's limit never rewritten.
void limit_offset_walk() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"data":[{"id":"a"},{"id":"b"}],"total":9})",
        R"({"data":[],"total":9})",
    });
    Client client(transport);
    ListWidgetsInput input;
    input.limit = JsonInteger(2);
    input.filter = "active";
    int pages = 0;
    Presence<ListWidgetsSuccess> page;
    for (auto pager = client.list_widgets_pages(input); pager.next(page);) {
        ++pages;
        expect(std::get_if<ListWidgetsStatus200>(&pager.value()) != nullptr, "delivered page variant");
        if (pages == 2) break;
    }
    expect(pages == 2, "two-page walk delivered two pages");
    expect(transport->queries().size() == 2, "early break never started a third request");
    expect(transport->queries()[0] == "limit=2&offset=0&filter=active", "first page query");
    expect(transport->queries()[1] == "limit=2&offset=2&filter=active", "second page query");
}

// Full item walk: the empty third page stops the walk.
void item_walk_flattens_pages() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"data":[{"id":"a"},{"id":"b"}],"total":9})",
        R"({"data":[{"id":"c"}],"total":9})",
        R"({"data":[],"total":9})",
    });
    Client client(transport);
    ListWidgetsInput input;
    input.limit = JsonInteger(2);
    int items = 0;
    for (auto pager = client.list_widgets_items(input); pager.next();) {
        ++items;
        expect(pager.value().id == (items == 1 ? "a" : items == 2 ? "b" : "c"), "item order");
    }
    expect(items == 3, "item walk flattened three items");
    expect(transport->queries().size() == 3, "item walk made three requests");
    expect(transport->queries()[2] == "limit=2&offset=3", "final offset advanced by the item count");
}

// Cursor walk: the second request carries cursor=c2 and the missing token
// stops the walk.
void cursor_walk() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"items":["x","y"],"next_page_token":"c2"})",
        R"({"items":["z"]})",
    });
    Client client(transport);
    ListEventsInput events;
    int items = 0;
    for (auto pager = client.list_events_items(events); pager.next();) {
        ++items;
    }
    expect(items == 3, "cursor item walk flattened three items");
    expect(transport->queries().size() == 2, "cursor walk made two requests");
    expect(transport->queries()[0].empty(), "first request invented no cursor");
    expect(transport->queries()[1] == "cursor=c2", "second request carried the next cursor");
}

// Early break after the first item issues no second request.
void early_break() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"items":["x","y"],"next_page_token":"c2"})",
    });
    Client client(transport);
    ListEventsInput events;
    for (auto pager = client.list_events_items(events); pager.next();) {
        break;
    }
    expect(transport->queries().size() == 1, "early break issued no second request");
}

// A repeated identical continuation fails the walk with the typed pagination
// error instead of looping; the repeated page is still delivered exactly once.
void repeated_continuation_is_typed() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"items":["x"],"next_page_token":"again"})",
        R"({"items":["y"],"next_page_token":"again"})",
    });
    Client client(transport);
    ListEventsInput events;
    Presence<ListEventsSuccess> page;
    auto pager = client.list_events_pages(events);
    expect(pager.next(page), "first page delivered");
    expect(pager.next(page), "page carrying the repeated cursor delivered");
    expect(!pager.next(page), "the walk refused to loop");
    expect(pager.error().loop.has_value(), "typed loop report engaged");
    expect(pager.error().loop.value().value == "again", "loop report carries the value");
    expect(pager.error().cause.has_value(), "typed cause engaged");
    expect(transport->queries().size() == 2, "the walk stopped without another request");
}

// The next-input builder fetches the page for the input and returns the
// rebuilt input for the following page.
void next_page_builder() {
    auto transport = std::make_shared<ScriptedTransport>(std::vector<std::string>{
        R"({"data":[{"id":"a"},{"id":"b"}],"total":9})",
    });
    Client client(transport);
    ListWidgetsInput input;
    input.limit = JsonInteger(2);
    input.filter = "active";
    auto next = client.list_widgets_next_page(input);
    expect(next.has_value(), "a following page exists");
    expect(next->offset.has_value() && next->offset.value().token() == "2", "next input carries the advanced offset");
    expect(next->limit.has_value() && next->limit.value().token() == "2", "next input preserved the limit");
    expect(next->filter.has_value() && next->filter.value() == "active", "next input preserved the filter");
    expect(transport->queries().size() == 1, "the builder issued exactly one request");
}

} // namespace

int main() {
    limit_offset_walk();
    item_walk_flattens_pages();
    cursor_walk();
    early_break();
    repeated_continuation_is_typed();
    next_page_builder();
    return failures == 0 ? 0 : 1;
}
"#;

const CONSUMER_CMAKE: &str = r#"cmake_minimum_required(VERSION 3.24)
project(PaginationConsumer LANGUAGES CXX)
set(SUSPECT_SDK_WITH_CURL OFF CACHE BOOL "" FORCE)
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../cpp pagination-build)
add_executable(consumer main.cpp)
target_link_libraries(consumer PRIVATE pagination_cpp::pagination_cpp)
set_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)
target_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)
enable_testing()
add_test(NAME pagination COMMAND consumer)
"#;

/// A second contract exercising every remaining walker template through
/// manual mappings: page-number walks with has-more, a top-level array page,
/// a nested next-offset pointer, and a required cursor parameter.
fn manual_document() -> Value {
    let rows = json!({
        "get": {
            "operationId": "listRows",
            "parameters": [
                {"name":"perPage","in":"query","schema":{"type":"integer","minimum":1}},
                {"name":"page","in":"query","schema":{"type":"integer","minimum":1}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["records","paging"],"properties":{
                        "records":{"type":"array","items":{"type":"integer"}},
                        "paging":{"type":"object","required":["more"],"properties":{
                            "more":{"type":"boolean"},
                            "total":{"type":"integer"}}
                        }
                    }}}}}
            }
        }
    });
    let blobs = json!({
        "get": {
            "operationId": "listBlobs",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"array","items":{"type":"string"}}}}}
            }
        }
    });
    let alerts = json!({
        "get": {
            "operationId": "listAlerts",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["entries","paging"],"properties":{
                        "entries":{"type":"array","items":{"type":"string"}},
                        "paging":{"type":"object","required":["count"],"properties":{
                            "count":{"type":"integer"}}
                        }
                    }}}}}
            }
        }
    });
    let audit = json!({
        "get": {
            "operationId": "listAuditEvents",
            "parameters": [
                {"name":"after","in":"query","required":true,"schema":{"type":"string"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["events","next"],"properties":{
                        "events":{"type":"array","items":{"type":"string"}},
                        "next":{"type":"string"}
                    }}}}}
            }
        }
    });
    json!({
        "openapi":"3.1.0",
        "info":{"title":"Manual pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths": {"/rows": rows, "/blobs": blobs, "/alerts": alerts, "/audit": audit}
    })
}

fn manual_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "pagination": {"mode":"auto","operations":{
                    "listRows": {
                        "pattern":"page-number",
                        "request":{"page":"page","limit":"perPage"},
                        "response":{"items":"/records","has-more":"/paging/more","total":"/paging/total"}
                    },
                    "listBlobs": {
                        "pattern":"limit-offset",
                        "request":{"limit":"limit","offset":"offset"},
                        "response":{"items":""},
                        "initial_offset":0,
                        "advance":"items-returned"
                    },
                    "listAlerts": {
                        "pattern":"limit-offset",
                        "request":{"limit":"limit","offset":"offset"},
                        "response":{"items":"/entries","next-offset":"/paging/count"},
                        "advance":"next-offset"
                    },
                    "listAuditEvents": {
                        "pattern":"cursor",
                        "request":{"cursor":"after"},
                        "response":{"items":"/events","next-cursor":"/next"}
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// Every supported walker template emits through the typed lowering, and the
/// generated header typechecks inside a compiled consumer package.
#[test]
fn manual_templates_emit_for_every_supported_shape_and_compile() {
    let files = generate_document(manual_document(), &manual_options());
    let pagination = files
        .iter()
        .find(|file| file.path.ends_with("pagination_cpp/pagination.hpp"))
        .expect("manual mappings emit the pagination walkers")
        .content
        .clone();
    for expected in [
        // Page-number walk with a nested has-more pointer and one-based advance.
        "inline Presence<bool> list_rows_page_has_more(const ListRowsStatus200& page)",
        "inline Presence<const std::vector<JsonInteger>*> list_rows_page_items(const ListRowsStatus200& page)",
        "std::int64_t base = 1;",
        "detail::pagination_advance(base, 1)",
        // Top-level array page: the body itself is the collection.
        "inline Presence<const std::vector<std::string>*> list_blobs_page_items(const ListBlobsStatus200& page)",
        "return &page.data;",
        // Nested next-offset pointer feeding the offset control.
        "inline Presence<JsonInteger> list_alerts_page_offset(const ListAlertsStatus200& page)",
        "previous_.value() == token.value()",
        // Required cursor control: the stall guard reads the caller's token.
        "inline Presence<std::string> list_audit_events_page_cursor(const ListAuditEventsStatus200& page)",
        "input_.after = token.value();",
    ] {
        assert!(
            pagination.contains(expected),
            "manual pagination.hpp is missing:\n{expected}\n--- emitted: ---\n{pagination}"
        );
    }
    let plan = files
        .iter()
        .find(|file| file.path.ends_with("pagination_cpp/client.hpp"))
        .unwrap()
        .content
        .clone();
    for expected in [
        "class ListRowsPager;",
        "class ListBlobsPager;",
        "class ListAlertsPager;",
        "class ListAuditEventsPager;",
        "ListRowsPager list_rows_pages(const ListRowsInput& input, CallOptions options = {}) const;",
        "ListAlertsPager list_alerts_pages(const ListAlertsInput& input, CallOptions options = {}) const;",
    ] {
        assert!(plan.contains(expected), "client.hpp lacks {expected}");
    }

    // The manual package typechecks: compile it with the same consumer gate.
    let Some(cmake) = tool("SUSPECT_CPP_CMAKE", "cmake") else {
        eprintln!("cpp_pagination: cmake not available; emission assertions only");
        return;
    };
    let Some(cxx) = tool("SUSPECT_CPP_CXX", "clang++") else {
        eprintln!("cpp_pagination: no C++20 compiler; emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(
        consumer.join("main.cpp"),
        "#include <pagination_cpp/sdk.hpp>\n#include <pagination_cpp/pagination.hpp>\n\nint main() { return 0; }\n",
    )
    .unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
}

fn tool(variable: &str, fallback: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Some(std::path::PathBuf::from(path));
    }
    let probe = Command::new(fallback).arg("--version").output().ok()?;
    if probe.status.success() {
        Some(std::path::PathBuf::from(fallback))
    } else {
        None
    }
}

/// Runs the command and fails the test with the retained log on failure.
fn checked(command: &mut Command, retained: &std::path::Path) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("required native tool {command:?}: {error}"));
    let log = retained.join("commands.log");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    let _ = writeln!(
        file,
        "\n{command:?}\nstatus: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "native pagination gate retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_walks_count_requests_and_preserve_input() {
    let Some(cmake) = tool("SUSPECT_CPP_CMAKE", "cmake") else {
        eprintln!("cpp_pagination: cmake not available; degrading to static assertions");
        return;
    };
    let Some(cxx) = tool("SUSPECT_CPP_CXX", "clang++") else {
        eprintln!("cpp_pagination: no C++20 compiler; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&configured_options()), root.path()).unwrap();
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(&consumer).unwrap();
    std::fs::write(consumer.join("main.cpp"), CONSUMER).unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"), CONSUMER_CMAKE).unwrap();
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&consumer)
        .arg("-B")
        .arg(root.path().join("build"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()))
        .arg("-DCMAKE_BUILD_TYPE=Debug");
    checked(&mut configure, root.path());
    checked(
        Command::new(&cmake)
            .arg("--build")
            .arg(root.path().join("build"))
            .args(["--parallel", "2"]),
        root.path(),
    );
    let ctest = cmake.with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.path().join("build"))
            .arg("--output-on-failure"),
        root.path(),
    );
}
