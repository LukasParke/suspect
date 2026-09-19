//! Independent rich-protocol Dart package, wire, framing and cleanup gates.
#![cfg(all(feature = "dart-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use suspect_codegen::dart_sdk::{self, DartConfig};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn repository() -> PathBuf {
    std::env::var_os("SUSPECT_DART_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
        })
}
fn document() -> Value {
    let reply = json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"}},"additionalProperties":false});
    let json_reply = json!({"description":"JSON reply","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}});
    let mut value = json!({"openapi":"3.2.0","info":{"title":"Dart rich protocol","version":"1"},
    "servers":[{"url":"https://first.test/base"},{"name":"relative","url":"../{version}","variables":{"version":{"default":"v2","enum":["v2","v3"]}}}],
    "security":[],"components":{"schemas":{"Reply":reply,"Request":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}},"additionalProperties":false},
        "Event":{"type":"object","required":["data"],"properties":{"data":{"type":"string"},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer"}},"additionalProperties":false},
        "Row":{"type":"object","required":["count"],"properties":{"count":{"type":"integer"}},"additionalProperties":false}},
        "securitySchemes":{"bearer":{"type":"http","scheme":"bearer"},"basic":{"type":"http","scheme":"basic"},
            "headerKey":{"type":"apiKey","in":"header","name":"X-Key"},"queryKey":{"type":"apiKey","in":"query","name":"access"},"cookieKey":{"type":"apiKey","in":"cookie","name":"token"},
            "oauth":{"type":"oauth2","flows":{"authorizationCode":{"authorizationUrl":"https://auth.test/authorize","tokenUrl":"https://auth.test/token","scopes":{"read":"Read data"}}}},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.test/.well-known/openid-configuration"}}},
    "paths":{
        "/echo/{id}":{"post":{"operationId":"echo","security":[{"basic":[],"headerKey":[]},{"bearer":[]},{}],
            "parameters":[{"name":"id","in":"path","required":true,"style":"matrix","schema":{"type":"string"}},
                {"name":"labels","in":"query","schema":{"type":"array","items":{"type":"string"}}},
                {"name":"filter","in":"query","style":"deepObject","explode":true,"schema":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"string"}},"additionalProperties":false}},
                {"name":"allow","in":"query","allowReserved":true,"schema":{"type":"string"}},
                {"name":"X-Flags","in":"header","schema":{"type":"array","items":{"type":"boolean"}}},
                {"name":"session","in":"cookie","style":"cookie","schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Request"},"example":{"message":"hello"}},"text/plain":{"schema":{"type":"string"},"example":"hello"},"application/octet-stream":{"schema":{"maxLength":64}}}},
            "responses":{"201":{"description":"Specific","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}},"X-Meta":{"style":"simple","explode":true,"schema":{"type":"object","properties":{"level":{"type":"integer"},"who":{"type":"string"}},"additionalProperties":false}},"X-Json":{"content":{"application/json":{"schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}},"additionalProperties":false}}}}},
                "links":{"other":{"operationId":"other","parameters":{"id":"$response.body#/message"}}},
                "content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}},"text/plain":{"schema":{"type":"string"}},"application/octet-stream":{}}},
                "2XX":{"description":"Binary range","content":{"application/*":{}}},"default":json_reply}}},
        "/other":{"get":{"operationId":"other","responses":{"200":json_reply}}},
        "/query-key":{"get":{"operationId":"queryKeyCall","security":[{"queryKey":[]}],"responses":{"200":json_reply}}},
        "/cookie-key":{"get":{"operationId":"cookieKeyCall","security":[{"cookieKey":[]}],"responses":{"200":json_reply}}},
        "/oauth":{"get":{"operationId":"oauthCall","security":[{"oauth":["read"]}],"responses":{"200":json_reply}}},
        "/oidc":{"get":{"operationId":"oidcCall","security":[{"oidc":[]}],"responses":{"200":json_reply}}},
        "/head":{"head":{"operationId":"headCall","responses":{"204":{"description":"No content","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}}}}}},
        "/undocumented":{"get":{"operationId":"undocumented","responses":{"200":{"description":"Unspecified payload"}}}},
        "/free":{"get":{"operationId":"freeJson","responses":{"200":{"description":"Schema free","content":{"application/json":{}}}}}},
        "/search":{"get":{"operationId":"search","parameters":[{"name":"query","in":"querystring","required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["q"],"properties":{"q":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}}}}],"responses":{"200":json_reply}}},
        "/form":{"post":{"operationId":"submitForm","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name"],"minProperties":1,"maxProperties":3,"properties":{"name":{"type":"string"},"tags":{"type":"array","items":{"type":"string"},"maxItems":3},"options":{"type":"object","properties":{"enabled":{"type":"boolean"}},"additionalProperties":false}},"additionalProperties":false}}}},"responses":{"200":json_reply}}},
        "/multipart":{"post":{"operationId":"upload","requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["file","title"],"properties":{"file":{"maxLength":16},"title":{"type":"string"},"meta":{"$ref":"#/components/schemas/Request"}},"additionalProperties":false},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer"}}}},"meta":{"contentType":"application/json"}}}}},"responses":{"200":json_reply}}},
        "/events":{"get":{"operationId":"events","responses":{"200":{"description":"Event envelope","content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}},"400":json_reply}}},
        "/rows":{"get":{"operationId":"rows","responses":{"200":{"description":"Exact rows","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Row"}}}},"400":json_reply}}},
        "/custom":{"query":{"operationId":"queryMethod","responses":{"200":json_reply}},"additionalOperations":{"PURGE":{"operationId":"purge","responses":{"200":json_reply}}}}
    }});
    value["paths"]["/echo/{id}"]["post"]["responses"]["default"]["content"]["application/json"]["schema"] =
        json!({"$ref":"#/components/schemas/Reply"});
    value["paths"]["/styles/{label}/{matrix}"] = json!({"get":{"operationId":"styles","parameters":[
        {"name":"label","in":"path","required":true,"style":"label","explode":true,"schema":{"type":"array","items":{"type":"string"}}},
        {"name":"matrix","in":"path","required":true,"style":"matrix","explode":true,"schema":{"type":"object","properties":{"role":{"type":"string"},"first":{"type":"string"}},"additionalProperties":false}},
        {"name":"space","in":"query","style":"spaceDelimited","explode":false,"schema":{"type":"array","items":{"type":"integer"}}},
        {"name":"pipe","in":"query","style":"pipeDelimited","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
        {"name":"object","in":"query","style":"form","explode":false,"schema":{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"boolean"}},"additionalProperties":false}}
    ],"responses":{"200":json_reply}}});
    value["paths"]["/content"] = json!({"get":{"operationId":"contentQuery","parameters":[
        {"name":"options","in":"query","required":true,"content":{"application/problem+json":{"schema":{"$ref":"#/components/schemas/Request"}}}},
        {"name":"X-On","in":"header","content":{"text/plain":{"schema":{"type":"boolean"}}}}
    ],"responses":{"200":{"description":"Header content","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}},"headers":{"X-Number":{"required":true,"content":{"text/plain":{"schema":{"type":"integer"}}}}}}}}});
    value["paths"]["/negotiate"] = json!({"get":{"operationId":"negotiate","responses":{"200":{"description":"Specific representations","content":{
        "application/*":{},"text/*":{},"application/problem+json":{"schema":{"$ref":"#/components/schemas/Reply"}},
        "application/problem+json; profile=full":{"schema":{"type":"object","required":["message"],"properties":{"message":{"type":"string","const":"full"}},"additionalProperties":false}}
    }}}}});
    value["paths"]["/custom"]["additionalOperations"]["get"] =
        json!({"operationId":"lowerGet","responses":{"200":json_reply}});
    value["paths"]["/form"]["post"]["requestBody"]["content"]["application/x-www-form-urlencoded"]
        ["encoding"] = json!({"tags":{"style":"form","explode":false}});
    value
}
fn load(path: &Path) -> Arc<Contract> {
    let w = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&w, &Uri::from_path(path).unwrap()).unwrap())
}
fn root(name: &str) -> PathBuf {
    let p = std::env::var_os("SUSPECT_DART_GATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-expanded-native"));
    std::fs::create_dir_all(&p).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(p)
        .unwrap()
        .keep()
}
fn check(cmd: &mut Command, root: &Path, name: &str) {
    let out = cmd.output().unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    std::fs::write(
        root.join(format!("logs/{name}.log")),
        format!(
            "{cmd:?}\nstatus={}\n{}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
    .unwrap();
    assert!(
        out.status.success(),
        "{} {cmd:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn dart(root: &Path) -> Command {
    let binary = std::env::var_os("SUSPECT_DART_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-tools/dart-sdk/bin/dart"));
    let mut c = Command::new(binary);
    c.current_dir(root)
        .env("HOME", repository().join("target/sdk-dart-tools/home"))
        .env("PUB_CACHE", root.join("pub-cache"))
        .env("CI", "true")
        .env("DART_SUPPRESS_ANALYTICS", "true");
    c
}
#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    records: Arc<Mutex<Vec<Request>>>,
}
impl Server {
    fn start(
        handler: impl Fn(std::net::TcpStream, &Request, &str) + Send + Sync + 'static,
    ) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(Mutex::new(Vec::new()));
        let (stopped, seen, base, handler) = (
            stop.clone(),
            records.clone(),
            url.clone(),
            Arc::new(handler),
        );
        let thread = std::thread::spawn(move || {
            let mut children = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let (seen, base, handler) = (seen.clone(), base.clone(), handler.clone());
                        children.push(std::thread::spawn(move || {
                            if let Some(request) = read_request(&mut stream) {
                                seen.lock().unwrap().push(request.clone());
                                handler(stream, &request, &base);
                            }
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            url,
            stop,
            records,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let end = loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
        if bytes.len() > 65536 {
            return None;
        }
        if let Some(n) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break n + 4;
        }
    };
    let header = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let mut lines = header.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.into();
    let path = first.next()?.into();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        .collect::<Vec<_>>();
    let size = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .map_or(Some(0), |(_, v)| v.parse::<usize>().ok())?;
    if size > 8 * 1024 * 1024 {
        return None;
    }
    while bytes.len() < end + size {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Some(Request {
        method,
        path,
        headers,
        body: bytes[end..end + size].to_vec(),
    })
}
fn reply(
    mut stream: std::net::TcpStream,
    status: u16,
    media: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) {
    let mut text = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    text.push_str("\r\n");
    let _ = stream.write_all(text.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
fn install(root: &Path, plan: &dart_sdk::Plan) {
    suspect_codegen::write_files(&plan.render(), root).unwrap();
    let archive = root.join("package.tar.gz");
    check(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(root.join("dart"))
            .arg("."),
        root,
        "archive",
    );
    let bytes = std::fs::read(archive).unwrap();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let hosted = Server::start(move |stream, request, url| {
        if request.path == "/api/packages/generated_sdk" {
            let version = json!({"version":"0.0.0","pubspec":{"name":"generated_sdk","version":"0.0.0","environment":{"sdk":">=3.9.4 <4.0.0"}},"archive_url":format!("{url}/archive"),"archive_sha256":sha,"published":"2026-09-10T00:00:00Z"});
            reply(
                stream,
                200,
                "application/json",
                json!({"name":"generated_sdk","latest":version,"versions":[version]})
                    .to_string()
                    .as_bytes(),
                &[],
            );
        } else if request.path == "/archive" {
            reply(stream, 200, "application/octet-stream", &bytes, &[]);
        } else {
            reply(
                stream,
                200,
                "application/json",
                br#"{"advisories":[]}"#,
                &[],
            );
        }
    });
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("bin")).unwrap();
    std::fs::write(consumer.join("pubspec.yaml"),format!("name: dart_protocol_consumer\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\ndependencies:\n  generated_sdk:\n    hosted: {}\n    version: 0.0.0\n",hosted.url)).unwrap();
    std::fs::copy(
        root.join("dart/analysis_options.yaml"),
        consumer.join("analysis_options.yaml"),
    )
    .unwrap();
    check(
        dart(root).args(["pub", "get"]).current_dir(&consumer),
        root,
        "consumer-install",
    );
    drop(hosted);
    check(
        dart(root)
            .args(["pub", "get", "--offline"])
            .current_dir(&consumer),
        root,
        "consumer-offline",
    );
    check(
        dart(root)
            .args(["pub", "get", "--offline"])
            .current_dir(root.join("dart")),
        root,
        "package-pub",
    );
    check(
        dart(root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(root.join("dart")),
        root,
        "package-analyze",
    );
}

#[test]
fn typed_protocol_admission_retains_real_inputs_and_declines_undefined_profiles() {
    let root = root("admission-");
    let path = root.join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk(contract, &selected, DartConfig::default()).unwrap();
    assert!(plan.protocol().is_admitted());
    assert!(plan.operations().iter().any(|o| o.stream));
    assert!(
        plan.operations()
            .iter()
            .any(|o| o.wire.method().as_str() == "PURGE")
    );
    let roots = plan.protocol().codec_roots();
    assert!(
        !roots
            .iter()
            .any(|id| id.pointer().contains("/multipart/") && id.pointer().ends_with("/schema"))
    );
    assert!(roots.iter().any(|id| id.pointer().ends_with("/itemSchema")));
}

#[test]
#[ignore = "native pub, strict typing, wire/framing/cleanup gate on Dart floor/current and portable JS"]
fn native_rich_protocol() {
    let root = root("protocol-");
    let path = root.join("api.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk(
        contract,
        &selected,
        DartConfig {
            max_capture_bytes: 64,
            max_stream_item_bytes: 1024,
            max_stream_buffer_bytes: 4096,
            ..Default::default()
        },
    )
    .unwrap();
    install(&root, &plan);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/portable.dart"),
        include_str!("../src/dart_sdk/native_protocol.dart"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("bin/main.dart"),
        include_str!("../src/dart_sdk/native_protocol_io.dart"),
    )
    .unwrap();
    check(
        dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "consumer-analyze",
    );
    check(
        dart(&root)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root.join("consumer-vm"))
            .current_dir(&consumer),
        &root,
        "consumer-compile-vm",
    );
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let exact_calls = std::sync::atomic::AtomicUsize::new(0);
    let marker = root.clone();
    let server = Server::start(move |mut stream, request, _| {
        let path = request.path.split('?').next().unwrap();
        if path == "/base/custom" && request.method == "get" {
            match exact_calls.fetch_add(1, Ordering::SeqCst) {
                0 => reply(stream, 200, "application/json", br#"{"message":"ok"}"#, &[]),
                1 => {
                    let body = br#"{"message":"chunked"}"#;
                    stream.write_all(format!("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x};fixture=1\r\n",body.len()).as_bytes()).unwrap();
                    stream.write_all(body).unwrap();
                    stream
                        .write_all(b"\r\n0\r\nX-Trailer: retained-as-unmodeled\r\n\r\n")
                        .unwrap();
                }
                _ => {
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                        .unwrap();
                    if matches!(stream.read(&mut [0u8; 1]), Ok(0)) {
                        std::fs::write(marker.join("exact-peer-closed"), b"EOF").unwrap();
                    }
                }
            }
            return;
        }
        if path == "/base/events" && calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let body = b"data: cancel me\n\n";
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n",body.len()).as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            if matches!(stream.read(&mut [0u8; 1]), Ok(0)) {
                std::fs::write(marker.join("event-peer-closed"), b"EOF").unwrap();
            }
        } else if path == "/base/events" {
            reply(
                stream,
                200,
                "text/event-stream",
                b"data: first\n\ndata: [DONE]\n\n",
                &[],
            );
        } else if path == "/base/rows" {
            reply(
                stream,
                200,
                "application/x-ndjson",
                b"{\"count\":9007199254740993}\n{\"count\":1e+000008}\n",
                &[],
            );
        } else if path == "/base/head" {
            reply(stream, 204, "application/json", b"", &[("X-Count", "2")]);
        } else if path == "/base/undocumented" {
            reply(stream, 200, "image/jpeg", &[0, 255, 1], &[]);
        } else if path.starts_with("/base/echo/") {
            if path.ends_with("bytes") {
                reply(stream, 202, "application/octet-stream", &[0, 255, 4], &[]);
            } else if path.ends_with("text") {
                reply(stream, 201, "text/plain", b"reply", &[("X-Count", "1.0")]);
            } else {
                reply(
                    stream,
                    201,
                    "application/json",
                    br#"{"message":"ok"}"#,
                    &[("X-Count", "1.0")],
                );
            }
        } else {
            reply(stream, 200, "application/json", br#"{"message":"ok"}"#, &[]);
        }
    });
    check(
        Command::new(root.join("consumer-vm"))
            .env("SUSPECT_DART_HTTP_BASE", format!("{}/base", server.url))
            .env("SUSPECT_DART_HTTP_MARKERS", &root),
        &root,
        "consumer-run-vm",
    );
    {
        let records = server.records.lock().unwrap();
        assert_eq!(records.len(), 18, "native exchanges");
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/base/echo/;id=a%2Fb%20%E9%9B%AA");
        assert_eq!(records[0].body, br#"{"message":"hello"}"#);
        assert!(
            records[0]
                .headers
                .iter()
                .any(|(k, v)| k == "authorization" && v == "Basic dTpw")
        );
        assert!(
            records[0]
                .headers
                .iter()
                .any(|(k, v)| k == "x-key" && v == "key")
        );
        assert_eq!(records[1].body, "hello 雪".as_bytes());
        assert_eq!(records[2].body, [0, 255, 4]);
        assert_eq!(records[3].path, "/base/query-key?access=q%2Fk");
        assert!(
            records[4]
                .headers
                .iter()
                .any(|(k, v)| k == "cookie" && v == "token=cookie")
        );
        assert_eq!(records[5].method, "HEAD");
        assert_eq!(records[7].body, b"name=form+value&tags=x&tags=y");
        assert!(records[8].body.windows(4).any(|v| v == [0, 255, 13, 10]));
        assert!(String::from_utf8_lossy(&records[8].body).contains("x-part: 1.0\r\n"));
        assert_eq!(records[9].path, "/base/search?q=a+b&tags=x&tags=y");
        assert_eq!(records[10].method, "QUERY");
        assert_eq!(records[11].method, "PURGE");
        assert_eq!(records[12].method, "get");
        assert_eq!(records[13].method, "get");
        assert_eq!(records[14].method, "get");
        std::fs::write(root.join("wire-records.txt"), format!("{records:#?}")).unwrap();
    }
    check(
        dart(&root)
            .args(["compile", "js", "bin/portable.dart", "-o"])
            .arg(root.join("portable.js"))
            .current_dir(&consumer),
        &root,
        "consumer-compile-js",
    );
    check(
        Command::new("node")
            .args([
                "-e",
                "globalThis.self=globalThis; require(process.argv[1]);",
            ])
            .arg(root.join("portable.js")),
        &root,
        "consumer-run-js",
    );
    for example in ["source_examples", "quickstart"] {
        check(
            dart(&root)
                .args(["compile", "exe"])
                .arg(format!("example/{example}.dart"))
                .arg("-o")
                .arg(root.join(example))
                .current_dir(root.join("dart")),
            &root,
            &format!("compile-{example}"),
        );
        if example == "source_examples" {
            check(
                &mut Command::new(root.join(example)),
                &root,
                "run-source-examples",
            );
        }
    }
    check(
        dart(&root)
            .args(["doc", "--validate-links", "--output"])
            .arg(root.join("dartdoc"))
            .current_dir(root.join("dart")),
        &root,
        "dartdoc",
    );
    for (i, source) in [
        "void main(){ EchoBodyBytes('not bytes'); }",
        "void main(){ Credentials(oauth: 'not a callback'); }",
        "void main(){ UploadBodyFields(title: 'missing bytes'); }",
        "void main(){ UploadBodyFieldsFileHeaders(xPart: 1.5); }",
        "void use(Client client){ client.events().then((value){}); }",
    ]
    .iter()
    .enumerate()
    {
        let path = consumer.join(format!("negative-{i}.dart"));
        std::fs::write(
            &path,
            format!("import 'package:generated_sdk/generated_sdk.dart';\n{source}\n"),
        )
        .unwrap();
        let result = dart(&root)
            .args(["analyze", "--fatal-infos"])
            .arg(&path)
            .current_dir(&consumer)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        std::fs::write(root.join(format!("logs/negative-{i}.log")), &text).unwrap();
        assert!(
            !result.status.success() && !text.contains("uri_does_not_exist"),
            "negative type failure: {text}"
        );
    }
    println!("DART_PROTOCOL_GATE_ROOT={}", root.display());
}

#[test]
#[ignore = "native sparse selections must analyze without invented JSON schemas or unused runtime branches"]
fn native_sparse_selections() {
    for (label, method, operation) in [
        (
            "no-body",
            "get",
            json!({"operationId":"empty","responses":{"204":{"description":"No content"}}}),
        ),
        (
            "bytes",
            "post",
            json!({"operationId":"bytes","requestBody":{"required":true,"content":{"application/octet-stream":{}}},"responses":{"200":{"description":"Bytes","content":{"application/octet-stream":{}}}}}),
        ),
        (
            "json",
            "get",
            json!({"operationId":"json","responses":{"200":{"description":"JSON","content":{"application/json":{"schema":{"type":"boolean"}}}}}}),
        ),
    ] {
        let root = root(&format!("sparse-{label}-"));
        let path = root.join("api.json");
        std::fs::write(&path,json!({"openapi":"3.2.0","info":{"title":"Sparse native selection","version":"1"},"servers":[{"url":"https://example.test"}],"security":[],"paths":{"/value":{method:operation}}}).to_string()).unwrap();
        let contract = load(&path);
        let selected = contract
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let plan = dart_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
        if label != "json" {
            assert!(plan.protocol().codec_roots().is_empty());
        }
        install(&root, &plan);
        check(
            dart(&root)
                .args(["compile", "exe", "example/source_examples.dart", "-o"])
                .arg(root.join("source-examples"))
                .current_dir(root.join("dart")),
            &root,
            "source-example-compile",
        );
        check(
            &mut Command::new(root.join("source-examples")),
            &root,
            "source-example-run",
        );
        println!("DART_SPARSE_GATE_ROOT={}", root.display());
    }
}

#[test]
#[ignore = "explicit compatibility option with installed native bytes and JSON-context witnesses"]
fn native_explicit_legacy_binary_profile() {
    use suspect_codegen::{
        backend::{self, Backend, GenerationOptions, TargetConfig},
        compatibility::{self, PlanStatus},
        http_protocol::CompatibilityProfile,
    };
    let root = root("legacy-profile-");
    let path = root.join("api.json");
    let document = json!({"openapi":"3.2.0","info":{"title":"Explicit legacy marker","version":"1"},"servers":[{"url":"https://example.test"}],"security":[],"paths":{
        "/bytes":{"post":{"operationId":"legacyBytes","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"Raw bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}},
        "/json":{"post":{"operationId":"jsonString","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"Still JSON","content":{"application/json":{"schema":{"type":"string","format":"binary"}}}}}}}
    }});
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let refused = dart_sdk::plan_sdk(contract.clone(), &selected, Default::default()).unwrap_err();
    assert!(
        refused
            .iter()
            .any(|d| d.code == "http-binary-legacy-marker")
    );
    let mut options = GenerationOptions::default();
    options
        .compatibility_profiles
        .insert(CompatibilityProfile::LegacyBinaryStringV1);
    let plan = dart_sdk::plan_sdk_with_profiles(
        contract.clone(),
        &selected,
        Default::default(),
        &options.compatibility_profiles,
    )
    .unwrap();
    let binary = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "legacyBytes")
        .unwrap();
    let json = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "jsonString")
        .unwrap();
    assert_eq!(binary.body.as_ref().unwrap().native_type, "Uint8List");
    assert_eq!(json.body.as_ref().unwrap().native_type, "String");
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .iter()
            .any(|id| id.pointer().contains("~1bytes"))
    );
    let target = TargetConfig {
        backend: Backend::DartHttp,
        package_name: "generated_sdk".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    };
    assert_eq!(
        backend::generate_with_options(contract.clone(), &selected, &target, &options).unwrap(),
        plan.render()
    );
    let snapshot =
        compatibility::snapshot_with_options(contract, &[], &[target], &options).unwrap();
    assert_eq!(snapshot.native[0].status, PlanStatus::Planned);
    assert_eq!(snapshot.native[0].generation, options);
    install(&root, &plan);
    let consumer = root.join("consumer");
    std::fs::write(
        consumer.join("bin/main.dart"),
        include_str!("../src/dart_sdk/native_legacy.dart"),
    )
    .unwrap();
    check(
        dart(&root)
            .args(["analyze", "--fatal-infos"])
            .current_dir(&consumer),
        &root,
        "legacy-analyze",
    );
    check(
        dart(&root)
            .args(["compile", "exe", "bin/main.dart", "-o"])
            .arg(root.join("legacy-vm"))
            .current_dir(&consumer),
        &root,
        "legacy-compile-vm",
    );
    check(
        &mut Command::new(root.join("legacy-vm")),
        &root,
        "legacy-run-vm",
    );
    check(
        dart(&root)
            .args(["compile", "js", "bin/main.dart", "-o"])
            .arg(root.join("legacy.js"))
            .current_dir(&consumer),
        &root,
        "legacy-compile-js",
    );
    check(
        Command::new("node")
            .args(["-e", "globalThis.self=globalThis;require(process.argv[1]);"])
            .arg(root.join("legacy.js")),
        &root,
        "legacy-run-js",
    );
    println!("DART_LEGACY_PROFILE_GATE_ROOT={}", root.display());
}
