//! Emitted pagination helpers: generation-time emission plus native behavior.
//!
//! The configured policy adds exactly one file (`go/pagination.go`) plus the
//! per-operation walk code inside it; the no-policy generation for the same
//! source stays byte-identical, which is what keeps the pinned inventory in
//! `go_credential_env.rs` at its 35 no-policy files.
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"Pagination","version":"1"},
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
                    {"name":"cursor","in":"query","schema":{"type":"string"}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
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
    generate_with_options(
        contract(),
        &contract()
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>(),
        &TargetConfig {
            backend: Backend::GoHttp,
            package_name: "example.com/pagination-sdk".into(),
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
fn configured_emission_adds_only_pagination_go() {
    let mut configured = generate(&configured_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    assert!(
        !control.iter().any(|file| file.path == "go/pagination.go"),
        "no-policy output must not carry pagination helpers"
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
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let pagination = configured
        .iter()
        .find(|file| file.path == "go/pagination.go")
        .expect("pagination helpers emitted");
    for expected in [
        "type PaginationError struct",
        "func (e *PaginationError) Error() string",
        "func (e *PaginationError) Unwrap() error",
        // Limit/offset walk: arithmetic offset advance with the documented
        // zero-item stop and the caller's filter/limit preserved.
        "func (c *Client) ListWidgetsPages(ctx context.Context, input ListWidgetsInput) *ListWidgetsPageIterator",
        "func (c *Client) ListWidgetsItems(ctx context.Context, input ListWidgetsInput) *ListWidgetsItemIterator",
        "func (c *Client) ListWidgetsNextPage(ctx context.Context, input ListWidgetsInput) (ListWidgetsInput, bool)",
        "type ListWidgetsPageIterator struct",
        "type ListWidgetsItemIterator struct",
        "func (it *ListWidgetsPageIterator) Next() bool",
        "func (it *ListWidgetsPageIterator) Page() ListWidgetsResult",
        "func (it *ListWidgetsPageIterator) Err() error",
        "func (it *ListWidgetsPageIterator) Close()",
        "func (it *ListWidgetsItemIterator) Next() bool",
        "func (it *ListWidgetsItemIterator) Item()",
        "func (it *ListWidgetsItemIterator) Err() error",
        "func (page ListWidgetsStatus200) httpPaginationItems() []",
        "return page.Data.Data",
        "it.input.Offset = OptionalSome(httpPageInteger(0))",
        "next, err := httpPageAdvance(it.offset, count)",
        // Cursor walk: pointer-driven continuation with the repeat guard.
        "func (c *Client) ListEventsPages(ctx context.Context, input ListEventsInput) *ListEventsPageIterator",
        "func (c *Client) ListEventsItems(ctx context.Context, input ListEventsInput) *ListEventsItemIterator",
        "func (c *Client) ListEventsNextPage(ctx context.Context, input ListEventsInput) (ListEventsInput, bool)",
        "func (page ListEventsStatus200) httpPaginationCursor() (string, bool)",
        "return page.Data.NextPageToken.Value, true",
        "if !present || token == \"\" {",
        "the server repeated the same continuation cursor",
        "it.input.Cursor = OptionalSome(token)",
    ] {
        assert!(
            pagination.content.contains(expected),
            "pagination.go is missing:\n{expected}\n--- emitted: ---\n{}",
            pagination.content
        );
    }
}

#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1","pagination":"off"
            }))
            .unwrap(),
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
fn plan_carries_the_pagination_outcome_only_when_configured() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = go_http::plan_http(
        contract.clone(),
        &selected,
        go_http::HttpConfig {
            sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    let pagination = configured
        .pagination()
        .expect("configured policy is carried");
    assert!(pagination.emits());
    assert_eq!(pagination.error_type.as_deref(), Some("PaginationError"));
    assert_eq!(pagination.operations.len(), 2);
    let widgets = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "ListWidgets")
        .expect("limit/offset operation");
    assert_eq!(
        widgets.advance,
        go_http::pagination::EmittedAdvance::OffsetItemCount
    );
    assert_eq!(widgets.initial_offset, Some(0));
    assert_eq!(widgets.pages, "ListWidgetsPages");
    assert_eq!(widgets.items.as_deref(), Some("ListWidgetsItems"));
    assert_eq!(widgets.page_iterator, "ListWidgetsPageIterator");
    assert_eq!(
        widgets.item_iterator.as_deref(),
        Some("ListWidgetsItemIterator")
    );
    let events = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "ListEvents")
        .expect("cursor operation");
    assert_eq!(
        events.advance,
        go_http::pagination::EmittedAdvance::CursorPointer
    );
    assert_eq!(events.initial_offset, None);
    assert_eq!(events.pages, "ListEventsPages");
    let control = go_http::plan_http(contract, &selected, go_http::HttpConfig::default()).unwrap();
    assert!(control.pagination().is_none());
}

const BEHAVIOR: &str = r#"package sdk_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"

	sdk "example.com/pagination-sdk"
)

type fake struct {
	mu       sync.Mutex
	requests []string
	pages    []string
}

func (f *fake) Do(request *http.Request) (*http.Response, error) {
	f.mu.Lock()
	index := len(f.requests)
	f.requests = append(f.requests, request.URL.RawQuery)
	f.mu.Unlock()
	if index >= len(f.pages) {
		return nil, fmt.Errorf("unexpected request %d for %s", index, request.URL.RawQuery)
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": {"application/json"}}, Body: io.NopCloser(strings.NewReader(f.pages[index]))}, nil
}

func client(transport *fake) *sdk.Client {
	created, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: transport})
	if err != nil {
		panic(err)
	}
	return created
}

func TestLimitOffsetWalkIssuesOffsetZeroThenTwoAndPreservesFilters(t *testing.T) {
	limit, _ := sdk.ParseInteger("2")
	transport := &fake{pages: []string{
		`{"data":[{"id":"a"},{"id":"b"}],"total":9}`,
		`{"data":[],"total":9}`,
	}}
	iterator := client(transport).ListWidgetsPages(context.Background(), sdk.NewListWidgetsInput().WithLimit(limit).WithFilter("x"))
	pages := 0
	for iterator.Next() {
		pages++
		page := iterator.Page().(sdk.ListWidgetsStatus200)
		if page.Status != 200 {
			t.Fatal("unexpected status", page.Status)
		}
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
	if pages != 2 || len(transport.requests) != 2 {
		t.Fatal("walk shape changed", pages, transport.requests)
	}
	for _, request := range transport.requests {
		if !strings.Contains(request, "limit=2") || !strings.Contains(request, "filter=x") {
			t.Fatal("pagination did not preserve other input members", request)
		}
	}
	if !strings.Contains(transport.requests[0], "offset=0") || !strings.Contains(transport.requests[1], "offset=2") {
		t.Fatal("offset controls did not advance 0 then 2", transport.requests)
	}
}

func TestItemWalkFlattensPagesAndHonorsCallerOffset(t *testing.T) {
	limit, _ := sdk.ParseInteger("2")
	offset, _ := sdk.ParseInteger("10")
	transport := &fake{pages: []string{
		`{"data":[{"id":"a"},{"id":"b"}],"total":9}`,
		`{"data":[{"id":"c"}],"total":9}`,
		`{"data":[],"total":9}`,
	}}
	iterator := client(transport).ListWidgetsItems(context.Background(), sdk.NewListWidgetsInput().WithLimit(limit).WithOffset(offset).WithFilter("x"))
	count := 0
	for iterator.Next() {
		count++
		_ = iterator.Item()
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
	if count != 3 || len(transport.requests) != 3 {
		t.Fatal("walk shape changed", count, transport.requests)
	}
	if !strings.Contains(transport.requests[0], "offset=10") || !strings.Contains(transport.requests[1], "offset=12") || !strings.Contains(transport.requests[2], "offset=13") {
		t.Fatal("caller offset was not respected", transport.requests)
	}
}

func TestCursorWalkSendsSecondCursorAndStopsWhenMissing(t *testing.T) {
	transport := &fake{pages: []string{
		`{"items":["i1"],"next_page_token":"c2"}`,
		`{"items":["i2"]}`,
	}}
	iterator := client(transport).ListEventsItems(context.Background(), sdk.NewListEventsInput().WithFilter("f"))
	count := 0
	for iterator.Next() {
		count++
		_ = iterator.Item()
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
	if count != 2 || len(transport.requests) != 2 {
		t.Fatal("walk shape changed", count, transport.requests)
	}
	if strings.Contains(transport.requests[0], "cursor=") {
		t.Fatal("first request invented a cursor", transport.requests)
	}
	if !strings.Contains(transport.requests[1], "cursor=c2") {
		t.Fatal("second request did not carry the next cursor", transport.requests)
	}
}

func TestEarlyBreakIssuesNoSecondRequest(t *testing.T) {
	transport := &fake{pages: []string{`{"items":["i1"],"next_page_token":"c2"}`}}
	iterator := client(transport).ListEventsItems(context.Background(), sdk.NewListEventsInput())
	if !iterator.Next() {
		t.Fatal("first item missing", iterator.Err())
	}
	_ = iterator.Item()
	if len(transport.requests) != 1 {
		t.Fatal("early break issued another request", transport.requests)
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
}

func TestCancellationStopsTheWalkBeforeAnyRequest(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	transport := &fake{pages: []string{`{"items":["i1"],"next_page_token":"c2"}`}}
	iterator := client(transport).ListEventsPages(ctx, sdk.NewListEventsInput())
	if iterator.Next() {
		t.Fatal("cancelled walk fetched a page")
	}
	if !errors.Is(iterator.Err(), context.Canceled) {
		t.Fatal("cancellation lost", iterator.Err())
	}
	if len(transport.requests) != 0 {
		t.Fatal("cancelled walk reached the transport", transport.requests)
	}
}

func TestRepeatedCursorFailsWithTheTypedPaginationError(t *testing.T) {
	transport := &fake{pages: []string{
		`{"items":["i1"],"next_page_token":"c1"}`,
		`{"items":["i2"],"next_page_token":"c1"}`,
	}}
	iterator := client(transport).ListEventsPages(context.Background(), sdk.NewListEventsInput())
	if !iterator.Next() {
		t.Fatal("first page missing", iterator.Err())
	}
	// The page that repeats the token is still delivered exactly once; the
	// walk stops on the following Next with the typed error, issuing no
	// further request.
	if !iterator.Next() {
		t.Fatal("page carrying the repeated cursor was not delivered", iterator.Err())
	}
	if iterator.Next() {
		t.Fatal("non-progress looped")
	}
	var failure *sdk.PaginationError
	if !errors.As(iterator.Err(), &failure) {
		t.Fatal("repeated cursor did not produce the typed pagination error", iterator.Err())
	}
	if len(transport.requests) != 2 {
		t.Fatal("non-progress looped", transport.requests)
	}
}

func TestNextPageBuildsTheFollowingInput(t *testing.T) {
	limit, _ := sdk.ParseInteger("2")
	transport := &fake{pages: []string{`{"data":[{"id":"a"},{"id":"b"}],"total":9}`}}
	next, more := client(transport).ListWidgetsNextPage(context.Background(), sdk.NewListWidgetsInput().WithLimit(limit).WithFilter("x"))
	if !more {
		t.Fatal("expected a following page")
	}
	if len(transport.requests) != 1 {
		t.Fatal("builder issued extra requests", transport.requests)
	}
	if !next.Offset.IsSet || next.Offset.Value.String() != "2" {
		t.Fatal("next input lost the advanced offset", next.Offset)
	}
	if !next.Limit.IsSet || !next.Filter.IsSet || next.Filter.Value != "x" {
		t.Fatal("next input dropped other members", next)
	}
}
"#;

fn go_toolchain() -> Option<String> {
    let output = Command::new("go").arg("version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    // "go version go1.27.1 darwin/arm64" — require at least the emitted go.mod
    // language version so the module builds with the installed toolchain.
    let version = text.split_whitespace().nth(2)?;
    let minor = version
        .strip_prefix("go1.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 23).then_some(text)
}

#[test]
fn native_walks_count_requests_and_preserve_input() {
    let Some(version) = go_toolchain() else {
        eprintln!(
            "go_pagination: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    eprintln!("go_pagination: {version}");
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&configured_options()), root.path()).unwrap();
    std::fs::write(root.path().join("go/pagination_behavior_test.go"), BEHAVIOR).unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "."])
        .current_dir(root.path().join("go"))
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native pagination behavior failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "go_pagination: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}

#[test]
fn native_module_builds_with_the_pagination_file() {
    let Some(_) = go_toolchain() else {
        eprintln!(
            "go_pagination: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&configured_options()), root.path()).unwrap();
    for arguments in [["build", "./..."], ["vet", "."]] {
        let output = Command::new("go")
            .args(arguments)
            .current_dir(root.path().join("go"))
            .env("GOWORK", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "go {} failed\n{}{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn page_size_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "pagination": {"mode": "auto", "page_size": 5}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

const PAGE_SIZE_BEHAVIOR: &str = r#"package sdk_test

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"

	sdk "example.com/pagination-sdk"
)

type sizeFake struct {
	mu       sync.Mutex
	requests []string
	pages    []string
}

func (f *sizeFake) Do(request *http.Request) (*http.Response, error) {
	f.mu.Lock()
	index := len(f.requests)
	f.requests = append(f.requests, request.URL.RawQuery)
	f.mu.Unlock()
	if index >= len(f.pages) {
		return nil, fmt.Errorf("unexpected request %d for %s", index, request.URL.RawQuery)
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": {"application/json"}}, Body: io.NopCloser(strings.NewReader(f.pages[index]))}, nil
}

func sizeClient(transport *sizeFake) *sdk.Client {
	created, err := sdk.NewClient(sdk.Credentials{}, sdk.ClientOptions{Transport: transport})
	if err != nil {
		panic(err)
	}
	return created
}

func TestPageSizeFallbackSuppliesTheFirstPageLimit(t *testing.T) {
	offset, _ := sdk.ParseInteger("0")
	transport := &sizeFake{pages: []string{
		`{"data":[{"id":"a"},{"id":"b"},{"id":"c"},{"id":"d"},{"id":"e"}],"total":8}`,
		`{"data":[{"id":"f"},{"id":"g"},{"id":"h"}],"total":8}`,
		`{"data":[],"total":8}`,
	}}
	iterator := sizeClient(transport).ListWidgetsPages(context.Background(), sdk.NewListWidgetsInput().WithOffset(offset).WithFilter("x"))
	pages := 0
	for iterator.Next() {
		pages++
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
	if pages != 3 || len(transport.requests) != 3 {
		t.Fatal("walk shape changed", pages, transport.requests)
	}
	for index, request := range transport.requests {
		if !strings.Contains(request, "limit=5") {
			t.Fatal("request", index, "lost the documented fallback page size", request)
		}
		if !strings.Contains(request, "filter=x") {
			t.Fatal("request", index, "dropped other input members", request)
		}
	}
	if !strings.Contains(transport.requests[0], "offset=0") || !strings.Contains(transport.requests[1], "offset=5") || !strings.Contains(transport.requests[2], "offset=8") {
		t.Fatal("offset did not advance by the fetched page sizes", transport.requests)
	}
}

func TestExplicitLimitWinsOverThePageSizeFallback(t *testing.T) {
	limit, _ := sdk.ParseInteger("2")
	offset, _ := sdk.ParseInteger("0")
	transport := &sizeFake{pages: []string{
		`{"data":[{"id":"a"},{"id":"b"}],"total":4}`,
		`{"data":[],"total":4}`,
	}}
	iterator := sizeClient(transport).ListWidgetsPages(context.Background(), sdk.NewListWidgetsInput().WithLimit(limit).WithOffset(offset))
	for iterator.Next() {
	}
	if err := iterator.Err(); err != nil {
		t.Fatal(err)
	}
	if len(transport.requests) != 2 {
		t.Fatal("walk shape changed", transport.requests)
	}
	for index, request := range transport.requests {
		if !strings.Contains(request, "limit=2") || strings.Contains(request, "limit=5") {
			t.Fatal("request", index, "did not keep the caller's limit", request)
		}
	}
}
"#;

#[test]
fn page_size_fallback_supplies_the_first_page_limit_only() {
    let Some(_) = go_toolchain() else {
        eprintln!(
            "go_pagination: Go toolchain (>= 1.23) not installed; degrading to static assertions"
        );
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&page_size_options()), root.path()).unwrap();
    // Static shape: the first-page fill guards an unset optional limit.
    let pagination = std::fs::read_to_string(root.path().join("go/pagination.go")).unwrap();
    for expected in [
        "if !it.input.Limit.IsSet {",
        "it.input.Limit = OptionalSome(httpPageInteger(5))",
    ] {
        assert!(
            pagination.contains(expected),
            "pagination.go is missing the page-size fallback:\n{expected}"
        );
    }
    std::fs::write(
        root.path().join("go/pagination_page_size_test.go"),
        PAGE_SIZE_BEHAVIOR,
    )
    .unwrap();
    let output = Command::new("go")
        .args(["test", "-count=1", "-timeout=120s", "."])
        .current_dir(root.path().join("go"))
        .env("GOWORK", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native page-size behavior failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
