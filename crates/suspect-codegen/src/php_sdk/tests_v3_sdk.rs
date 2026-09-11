//! Actual SDK operation boundary for native v3 adoption.
use super::*;

pub(super) fn verify() {
    let mut document = api(json!({
        "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","required":["name"],"properties":{"name":{"type":"string"},"children":{"type":"array","items":{"$dynamicRef":"#node"}}},"examples":[{"name":"example","children":[{"name":"child"}]}]},
        "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false,"examples":[{"name":"strict","children":[{"name":"child"}]}]},
        "Selector":{"$id":"urn:selector","const":false,"$defs":{"binding":{"$dynamicAnchor":"value","$ref":"urn:integer"},"entry":{"$ref":"urn:base#/$defs/use"}}},
        "Base":{"$id":"urn:base","$defs":{"value":{"$dynamicAnchor":"value","type":"string"},"use":{"$dynamicRef":"#value"}}},
        "Integer":{"$id":"urn:integer","type":"integer"},
        "UnionBase":{"$id":"urn:union-base","type":"object","required":["value"],"properties":{"value":{"anyOf":[{"$dynamicRef":"#scalar"},{"type":"null"}]}},"$defs":{"scalar":{"$dynamicAnchor":"scalar","type":"string"}}},
        "UnionOuter":{"$id":"urn:union-outer","$ref":"urn:union-base","$defs":{"scalar":{"$dynamicAnchor":"scalar","type":"integer"}}}
    }));
    document["servers"] = json!([{"url":"https://fixture.test"}]);
    document["paths"] = json!({});
    for (path, id, reference, example) in [
        (
            "/loose",
            "loose",
            "urn:tree",
            json!({"name":"root","children":[{"name":"child"}]}),
        ),
        (
            "/strict",
            "strictTree",
            "urn:strict",
            json!({"name":"root","children":[{"name":"child"}]}),
        ),
        ("/nested", "nested", "urn:selector#/$defs/entry", json!(7)),
        (
            "/union",
            "resourceUnion",
            "urn:union-outer",
            json!({"value":7}),
        ),
    ] {
        let media = json!({"schema":{"$ref":reference},"example":example});
        document["paths"][path] = json!({"post":{"operationId":id,"security":[],"requestBody":{"required":true,"content":{"application/json":media}},"responses":{"200":{"description":"Resource echo","content":{"application/json":media}}}}});
    }
    let root = root("sdk-");
    fs::write(root.join("api.json"), document.to_string()).unwrap();
    let contract = provided(document, vec![]);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = PhpConfig {
        namespace: "ResourceSdk".into(),
        package_name: "fixture/php-v3-sdk".into(),
        package_version: "0.1.0".into(),
        ..Default::default()
    };
    let plan = protocol::plan_sdk(contract, &selected, config, protocol::capabilities()).unwrap();
    assert_eq!(plan.program().version, OwnedProgram::V3_VERSION);
    let nested = plan
        .operations()
        .iter()
        .find(|op| op.id == "nested")
        .unwrap();
    let protocol::Payload::Schema(id) = &nested.body[0].payload else {
        panic!("actual JSON root")
    };
    assert_eq!(
        plan.models().type_name(id, false),
        "JsonValue",
        "dynamic targets do not become their fallback's static carrier"
    );
    assert_eq!(plan.examples().operations().len(), 4);
    assert!(
        plan.examples()
            .operations()
            .iter()
            .all(|op| !op.entries.is_empty())
    );
    let files = plan.render();
    let consumer = install(&root, &files, "php-v3-sdk");
    let installed = consumer.join("vendor/fixture/php-v3-sdk");
    fs::write(
        root.join("checked-program.json"),
        serde_json::to_string_pretty(plan.program()).unwrap(),
    )
    .unwrap();
    run(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file",
            ])
            .arg(consumer.join("vendor/autoload.php"))
            .current_dir(&installed),
        &root,
        "package-types",
    );
    for name in ["codecs", "client", "quickstart"] {
        run(
            Command::new(php())
                .args(["-n", "-d", "error_reporting=-1"])
                .arg(installed.join(format!("examples/{name}.php")))
                .env("SUSPECT_SDK_AUTOLOAD", consumer.join("vendor/autoload.php"))
                .current_dir(&consumer),
            &root,
            &format!("native-example-{name}"),
        );
    }
    let server = Echo::start();
    fs::write(
        consumer.join("positive.php"),
        SDK_CONSUMER.replace("__BASE__", &server.url),
    )
    .unwrap();
    typecheck(&root, &consumer, &["positive.php"], "consumer-types");
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .current_dir(&consumer),
        &root,
        "native-sdk",
    );
    let records = server.finish();
    assert_eq!(records.len(), 5);
    assert_eq!(records[0].0, "/loose");
    assert_eq!(records[1].0, "/strict");
    assert_eq!(records[2].0, "/loose");
    assert_eq!(records[3], ("/nested".into(), "9007199254740993".into()));
    assert_eq!(records[4], ("/union".into(), "{\"value\":7}".into()));
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&records).unwrap(),
    )
    .unwrap();
    fs::write(
        consumer.join("docs.php"),
        include_str!("native_protocol_docs.php"),
    )
    .unwrap();
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "docs.php"])
            .arg(&installed)
            .current_dir(&consumer),
        &root,
        "native-docs",
    );
    fs::write(consumer.join("negative.php"), SDK_NEGATIVE).unwrap();
    let output = Command::new(php())
        .arg("-n")
        .arg(phpstan())
        .args([
            "analyse",
            "--no-progress",
            "--level=max",
            "--error-format=json",
            "--autoload-file=vendor/autoload.php",
            "negative.php",
        ])
        .current_dir(&consumer)
        .output()
        .unwrap();
    fs::write(root.join("negative-types.json"), &output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let errors: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        errors["totals"]["file_errors"].as_u64().unwrap() >= 3,
        "{errors}"
    );
    println!("PHP v3 SDK evidence: {}", root.display());
}

const SDK_CONSUMER: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ResourceSdk as S;
function check(bool $ok):void{if(!$ok){throw new RuntimeException('dynamic SDK mismatch');}}
$client=new S\Client(new S\Credentials([]),new S\CurlTransport(),new S\ClientOptions(serverUrl:'__BASE__'));
$tree=new S\Tree(name:'root',children:[S\JsonValue::fromObject(['name'=>S\JsonValue::fromString('child')])]);
check($client->loose(new S\LooseInput(body:$tree))->body->name==='root');
check($client->strictTree(new S\StrictTreeInput(body:$tree))->body->name==='root');
$tree->children=[S\JsonValue::fromObject(['name'=>S\JsonValue::fromString('child'),'unexpected'=>S\JsonValue::null()])];
try{$client->strictTree(new S\StrictTreeInput(body:$tree));throw new RuntimeException('outer strict binding ignored');}catch(S\SdkError $error){check($error->kind==='request_validation');}
$loose=$client->loose(new S\LooseInput(body:$tree));if(!is_array($loose->body->children)){throw new RuntimeException('children absent');}check(array_key_exists('unexpected',$loose->body->children[0]->asObject()));
try{S\Codecs::encodeStrict($tree);throw new RuntimeException('model-only strict codec bypass');}catch(S\ValidationError $error){check(str_contains($error->source,'/Strict/unevaluatedProperties')&&$error->instancePath==='/children/0/unexpected');}
$number=S\JsonValue::fromNumber(S\JsonNumber::fromString('9007199254740993'));
check($client->nested(new S\NestedInput(body:$number))->body->asNumber()->token==='9007199254740993');
try{$client->nested(new S\NestedInput(body:S\JsonValue::fromString('fallback string')));throw new RuntimeException('initial fallback became static binding');}catch(S\SdkError $error){check($error->kind==='request_validation');}
$union=new S\UnionBase(value:S\JsonValue::fromNumber(S\JsonNumber::fromInt(7)));check($client->resourceUnion(new S\ResourceUnionInput(body:$union))->body->value->asNumber()->toInt()===7);
echo 'installed dynamic SDK, outer override, nested entry, exact bytes and codec obligations passed',PHP_EOL;
"#;
const SDK_NEGATIVE: &str = r#"<?php
declare(strict_types=1);
require __DIR__.'/vendor/autoload.php';
use ResourceSdk as S;
new S\Tree(name:1);
new S\Tree(name:'root',children:[new stdClass()]);
new S\NestedInput(body:1);
"#;

struct Echo {
    url: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    records: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Echo {
    fn start() -> Self {
        use std::{
            io::{Read, Write},
            sync::{
                Mutex,
                atomic::{AtomicBool, Ordering},
            },
            time::Duration,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(Mutex::new(Vec::new()));
        let done = stop.clone();
        let captured = records.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut byte = [0];
                        while !bytes.ends_with(b"\r\n\r\n") {
                            socket.read_exact(&mut byte).unwrap();
                            bytes.push(byte[0]);
                            assert!(bytes.len() < 65536);
                        }
                        let head = String::from_utf8(bytes).unwrap();
                        let target = head
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                            .to_owned();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        assert!(length < 1024 * 1024);
                        let mut body = vec![0; length];
                        socket.read_exact(&mut body).unwrap();
                        captured
                            .lock()
                            .unwrap()
                            .push((target, String::from_utf8(body.clone()).unwrap()));
                        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").unwrap();
                        socket.write_all(&body).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("{e}"),
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
    fn finish(mut self) -> Vec<(String, String)> {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        std::mem::take(&mut *self.records.lock().unwrap())
    }
}
impl Drop for Echo {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
