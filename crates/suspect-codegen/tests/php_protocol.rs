//! Native evidence for PHP's rich protocol adapter. Emitted bytes are used unchanged.
#![cfg(all(feature = "php-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use suspect_codegen::{
    http_protocol::{Capabilities, Capability},
    php_sdk::{self, PhpConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn repo() -> PathBuf {
    std::env::var_os("SUSPECT_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
        })
}

#[derive(Debug, serde::Serialize)]
struct Request {
    method: String,
    target: String,
    headers: std::collections::BTreeMap<String, String>,
    body: Vec<u8>,
}
struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<Request>>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start(streaming: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(Mutex::new(Vec::new()));
        let done = stop.clone();
        let recorded = records.clone();
        let thread = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let request = read_request(&mut socket);
                        let path = request.target.clone();
                        let method = request.method.clone();
                        let input = request.body.clone();
                        recorded.lock().unwrap().push(request);
                        let (status, kind, body, extra) = if path.ends_with("/oauth") {
                            (204, "application/octet-stream", Vec::new(), "")
                        } else if path.ends_with("/head") {
                            (200, "application/octet-stream", Vec::new(), "")
                        } else if path.contains("/text?") {
                            (200, "text/plain", input, "")
                        } else if path.ends_with("/bytes") {
                            (200, "application/octet-stream", input, "")
                        } else if path.ends_with("/push-lines") || path.ends_with("/push-events") {
                            (200, "text/plain", b"ok".to_vec(), "")
                        } else if path.ends_with("/events") {
                            (
                                200,
                                "text/event-stream",
                                b"data: first\n\ndata: second\n\n".to_vec(),
                                "",
                            )
                        } else {
                            (
                                200,
                                "application/problem+json; profile=fixture",
                                b"{\"id\":\"recorded\"}".to_vec(),
                                "X-Count: 1e3\r\n",
                            )
                        };
                        socket
                            .set_read_timeout(Some(Duration::from_millis(700)))
                            .unwrap();
                        let head = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Connection: keep-alive\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes());
                        if method != "HEAD" {
                            if streaming && path.ends_with("/events") {
                                for chunk in body.chunks(1) {
                                    if socket.write_all(chunk).is_err() {
                                        break;
                                    }
                                    thread::sleep(Duration::from_millis(2));
                                }
                            } else {
                                let _ = socket.write_all(&body);
                            }
                        }
                        let mut one = [0];
                        match socket.read(&mut one) {
                            Ok(0) => {}
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::ConnectionReset
                                        | std::io::ErrorKind::ConnectionAborted
                                ) => {}
                            other => panic!("native HTTP body/connection leaked: {other:?}"),
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("listener: {e}"),
                }
            }
        });
        Self {
            url,
            stop,
            records,
            thread: Some(thread),
        }
    }
    fn finish(mut self) -> Vec<Request> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        std::mem::take(&mut *self.records.lock().unwrap())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
fn read_request(socket: &mut TcpStream) -> Request {
    let mut data = Vec::new();
    let mut one = [0];
    while !data.ends_with(b"\r\n\r\n") {
        socket.read_exact(&mut one).unwrap();
        data.push(one[0]);
        assert!(data.len() < 65536);
    }
    let text = String::from_utf8(data).unwrap();
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap().split(' ').collect::<Vec<_>>();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().into()))
        .collect::<std::collections::BTreeMap<String, String>>();
    let count = headers
        .get("content-length")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(0);
    assert!(count < 8 * 1024 * 1024);
    let mut body = vec![0; count];
    socket.read_exact(&mut body).unwrap();
    Request {
        method: first[0].into(),
        target: first[1].into(),
        headers,
        body,
    }
}

#[test]
#[ignore = "requires native PHP/Composer/PHPStan and independent loopback sockets"]
fn native_curl_protocol_bytes_auth_headers_and_stream_lifetime() {
    let mut value = source();
    value["paths"]["/events"] = json!({"get":{"operationId":"events","security":[],"responses":{"200":{"description":"Events","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}}}}}}}}});
    let (root, plan) = package(&value, capabilities().with(Capability::ServerSentEvents));
    let server = Server::start(true);
    let parameter = plan
        .operations()
        .iter()
        .find(|o| o.id == "readItems")
        .unwrap()
        .parameters
        .iter()
        .find(|p| p.wire.name() == "filter")
        .unwrap();
    let script = CURL_CONSUMER.replace("__BASE__", &server.url).replace(
        "__FILTER__",
        &plan.models().type_name(&parameter.schema, false),
    );
    consumer(&root, &script, "native-curl");
    let requests = server.finish();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests[0].target,
        "/api/v2/items/.1.2?filter%5Bname%5D=a%2Fb"
    );
    assert_eq!(requests[0].headers["authorization"], "Bearer bearer-key");
    assert_eq!(requests[0].headers["x-labels"], "x,y");
    assert_eq!(requests[0].headers["cookie"], "pref=dark");
    assert_eq!(requests[1].target, "/api/v2/text?key=a%2Fb");
    assert_eq!(requests[1].body, b"hello");
    assert_eq!(requests[1].headers["x-api-key"], "header-key");
    assert_eq!(requests[1].headers["cookie"], "session=cookie-key");
    assert_eq!(requests[2].body, b"\0\xff");
    assert_eq!(
        requests[2].headers["authorization"],
        "Basic dXNlcjpwYXNzOnNlY3JldA=="
    );
    assert_eq!(requests[4].method, "HEAD");
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&requests).unwrap(),
    )
    .unwrap();
}
const CURL_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('native cURL mismatch');}}
$credentials=new S\Credentials(['token'=>'bearer-key','basic'=>new S\BasicCredential('user','pass:secret'),'head'=>new S\ApiKeyCredential('header-key'),'query'=>new S\ApiKeyCredential('a/b'),'cookie'=>new S\ApiKeyCredential('cookie-key'),'oauth'=>static fn(S\CredentialRequest $r):S\AuthorizationCredential=>new S\AuthorizationCredential('Bearer oauth-key')]);
$client=new S\Client($credentials,new S\CurlTransport(),new S\ClientOptions(serverBaseUrl:'__BASE__/base/file',serverVariables:['version'=>'v2']));
$result=$client->readItems(new S\ReadItemsInput(ids:[S\JsonNumber::fromInt(1),S\JsonNumber::fromInt(2)],filter:new S\__FILTER__(name:'a/b'),xLabels:['x','y'],pref:'dark'),new S\RequestOptions(securityAlternative:1));
if(!$result instanceof S\ReadItemsStatus200){throw new RuntimeException('wrong native variant');}check($result->body->id==='recorded');check($result->headers->xCount->toInt()===1000);
check($client->sendText(new S\SendTextInput(body:'hello'))->body==='hello');
check($client->sendBytes(new S\SendBytesInput(body:new S\Bytes("\x00\xff")))->body->value==="\x00\xff");
$client->useOauth();$client->readHead();
$response=$client->events();try{foreach($response->body as$event){check($event->data==='first');break;}}finally{$response->body->close();}
echo 'independent cURL protocol and closable stream passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires native PHP; explicit versioned binary interpretation"]
fn explicit_legacy_binary_profile_preserves_native_bytes() {
    use suspect_codegen::backend::GenerationOptions;
    use suspect_codegen::http_protocol::CompatibilityProfile;
    let mut value = source();
    value["paths"] = json!({"/bytes":{"post":{"operationId":"legacyBytes","security":[],"requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"Bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}});
    let temporary = repo().join("target/sdk-php-protocol/legacy-source");
    fs::create_dir_all(&temporary).unwrap();
    fs::write(temporary.join("api.json"), value.to_string()).unwrap();
    let contract = load(&temporary.join("api.json"));
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert!(
        php_sdk::protocol::plan_sdk(
            contract.clone(),
            &selected,
            PhpConfig::default(),
            capabilities()
        )
        .is_err()
    );
    let options = GenerationOptions {
        compatibility_profiles: [CompatibilityProfile::LegacyBinaryStringV1]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let (root, _) = package(&value, options.apply_to(capabilities()));
    consumer(&root, LEGACY_CONSUMER, "native-legacy-profile");
}
const LEGACY_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
$transport=new class implements S\Transport {
    public function send(S\HttpRequest $request):S\HttpResponse {
        if($request->body!=="\x00\xffnative"){throw new RuntimeException('legacy marker became JSON');}
        return new S\HttpResponse(200,['Content-Type'=>'application/octet-stream'],$request->body);
    }
};
$client=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
$response=$client->legacyBytes(new S\LegacyBytesInput(body:new S\Bytes("\x00\xffnative")));
if($response->body->value!=="\x00\xffnative"){throw new RuntimeException('response byte corruption');}
echo 'explicit LegacyBinaryStringV1 native byte witness passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires native PHP/Composer/PHPStan; parameter/media/status matrix"]
fn native_parameter_and_response_dispatch_matrix() {
    let mut value = source();
    value["paths"] = json!({
        "/matrix/{point}":{"get":{"operationId":"matrix","security":[],"parameters":[
            {"name":"point","in":"path","required":true,"style":"matrix","explode":true,"schema":{"type":"object","required":["x","y"],"properties":{"x":{"type":"integer"},"y":{"type":"integer"}},"additionalProperties":false}},
            {"name":"csv","in":"query","style":"form","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
            {"name":"space","in":"query","style":"spaceDelimited","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
            {"name":"pipe","in":"query","style":"pipeDelimited","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
            {"name":"reserved","in":"query","allowReserved":true,"schema":{"type":"string"}},
            {"name":"document","in":"query","content":{"application/json":{"schema":{"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":false}}}}
        ],"responses":{"200":{"description":"Specific","content":{"application/json":{"schema":{"type":"string"}},"application/json; profile=exact":{"schema":{"type":"integer"}},"application/*":{},"*/*":{}}},"2XX":{"description":"Range","content":{"text/plain":{}}},"default":{"description":"Default","content":{"application/octet-stream":{}}}}}},
        "/default":{"get":{"operationId":"fallback","security":[],"responses":{"default":{"description":"Either status class","content":{"text/plain":{"schema":{"type":"string"}}}}}}},
        "/whole":{"query":{"operationId":"whole","security":[],"parameters":[{"name":"raw","in":"querystring","required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","properties":{"q":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}}}}],"responses":{"200":{"description":"No content declaration"}}}},
        "/custom":{"additionalOperations":{"PURGE":{"operationId":"purge","security":[],"responses":{"200":{"description":"No content declaration"}}},"custom-Verb":{"operationId":"customVerb","security":[],"responses":{"200":{"description":"Custom method"}}}}},
        "/undocumented":{"get":{"operationId":"undocumented","security":[]}}
    });
    for (method, id) in [
        ("options", "verbOptions"),
        ("trace", "verbTrace"),
        ("delete", "verbDelete"),
        ("patch", "verbPatch"),
    ] {
        value["paths"][format!("/method-{method}")] = json!({});
        value["paths"][format!("/method-{method}")][method] = json!({"operationId":id,"security":[],"responses":{"200":{"description":"Opaque response bytes"}}});
    }
    let caps = capabilities()
        .with(Capability::CustomMethods)
        .with(Capability::QuerystringParameters)
        .with(Capability::QuerystringForm)
        .with(Capability::FormBodies);
    let (root, plan) = package(&value, caps);
    let matrix = plan.operations().iter().find(|o| o.id == "matrix").unwrap();
    let ty = |wire: &str| {
        plan.models().type_name(
            &matrix
                .parameters
                .iter()
                .find(|p| p.wire.name() == wire)
                .unwrap()
                .schema,
            false,
        )
    };
    let whole = plan.operations().iter().find(|o| o.id == "whole").unwrap();
    let mut text = MATRIX_CONSUMER
        .replace("__POINT__", &ty("point"))
        .replace("__DOCUMENT__", &ty("document"))
        .replace(
            "__WHOLE__",
            &plan.models().type_name(&whole.parameters[0].schema, false),
        );
    for (marker, media) in [
        ("__JSON__", "application/json"),
        ("__APPLICATION__", "application/*"),
        ("__ANY__", "*/*"),
        ("__PARAMETERIZED__", "application/json; profile=exact"),
    ] {
        let response = matrix
            .responses
            .iter()
            .find(|r| {
                r.media
                    .as_ref()
                    .is_some_and(|m| m.wire.media_type().declared() == media)
            })
            .unwrap();
        text = text.replace(marker, &response.name);
    }
    let server = Server::start(false);
    text = text.replace("__BASE__", &server.url);
    consumer(&root, &text, "native-dispatch");
    let requests = server.finish();
    assert_eq!(requests.len(), 8);
    assert_eq!(
        requests[0].target,
        "/matrix/;x=1;y=2?csv=a%2Cb,c&space=a%20b&pipe=c%7Cd&reserved=a/b%26x&document=%7B%22a%22%3A%22x%22%7D"
    );
    assert_eq!(requests[1].target, "/whole?q=a+b%2B&tags=x&tags=y");
    assert_eq!(
        requests
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        [
            "GET",
            "QUERY",
            "PURGE",
            "custom-Verb",
            "OPTIONS",
            "TRACE",
            "DELETE",
            "PATCH"
        ]
    );
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&requests).unwrap(),
    )
    .unwrap();
}
const MATRIX_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('matrix mismatch');}}
$transport=new class implements S\Transport {
    public int $status=200;public ?string $media='application/json';public string $body='"json"';
    /** @var list<S\HttpRequest> */public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse{$this->requests[]=$request;return new S\HttpResponse($this->status,$this->media===null?[]:['Content-Type'=>$this->media],$this->body);}
    public function last():S\HttpRequest{$key=array_key_last($this->requests)??throw new RuntimeException('no request');return $this->requests[$key];}
};
$client=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
$input=new S\MatrixInput(point:new S\__POINT__(x:S\JsonNumber::fromInt(1),y:S\JsonNumber::fromInt(2)),csv:['a,b','c'],space:['a','b'],pipe:['c','d'],reserved:'a/b%26x',document:new S\__DOCUMENT__(a:'x'));
$json=$client->matrix($input);check($json instanceof S\__JSON__&&$json->body==='json');
check($transport->requests[0]->url==='https://fixture.test/matrix/;x=1;y=2?csv=a%2Cb,c&space=a%20b&pipe=c%7Cd&reserved=a/b%26x&document=%7B%22a%22%3A%22x%22%7D');
$transport->status=201;$transport->media='text/plain';$transport->body='range';$range=$client->matrix($input);check($range->body==='range');
$transport->status=200;$transport->media='application/octet-stream';$transport->body="\x00\xff";$bytes=$client->matrix($input);check($bytes instanceof S\__APPLICATION__&&$bytes->body->value==="\x00\xff");
$transport->media='image/png';check($client->matrix($input) instanceof S\__ANY__);
$transport->media='Application/JSON; profile="exact"; charset=UTF-8';$transport->body='1e3';$specific=$client->matrix($input);check($specific instanceof S\__PARAMETERIZED__&&$specific->body->toInt()===1000);
$transport->media='application/json';$transport->body='not-json';try{$client->matrix($input);throw new RuntimeException('specific media fell back to bytes');}catch(S\SdkError $e){check($e->kind==='response_validation');}
$transport->status=201;$transport->media='application/octet-stream';try{$client->matrix($input);throw new RuntimeException('range media fell back to default');}catch(S\SdkError $e){check($e->kind==='unexpected_media');}
$transport->status=200;$transport->media='text/plain';$transport->body='success';check($client->fallback()->body==='success');
$transport->status=409;$transport->body='failure';try{$client->fallback();throw new RuntimeException('default classified success');}catch(S\FallbackApiError $e){check($e->response->status===409);}
$transport->body='';$transport->media=null;foreach([204,205] as$status){$transport->status=$status;check($client->matrix($input)->body===S\NoBody::Value);check($client->fallback()->body===S\NoBody::Value);}
$transport->status=304;try{$client->fallback();throw new RuntimeException('304 was successful');}catch(S\FallbackApiError $e){check($e->response->body==='');}
$transport->status=204;$transport->body='illegal';try{$client->matrix($input);throw new RuntimeException('204 content accepted');}catch(S\SdkError $e){check($e->kind==='response_validation');}
$transport->status=200;$transport->body='opaque';$client->whole(new S\WholeInput(raw:new S\__WHOLE__(q:'a b+',tags:['x','y'])));
$last=$transport->last();check($last->method==='QUERY'&&$last->url==='https://fixture.test/whole?q=a+b%2B&tags=x&tags=y');
$client->purge();check($transport->last()->method==='PURGE');
$opaque=$client->customVerb();check($transport->last()->method==='custom-Verb'&&$opaque->body->value==='opaque');
try{$client->undocumented(options:new S\RequestOptions(maxCaptureBytes:3));}catch(S\SdkError $e){check($e->kind==='unexpected_status'&&$e->response?->body==='opa'&&$e->response->truncated);}
$wire=new S\Client(new S\Credentials([]),new S\CurlTransport(),new S\ClientOptions(serverUrl:'__BASE__'));
$wire->matrix($input);$wire->whole(new S\WholeInput(raw:new S\__WHOLE__(q:'a b+',tags:['x','y'])));$wire->purge();$wire->customVerb();$wire->verbOptions();$wire->verbTrace();$wire->verbDelete();$wire->verbPatch();
echo 'native style, querystring, media specificity and actual-status matrix passed',PHP_EOL;
"#;
fn php() -> PathBuf {
    std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/sdk-php-tools/php-8.3.32/php"))
}
fn capabilities() -> Capabilities {
    php_sdk::protocol::capabilities()
}
fn source() -> Value {
    json!({"openapi":"3.2.0","info":{"title":"PHP protocol","version":"1"},"servers":[{"url":"/api/{version}","variables":{"version":{"default":"v1","enum":["v1","v2"]}}},{"url":"/alternate"}],"security":[{}, {"token":[]}],
    "components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"},"basic":{"type":"http","scheme":"basic"},"head":{"type":"apiKey","in":"header","name":"X-Api-Key"},"query":{"type":"apiKey","in":"query","name":"key"},"cookie":{"type":"apiKey","in":"cookie","name":"session"},"oauth":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://issuer.test/token","scopes":{"read":"Read"}}}}},"schemas":{"Item":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}}},
    "paths":{
        "/items/{ids}":{"get":{"operationId":"readItems","parameters":[{"name":"ids","in":"path","required":true,"style":"label","explode":true,"schema":{"type":"array","items":{"type":"integer"}}},{"name":"filter","in":"query","style":"deepObject","explode":true,"schema":{"type":"object","properties":{"name":{"type":"string"}},"additionalProperties":false}},{"name":"X-Labels","in":"header","schema":{"type":"array","items":{"type":"string"}}},{"name":"pref","in":"cookie","schema":{"type":"string"}}],"responses":{"200":{"description":"Item","content":{"application/problem+json; profile=fixture":{"schema":{"$ref":"#/components/schemas/Item"}}},"headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}}},"2XX":{"description":"Text","content":{"text/plain":{}}},"default":{"description":"Bytes","content":{"application/octet-stream":{}}}}}},
        "/text":{"post":{"operationId":"sendText","security":[{"head":[],"query":[],"cookie":[]}],"requestBody":{"required":true,"content":{"text/plain":{"schema":{"type":"string"}}}},"responses":{"200":{"description":"Text","content":{"text/plain":{"schema":{"type":"string"}}}}}}},
        "/bytes":{"put":{"operationId":"sendBytes","security":[{"basic":[]}],"requestBody":{"required":true,"content":{"application/octet-stream":{}}},"responses":{"200":{"description":"Bytes","content":{"application/octet-stream":{}}}}}},
        "/oauth":{"get":{"operationId":"useOauth","security":[{"oauth":["read"]}],"responses":{"204":{"description":"No body"}}}},
        "/head":{"head":{"operationId":"readHead","responses":{"200":{"description":"Metadata"}}}}
    }})
}
fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn command(command: &mut Command, root: &Path, label: &str) {
    let output = command.output().unwrap();
    writeln!(fs::OpenOptions::new().create(true).append(true).open(root.join("commands.jsonl")).unwrap(),"{}",json!({"label":label,"command":format!("{command:?}"),"exit":output.status.code(),"stdoutSha256":format!("{:x}",Sha256::digest(&output.stdout)),"stderrSha256":format!("{:x}",Sha256::digest(&output.stderr))})).unwrap();
    fs::write(
        root.join(format!("{label}.log")),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn package(value: &Value, caps: Capabilities) -> (PathBuf, php_sdk::protocol::SdkPlan) {
    let root = repo().join("target/sdk-php-protocol");
    fs::create_dir_all(&root).unwrap();
    let root = tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(root)
        .unwrap()
        .keep();
    let api = root.join("api.json");
    fs::write(&api, value.to_string()).unwrap();
    let contract = load(&api);
    package_contract(root, contract, caps)
}

fn package_contract(
    root: PathBuf,
    contract: Arc<Contract>,
    caps: Capabilities,
) -> (PathBuf, php_sdk::protocol::SdkPlan) {
    let api = root.join("api.json");
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let config = PhpConfig {
        package_name: "fixture/php-protocol".into(),
        package_version: "0.1.0".into(),
        namespace: "ProtocolSdk".into(),
        ..Default::default()
    };
    let plan = php_sdk::protocol::plan_sdk(contract, &selected, config, caps).unwrap();
    let emitted = plan.render();
    fs::write(root.join("emitted-manifest.json"),serde_json::to_string_pretty(&json!({"sourceSha256":format!("{:x}",Sha256::digest(fs::read(&api).unwrap())),"operations":plan.operations().iter().map(|op|op.id.as_str()).collect::<Vec<_>>(),"php":php(),"files":emitted.iter().map(|f|json!({"path":f.path,"bytes":f.content.len(),"sha256":format!("{:x}",Sha256::digest(f.content.as_bytes()))})).collect::<Vec<_>>()})).unwrap()).unwrap();
    for file in &emitted {
        let path = root.join(&file.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &file.content).unwrap();
    }
    let package = root.join("php");
    let tools = repo().join("target/sdk-php-tools");
    command(
        Command::new(php())
            .arg("-n")
            .arg(tools.join("composer-2.10.3.phar"))
            .args([
                "archive",
                "--format=zip",
                "--dir=build",
                "--file=package",
                "--no-plugins",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .env("COMPOSER_CACHE_DIR", tools.join("composer-cache"))
            .current_dir(&package),
        &root,
        "composer",
    );
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(package.join("composer.json")).unwrap()).unwrap();
    manifest["dist"] = json!({"type":"zip","url":Uri::from_path(&package.join("build/package.zip")).unwrap().to_string()});
    let installed = root.join("consumer");
    fs::create_dir(&installed).unwrap();
    fs::write(installed.join("composer.json"),json!({"name":"fixture/consumer","require":{"fixture/php-protocol":"0.1.0"},"repositories":[{"type":"package","package":manifest},{"packagist.org":false}],"config":{"allow-plugins":false}}).to_string()).unwrap();
    command(
        Command::new(php())
            .arg("-n")
            .arg(tools.join("composer-2.10.3.phar"))
            .args([
                "install",
                "--no-dev",
                "--no-plugins",
                "--no-scripts",
                "--no-progress",
            ])
            .env("COMPOSER_HOME", root.join("composer-home"))
            .env("COMPOSER_CACHE_DIR", tools.join("composer-cache"))
            .current_dir(&installed),
        &root,
        "consumer-install",
    );
    let package = installed.join("vendor/fixture/php-protocol");
    for file in &emitted {
        assert_eq!(
            fs::read(package.join(file.path.strip_prefix("php/").unwrap())).unwrap(),
            file.content.as_bytes()
        );
    }
    command(
        Command::new(php())
            .arg("-n")
            .arg(tools.join("phpstan-2.2.13.phar"))
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file",
            ])
            .arg(installed.join("vendor/autoload.php"))
            .current_dir(&package),
        &root,
        "phpstan",
    );
    for example in ["codecs", "quickstart", "client"] {
        command(
            Command::new(php())
                .args(["-n", "-d", "error_reporting=-1"])
                .arg(package.join(format!("examples/{example}.php")))
                .env(
                    "SUSPECT_SDK_AUTOLOAD",
                    installed.join("vendor/autoload.php"),
                )
                .current_dir(&installed),
            &root,
            &format!("example-{example}"),
        );
    }
    fs::write(
        installed.join("docs.php"),
        include_str!("../src/php_sdk/native_protocol_docs.php"),
    )
    .unwrap();
    command(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "docs.php"])
            .arg(&package)
            .current_dir(&installed),
        &root,
        "native-docs",
    );
    (root, plan)
}

fn consumer(root: &Path, source: &str, label: &str) {
    let package = root.join("consumer");
    let tools = repo().join("target/sdk-php-tools");
    fs::write(package.join("consumer.php"), source).unwrap();
    command(
        Command::new(php())
            .arg("-n")
            .arg(tools.join("phpstan-2.2.13.phar"))
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--level=max",
                "--autoload-file=vendor/autoload.php",
                "consumer.php",
            ])
            .current_dir(&package),
        root,
        &format!("{label}-types"),
    );
    command(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "consumer.php"])
            .current_dir(&package),
        root,
        label,
    );
    println!("PHP protocol evidence: {}", root.display());
}

fn rejected_consumer(root: &Path, source: &str, minimum_type_errors: usize) {
    let installed = root.join("consumer");
    fs::write(installed.join("negative.php"), source).unwrap();
    let mut command = Command::new(php());
    command
        .arg("-n")
        .arg(repo().join("target/sdk-php-tools/phpstan-2.2.13.phar"))
        .args([
            "analyse",
            "--no-progress",
            "--memory-limit=1G",
            "--level=max",
            "--error-format=json",
            "--autoload-file=vendor/autoload.php",
            "negative.php",
        ])
        .current_dir(&installed);
    let output = command.output().unwrap();
    fs::write(root.join("negative-types.json"), &output.stdout).unwrap();
    fs::write(root.join("negative-types.stderr"), &output.stderr).unwrap();
    writeln!(fs::OpenOptions::new().create(true).append(true).open(root.join("commands.jsonl")).unwrap(),"{}",json!({"label":"negative-types","command":format!("{command:?}"),"expectedExit":1,"exit":output.status.code()})).unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "PHPStan must reject the external negative consumer"
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let count = report["files"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|f| f["messages"].as_array().unwrap())
        .filter(|message| message["identifier"] == "argument.type")
        .count();
    assert!(
        count >= minimum_type_errors,
        "expected typed byte/body/header argument failures: {report}"
    );
}

#[test]
#[ignore = "requires isolated PHP, Composer and PHPStan"]
fn native_protocol_package_types_and_injected_wire() {
    let (root, plan) = package(&source(), capabilities());
    let input = plan
        .operations()
        .iter()
        .find(|o| o.id == "readItems")
        .unwrap();
    let filter = input
        .parameters
        .iter()
        .find(|p| p.wire.name() == "filter")
        .unwrap();
    consumer(
        &root,
        &CONSUMER.replace(
            "__FILTER__",
            &plan.models().type_name(&filter.schema, false),
        ),
        "native-wire",
    );
}
const CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $v): void {if(!$v){throw new RuntimeException('protocol mismatch');}}
$transport=new class implements S\Transport {
    /** @var list<S\HttpRequest> */
    public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse {
        $this->requests[]=$request;$request->check();
        if(str_ends_with($request->url,'/oauth')){return new S\HttpResponse(204,[], '');}
        if(str_ends_with($request->url,'/head')){return new S\HttpResponse(200,[], '');}
        if(str_contains($request->url,'/text?')){return new S\HttpResponse(200,['Content-Type'=>'text/plain'],'hello');}
        if(str_ends_with($request->url,'/bytes')){return new S\HttpResponse(200,['Content-Type'=>'application/octet-stream'],"\x00\xff");}
        return new S\HttpResponse(200,['Content-Type'=>'application/problem+json; profile=fixture','X-Count'=>'1e3'],'{"id":"x"}');
    }
};
$credentials=new S\Credentials(['token'=>'bearer-key','basic'=>new S\BasicCredential('user','pass:secret'),'head'=>new S\ApiKeyCredential('header-key'),'query'=>new S\ApiKeyCredential('a/b'),'cookie'=>new S\ApiKeyCredential('cookie-key'),'oauth'=>static function(S\CredentialRequest $request):S\AuthorizationCredential {check($request->permissions===['read']);return new S\AuthorizationCredential('Bearer oauth-key');}]);
$client=new S\Client($credentials,$transport,new S\ClientOptions(serverBaseUrl:'https://fixture.test/base/file',serverVariables:['version'=>'v2']));
$result=$client->readItems(new S\ReadItemsInput(ids:[S\JsonNumber::fromInt(1),S\JsonNumber::fromInt(2)],filter:new S\__FILTER__(name:'a/b'),xLabels:['x','y'],pref:'dark'),new S\RequestOptions(securityAlternative:1));
if(!$result instanceof S\ReadItemsStatus200){throw new RuntimeException('unexpected native status variant');}
check($result->body->id==='x');check($result->headers->xCount->toInt()===1000);
check($transport->requests[0]->url==='https://fixture.test/api/v2/items/.1.2?filter%5Bname%5D=a%2Fb');
check($transport->requests[0]->headers['authorization']==='Bearer bearer-key');
check($transport->requests[0]->headers['cookie']==='pref=dark');
$text=$client->sendText(new S\SendTextInput(body:'hello'));check($text->body==='hello');
check($transport->requests[1]->headers['x-api-key']==='header-key');check(str_ends_with($transport->requests[1]->url,'?key=a%2Fb'));check($transport->requests[1]->headers['cookie']==='session=cookie-key');
$bytes=$client->sendBytes(new S\SendBytesInput(body:new S\Bytes("\x00\xff")));check($bytes->body->value==="\x00\xff");
check($transport->requests[2]->headers['authorization']==='Basic '.base64_encode('user:pass:secret'));
$none=$client->useOauth();check($none->response->status===204&&$none->response->body==='');
$head=$client->readHead();check($head->response->status===200&&$head->response->body==='');
echo 'native rich protocol calls passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires isolated PHP, Composer and PHPStan"]
fn native_form_and_named_multipart_use_typed_parts_and_real_bytes() {
    let mut value = source();
    value["paths"] = json!({
        "/form":{"post":{"operationId":"sendForm","security":[],"requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}},"settings":{"type":"object","properties":{"enabled":{"type":"boolean"}},"additionalProperties":false}},"additionalProperties":false}}}},"responses":{"200":{"description":"Echo","content":{"text/plain":{"schema":{"type":"string"}}}}}}},
        "/upload":{"post":{"operationId":"upload","security":[],"requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["file","metadata"],"properties":{"file":{},"metadata":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}},"additionalProperties":false}},"additionalProperties":false},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer"}}}},"metadata":{"contentType":"application/json"}}}}},"responses":{"200":{"description":"Uploaded","content":{"text/plain":{"schema":{"type":"string"}}}}}}}
    });
    let caps = capabilities()
        .with(Capability::FormBodies)
        .with(Capability::MultipartBodies)
        .with(Capability::PartEncodings);
    let (root, plan) = package(&value, caps);
    let form = plan.surface.objects.iter().find(|o| !o.multipart).unwrap();
    let multipart = plan.surface.objects.iter().find(|o| o.multipart).unwrap();
    let file = multipart.parts.iter().find(|p| p.name == "file").unwrap();
    let metadata = multipart
        .parts
        .iter()
        .find(|p| p.name == "metadata")
        .unwrap();
    let php_sdk::protocol::Payload::Schema(meta_schema) = &metadata.payload else {
        panic!()
    };
    let settings = form.parts.iter().find(|p| p.name == "settings").unwrap();
    let php_sdk::protocol::Payload::Schema(settings_schema) = &settings.payload else {
        panic!()
    };
    let text = FORM_CONSUMER
        .replace("__FORM__", &form.name)
        .replace(
            "__SETTINGS__",
            &plan.models().type_name(settings_schema, false),
        )
        .replace("__MULTIPART__", &multipart.name)
        .replace("__FILE__", file.wrapper.as_ref().unwrap())
        .replace("__HEADERS__", &file.headers.name)
        .replace("__METADATA__", metadata.wrapper.as_ref().unwrap())
        .replace(
            "__METADATA_VALUE__",
            &plan.models().type_name(meta_schema, false),
        );
    consumer(&root, &text, "native-parts");
    let negative = format!(
        "{text}\nnew S\\{}(value: 'not bytes', headers: new S\\{}(xPart: 1));\n$client->upload(new S\\UploadInput(body: new S\\Bytes('not a multipart aggregate')));\n",
        file.wrapper.as_ref().unwrap(),
        file.headers.name
    );
    rejected_consumer(&root, &negative, 3);
}
const FORM_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('part mismatch');}}
$transport=new class implements S\Transport {
    /** @var list<S\HttpRequest> */ public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse{$this->requests[]=$request;return new S\HttpResponse(200,['Content-Type'=>'text/plain'],'ok');}
};
$client=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverBaseUrl:'https://fixture.test/base'));
$client->sendForm(new S\SendFormInput(body:new S\__FORM__(name:'a b+/',tags:['x','y'],settings:new S\__SETTINGS__(enabled:true))));
check($transport->requests[0]->body==='name=a+b%2B%2F&settings=%7B%22enabled%22%3Atrue%7D&tags=x&tags=y');
$file=new S\__FILE__(value:new S\Bytes("\x00\xff\r\nexact"),headers:new S\__HEADERS__(xPart:S\JsonNumber::fromString('1.0')),filename:'a"b.bin');
$body=new S\__MULTIPART__(file:$file,metadata:new S\__METADATA__(new S\__METADATA_VALUE__(id:'meta')));
$client->upload(new S\UploadInput(body:$body));
$request=$transport->requests[1];$wire=$request->body??throw new RuntimeException('missing MIME body');
check(str_contains($request->headers['content-type'],'boundary='));
check(str_contains($wire,"\x00\xff\r\nexact"));check(str_contains($wire,'filename="a\\"b.bin"'));check(str_contains($wire,'X-Part: 1.0'));
check(str_contains($wire,'{"id":"meta"}'));check(!str_contains($wire,'"value":"'));
$file->headers->xPart=S\JsonNumber::fromString('1.5');
try{$client->upload(new S\UploadInput(body:$body));throw new RuntimeException('mutated integer part header accepted');}catch(S\SdkError $error){check($error->kind==='request_validation');}
echo 'native form and binary multipart part checks passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires isolated PHP, Composer and PHPStan"]
fn native_stream_items_preserve_frames_and_close_on_break_and_failure() {
    let mut value = source();
    value["paths"] = json!({
        "/events":{"get":{"operationId":"events","security":[],"responses":{"200":{"description":"Events","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer"}},"additionalProperties":false}}}}}}},
        "/lines":{"get":{"operationId":"lines","security":[],"responses":{"200":{"description":"Lines","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Item"}}}}}}}
    });
    let (root, _) = package(
        &value,
        capabilities()
            .with(Capability::ServerSentEvents)
            .with(Capability::JsonLines),
    );
    consumer(&root, STREAM_CONSUMER, "native-stream");
}
const STREAM_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('stream mismatch');}}
final class ProbeReader implements S\BodyReader {
    private int $at=0;public bool $closed=false;public int $reads=0;public bool $failRead=false;public bool $stall=false;
    public function __construct(private string $text,private bool $failClose=false){}
    public function read():?string{++$this->reads;if($this->failRead){throw new RuntimeException('private-read-failure');}if($this->stall){return '';}if($this->at===strlen($this->text)){return null;}return $this->text[$this->at++];}
    public function close():void{$this->closed=true;if($this->failClose){throw new RuntimeException('private-close-failure');}}
}
$transport=new class implements S\StreamTransport {
    public ?ProbeReader $reader=null;
    public ?string $override=null;
    public bool $failClose=false;
    public function send(S\HttpRequest $request):S\HttpResponse{throw new RuntimeException('stream was eagerly buffered');}
    public function open(S\HttpRequest $request):S\StreamResponse{
        $sse=str_ends_with($request->url,'/events');
        $text=$sse?"\xEF\xBB\xBF: comment\r\ndata: {\"x\":1}\r\ndata: 雪\r\nevent: tick\r\nretry: 00015\r\nid: one\r\n\r\ndata: [DONE]\r\r":"{\"id\":\"one\"}\n{\"id\":\"two\"}";
        $this->reader=new ProbeReader($this->override??$text,$this->failClose);
        return new S\StreamResponse(200,['Content-Type'=>$sse?'text/event-stream':'application/x-ndjson'],$this->reader);
    }
};
$client=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverBaseUrl:'https://fixture.test/base'));
$response=$client->events();$events=iterator_to_array($response->body);
check(count($events)===2);check($events[0]->data==="{\"x\":1}\n雪");check($events[0]->retry instanceof S\JsonNumber&&$events[0]->retry->toInt()===15);
check($events[1]->data==='[DONE]');
check($transport->reader?->closed===true);
$lines=$client->lines();$items=iterator_to_array($lines->body);check($items[0]->id==='one'&&$items[1]->id==='two');
$stream=$client->events()->body;try{foreach($stream as$item){check($item->data!== '');break;}}finally{$stream->close();}
check($transport->reader?->closed===true);
$transport->override="{}\n";$transport->failClose=true;
try{iterator_to_array($client->lines()->body);throw new RuntimeException('bad item passed');}catch(S\SdkError $error){check($error->kind==='response_validation');check(!str_contains((string)$error,'private-close-failure'));}
check($transport->reader?->closed===true);
$transport->failClose=false;$transport->override="data: first\n\n";
$token=new S\CancellationToken();$stream=$client->events(options:new S\RequestOptions(cancellation:$token))->body;$token->cancel();
try{iterator_to_array($stream);throw new RuntimeException('cancelled stream passed');}catch(S\SdkError $error){check($error->kind==='cancelled');}
check($transport->reader?->closed===true&&$transport->reader->reads===0);
$stream=$client->events(options:new S\RequestOptions(timeoutMilliseconds:50))->body;usleep(80000);
try{iterator_to_array($stream);throw new RuntimeException('expired stream passed');}catch(S\SdkError $error){check($error->kind==='timeout');}
check($transport->reader?->closed===true&&$transport->reader->reads===0);
$transport->override=": ignored\r\nunknown: field\r\nid: ignored\0id\r\nretry: -1\r\ndata: \xff\xc3(\r\n\r\n";
$events=iterator_to_array($client->events()->body);check(count($events)===1&&$events[0]->data==="\xef\xbf\xbd\xef\xbf\xbd("&&$events[0]->id===S\Absent::Value&&$events[0]->retry===S\Absent::Value);
$transport->override="data:\r\r";$events=iterator_to_array($client->events()->body);check(count($events)===1&&$events[0]->data==='');
$transport->override="data: unfinished";check(iterator_to_array($client->events()->body)===[]);
foreach(["\xEF\xBB\xBF{\"id\":\"one\"}\n","{\"id\":\"\xff\"}\n","{\"id\":\"one\"}\r{\"id\":\"two\"}"] as$text){
    $transport->override=$text;try{iterator_to_array($client->lines()->body);throw new RuntimeException('invalid JSON-lines framing accepted');}catch(S\SdkError $error){check($error->kind==='response_validation');}check($transport->reader?->closed===true);
}
$transport->override="data: first\n\n";
$stream=$client->events()->body;$reader=$transport->reader??throw new RuntimeException('missing reader');$reader->failRead=true;
try{iterator_to_array($stream);throw new RuntimeException('reader failure accepted');}catch(S\SdkError $error){check($error->kind==='transport'&&!str_contains((string)$error,'private-read-failure'));}check($reader->closed);
$stream=$client->events()->body;$reader=$transport->reader??throw new RuntimeException('missing reader');$reader->stall=true;
try{iterator_to_array($stream);throw new RuntimeException('empty chunk accepted');}catch(S\SdkError $error){check($error->kind==='transport');}check($reader->closed&&$reader->reads===1);
$stream=$client->events(options:new S\RequestOptions(maxResponseBytes:8,maxCaptureBytes:3))->body;
try{iterator_to_array($stream);throw new RuntimeException('stream total limit ignored');}catch(S\SdkError $error){check($error->kind==='resource_limit'&&$error->response?->body==='dat');}check($transport->reader->closed);
echo 'native chunked SSE envelopes and JSON-lines items passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires native PHP/Composer/PHPStan and independent request-item wire checks"]
fn native_request_iterables_are_typed_bounded_and_single_pass() {
    let mut value = source();
    value["components"]["schemas"]["Event"] = json!({"type":"object","required":["data"],"properties":{"data":{"type":"string"},"id":{"type":"string"},"event":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false});
    value["paths"] = json!({
        "/push-lines":{"post":{"operationId":"pushLines","security":[],"requestBody":{"required":true,"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Item"}}}},"responses":{"200":{"description":"Accepted","content":{"text/plain":{"schema":{"type":"string"}}}}}}},
        "/push-events":{"post":{"operationId":"pushEvents","security":[],"requestBody":{"required":true,"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}},"responses":{"200":{"description":"Accepted","content":{"text/plain":{"schema":{"type":"string"}}}}}}}
    });
    let (root, plan) = package(&value, capabilities());
    let item_type = |id: &str| {
        let op = plan.operations().iter().find(|op| op.id == id).unwrap();
        let php_sdk::protocol::Payload::Stream(schema) = &op.body[0].payload else {
            panic!("native request iterable")
        };
        plan.models().type_name(schema, false)
    };
    let server = Server::start(false);
    let script = REQUEST_STREAM_CONSUMER
        .replace("__BASE__", &server.url)
        .replace("__ITEM__", &item_type("pushLines"))
        .replace("__EVENT__", &item_type("pushEvents"));
    consumer(&root, &script, "native-request-items");
    let requests = server.finish();
    assert_eq!(requests.len(), 2, "request streams are never replayed");
    assert_eq!(requests[0].target, "/push-lines");
    assert_eq!(requests[0].headers["content-type"], "application/x-ndjson");
    assert_eq!(requests[0].body, b"{\"id\":\"one\"}\n{\"id\":\"two\"}\n");
    assert_eq!(requests[1].headers["content-type"], "text/event-stream");
    assert_eq!(
        requests[1].body,
        "event: tick\nid: one\nretry: 15\ndata: {\"native\":true}\ndata: 雪\n\ndata: [DONE]\n\n"
            .as_bytes()
    );
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&requests).unwrap(),
    )
    .unwrap();
}
const REQUEST_STREAM_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('request iterable mismatch');}}
/** @return Generator<int,S\__ITEM__> */
function lines():Generator{yield new S\__ITEM__(id:'one');yield new S\__ITEM__(id:'two');}
$client=new S\Client(new S\Credentials([]),new S\CurlTransport(),new S\ClientOptions(serverUrl:'__BASE__'));
check($client->pushLines(new S\PushLinesInput(body:lines()))->body==='ok');
check($client->pushEvents(new S\PushEventsInput(body:[new S\__EVENT__(data:"{\"native\":true}\n雪",event:'tick',id:'one',retry:S\JsonNumber::fromString('15.0')),new S\__EVENT__(data:'[DONE]')]))->body==='ok');
$parameter=(new ReflectionMethod(S\PushLinesInput::class,'__construct'))->getParameters()[0];check((string)$parameter->getType()==='iterable');
$transport=new class implements S\Transport {public int $sends=0;public function send(S\HttpRequest $request):S\HttpResponse{++$this->sends;return new S\HttpResponse(200,['Content-Type'=>'text/plain'],'ok');}};
$bounded=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test',maxRequestBytes:64));
$produced=0;
/** @var Closure():Generator<int,S\__ITEM__> $endless */
$endless=static function()use(&$produced):Generator{while(true){++$produced;yield new S\__ITEM__(id:'more');}};
try{$bounded->pushLines(new S\PushLinesInput(body:$endless()));throw new RuntimeException('unbounded input accepted');}catch(S\SdkError $e){check($e->kind==='request_validation');}
check($produced<=6&&$transport->sends===0);
$token=new S\CancellationToken();
/** @var Closure():Generator<int,S\__ITEM__> $cancelled */
$cancelled=static function()use($token):Generator{yield new S\__ITEM__(id:'first');$token->cancel();yield new S\__ITEM__(id:'second');};
try{$bounded->pushLines(new S\PushLinesInput(body:$cancelled()),new S\RequestOptions(cancellation:$token));throw new RuntimeException('cancelled producer accepted');}catch(S\SdkError $e){check($e->kind==='cancelled');}
try{$bounded->pushEvents(new S\PushEventsInput(body:[new S\__EVENT__(data:"not\rrepresentable")]));throw new RuntimeException('lossy SSE request accepted');}catch(S\SdkError $e){check($e->kind==='request_validation');}
check($transport->sends===0);
echo 'native request iterable wire, finite preparation and cancellation passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires native PHP/Composer/PHPStan; auth, source metadata and server choices"]
fn native_security_alternatives_hooks_servers_and_links() {
    let mut value = source();
    value["servers"][1] = json!({"url":"../a//b/{region}","variables":{"region":{"default":"us","enum":["us","eu"]}}});
    value["components"]["securitySchemes"]["oidc"] = json!({"type":"openIdConnect","openIdConnectUrl":"https://issuer.test/.well-known/openid-configuration"});
    value["components"]["securitySchemes"]["oauth"]["oauth2MetadataUrl"] =
        json!("https://issuer.test/.well-known/oauth-authorization-server");
    value["paths"] = json!({
        "/anonymous":{"get":{"operationId":"anonymous","responses":{"204":{"description":"Anonymous or bearer","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"links":{"next":{"operationId":"useOidc","parameters":{"cursor":"$response.header.X-Count"}}}}}}},
        "/and":{"get":{"operationId":"allKeys","security":[{"head":[],"query":[],"cookie":[]},{"basic":[]}],"parameters":[{"name":"X-Api-Key","in":"header","schema":{"type":"string"}}],"responses":{"204":{"description":"Complete alternative"}}}},
        "/oauth":{"get":{"operationId":"useOauth","security":[{"oauth":["read"]}],"responses":{"204":{"description":"Caller OAuth"}}}},
        "/oidc":{"get":{"operationId":"useOidc","security":[{"oidc":["openid","profile"]}],"responses":{"204":{"description":"Caller OIDC"}}}},
        "/role":{"get":{"operationId":"role","security":[{"token":["admin"]}],"responses":{"204":{"description":"Source role metadata"}}}}
    });
    fs::create_dir_all(repo().join("target/sdk-php-protocol")).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("refusal-")
        .tempdir_in(repo().join("target/sdk-php-protocol"))
        .unwrap()
        .keep();
    let path = directory.join("api.json");
    fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let findings =
        php_sdk::protocol::plan_sdk(contract, &selected, PhpConfig::default(), capabilities())
            .unwrap_err();
    assert!(
        findings
            .iter()
            .any(|f| f.code == "http-security-parameter-conflict"
                && f.source.pointer() == "/paths/~1and/get/security/0/head"
                && !f.at.is_empty())
    );
    fs::write(directory.join("findings.txt"), format!("{findings:#?}")).unwrap();
    value["paths"]["/and"]["get"]
        .as_object_mut()
        .unwrap()
        .remove("parameters");
    let (root, _) = package(&value, capabilities());
    consumer(&root, SECURITY_CONSUMER, "native-security");
}
const SECURITY_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('security/server mismatch');}}
function member(S\JsonValue $value,string $key):S\JsonValue{return $value->asObject()[$key]??throw new RuntimeException('source metadata missing');}
$transport=new class implements S\Transport {
    /** @var list<S\HttpRequest> */public array $requests=[];
    public function send(S\HttpRequest $request):S\HttpResponse{$this->requests[]=$request;return new S\HttpResponse(204,['X-Count'=>'2.0'],'');}
    public function last():S\HttpRequest{$key=array_key_last($this->requests)??throw new RuntimeException('no request');return $this->requests[$key];}
};
$hooks=0;
$credentials=new S\Credentials(['token'=>'bearer-key','head'=>new S\ApiKeyCredential('header-key'),'query'=>new S\ApiKeyCredential('a/b'),'cookie'=>new S\ApiKeyCredential('cookie-key'),
    'oauth'=>static function(S\CredentialRequest $request)use(&$hooks):S\AuthorizationCredential{++$hooks;check($request->operationId==='useOauth'&&$request->permissions===['read']);$metadata=member($request->metadata,'credential');check(member(member($metadata,'metadata_url'),'value')->asString()==='https://issuer.test/.well-known/oauth-authorization-server');$flow=member($metadata,'flows')->asArray()[0];check(member(member($flow,'token_url'),'value')->asString()==='https://issuer.test/token');return new S\AuthorizationCredential('Bearer oauth-key');},
    'oidc'=>static function(S\CredentialRequest $request)use(&$hooks):S\AuthorizationCredential{++$hooks;check($request->kind==='open-id-connect'&&$request->permissions===['openid','profile']);$metadata=member($request->metadata,'credential');check(member(member($metadata,'discovery_url'),'value')->asString()==='https://issuer.test/.well-known/openid-configuration');return new S\AuthorizationCredential('Bearer oidc-key');}]);
$client=new S\Client($credentials,$transport,new S\ClientOptions(serverBaseUrl:'https://fixture.test/base/spec.json',serverVariables:['version'=>'v2']));
$response=$client->anonymous();check(!isset($transport->last()->headers['authorization'])&&$hooks===0);check($response->headers->xCount->toInt()===2);check($response->links[0]->name==='next'&&str_contains($response->links[0]->metadata->toJson(),'$response.header.X-Count'));
$client->anonymous(options:new S\RequestOptions(securityAlternative:1));check($transport->last()->headers['authorization']==='Bearer bearer-key');
$client->allKeys();check($transport->last()->url==='https://fixture.test/api/v2/and?key=a%2Fb'&&$transport->last()->headers['x-api-key']==='header-key'&&$transport->last()->headers['cookie']==='session=cookie-key');
$client->useOauth();check($transport->last()->headers['authorization']==='Bearer oauth-key');$client->useOidc();check($transport->last()->headers['authorization']==='Bearer oidc-key'&&$hooks===2);
$client->role();check($transport->last()->headers['authorization']==='Bearer bearer-key');
$relative=new S\Client($credentials,$transport,new S\ClientOptions(serverBaseUrl:'https://fixture.test/directory/spec.json',serverIndex:1,serverVariables:['region'=>'eu']));$relative->anonymous();check($transport->last()->url==='https://fixture.test/a//b/eu/anonymous');
$sent=count($transport->requests);
try{$client->anonymous(options:new S\RequestOptions(securityAlternative:99));throw new RuntimeException('invalid auth alternative accepted');}catch(S\SdkError $error){check($error->kind==='credentials');}
$partial=new S\Client(new S\Credentials(['head'=>new S\ApiKeyCredential('incomplete')]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
try{$partial->allKeys();throw new RuntimeException('partial AND accepted');}catch(S\SdkError $error){check($error->kind==='credentials');}
$wrong=new S\Client(new S\Credentials(['head'=>'bearer-is-not-an-api-key','query'=>new S\ApiKeyCredential('x'),'cookie'=>new S\ApiKeyCredential('y')]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
try{$wrong->allKeys();throw new RuntimeException('wrong credential type accepted');}catch(S\SdkError $error){check($error->kind==='credentials');}
$broken=new S\Client(new S\Credentials(['oidc'=>static function(S\CredentialRequest $request):S\AuthorizationCredential{throw new RuntimeException('private-hook-secret');}]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
try{$broken->useOidc();throw new RuntimeException('hook failure ignored');}catch(S\SdkError $error){check($error->kind==='credentials'&&!str_contains((string)$error,'private-hook-secret'));}
foreach([new S\ClientOptions(serverBaseUrl:'https://fixture.test/base/spec.json',serverVariables:['version'=>'v3']),new S\ClientOptions(serverBaseUrl:'https://fixture.test/base/spec.json',serverVariables:['unknown'=>'v1']),new S\ClientOptions(serverBaseUrl:'https://fixture.test/base/spec.json',serverIndex:99)] as$options){
    try{(new S\Client($credentials,$transport,$options))->anonymous();throw new RuntimeException('invalid server choice accepted');}catch(S\SdkError $error){check($error->kind==='configuration');}
}
check(count($transport->requests)===$sent);
echo 'native anonymous/OR/AND, OAuth/OIDC metadata, roles, links and server choices passed',PHP_EOL;
"#;

#[test]
#[ignore = "requires native PHP/Composer/PHPStan; independent form/MIME response bytes"]
fn native_response_forms_and_multipart_validate_structure_and_metadata() {
    let mut value = source();
    value["paths"] = json!({
        "/parts":{"get":{"operationId":"parts","security":[],"responses":{"200":{"description":"Named parts","content":{"multipart/form-data":{"schema":{"type":"object","required":["file","metadata","tags","styled"],"minProperties":4,"maxProperties":4,"properties":{"file":{},"metadata":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}},"additionalProperties":false},"tags":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"string"}},"styled":{"type":"string"}},"additionalProperties":false},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer"}}}},"styled":{"style":"form"}}}}}}}},
        "/form":{"get":{"operationId":"form","security":[],"responses":{"200":{"description":"Form","content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["name","count","flags"],"properties":{"name":{"type":"string"},"count":{"type":"integer"},"flags":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"boolean"}}},"additionalProperties":false}}}}}}}
    });
    let (root, _) = package(&value, capabilities());
    consumer(&root, RESPONSE_PARTS_CONSUMER, "native-response-parts");
}
const RESPONSE_PARTS_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('response part mismatch');}}
function part(string $name,string $data,?string $type='text/plain',string $headers=''):string{return "--native-boundary\r\nContent-Disposition: form-data; name=\"".$name."\"\r\n".($type===null?'':'Content-Type: '.$type."\r\n").$headers."\r\n".$data."\r\n";}
$bytes="\x00\xff\r\n--native-boundary-prefix\r\nexact";
$multipart="ignored preamble\r\n".part('file',$bytes,'application/octet-stream',"X-Part: 3.0\r\n").part('metadata','{"id":"meta"}','application/json').part('tags','one').part('tags','two').part('styled','styled=a%2Fb',null)."--native-boundary--\r\nignored epilogue";
$transport=new class implements S\Transport {
    public string $body='';public string $media='multipart/form-data; boundary="native-boundary"';
    public function send(S\HttpRequest $request):S\HttpResponse{return new S\HttpResponse(200,['Content-Type'=>$this->media],$this->body);}
};
$client=new S\Client(new S\Credentials([]),$transport,new S\ClientOptions(serverUrl:'https://fixture.test'));
$transport->body=$multipart;$response=$client->parts();check($response->body->file->value->value===$bytes&&$response->body->file->headers->xPart->toInt()===3);check($response->body->metadata->value->id==='meta');check($response->body->tags[0]->value==='one'&&$response->body->tags[1]->value==='two');check($response->body->styled->value==='a%2Fb');
foreach([str_replace("X-Part: 3.0\r\n",'', $multipart),str_replace('X-Part: 3.0','X-Part: 3.5',$multipart),str_replace(part('tags','two'),'',$multipart),str_replace('{"id":"meta"}','{}',$multipart),str_replace('--native-boundary--',part('unknown','bad').'--native-boundary--',$multipart),str_replace('X-Part: 3.0',"X-Part: 3.0\r\nContent-Transfer-Encoding: base64",$multipart),str_replace('--native-boundary--','--native-boundary-not-closed',$multipart)] as$bad){
    $transport->body=$bad;try{$client->parts();throw new RuntimeException('invalid MIME accepted');}catch(S\SdkError $error){check($error->kind==='response_validation');}
}
$transport->media='application/x-www-form-urlencoded; charset=utf-8';$transport->body='name=a+b%2F%2B&count=1e3&flags=true&flags=false';$form=$client->form();check($form->body->name==='a b/+'&&$form->body->count->toInt()===1000&&$form->body->flags===[true,false]);
foreach(['name=x&name=y&count=2&flags=true&flags=false','name=%ZZ&count=2&flags=true&flags=false','name=x&count=2&flags=true','name=x&count=2.5&flags=true&flags=false','name=x&count=2&flags=true&flags=false&unknown=y'] as$bad){$transport->body=$bad;try{$client->form();throw new RuntimeException('invalid form accepted');}catch(S\SdkError $error){check($error->kind==='response_validation');}}
echo 'native independent MIME/form bytes, structural bounds and typed part headers passed',PHP_EOL;
"#;

#[test]
#[ignore = "native physical document/redirect/encoded server-base witness; Composer and PHP required"]
fn native_document_relative_servers_keep_physical_bases() {
    use suspect_ref::{DocumentProvider, ProvidedDocument};
    let first = Server::start(false);
    let second = Server::start(false);
    let requested = "https://requested.invalid/root-alias.json";
    let effective = format!("{}/served/releases/api.json", first.url);
    let external = format!("{}/physical/components/paths.json", second.url);
    let root_value = json!({"openapi":"3.2.0","$self":"https://logical.invalid/catalog/root.json","info":{"title":"Physical base","version":"1"},"servers":[{"url":"{prefix}/Api%2Fv1/","variables":{"prefix":{"default":"%2e%2e","enum":["%2e%2e","../version"]}}}],
        "components":{"securitySchemes":{"oidc":{"type":"openIdConnect","openIdConnectUrl":"discovery"}}},
        "paths":{"/probe":{"get":{"operationId":"documentProbe","security":[{"oidc":["openid"]}],"responses":{"200":{"description":"Opaque"}}}},
            "/external":{"$ref":"https://logical.invalid/catalog/paths.json#/components/pathItems/External"},
            "/empty":{"$ref":"https://logical.invalid/catalog/paths.json#/components/pathItems/Empty"}}});
    let external_value = json!({"openapi":"3.2.0","$self":"https://logical.invalid/catalog/paths.json","info":{"title":"External paths","version":"1"},"components":{"pathItems":{
        "External":{"servers":[{"url":"../services/%2E/"}],"get":{"operationId":"externalProbe","security":[],"responses":{"200":{"description":"Opaque"}}}},
        "Empty":{"get":{"operationId":"emptyProbe","security":[],"servers":[],"responses":{"200":{"description":"Opaque"}}}}
    }}});
    let provider = Arc::new(
        DocumentProvider::new([
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(&effective).unwrap(),
                serde_json::to_vec(&root_value).unwrap(),
            )
            .unwrap(),
            ProvidedDocument::new(
                Uri::parse("https://requested.invalid/paths-alias.json").unwrap(),
                Uri::parse(&external).unwrap(),
                serde_json::to_vec(&external_value).unwrap(),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(requested).unwrap()).unwrap());
    let parent = repo().join("target/sdk-php-protocol");
    fs::create_dir_all(&parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("document-base-")
        .tempdir_in(parent)
        .unwrap()
        .keep();
    fs::write(root.join("api.json"), root_value.to_string()).unwrap();
    fs::write(root.join("provided-documents.json"),serde_json::to_string_pretty(&json!({"requested":requested,"effective":effective,"externalEffective":external,"root":root_value,"external":external_value})).unwrap()).unwrap();
    let (root, plan) = package_contract(root, contract, capabilities());
    assert!(
        plan.surface.protocol.codec_roots().is_empty(),
        "URL metadata must not become schema codec inputs"
    );
    let server = |id: &str| {
        &plan
            .operations()
            .iter()
            .find(|op| op.id == id)
            .unwrap()
            .wire
            .servers()
            .candidates()[0]
    };
    assert_eq!(
        server("documentProbe")
            .document_base()
            .source()
            .document()
            .as_str(),
        effective
    );
    assert_eq!(
        server("externalProbe")
            .document_base()
            .source()
            .document()
            .as_str(),
        external
    );
    assert_eq!(
        server("emptyProbe")
            .document_base()
            .source()
            .document()
            .as_str(),
        external
    );
    assert_eq!(
        server("documentProbe")
            .source()
            .unwrap()
            .terminal_resource()
            .unwrap()
            .base_uri(),
        "https://logical.invalid/catalog/root.json"
    );
    let script = DOCUMENT_BASE_CONSUMER.replace("__FIRST__", &first.url);
    consumer(&root, &script, "native-document-base");
    let first_requests = first.finish();
    let second_requests = second.finish();
    assert_eq!(
        first_requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        [
            "/served/releases/%2e%2e/Api%2Fv1/probe",
            "/version/Api%2Fv1/probe",
            "/direct/%2E/%2f/probe"
        ]
    );
    assert_eq!(
        second_requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        ["/physical/services/%2E/external", "/empty"]
    );
    assert!(
        first_requests
            .iter()
            .all(|r| r.headers["authorization"] == "Bearer document-token")
    );
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(
            &json!({"entryOrigin":first_requests,"externalOrigin":second_requests}),
        )
        .unwrap(),
    )
    .unwrap();
}
const DOCUMENT_BASE_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ProtocolSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('physical document base mismatch');}}
/** @var list<string> $seen */$seen=[];
$credentials=new S\Credentials(['oidc'=>static function(S\CredentialRequest $request)use(&$seen):S\AuthorizationCredential{
    $seen[]=$request->serverUrl??throw new RuntimeException('missing effective-server metadata base');
    $credential=$request->metadata->asObject()['credential']->asObject();check($credential['discovery_url']->asObject()['value']->asString()==='discovery');
    return new S\AuthorizationCredential('Bearer document-token');
}]);
$client=new S\Client($credentials,new S\CurlTransport());$client->documentProbe();$client->externalProbe();$client->emptyProbe();
$override=new S\Client($credentials,new S\CurlTransport(),new S\ClientOptions(serverBaseUrl:'__FIRST__/override/openapi.json?revision=2',serverVariables:['prefix'=>'../version']));$override->documentProbe();
$direct=new S\Client($credentials,new S\CurlTransport(),new S\ClientOptions(serverUrl:'__FIRST__/direct/%2E/%2f/'));$direct->documentProbe();
check($seen===['__FIRST__/served/releases/%2e%2e/Api%2Fv1/','__FIRST__/version/Api%2Fv1/','__FIRST__/direct/%2E/%2f/']);
echo 'physical retrieval, redirect aliases, logical self, encoded paths and explicit/effective-server bases passed',PHP_EOL;
"#;
