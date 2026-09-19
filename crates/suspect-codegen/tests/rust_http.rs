//! OpenAPI -> native Rust HTTP package, consumer, wire behavior and Rustdoc.

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

fn api() -> Value {
    json!({
        "openapi":"3.1.0", "info":{"title":"Native HTTP", "version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}], "security":[{"apiKey":[]}],
        "components":{
            "securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
            "schemas":{
                "Credit":{"type":"object","required":["amount"],"properties":{"amount":{"type":"number"}}},
                "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
            }
        },
        "paths":{"/credits/{account}":{"get":{
            "operationId":"getCredits", "description":"Read exact credits.",
            "parameters":[
                {"name":"account","in":"path","required":true,"schema":{"type":"string","minLength":1}},
                {"name":"tags","in":"query","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
                {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}}
            ],
            "responses":{
                "200":{"description":"Credit","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Credit"}}}},
                "401":{"description":"Denied","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Failure"}}}}
            }
        }}}
    })
}

fn plan(value: Value) -> HttpPlan {
    let contract = fixture(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_http(contract, &selected, HttpConfig::default()).unwrap()
}

fn checked(command: &mut Command, fixture: &Path) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "native fixture retained at {}\ncommand: {command:?}\n{}{}",
        fixture.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn native(plan: &HttpPlan, source: &str) {
    // Cargo releases its build lock while Rustdoc invokes doctest compilers.
    // Keep same-named fixture packages from evicting each other's rlibs then.
    static NATIVE_CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _cache = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
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
        "../../target/native-rust-http-msrv"
    } else {
        "../../target/native-rust-http"
    });
    let generated = directory.join("rust");
    for (mode, package, extra) in [
        ("test", consumer.as_path(), vec![]),
        (
            "test",
            generated.as_path(),
            vec!["--doc", "--features", "http"],
        ),
        (
            "doc",
            generated.as_path(),
            vec!["--no-deps", "--features", "http"],
        ),
    ] {
        let mut command = Command::new("cargo");
        command
            .args([mode, "--offline", "--quiet", "--manifest-path"])
            .arg(package.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .args(extra)
            .env_remove("RUST_MIN_STACK")
            .env("RUSTFLAGS", "-D warnings")
            .env("RUSTDOCFLAGS", "-D warnings");
        if let Some(toolchain) = &toolchain {
            command.env("RUSTUP_TOOLCHAIN", toolchain);
        }
        checked(&mut command, &directory);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires native Cargo and the pinned url crate cache"]
fn native_client_preserves_exact_source_wire_values_and_typed_failures() {
    let plan = plan(api());
    native(
        &plan,
        r##"
#[cfg(test)]
mod tests {
    use native_http_sdk::{Client, ClientOptions, Credentials, JsonInteger};
    use native_http_sdk::http::{BoxError, Request, ResponseBody, Transport, TransportResponse};
    use native_http_sdk::operations::get_credits::{GetCredits, GetCreditsSuccess, GetCreditsError, GetCreditsApiError};
    use std::{future::Future, sync::{Arc, atomic::{AtomicUsize, Ordering}}, task::{Context, Poll, Wake, Waker}};
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake { fn wake(self: Arc<Self>) { self.0.unpark(); } }
    fn block_on<F:Future>(future:F) -> F::Output {
        let mut future=std::pin::pin!(future);
        let waker=Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut cx=Context::from_waker(&waker);
        loop { match future.as_mut().poll(&mut cx) { Poll::Ready(result)=>return result, Poll::Pending=>std::thread::park() } }
    }
    struct Body(Option<Vec<u8>>);
    impl ResponseBody for Body {
        async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError> { Ok(self.0.take()) }
    }
    struct Recording { calls:Arc<AtomicUsize>, status:u16, body:&'static str }
    impl Transport for Recording {
        type Body=Body;
        async fn send(&self, request:Request)->Result<TransportResponse<Body>,BoxError> {
            self.calls.fetch_add(1,Ordering::SeqCst);
            assert_eq!(request.method,"GET");
            assert_eq!(request.url,"https://example.test/api/v1/credits/a%2Fb%20%E9%9B%AA%21%27%28%29%2A?tags=x%2Cy,z%20z&limit=9007199254740993");
            assert!(request.body.is_none());
            assert!(request.headers.iter().any(|(name,value)|name.eq_ignore_ascii_case("authorization") && value==b"Bearer secret"));
            assert!(request.headers.iter().any(|(name,value)|name.eq_ignore_ascii_case("accept") && value==b"application/json"));
            Ok(TransportResponse{status:self.status,headers:vec![("Content-Type".into(),b"Application/JSON; charset=utf-8".to_vec())],body:Body(Some(self.body.as_bytes().to_vec()))})
        }
    }
    fn input()->GetCredits {
        GetCredits::new("a/b 雪!'()*".into()).with_tags(vec!["x,y".into(),"z z".into()]).with_limit("9007199254740993".parse::<JsonInteger>().unwrap())
    }
    #[test] fn exact_success_and_error() {
        let calls=Arc::new(AtomicUsize::new(0));
        let client=Client::with_transport(Recording{calls:calls.clone(),status:200,body:r#"{"amount":9007199254740993.000000000000000001}"#},Credentials::api_key("secret"));
        let GetCreditsSuccess::Status200(response)=block_on(client.get_credits(input())).unwrap();
        assert_eq!(response.data.amount.as_str(),"9007199254740993.000000000000000001");
        assert_eq!(response.status,200);
        let client=Client::with_transport(Recording{calls:calls.clone(),status:401,body:r#"{"message":"denied"}"#},Credentials::api_key("secret"));
        match block_on(client.get_credits(input())).unwrap_err() {
            GetCreditsError::Api(error)=>match *error { GetCreditsApiError::Status401(response)=>assert_eq!(response.data.message,"denied") },
            other=>panic!("wrong failure: {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst),2);
    }
    #[test] fn invalid_inputs_never_enter_transport() {
        use native_http_sdk::http::SdkErrorKind;
        let calls=Arc::new(AtomicUsize::new(0));
        let client=Client::with_transport(Recording{calls:calls.clone(),status:200,body:r#"{"amount":1}"#},Credentials::api_key("secret"));
        for (request,kind) in [
            (GetCredits::new(String::new()),SdkErrorKind::RequestValidation),
            (input().with_limit("0".parse().unwrap()),SdkErrorKind::RequestValidation),
            (GetCredits::new("..".into()),SdkErrorKind::RequestRepresentation),
        ] {
            match block_on(client.get_credits(request)).unwrap_err() {
                GetCreditsError::Sdk(error)=>{assert_eq!(error.kind,kind);assert_eq!(error.operation_source.pointer,"/paths/~1credits~1{account}/get");},
                other=>panic!("wrong failure: {other:?}"),
            }
            assert_eq!(calls.load(Ordering::SeqCst),0);
        }
        let client=Client::with_transport(Recording{calls:calls.clone(),status:200,body:"1"},Credentials::api_key("secret\r\nInjected: value"));
        let error=block_on(client.get_credits(input())).unwrap_err();
        assert!(!format!("{error:?}").contains("Injected"));
        assert!(matches!(error,GetCreditsError::Sdk(error) if error.kind==SdkErrorKind::RequestValidation));
        assert_eq!(calls.load(Ordering::SeqCst),0);
    }
    #[test] fn malformed_declared_errors_and_unknown_statuses_keep_metadata() {
        use native_http_sdk::http::SdkErrorKind;
        let calls=Arc::new(AtomicUsize::new(0));
        for (status,body,kind) in [
            (401,r#"{"message":123}"#,SdkErrorKind::ResponseDecoding),
            (200,r#"{"amount":"not a number"}"#,SdkErrorKind::ResponseDecoding),
            (418,"undeclared body",SdkErrorKind::UnexpectedResponse),
        ] {
            let client=Client::with_transport(Recording{calls:calls.clone(),status,body},Credentials::api_key("secret"))
                .with_options(ClientOptions{max_error_capture_bytes:5,..Default::default()});
            match block_on(client.get_credits(input())).unwrap_err() {
                GetCreditsError::Sdk(error)=>{
                    assert_eq!(error.kind,kind);assert_eq!(error.status,Some(status));
                    assert_eq!(error.raw_capture,body.as_bytes()[..5]);assert!(error.truncated);
                    assert_eq!(error.headers[0].1,b"Application/JSON; charset=utf-8");
                },
                other=>panic!("wrong failure: {other:?}"),
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst),3);
    }
}
"##,
    );
}

#[test]
fn unsupported_rust_http_semantics_retain_source_diagnostics() {
    let mut cases = Vec::new();
    let mut directional = api();
    directional["components"]["schemas"]["Credit"]["properties"]["amount"]["readOnly"] =
        json!(true);
    cases.push((
        directional,
        "http-directional-codec-unsupported",
        "/components/schemas/Credit/properties/amount/readOnly",
    ));
    let mut nullable_query = api();
    nullable_query["paths"]["/credits/{account}"]["get"]["parameters"][2]["schema"]["type"] =
        json!(["integer", "null"]);
    cases.push((
        nullable_query,
        "http-parameter-shape-unsupported",
        "/paths/~1credits~1{account}/get/parameters/2/schema",
    ));
    for (value, code, pointer) in cases {
        let contract = fixture(value);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let errors = plan_http(contract, &selected, HttpConfig::default()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == code
                && error.source.pointer() == pointer
                && !error.at.is_empty()),
            "{errors:?}"
        );
    }
}

#[test]
fn malformed_parameter_collections_cannot_disappear_from_rust_protocol_admission() {
    for parameters in [json!(42), json!(null), json!({}), json!([null])] {
        let code = if parameters.is_array() {
            "http-metadata-object"
        } else {
            "http-parameters-invalid"
        };
        let mut spec = api();
        let mut operation = spec["paths"]["/credits/{account}"]["get"].clone();
        operation["parameters"] = parameters;
        spec["paths"] = json!({"/credits":{"get":operation}});
        let contract = fixture(spec);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let rust = plan_http(contract.clone(), &selected, HttpConfig::default()).unwrap_err();
        assert!(
            rust.iter().any(|d| d.code == code
                && d.source
                    .pointer()
                    .starts_with("/paths/~1credits/get/parameters")),
            "{rust:?}"
        );
    }
}

#[test]
#[ignore = "requires native Cargo and the pinned url crate cache"]
fn native_operation_names_do_not_shadow_runtime_types_or_documentation_imports() {
    let mut spec = api();
    let operation = spec["paths"]["/credits/{account}"]["get"].clone();
    spec["paths"] = json!({});
    for name in [
        "From",
        "Vec",
        "Result",
        "Transport",
        "drop",
        "withTransport",
        "limits",
        "Option",
        "Some",
    ] {
        let mut op = operation.clone();
        op["operationId"] = json!(name);
        op["description"] = json!(
            "Keep generated_models:: tokens in source prose. ```rust\npanic!(\"not executable\");\n``` <script>untrusted</script> [not a symbol]"
        );
        spec["paths"][format!("/{name}/{{account}}")] = json!({"get":op});
    }
    native(
        &plan(spec),
        r#"
/// Required operation parameters cannot be omitted.
/// ```compile_fail
/// native_http_sdk::operations::from::From::new();
/// ```
pub struct RequiredInput;

#[test]
fn constructors_use_native_names_without_import_collisions() {
    use native_http_sdk::operations;
    let _ = operations::from::From::new("account".into());
    let _ = operations::vec::Vec::new("account".into());
    let _ = operations::result::Result::new("account".into());
    let _ = operations::transport::Transport::new("account".into());
    let _ = operations::option::Option::new("account".into());
    let _ = operations::some::Some::new("account".into());
}
"#,
    );
}
