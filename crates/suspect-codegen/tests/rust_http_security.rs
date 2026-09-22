//! Security and wire-fidelity regressions for the generated Rust HTTP stack,
//! through generated clients over custom transports (generated-package ->
//! native consumer seam). Native tests are opt-in like tests/rust_http.rs:
//! Main runs `cargo test --test rust_http_security -- --ignored`.
//!
//! Server path prefixes are retained; standard authority case/default-port
//! normalization preserves identity. Whitespace and route-changing forms fail.

use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::rust_http::{HttpConfig, HttpPlan, PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = WorkspaceBuilder::new()
        .root(path.parent().unwrap())
        .build()
        .unwrap();
    Arc::new(
        Contract::from_workspace(&Arc::new(workspace), &Uri::from_path(path).unwrap()).unwrap(),
    )
}

fn fixture(value: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    contract(&path)
}

fn plan(value: Value, max_response_bytes: usize) -> HttpPlan {
    let contract = fixture(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_http(
        contract,
        &selected,
        HttpConfig {
            max_response_bytes,
            ..HttpConfig::default()
        },
    )
    .unwrap()
}

fn native(plan: &HttpPlan, source: &str) {
    // Retain failures so the exact unmodified emitted package can be diagnosed.
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.keep();
    let files = emit_http(
        plan,
        &PackageConfig {
            name: "native-http-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &directory).unwrap();
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(consumer.join("Cargo.toml"), "[package]\nname=\"http-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nnative-http-sdk={path=\"../rust\",features=[\"http\"]}\n").unwrap();
    std::fs::write(consumer.join("src/lib.rs"), source).unwrap();
    let toolchain = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join(if toolchain.is_some() {
        "../../target/native-rust-http-security-msrv"
    } else {
        "../../target/native-rust-http-security"
    });
    let mut command = Command::new("cargo");
    command
        .args(["test", "--offline", "--quiet", "--manifest-path"])
        .arg(consumer.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings");
    if let Some(toolchain) = &toolchain {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "native fixture retained at {}\ncommand: {command:?}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn media_api() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"Media","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
            "schemas":{"Credit":{"type":"object","required":["amount"],"properties":{"amount":{"type":"number"}}}}},
        "paths":{"/widgets/{account}":{"get":{"operationId":"getWidget","description":"Read one widget.",
            "parameters":[
                {"name":"account","in":"path","required":true,"schema":{"type":"string","minLength":1}},
                {"name":"tags","in":"query","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
                {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}}],
            "responses":{"200":{"description":"Credit","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Credit"}}}}}}}}
    })
}

fn stream_api() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"Stream","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
            "schemas":{"Credit":{"type":"object","required":["amount"],"properties":{"amount":{"type":"number"}}}}},
        "paths":{"/items/{id}":{"get":{"operationId":"listItems","description":"Read one item.",
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string","minLength":1}}],
            "responses":{"200":{"description":"Credit","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Credit"}}}}}}}}
    })
}

const MEDIA_CONSUMER: &str = r##"
#[cfg(test)]
mod tests {
    use native_http_sdk::http::{BoxError, Request, ResponseBody, SdkErrorKind, Transport, TransportResponse};
    use native_http_sdk::operations::get_widget::{GetWidget, GetWidgetError, GetWidgetSuccess};
    use native_http_sdk::{Client, Credentials, JsonInteger};
    use std::{future::Future, sync::{atomic::{AtomicUsize, Ordering}, Arc}, task::{Context, Poll, Wake, Waker}};

    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake { fn wake(self: Arc<Self>) { self.0.unpark(); } }
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        loop { match future.as_mut().poll(&mut cx) { Poll::Ready(result) => return result, Poll::Pending => std::thread::park() } }
    }
    struct Body(Option<Vec<u8>>);
    impl ResponseBody for Body {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> { Ok(self.0.take()) }
    }

    const EXPECTED_URL: &str = "https://example.test/api/v1/widgets/w%2Fx%20%E9%9B%AA%21%27%28%29%2A?tags=x%2Cy,z%20z&limit=9007199254740993";
    const BODY: &[u8] = br#"{"amount":123}"#;

    struct Strict { calls: Arc<AtomicUsize>, content_type: Vec<(&'static str, &'static str)> }
    impl Transport for Strict {
        type Body = Body;
        async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, EXPECTED_URL);
            assert!(request.body.is_none());
            assert_eq!(request.headers.len(), 2, "bodyless GET carries exactly profile and credential headers");
            assert!(request.headers.iter().any(|(name, value)| name == "accept" && value == b"application/json"));
            assert!(request.headers.iter().any(|(name, value)| name == "authorization" && value == b"Bearer secret"));
            assert!(!format!("{request:?}").contains("secret"), "request debug must not carry credential values");
            let headers = self.content_type.iter()
                .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec())).collect();
            Ok(TransportResponse { status: 200, headers, body: Body(Some(BODY.to_vec())) })
        }
    }
    fn input() -> GetWidget {
        GetWidget::new("w/x 雪!'()*".into())
            .with_tags(vec!["x,y".into(), "z z".into()])
            .with_limit("9007199254740993".parse::<JsonInteger>().unwrap())
    }

    #[test]
    fn declared_media_with_parameters_decodes_exact_bytes() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = Client::with_transport(
            Strict { calls: calls.clone(), content_type: vec![("Content-Type", "Application/JSON; charset=utf-8")] },
            Credentials::api_key("secret"),
        );
        let GetWidgetSuccess::Status200(response) = block_on(client.get_widget(input())).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        assert_eq!(response.data.amount.as_str(), "123");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_and_malformed_content_type_never_matches_a_declaration() {
        let calls = Arc::new(AtomicUsize::new(0));
        for content_type in [
            vec![("Content-Type", "application/json"), ("content-type", "application/json")],
            vec![("Content-Type", "application/json;charset=utf-8;charset=utf-8")],
            vec![("Content-Type", "application/json;charset=\"utf-8")],
            vec![("Content-Type", "application/json garbage")],
        ] {
            let count = content_type.len();
            let client = Client::with_transport(Strict { calls: calls.clone(), content_type }, Credentials::api_key("secret"));
            match block_on(client.get_widget(input())).unwrap_err() {
                GetWidgetError::Sdk(error) => {
                    assert_eq!(error.kind, SdkErrorKind::UnexpectedResponse);
                    assert_eq!(error.status, Some(200));
                    assert_eq!(error.raw_capture, BODY);
                    assert!(!error.truncated);
                    assert_eq!(error.headers.len(), count);
                }
                other => panic!("wrong failure: {other:?}"),
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
}
"##;

const STREAM_CONSUMER: &str = r##"
#[cfg(test)]
mod tests {
    use native_http_sdk::http::{BoxError, Request, ResponseBody, SdkErrorKind, Transport, TransportResponse};
    use native_http_sdk::operations::list_items::{ListItems, ListItemsError, ListItemsSuccess};
    use native_http_sdk::{Client, ClientOptions, Credentials};
    use std::{future::Future, sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex}, task::{Context, Poll, Wake, Waker}};

    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake { fn wake(self: Arc<Self>) { self.0.unpark(); } }
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        loop { match future.as_mut().poll(&mut cx) { Poll::Ready(result) => return result, Poll::Pending => std::thread::park() } }
    }

    /// A `None` chunk means a scripted mid-stream failure.
    struct Counted { chunks: std::vec::IntoIter<Option<Vec<u8>>>, drops: Arc<AtomicUsize> }
    impl Drop for Counted { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); } }
    impl ResponseBody for Counted {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
            match self.chunks.next() {
                None => Ok(None),
                Some(Some(chunk)) => Ok(Some(chunk)),
                Some(None) => Err("scripted mid-stream failure".into()),
            }
        }
    }

    struct Scripted { calls: Arc<AtomicUsize>, status: u16, chunks: Mutex<Vec<Option<Vec<u8>>>>, drops: Arc<AtomicUsize>, last_url: Arc<Mutex<String>> }
    impl Transport for Scripted {
        type Body = Counted;
        async fn send(&self, request: Request) -> Result<TransportResponse<Counted>, BoxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_url.lock().unwrap() = request.url.clone();
            let chunks = std::mem::take(&mut *self.chunks.lock().unwrap());
            Ok(TransportResponse {
                status: self.status,
                headers: vec![("content-type".to_string(), b"application/json".to_vec())],
                body: Counted { chunks: chunks.into_iter(), drops: self.drops.clone() },
            })
        }
    }
    fn capture_options() -> ClientOptions { ClientOptions { max_error_capture_bytes: 8, ..Default::default() } }
    fn client(transport: Scripted) -> Client<Scripted> {
        Client::with_transport(transport, Credentials::api_key("secret")).with_options(capture_options())
    }
    fn scripted(calls: Arc<AtomicUsize>, status: u16, chunks: Vec<Option<Vec<u8>>>, drops: Arc<AtomicUsize>) -> Scripted {
        Scripted { calls, status, chunks: Mutex::new(chunks), drops, last_url: Arc::new(Mutex::new(String::new())) }
    }

    #[test]
    fn streamed_success_drops_body_once_and_decodes_exact_bytes() {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let chunks = vec![Some(br#"{"amo"#.to_vec()), Some(br#"unt":123}"#.to_vec())];
        let transport = client(scripted(calls.clone(), 200, chunks, drops.clone()));
        let ListItemsSuccess::Status200(response) = block_on(transport.list_items(ListItems::new("7".into()))).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.data.amount.as_str(), "123");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn excess_streamed_body_is_resource_limited_with_bounded_capture() {
        // Generated ceiling is 64 bytes; 80 streamed bytes exceed it.
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let chunks = vec![Some(vec![b'a'; 40]), Some(vec![b'b'; 40])];
        let transport = client(scripted(calls.clone(), 200, chunks, drops.clone()));
        match block_on(transport.list_items(ListItems::new("7".into()))).unwrap_err() {
            ListItemsError::Sdk(error) => {
                assert_eq!(error.kind, SdkErrorKind::ResourceLimit);
                assert_eq!(error.status, Some(200));
                assert_eq!(error.raw_capture, b"aaaaaaaa");
                assert!(error.truncated);
                assert!(error.headers.iter().any(|(name, value)| name == "content-type" && value == b"application/json"));
            }
            other => panic!("wrong failure: {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn mid_stream_failure_keeps_status_and_partial_capture() {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let chunks = vec![Some(b"abc".to_vec()), None];
        let transport = client(scripted(calls.clone(), 200, chunks, drops.clone()));
        match block_on(transport.list_items(ListItems::new("7".into()))).unwrap_err() {
            ListItemsError::Sdk(error) => {
                assert_eq!(error.kind, SdkErrorKind::Transport);
                assert_eq!(error.status, Some(200));
                assert_eq!(error.raw_capture, b"abc");
                assert!(error.truncated);
            }
            other => panic!("wrong failure: {other:?}"),
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn caller_cannot_raise_generated_response_ceiling() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = scripted(calls.clone(), 200, vec![], Arc::new(AtomicUsize::new(0)));
        let options = ClientOptions { max_response_bytes: Some(128), ..capture_options() };
        let client = Client::with_transport(transport, Credentials::api_key("secret")).with_options(options);
        assert!(block_on(client.list_items(ListItems::new("7".into()))).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn source_and_override_server_path_prefixes_keep_url_identity() {
        // Default ports and authority case have standard URL equivalence;
        // preserving the path prefix does not require rejecting equivalent URLs.
        for (override_url, expected) in [
            (None, "https://example.test/api/v1/items/7"),
            (Some("HTTPS://EXAMPLE.TEST:443/api/v2"), "https://example.test/api/v2/items/7"),
            (Some("http://127.0.0.1:8081/api/v2"), "http://127.0.0.1:8081/api/v2/items/7"),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let drops = Arc::new(AtomicUsize::new(0));
            let last_url = Arc::new(Mutex::new(String::new()));
            let mut transport = scripted(calls.clone(), 200, vec![Some(br#"{"amount":123}"#.to_vec())], drops.clone());
            transport.last_url = last_url.clone();
            let options = ClientOptions { server_url: override_url.map(|url| url.to_string()), ..capture_options() };
            let client = Client::with_transport(transport, Credentials::api_key("secret")).with_options(options);
            block_on(client.list_items(ListItems::new("7".into()))).unwrap();
            assert_eq!(*last_url.lock().unwrap(), expected);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn malformed_overrides_never_enter_transport() {
        for override_url in [
            "https://user:pw@example.test:443/api/v2",
            "http://example.test/api/v2",
            "https://example.test/api/v2?q=1",
            "https://example.test/api/v2#frag",
            " https://example.test/api/v2",
            "https://example.test/api/v2 ",
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let transport = scripted(calls.clone(), 200, vec![], Arc::new(AtomicUsize::new(0)));
            let options = ClientOptions { server_url: Some(override_url.to_string()), ..capture_options() };
            let client = Client::with_transport(transport, Credentials::api_key("secret")).with_options(options);
            assert!(block_on(client.list_items(ListItems::new("7".into()))).is_err(), "{override_url}");
            assert_eq!(calls.load(Ordering::SeqCst), 0, "{override_url}");
        }
    }
}
"##;

#[test]
#[ignore = "requires native Cargo and the pinned url crate cache"]
fn native_strict_content_type_and_exact_request_witness() {
    native(&plan(media_api(), 8 * 1024 * 1024), MEDIA_CONSUMER);
}

#[test]
#[ignore = "requires native Cargo and the pinned url crate cache"]
fn native_bounded_streaming_drops_and_server_overrides() {
    native(&plan(stream_api(), 64), STREAM_CONSUMER);
}
