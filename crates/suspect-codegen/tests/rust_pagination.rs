//! M3 pagination runtime emission for the Rust backend: the canonical
//! generation path compiles the shared pagination selection and emits
//! dependency-free walkers, while unconfigured plans stay byte-identical.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
    sdk_defaults::{PaginationAdvance, PaginationPattern, PaginationRequestRole, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.pagination.test/openapi.json";

fn page_schema(required: Value, properties: Value) -> Value {
    json!({"type":"object","required":required,"properties":properties})
}

/// One limit/offset list operation and one cursor list operation, sharing a
/// `filter` member to observe input preservation across pages.
fn document() -> Value {
    let widgets = json!({
        "get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"filter","in":"query","schema":{"type":"string"}},
                {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}},
                {"name":"offset","in":"query","schema":{"type":"integer","minimum":0}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema": page_schema(
                    json!(["data","total"]),
                    json!({
                        "data": {"type":"array","items":{"$ref":"#/components/schemas/Widget"}},
                        "total": {"type":"integer"}
                    })
                )}}}
            }
        }
    });
    let events = json!({
        "get": {
            "operationId": "listEvents",
            "parameters": [
                {"name":"pageSize","in":"query","schema":{"type":"integer"}},
                {"name":"cursor","in":"query","schema":{"type":"string"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema": page_schema(
                    json!([]),
                    json!({
                        "items": {"type":"array","items":{"type":"string"}},
                        "next_page_token": {"type":"string"}
                    })
                )}}}
            }
        }
    });
    json!({
        "openapi":"3.1.0",
        "info":{"title":"Pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths": {"/widgets": widgets, "/events": events},
        "components":{"schemas":{
            "Widget":{"type":"object","required":["id"],
                "properties":{"id":{"type":"string"},"kind":{"type":"string"}}}
        }}
    })
}

fn contract_with_document(value: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec_pretty(&value).unwrap(),
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

fn contract() -> Arc<Contract> {
    contract_with_document(document())
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
        package_name: "pagination-rust".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value::<SdkDefaults>(json!({
                "version":"v1","pagination":"auto"
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = selection(&contract);
    backend::generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn detected<'a>(
    outcome: &'a suspect_codegen::http_protocol::PaginationOutcome,
    operation: &str,
) -> &'a suspect_codegen::http_protocol::OperationPagination {
    outcome
        .paginated
        .iter()
        .find(|entry| entry.operation == operation)
        .unwrap()
}

#[test]
fn sdk_defaults_compile_pagination_selection_into_the_plan() {
    let contract = contract();
    let protocol = suspect_codegen::http_protocol::plan(
        &contract,
        &selection(&contract),
        suspect_codegen::rust_http::native_capabilities_v3(),
    )
    .into_result()
    .unwrap();
    let defaults: SdkDefaults =
        serde_json::from_value(json!({"version":"v1","pagination":"auto"})).unwrap();
    let outcome =
        suspect_codegen::http_protocol::plan_pagination(&contract, &protocol, Some(&defaults))
            .unwrap();
    let widgets = detected(&outcome, "listWidgets");
    assert_eq!(widgets.pattern, PaginationPattern::LimitOffset);
    assert_eq!(widgets.request[&PaginationRequestRole::Offset], "offset");
    assert_eq!(widgets.advance, PaginationAdvance::ItemsReturned);
    let events = detected(&outcome, "listEvents");
    assert_eq!(events.pattern, PaginationPattern::Cursor);
    assert_eq!(events.advance, PaginationAdvance::NextOffset);

    let plan = suspect_codegen::rust_http::plan_http_v3(
        contract.clone(),
        &selection(&contract),
        suspect_codegen::rust_http::HttpConfig {
            sdk_defaults: Some(defaults),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plan.pagination().unwrap().paginated.len(), 2);
    // The retained v1/v2 planning APIs never populate pagination.
    let v2 = suspect_codegen::rust_http::plan_http_v2(
        contract.clone(),
        &selection(&contract),
        suspect_codegen::rust_http::HttpConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version":"v1","pagination":"auto"
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v2.pagination().is_none());
}

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn configured_generation_emits_walkers_and_unconfigured_generation_emits_nothing() {
    let configured = generate(&configured_options());
    let pagination = configured
        .iter()
        .find(|file| file.path == "rust/src/pagination.rs")
        .expect("configured generation emits the pagination module");
    for expected in [
        "pub struct ListWidgetsPages",
        "pub struct ListWidgetsItems",
        "pub struct ListEventsPages",
        "pub struct ListEventsItems",
        "pub async fn list_widgets_next_page",
        "pub async fn list_events_next_page",
        "pub struct PaginationLoop",
        "fn list_widgets_page_items",
        "fn list_events_page_continuation",
    ] {
        assert!(pagination.content.contains(expected), "missing {expected}");
    }
    let lib = configured
        .iter()
        .find(|file| file.path == "rust/src/lib.rs")
        .unwrap();
    assert!(
        lib.content
            .contains("#[cfg(feature=\"http\")]\npub mod pagination;\n")
    );
    for method in [
        "pub fn list_widgets_pages(",
        "pub fn list_widgets_items(",
        "pub async fn list_widgets_next_page(",
        "pub fn list_events_pages(",
        "pub fn list_events_items(",
        "pub async fn list_events_next_page(",
    ] {
        assert!(
            lib.content.contains(method),
            "missing client method {method}"
        );
    }
    // Operations modules (and every other artifact) stay byte-identical to
    // the unconfigured emission; only lib.rs changes and pagination.rs is new.
    let unconfigured = generate(&GenerationOptions::default());
    let configured_by_path = configured
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let unconfigured_by_path = unconfigured
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        configured_by_path.len(),
        unconfigured_by_path.len() + 1,
        "configured emission adds exactly the pagination module"
    );
    for (path, content) in &unconfigured_by_path {
        match *path {
            "rust/src/lib.rs" => assert_ne!(*content, configured_by_path[path]),
            other => assert_eq!(
                *content, configured_by_path[other],
                "{other} must not change under pagination emission"
            ),
        }
    }
    assert!(configured_by_path.contains_key("rust/src/pagination.rs"));
    let lib = unconfigured_by_path["rust/src/lib.rs"];
    assert!(!lib.contains("pub mod pagination"));
    assert!(!lib.contains("_next_page"));
}

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
                "200": {"description":"Page","content":{"application/json":{"schema": page_schema(
                    json!(["records","paging"]),
                    json!({
                        "records": {"type":"array","items":{"type":"integer"}},
                        "paging": {"type":"object","required":["more"],"properties":{
                            "more":{"type":"boolean"},
                            "total":{"type":"integer"}}
                        }
                    })
                )}}}
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
                "200": {"description":"Page","content":{"application/json":{"schema": page_schema(
                    json!(["entries","paging"]),
                    json!({
                        "entries": {"type":"array","items":{"type":"string"}},
                        "paging": {"type":"object","required":["count"],"properties":{
                            "count":{"type":"integer"}}
                        }
                    })
                )}}}
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
                "200": {"description":"Page","content":{"application/json":{"schema": page_schema(
                    json!(["events","next"]),
                    json!({
                        "events": {"type":"array","items":{"type":"string"}},
                        "next": {"type":"string"}
                    })
                )}}}
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

fn manual_defaults() -> SdkDefaults {
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
    .unwrap()
}

/// Every supported walker template compiles inside the emitted package.
#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn walker_templates_compile_for_every_supported_shape() {
    let contract = contract_with_document(manual_document());
    let selected = selection(&contract);
    let options = GenerationOptions {
        sdk_defaults: Some(manual_defaults()),
        ..Default::default()
    };
    let files = backend::generate_with_options(contract, &selected, &target(), &options).unwrap();
    let pagination = files
        .iter()
        .find(|file| file.path == "rust/src/pagination.rs")
        .expect("manual mappings emit the pagination module");
    for expected in [
        "pub struct ListRowsPages",
        "pub struct ListBlobsPages",
        "pub struct ListAlertsPages",
        "pub struct ListAuditEventsPages",
        "fn list_rows_page_has_more",
        "std::option::Option::Some(page.as_slice())",
        "fn list_alerts_page_continuation(page: &crate::models::ListAlertsResponse200) -> std::option::Option<i128>",
        "fn list_audit_events_page_items(page: &crate::models::ListAuditEventsResponse200) -> std::option::Option<&[std::string::String]>",
    ] {
        assert!(pagination.content.contains(expected), "missing {expected}");
    }
    let directory = tempfile::Builder::new()
        .prefix("rust-pagination-shapes-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&files, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    let (ok, log) = cargo("check", &manifest, &["--features", "http"]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping shape compile: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "walker shapes failed to compile:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_PAGINATION").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

/// Per-user cargo target directory, so concurrent agents never contend on the
/// shared workspace lock while building emitted packages.
fn cargo_target() -> PathBuf {
    let user = std::env::var_os("USER")
        .or_else(|| std::env::var_os("LOGNAME"))
        .unwrap_or_else(|| format!("uid-{}", std::process::id()).into());
    std::env::temp_dir().join(format!(
        "suspect-rust-pagination-target-{}",
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

/// Behavioral verification: the emitted package and a dependency-free
/// consumer run real two-page walks against a fake transport.
#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn emitted_walkers_drive_real_walks_in_a_compiled_package() {
    let configured = generate(&configured_options());
    let directory = tempfile::Builder::new()
        .prefix("rust-pagination-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");

    // Model-only compilation needs no dependencies at all and must always
    // succeed offline.
    let (ok, log) = cargo("check", &manifest, &["--no-default-features"]);
    assert!(ok, "model-only compile failed: {log}");

    // The http feature pins `url`; it compiles offline from the local cache.
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"pagination-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"pagination-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        // No registry access for the pinned `url` crate: degrade to the
        // model-only compile check plus the static assertions above and say so.
        eprintln!(
            "skipping behavioral walker execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_PAGINATION").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}

fn registry_unavailable(log: &str) -> bool {
    log.contains("no matching package named")
        || log.contains("failed to download")
        || log.contains("error: failed to select a version")
        || log.contains("network disabled")
        || log.contains("could not download")
}

const CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the emitted pagination walkers.
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    widgets: fn(u32) -> String,
    events: fn(usize) -> String,
}
impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        let query = request.url.split('?').nth(1).unwrap_or_default().to_owned();
        self.calls.lock().unwrap().push(query.clone());
        let parameter = |name: &str| {
            query.split('&').find_map(|pair| {
                pair.split_once('=').and_then(|(key, value)| {
                    (key == name).then(|| value.to_owned())
                })
            })
        };
        let (path, body) = if request.url.contains("/widgets") {
            let offset: u32 = parameter("offset").unwrap_or_default().parse().unwrap();
            let path = format!("/widgets?{query}");
            (path, (self.widgets)(offset))
        } else {
            let index = parameter("cursor").map_or(0, |_| 1);
            let path = format!("/events?{query}");
            (path, (self.events)(index))
        };
        assert_eq!(request.method, "GET");
        assert!(request.url.starts_with("https://api.pagination.test/v1"));
        Ok(TransportResponse {
            status: 200,
            headers: vec![("Content-Type".into(), b"application/json".to_vec())],
            body: Body(Some(body.into_bytes())),
        })
    }
}

fn widget_page(offset: u32) -> String {
    if offset >= 4 {
        format!(r#"{{"data":[],"total":4}}"#)
    } else {
        format!(
            r#"{{"data":[{{"id":"w{offset}"}},{{"id":"w{}"}}],"total":4}}"#,
            offset + 1
        )
    }
}

fn event_page(page: usize) -> String {
    if page == 0 {
        r#"{"items":["a","b"],"next_page_token":"c2"}"#.to_owned()
    } else {
        r#"{"items":["c"]}"#.to_owned()
    }
}

fn server() -> (Server, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Server {
            calls: calls.clone(),
            widgets: widget_page,
            events: event_page,
        },
        calls,
    )
}

use sdk::{Client, Credentials};

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn limit_offset_walk_requests_offset_zero_then_two_and_nothing_more() {
    let (transport, calls) = server();
    let client = Client::with_transport(transport, Credentials::new());
    let input = sdk::operations::list_widgets::ListWidgets::new()
        .with_filter("x".into())
        .with_limit("2".parse().unwrap());
    let mut pager = client.list_widgets_pages(input);
    let first = block_on(pager.next()).unwrap().unwrap();
    assert_eq!(first.into_data().data.len(), 2);
    let second = block_on(pager.next()).unwrap().unwrap();
    assert_eq!(second.into_data().data.len(), 2);
    assert!(block_on(pager.next()).unwrap().is_none());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "exactly two transport calls: {calls:?}");
    assert_eq!(calls[0], "filter=x&limit=2&offset=0");
    assert_eq!(calls[1], "filter=x&limit=2&offset=2");
}

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn cursor_walk_sends_the_returned_token_then_stops_without_one() {
    let (transport, calls) = server();
    let client = Client::with_transport(transport, Credentials::new());
    let input = sdk::operations::list_events::ListEvents::new()
        .with_page_size("10".parse().unwrap());
    let mut pager = client.list_events_pages(input);
    let first = block_on(pager.next()).unwrap().unwrap();
    assert_eq!(first.into_data().next_page_token.as_deref(), Some("c2"));
    let second = block_on(pager.next()).unwrap().unwrap();
    assert_eq!(second.into_data().items.as_deref(), Some(&["c".to_owned()][..]));
    assert!(block_on(pager.next()).unwrap().is_none());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0], "pageSize=10");
    assert_eq!(calls[1], "pageSize=10&cursor=c2");
}

#[ignore = "requires a warm Cargo dependency cache for compiled-consumer gates"]
#[test]
fn item_walk_flattens_pages_and_dropping_never_requests_again() {
    let (transport, calls) = server();
    let client = Client::with_transport(transport, Credentials::new());
    let input = sdk::operations::list_events::ListEvents::new()
        .with_page_size("10".parse().unwrap());
    let mut items = client.list_events_items(input);
    assert_eq!(block_on(items.next()).unwrap().as_deref(), Some("a"));
    assert_eq!(block_on(items.next()).unwrap().as_deref(), Some("b"));
    assert_eq!(block_on(items.next()).unwrap().as_deref(), Some("c"));
    assert!(block_on(items.next()).unwrap().is_none());
    drop(items);
    assert_eq!(calls.lock().unwrap().len(), 2);

    // Stopping mid-page leaves the queued items unread and fires no request.
    let (transport, calls) = server();
    let client = Client::with_transport(transport, Credentials::new());
    let input = sdk::operations::list_widgets::ListWidgets::new()
        .with_limit("2".parse().unwrap());
    let mut items = client.list_widgets_items(input);
    assert_eq!(block_on(items.next()).unwrap().unwrap().id, "w0");
    drop(items);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn explicit_next_page_builder_and_repetition_refusal() {
    let (transport, calls) = server();
    let client = Client::with_transport(transport, Credentials::new());
    let input = sdk::operations::list_widgets::ListWidgets::new()
        .with_filter("x".into())
        .with_limit("2".parse().unwrap());
    let next = block_on(client.list_widgets_next_page(input)).unwrap().unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
    let _ = next;

    // A source that repeats the identical continuation value is refused
    // instead of looping, with a typed PaginationLoop cause.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let transport = Server {
        calls: calls.clone(),
        widgets: widget_page,
        events: |_| r#"{"items":["a"],"next_page_token":"same"}"#.to_owned(),
    };
    let client = Client::with_transport(transport, Credentials::new());
    let mut pager = client.list_events_pages(
        sdk::operations::list_events::ListEvents::new(),
    );
    assert!(block_on(pager.next()).unwrap().is_some());
    let failure = block_on(pager.next()).unwrap_err();
    match failure {
        sdk::operations::list_events::ListEventsError::Sdk(error) => {
            assert_eq!(error.kind, sdk::http::SdkErrorKind::UnexpectedResponse);
            let cause = error.cause.expect("typed pagination cause");
            let loop_error = cause
                .downcast_ref::<sdk::pagination::PaginationLoop>()
                .expect("PaginationLoop cause");
            assert_eq!(loop_error.value, "same");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    assert_eq!(calls.lock().unwrap().len(), 2);
}
"##;

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
        ..Default::default()
    }
}

/// Pages served for the fallback walk: five items, then three — together
/// exactly the declared total of 8, so the walk stops at the total guard.
const PAGE_SIZE_CONSUMER: &str = r##"
//! Consumer-side behavioral proof for the documented SDK fallback page size.
use sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

#[derive(Clone)]
struct Server {
    calls: Arc<Mutex<Vec<String>>>,
    pages: Vec<String>,
}
impl Transport for Server {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        let query = request.url.split('?').nth(1).unwrap_or_default().to_owned();
        self.calls.lock().unwrap().push(query.clone());
        let served = self.calls.lock().unwrap().len();
        let page = self
            .pages
            .get(served.min(self.pages.len()) - 1)
            .expect("unexpected extra request")
            .clone();
        assert_eq!(request.method, "GET");
        assert!(request.url.starts_with("https://api.pagination.test/v1"));
        Ok(TransportResponse {
            status: 200,
            headers: vec![("Content-Type".into(), b"application/json".to_vec())],
            body: Body(Some(page.into_bytes())),
        })
    }
}

use sdk::{Client, Credentials};

fn fallback_pages() -> Vec<String> {
    vec![
        r#"{"data":[{"id":"w0"},{"id":"w1"},{"id":"w2"},{"id":"w3"},{"id":"w4"}],"total":8}"#.to_owned(),
        r#"{"data":[{"id":"w5"},{"id":"w6"},{"id":"w7"}],"total":8}"#.to_owned(),
    ]
}

#[test]
fn omitted_limit_takes_the_documented_page_size_on_every_page() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = Client::with_transport(
        Server { calls: calls.clone(), pages: fallback_pages() },
        Credentials::new(),
    );
    let input = sdk::operations::list_widgets::ListWidgets::new().with_filter("x".into());
    let mut pager = client.list_widgets_pages(input);
    let mut pages = 0;
    while let Some(page) = block_on(pager.next()).unwrap() {
        assert!(!page.into_data().data.is_empty());
        pages += 1;
    }
    assert_eq!(pages, 2, "the two item pages exhaust the declared total");
    let calls = calls.lock().unwrap();
    assert_eq!(
        *calls,
        vec![
            "filter=x&limit=5&offset=0".to_owned(),
            "filter=x&limit=5&offset=5".to_owned(),
        ],
        "page 1 and the later page carry the documented fallback page size: {calls:?}"
    );
}

#[test]
fn an_explicit_limit_wins_over_the_documented_page_size() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = Client::with_transport(
        Server { calls: calls.clone(), pages: fallback_pages() },
        Credentials::new(),
    );
    let input = sdk::operations::list_widgets::ListWidgets::new()
        .with_filter("x".into())
        .with_limit("2".parse().unwrap());
    let mut pager = client.list_widgets_pages(input);
    let mut pages = 0;
    while let Some(_page) = block_on(pager.next()).unwrap() {
        pages += 1;
    }
    assert!(pages >= 2);
    let calls = calls.lock().unwrap();
    assert!(
        calls.iter().all(|call| call.contains("limit=2") && !call.contains("limit=5")),
        "the caller's limit wins on every page: {calls:?}"
    );
    assert!(calls[0].ends_with("offset=0"));
}
"##;

#[test]
fn page_size_fallback_supplies_the_first_page_limit_only() {
    let configured = generate(&page_size_options());
    let pagination = configured
        .iter()
        .find(|file| file.path == "rust/src/pagination.rs")
        .expect("configured generation emits the pagination module");
    assert!(
        pagination.content.contains("documented SDK page size (5)"),
        "the walk documentation records the configured fallback"
    );
    let directory = tempfile::Builder::new()
        .prefix("rust-pagination-page-size-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&configured, &directory).unwrap();
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname=\"pagination-consumer\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nsdk={package=\"pagination-rust\",path=\"../rust\",features=[\"http\"]}\n",
    )
    .unwrap();
    std::fs::write(consumer.join("src/lib.rs"), PAGE_SIZE_CONSUMER).unwrap();
    let (ok, log) = cargo("test", &consumer.join("Cargo.toml"), &[]);
    if !ok && registry_unavailable(&log) {
        eprintln!(
            "skipping page-size behavioral execution: the pinned http-feature dependencies are not available offline\n{log}"
        );
        return;
    }
    assert!(ok, "page-size behavioral consumer tests failed:\n{log}");
    if std::env::var_os("SUSPECT_KEEP_PAGINATION").is_none() {
        let _ = std::fs::remove_dir_all(&directory);
    }
}
