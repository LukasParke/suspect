//! Generated-client resource, media, transport failure and cancellation boundaries.

use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::rust_http::{HttpConfig, PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn native(config: HttpConfig, consumer: &str) {
    static NATIVE_CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _cache = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let directory = tempfile::tempdir().unwrap().keep();
    let input = directory.join("api.json");
    std::fs::write(&input, serde_json::json!({
        "openapi":"3.1.0","info":{"title":"Runtime","version":"1"},
        "servers":[{"url":"https://example.test/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
        "paths":{"/value":{"get":{"operationId":"readValue","responses":{
            "200":{"description":"value","content":{"application/json":{"schema":{"type":"number","minimum":1}}}}
        }}}}
    }).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&directory).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, config).unwrap();
    suspect_codegen::write_files(
        &emit_http(
            &plan,
            &PackageConfig {
                name: "runtime-http-fixture".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        &directory,
    )
    .unwrap();
    let root = directory.join("consumer");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"),"[package]\nname=\"runtime-http-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nruntime-http-fixture={path=\"../rust\",features=[\"http\"]}\n").unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        format!("#![cfg(test)]\n{SUPPORT}\n{consumer}"),
    )
    .unwrap();
    let mut command = Command::new("cargo");
    command
        .args(["test", "--offline", "--quiet", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-http-runtime"))
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "fixture {}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

const SUPPORT: &str = r#"
use runtime_http_fixture::{Client, ClientOptions, Credentials};
use runtime_http_fixture::http::{BoxError, Headers, Request, ResponseBody, Transport, TransportResponse, SdkErrorKind};
use runtime_http_fixture::operations::read_value::{ReadValue, ReadValueError};
use std::{collections::VecDeque, future::Future, sync::{Arc, atomic::{AtomicUsize, Ordering}}, task::{Context,Poll,Wake,Waker}};
#[derive(Clone)] enum Step { Bytes(Vec<u8>), Fail, Pending }
struct Body { steps:VecDeque<Step>, drops:Arc<AtomicUsize> }
impl Drop for Body { fn drop(&mut self) { self.drops.fetch_add(1,Ordering::SeqCst); } }
impl ResponseBody for Body {
    async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError> {
        match self.steps.pop_front() {
            Some(Step::Bytes(bytes))=>Ok(Some(bytes)), Some(Step::Fail)=>Err(std::io::Error::other("private transport detail").into()),
            Some(Step::Pending)=>std::future::pending().await, None=>Ok(None),
        }
    }
}
struct Wire { steps:Vec<Step>, headers:Headers, calls:Arc<AtomicUsize>, drops:Arc<AtomicUsize> }
impl Transport for Wire {
    type Body=Body;
    async fn send(&self,request:Request)->Result<TransportResponse<Body>,BoxError> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        assert_eq!(request.url,"https://example.test/v1/value");
        Ok(TransportResponse{status:200,headers:self.headers.clone(),body:Body{steps:self.steps.clone().into(),drops:self.drops.clone()}})
    }
}
fn client(steps:Vec<Step>,headers:Headers)->(Client<Wire>,Arc<AtomicUsize>,Arc<AtomicUsize>) {
    let calls=Arc::new(AtomicUsize::new(0));let drops=Arc::new(AtomicUsize::new(0));
    (Client::with_transport(Wire{steps,headers,calls:calls.clone(),drops:drops.clone()},Credentials::api_key("credential")),calls,drops)
}
fn json_headers()->Headers { vec![("content-type".into(),b"application/json".to_vec())] }
struct WakeThread(std::thread::Thread);
impl Wake for WakeThread { fn wake(self:Arc<Self>) { self.0.unpark(); } }
fn block_on<F:Future>(future:F)->F::Output {
    let mut future=Box::pin(future); let waker=Waker::from(Arc::new(WakeThread(std::thread::current()))); let mut context=Context::from_waker(&waker);
    loop { match future.as_mut().poll(&mut context) { Poll::Ready(value)=>return value,Poll::Pending=>std::thread::park() } }
}
fn sdk(error:ReadValueError)->Box<runtime_http_fixture::http::SdkError> { match error {ReadValueError::Sdk(error)=>error,other=>panic!("{other:?}")} }
"#;

#[test]
#[ignore = "requires native Cargo and the pinned URL dependency"]
fn streamed_failures_honor_capture_limits_metadata_and_drop_ownership() {
    native(
        HttpConfig::default(),
        r#"
#[test] fn overflow_keeps_only_bounded_capture() {
    let (client,calls,drops)=client(vec![Step::Bytes(b"12345".to_vec()),Step::Bytes(b"67890".to_vec())],json_headers());
    let client=client.with_options(ClientOptions{max_response_bytes:Some(8),max_error_capture_bytes:2,..Default::default()});
    let error=sdk(block_on(client.read_value(ReadValue::new())).unwrap_err());
    assert_eq!(error.kind,SdkErrorKind::ResourceLimit); assert_eq!(error.status,Some(200));
    assert_eq!(error.raw_capture,b"12");assert!(error.truncated);assert_eq!(error.headers,json_headers());
    assert_eq!(calls.load(Ordering::SeqCst),1);assert_eq!(drops.load(Ordering::SeqCst),1);
}
#[test] fn stream_cause_is_retained_without_debug_disclosure() {
    let (client,_,drops)=client(vec![Step::Bytes(b"12".to_vec()),Step::Fail],json_headers());
    let error=sdk(block_on(client.read_value(ReadValue::new())).unwrap_err());
    assert_eq!(error.kind,SdkErrorKind::Transport);assert_eq!(error.status,Some(200));assert_eq!(error.raw_capture,b"12");assert!(error.truncated);
    assert!(error.cause.as_ref().unwrap().to_string().contains("private transport detail"));
    assert!(!format!("{error:?}").contains("private transport detail"));assert_eq!(drops.load(Ordering::SeqCst),1);
}
#[test] fn dropping_a_pending_operation_drops_its_body() {
    let (client,_,drops)=client(vec![Step::Pending],json_headers());
    let mut future=Box::pin(client.read_value(ReadValue::new()));
    let waker=Waker::from(Arc::new(WakeThread(std::thread::current())));
    assert!(matches!(future.as_mut().poll(&mut Context::from_waker(&waker)),Poll::Pending));
    drop(future);assert_eq!(drops.load(Ordering::SeqCst),1);
}
#[test] fn malformed_media_is_not_a_validated_response() {
    for headers in [
        vec![("content-type".into(),b"application/json;p=\"bad\\\rvalue\"".to_vec())],
        vec![("content-type".into(),b"application/json".to_vec()),("Content-Type".into(),b"application/json".to_vec())],
        vec![("content-type".into(),b"application/json;p=x;P=y".to_vec())],
    ] {
        let (client,_,drops)=client(vec![Step::Bytes(b"1".to_vec())],headers);
        let error=sdk(block_on(client.read_value(ReadValue::new())).unwrap_err());
        assert_eq!(error.kind,SdkErrorKind::UnexpectedResponse);assert_eq!(error.raw_capture,b"1");assert_eq!(drops.load(Ordering::SeqCst),1);
    }
}
"#,
    );
}

#[test]
#[ignore = "requires native Cargo and the pinned URL dependency"]
fn incomplete_response_codec_work_remains_a_resource_failure() {
    for layer in ["evaluation", "conversion", "json"] {
        let mut config = HttpConfig::default();
        match layer {
            "evaluation" => config.codecs.schema.max_evaluation_steps = 0,
            "conversion" => config.codecs.max_conversion_steps = 0,
            _ => config.codecs.json_limits.max_input_bytes = 0,
        }
        native(
            config,
            r#"
#[test] fn incomplete_is_not_schema_invalid() {
    let _=(Step::Fail,Step::Pending);
    let _=ClientOptions::default();
    let (client,calls,drops)=client(vec![Step::Bytes(b"1".to_vec())],json_headers());
    let error=sdk(block_on(client.read_value(ReadValue::new())).unwrap_err());
    assert_eq!(error.kind,SdkErrorKind::ResourceLimit);assert_eq!(error.status,Some(200));
    assert_eq!(error.raw_capture,b"1");assert!(error.cause.is_some());
    assert_eq!(calls.load(Ordering::SeqCst),1);assert_eq!(drops.load(Ordering::SeqCst),1);
}
"#,
        );
    }
}
