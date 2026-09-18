//! Installed Composer consumers of actual emitted bytes. Native gates are opt-in.
#![cfg(feature = "php-sdk")]

use serde_json::{Value, json};
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
use suspect_codegen::php_sdk::{PhpConfig, Plan, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn base() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-native")
}
fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-tools")
}
fn php() -> PathBuf {
    std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| tools().join("php-8.3.32/php"))
}
fn phpstan() -> PathBuf {
    std::env::var_os("SUSPECT_PHPSTAN_PHAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| tools().join("phpstan-2.2.13.phar"))
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
fn config() -> PhpConfig {
    PhpConfig {
        package_name: "fixture/php-sdk".into(),
        package_version: "0.1.0".into(),
        namespace: "FixtureSdk".into(),
        ..Default::default()
    }
}
fn canonical() -> Plan {
    let contract = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, config()).unwrap()
}
fn root(label: &str) -> PathBuf {
    fs::create_dir_all(base()).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base())
        .unwrap()
        .keep()
}
fn document(value: &Value) -> Arc<Contract> {
    let root = root("source-");
    let path = root.join("api.json");
    fs::write(&path, value.to_string()).unwrap();
    load(&path)
}
fn schema_document(schemas: Value) -> Arc<Contract> {
    document(
        &json!({"openapi":"3.1.0","info":{"title":"PHP vectors","version":"1"},"components":{"schemas":schemas}}),
    )
}
fn api_document(schemas: Value, model: &str) -> Value {
    let schema = json!({"$ref":format!("#/components/schemas/{model}")});
    let operation = json!({
        "operationId":"exercise",
        "requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},
        "responses":{"200":{"description":"Fixture","content":{"application/json":{"schema":schema}}}}
    });
    json!({
        "openapi":"3.1.0", "info":{"title":"Independent PHP fixture","version":"1"},
        "servers":[{"url":"https://fixture.example.test/api"}], "security":[{"apiKey":[]}],
        "paths":{"/probe":{"post":operation}},
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":schemas}
    })
}
fn plan_document(
    value: &Value,
    config: PhpConfig,
) -> Result<Plan, Vec<suspect_codegen::php_sdk::HttpDiagnostic>> {
    let contract = document(value);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, config)
}
fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        json!({"label":label,"command":format!("{command:?}"),"exit":output.status.code()})
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if label.starts_with("native-") || label.starts_with("example-") {
        assert!(
            output.stderr.is_empty(),
            "native diagnostics at {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !["Deprecated:", "Warning:", "Notice:", "Fatal error:"]
                .iter()
                .any(|notice| text.contains(notice)),
            "native runtime diagnostic at {}: {text}",
            root.display()
        );
    }
}
fn write_files(files: Vec<suspect_codegen::OutFile>, root: &Path) {
    for file in files {
        let path = root.join(file.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, file.content).unwrap();
    }
}
fn composer(root: &Path) -> Command {
    let mut cmd = Command::new(php());
    cmd.arg("-n")
        .arg(tools().join("composer-2.10.3.phar"))
        .env("COMPOSER_HOME", root.join("composer-home"))
        .env("COMPOSER_CACHE_DIR", tools().join("composer-cache"))
        .env("COMPOSER_NO_INTERACTION", "1");
    cmd
}
fn install(plan: &Plan, label: &str) -> PathBuf {
    let root = root(label);
    write_files(plan.render(), &root);
    checked(
        Command::new(php()).args(["-n", "-r", "echo PHP_VERSION, PHP_EOL;"]),
        &root,
        "php-version",
    );
    checked(
        composer(&root)
            .args(["validate", "--no-check-publish"])
            .current_dir(root.join("php")),
        &root,
        "composer-validate",
    );
    checked(
        composer(&root)
            .args(["archive", "--format=zip", "--dir=build", "--file=package"])
            .current_dir(root.join("php")),
        &root,
        "composer-archive",
    );
    let archive = fs::canonicalize(root.join("php/build/package.zip")).unwrap();
    let mut package: Value =
        serde_json::from_str(&fs::read_to_string(root.join("php/composer.json")).unwrap()).unwrap();
    package["dist"] = json!({"type":"zip","url":Uri::from_path(&archive).unwrap().to_string()});
    let consumer = root.join("consumer");
    fs::create_dir_all(&consumer).unwrap();
    fs::write(consumer.join("composer.json"),serde_json::to_string_pretty(&json!({
        "name":"fixture/independent-consumer","description":"Native installed SDK gate","license":"proprietary",
        "repositories":[{"type":"package","package":package},{"packagist.org":false}],
        "require":{plan.config().package_name.clone():plan.config().package_version.clone()},
        "config":{"allow-plugins":false}
    })).unwrap()).unwrap();
    checked(
        composer(&root)
            .args([
                "install",
                "--no-dev",
                "--no-plugins",
                "--no-scripts",
                "--no-progress",
            ])
            .current_dir(&consumer),
        &root,
        "consumer-install",
    );
    let installed = consumer.join("vendor").join(&plan.config().package_name);
    for file in plan.render() {
        let relative = file.path.strip_prefix("php/").unwrap();
        assert_eq!(
            fs::read(installed.join(relative)).unwrap(),
            file.content.as_bytes(),
            "installed bytes changed: {relative}"
        );
    }
    checked(
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
        "package-phpstan",
    );
    for name in ["codecs", "client", "quickstart"] {
        checked(
            Command::new(php())
                .args(["-n", "-d", "error_reporting=-1"])
                .arg(installed.join(format!("examples/{name}.php")))
                .env("SUSPECT_SDK_AUTOLOAD", consumer.join("vendor/autoload.php"))
                .current_dir(&consumer),
            &root,
            &format!("example-{name}"),
        );
    }
    fs::write(
        consumer.join("docs.php"),
        include_str!("../src/php_sdk/native_docs.php"),
    )
    .unwrap();
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "docs.php"])
            .arg(&installed)
            .current_dir(&consumer),
        &root,
        "native-reference",
    );
    root
}

#[test]
fn canonical_plan_retains_native_names_constructors_and_source_bound_codecs() {
    let plan = canonical();
    assert_eq!(plan.operations().len(), 4);
    assert_eq!(plan.render(), plan.render());
    let widget = plan
        .models()
        .nodes
        .values()
        .find(|n| n.name == "Widget")
        .unwrap();
    assert_eq!(widget.codecs.encode, "encodeWidget");
    assert_eq!(
        widget.constructor.as_ref().unwrap().parameters,
        ["amount", "id", "payload", "child", "meta"]
    );
    assert_eq!(plan.program().check(), Ok(()));
    for op in plan.operations() {
        assert_eq!(
            op.responses.len(),
            match op.id.as_str() {
                "createWidget" => 3,
                "listWidgets" | "getWidget" | "updateWidget" => 2,
                _ => panic!("unexpected M2 operation"),
            }
        );
    }
}

#[test]
fn checked_program_and_profile_rejections_are_located() {
    let contract = schema_document(json!({"Root":{"type":"number","minimum":1}}));
    let compiled = suspect_schema::OwnedCompiler::new(config().validation)
        .compile(contract.clone(), contract.schema_roots())
        .unwrap();
    let program = compiled.program();
    assert!(suspect_codegen::php_sdk::emit_validation(&program, &config()).is_ok());
    let mut modified = program.clone();
    modified.version = "future";
    assert!(suspect_codegen::php_sdk::emit_validation(&modified, &config()).is_err());
    modified = program.clone();
    modified.roots[0].target = usize::MAX;
    assert!(suspect_codegen::php_sdk::emit_validation(&modified, &config()).is_err());
    modified = program;
    modified.limits.max_depth = 129;
    assert!(suspect_codegen::php_sdk::emit_validation(&modified, &config()).is_err());
    let plan = canonical();
    for name in [
        "invalid",
        "Vendor/package",
        "../../outside",
        "vendor/pkg..name",
        "vendor/pkg-",
        "vendor--name/pkg",
    ] {
        let errors = plan_sdk(
            plan.contract().clone(),
            &plan
                .operations()
                .iter()
                .map(|o| o.source.clone())
                .collect::<Vec<_>>(),
            PhpConfig {
                package_name: name.into(),
                ..config()
            },
        )
        .unwrap_err();
        assert!(errors.iter().any(|e| e.code == "php-package-name"));
    }
    for name in [
        "vendor/pkg--name",
        "0vendor/2package",
        "vendor_name/pkg.name",
    ] {
        assert!(
            plan_sdk(
                plan.contract().clone(),
                &plan
                    .operations()
                    .iter()
                    .map(|op| op.source.clone())
                    .collect::<Vec<_>>(),
                PhpConfig {
                    package_name: name.into(),
                    ..config()
                }
            )
            .is_ok()
        );
    }
}

#[test]
fn native_type_graphs_memoize_shared_unions_and_reject_excessive_alias_depth() {
    let mut schemas = serde_json::Map::from_iter([("Case0".into(), json!({"type":"string"}))]);
    for index in 1..24 {
        let branch = json!({"$ref":format!("#/components/schemas/Case{}",index-1)});
        schemas.insert(format!("Case{index}"), json!({"anyOf":[branch,branch]}));
    }
    let plan = plan_document(&api_document(Value::Object(schemas), "Case23"), config()).unwrap();
    let source = plan
        .models()
        .nodes
        .keys()
        .find(|id| id.pointer() == "/components/schemas/Case23")
        .unwrap();
    assert_eq!(plan.models().type_name(source, true), "string");
    let mut schemas = serde_json::Map::from_iter([("Case0".into(), json!({"type":"string"}))]);
    for index in 1..150 {
        schemas.insert(
            format!("Case{index}"),
            json!({"$ref":format!("#/components/schemas/Case{}",index-1)}),
        );
    }
    let errors =
        plan_document(&api_document(Value::Object(schemas), "Case149"), config()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "php-unproductive-type-cycle")
    );
}

#[test]
#[ignore = "requires verified PHP 8.3+, Composer and PHPStan; real installed archive and native consumer"]
fn composer_installed_m2_native_package() {
    let plan = canonical();
    let root = install(&plan, "m2-");
    let consumer = root.join("consumer");
    fs::write(
        consumer.join("positive.php"),
        include_str!("../src/php_sdk/native_m2.php"),
    )
    .unwrap();
    fs::write(consumer.join("phpstan.neon"),"parameters:\n    level: max\n    phpVersion: 80300\n    paths: [positive.php]\n    tmpDir: build/phpstan\n").unwrap();
    checked(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file=vendor/autoload.php",
            ])
            .current_dir(&consumer),
        &root,
        "positive-types",
    );
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .current_dir(&consumer),
        &root,
        "native-models",
    );
    let negatives = [
        ("missing-required", "new FixtureSdk\\WidgetInput();"),
        ("wrong-scalar", "new FixtureSdk\\WidgetInput(name: 42);"),
        (
            "float-number",
            "new FixtureSdk\\WidgetInput(name: 'x', amount: 1.5);",
        ),
        (
            "null-nonnullable",
            "new FixtureSdk\\WidgetPatch(amount: null);",
        ),
        (
            "bad-union",
            "new FixtureSdk\\Widget(amount: FixtureSdk\\JsonNumber::fromInt(1), id: 'w', payload: new FixtureSdk\\WidgetInput(name: 'x'));",
        ),
        (
            "bad-enum",
            "FixtureSdk\\Codecs::encodeStandardPayloadKind('standard');",
        ),
        (
            "required-omission",
            "new FixtureSdk\\WidgetInput(name: FixtureSdk\\Absent::Value);",
        ),
        (
            "missing-operation-path",
            "(new FixtureSdk\\Client(new FixtureSdk\\Credentials([])))->getWidget();",
        ),
    ];
    for (label, statement) in negatives {
        fs::write(
            consumer.join("negative.php"),
            format!("<?php\ndeclare(strict_types=1);\n{statement}\n"),
        )
        .unwrap();
        let output = Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--autoload-file=vendor/autoload.php",
                "negative.php",
            ])
            .current_dir(&consumer)
            .output()
            .unwrap();
        fs::write(
            root.join(format!("negative-{label}.log")),
            [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
        )
        .unwrap();
        assert!(
            !output.status.success(),
            "negative PHPStan case passed: {label}"
        );
    }
    let server = RecordingServer::new(false);
    fs::write(
        consumer.join("wire.php"),
        include_str!("../src/php_sdk/native_http.php"),
    )
    .unwrap();
    checked(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file=vendor/autoload.php",
                "wire.php",
            ])
            .current_dir(&consumer),
        &root,
        "http-positive-types",
    );
    checked(
        Command::new(php())
            .args([
                "-n",
                "-d",
                "error_reporting=-1",
                "-d",
                "zend.exception_ignore_args=0",
                "wire.php",
            ])
            .arg(&server.url)
            .current_dir(&consumer)
            .env("OPENROUTER_API_KEY", "ambient-secret"),
        &root,
        "native-http",
    );
    let seen = server.finish();
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&seen).unwrap(),
    )
    .unwrap();
    let first = seen
        .iter()
        .take(5)
        .map(|r| (r.method.as_str(), r.target.as_str(), r.body.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        first,
        vec![
            ("POST", "/api/v1/widgets", "{\"name\":\"alpha\"}"),
            (
                "GET",
                "/api/v1/widgets?tag=a%2Fb%20%E9%9B%AA&tags=x&tags=y&labels=a%2Cb,c&limit=2",
                ""
            ),
            (
                "GET",
                "/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A",
                ""
            ),
            ("PATCH", "/api/v1/widgets/w1", "{}"),
            ("GET", "/api/v1/widgets/%2E%2E", ""),
        ]
    );
    for r in &seen {
        assert_eq!(
            r.headers.get("authorization").map(String::as_str),
            Some("Bearer test-key")
        );
        assert_eq!(
            r.headers.get("accept-encoding").map(String::as_str),
            Some("identity")
        );
        assert!(!r.headers.contains_key("cookie"));
        assert!(!r.target.contains("should-not-follow"));
    }
}

#[derive(Debug, serde::Serialize)]
struct RequestRecord {
    method: String,
    target: String,
    headers: std::collections::BTreeMap<String, String>,
    body: String,
}
struct RecordingServer {
    url: String,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<RequestRecord>>>,
    thread: Option<thread::JoinHandle<()>>,
    redirect_target: TcpListener,
}
impl RecordingServer {
    fn new(openrouter: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let redirect_target = TcpListener::bind("127.0.0.1:0").unwrap();
        redirect_target.set_nonblocking(true).unwrap();
        let target = redirect_target.local_addr().unwrap();
        let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let done = stop.clone();
        let records = seen.clone();
        let thread = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // macOS accepts inherit the listener's O_NONBLOCK flag.
                        stream.set_nonblocking(false).unwrap();
                        let request = read_request(&mut stream);
                        let (status, headers, body, delay) =
                            response(&request, openrouter, &target.to_string());
                        records.lock().unwrap().push(request);
                        if !delay.is_zero() {
                            thread::sleep(delay);
                        }
                        let raw = format!(
                            "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(raw.as_bytes());
                        if body.starts_with("split:") {
                            // Content-Length includes the sentinel; the client must cancel
                            // the real read before completion rather than accept this JSON.
                            let _ = stream.write_all(b"{");
                            thread::sleep(Duration::from_millis(180));
                            let _ = stream.write_all(body.as_bytes());
                        } else {
                            let _ = stream.write_all(body.as_bytes());
                        }
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        let mut trailing = [0];
                        match stream.read(&mut trailing) {
                            Ok(0) => {}
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::ConnectionReset
                                        | std::io::ErrorKind::ConnectionAborted
                                ) => {}
                            other => panic!(
                                "default transport retained a completed/aborted connection: {other:?}"
                            ),
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("loopback accept: {e}"),
                }
            }
        });
        Self {
            url,
            stop,
            seen,
            thread: Some(thread),
            redirect_target,
        }
    }
    fn finish(mut self) -> Vec<RequestRecord> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        assert!(
            matches!(self.redirect_target.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock),
            "cross-origin redirect was followed"
        );
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}
impl Drop for RecordingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn read_request(stream: &mut TcpStream) -> RequestRecord {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut data = Vec::new();
    let mut byte = [0];
    while !data.ends_with(b"\r\n\r\n") {
        assert!(data.len() < 65_536);
        stream.read_exact(&mut byte).unwrap();
        data.push(byte[0]);
    }
    let text = String::from_utf8(data).unwrap();
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap().split(' ').collect::<Vec<_>>();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let count = headers
        .get("content-length")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(0);
    assert!(count < 65_536);
    let mut body = vec![0; count];
    stream.read_exact(&mut body).unwrap();
    RequestRecord {
        method: first[0].into(),
        target: first[1].into(),
        headers,
        body: String::from_utf8(body).unwrap(),
    }
}
fn response(
    request: &RequestRecord,
    openrouter: bool,
    redirect: &str,
) -> (u16, String, String, Duration) {
    let json="Content-Type: Application/JSON; charset=\"utf-8\"; profile=\"fixture;test\"\r\nSet-Cookie: ambient=must-not-replay\r\n".to_owned();
    if openrouter {
        let fixtures: Value =
            serde_json::from_str(include_str!("fixtures/openrouter-five-responses.json")).unwrap();
        let key = if request.method == "POST" {
            "create"
        } else if request.method == "PATCH" {
            "update"
        } else if request.target.ends_with("/credits") {
            "credits"
        } else if request.target.contains('?') {
            "list"
        } else {
            "file"
        };
        return (
            if key == "create" { 201 } else { 200 },
            json,
            fixtures[key].as_str().unwrap().into(),
            Duration::ZERO,
        );
    }
    const WIDGET: &str = "{\"id\":\"w1\",\"amount\":9007199254740993.000000000000000001,\"meta\":null,\"payload\":{\"kind\":\"standard\",\"text\":\"plain\"},\"child\":{\"label\":\"root\"}}";
    let path = request.target.as_str();
    let normal = if path.contains('?') {
        format!("{{\"items\":[{WIDGET}]}}")
    } else {
        WIDGET.into()
    };
    if request.body == "{\"name\":\"deny\"}" {
        return (
            422,
            json,
            "{\"message\":\"denied-private-body\"}".into(),
            Duration::ZERO,
        );
    }
    if path.ends_with("redirect") {
        return (
            307,
            format!("{json}Location: http://{redirect}/should-not-follow\r\n"),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("media") {
        return (
            200,
            "Content-Type: text/plain\r\n".into(),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("duplicate") {
        return (
            200,
            format!("{json}Content-Type: application/json\r\n"),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("charset") {
        return (
            200,
            "Content-Type: application/json; charset=iso-8859-1\r\n".into(),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("encoding") {
        return (
            200,
            format!("{json}Content-Encoding: gzip\r\n"),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("invalid") {
        return (200, json, "{\"amount\":true}".into(), Duration::ZERO);
    }
    if path.ends_with("headers") {
        return (
            200,
            format!("{json}X-Large: {}\r\n", "h".repeat(600)),
            normal,
            Duration::ZERO,
        );
    }
    if path.ends_with("timeout") {
        return (200, json, normal, Duration::from_millis(140));
    }
    if path.ends_with("split") {
        return (200, json, "split:slow-body".into(), Duration::ZERO);
    }
    (200, json, normal, Duration::ZERO)
}

#[test]
#[ignore = "requires verified PHP; all 17 independently specified OwnedProgram vectors"]
fn native_shared_owned_program_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 17);
    let schemas = cases
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("Case{i:02}"), c["schema"].clone()))
        .collect::<serde_json::Map<_, _>>();
    let contract = schema_document(Value::Object(schemas));
    let compiled = suspect_schema::OwnedCompiler::new(config().validation)
        .compile(contract.clone(), contract.schema_roots())
        .unwrap();
    let program = compiled.program();
    let root = root("vectors-");
    write_files(
        suspect_codegen::php_sdk::emit_validation(&program, &config()).unwrap(),
        &root,
    );
    let mut script = String::from(
        "<?php\ndeclare(strict_types=1);\nnamespace FixtureSdk;\nforeach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }\n",
    );
    let mut count = 0;
    for (i, case) in cases.iter().enumerate() {
        let target = program
            .roots
            .iter()
            .find(|r| r.source.pointer == format!("/components/schemas/Case{i:02}"))
            .unwrap()
            .target;
        for expectation in ["valid", "invalid"] {
            for input in case[expectation].as_array().unwrap() {
                let text = input.as_str().unwrap();
                let literal = php_literal(text);
                script.push_str(&format!("$ok = true; try {{ Validator::validate({target}, JsonValue::parse({literal})); }} catch (ValidationError $e) {{ if ($e->kind !== 'invalid') {{ throw $e; }} $ok = false; }} if ($ok !== {}) {{ throw new \\RuntimeException('vector {i} mismatch'); }}\n",expectation=="valid"));
                count += 1;
            }
        }
    }
    script.push_str(&format!(
        "echo '17 schemas / {count} independent validation vectors passed', PHP_EOL;\n"
    ));
    fs::write(root.join("consumer.php"), script).unwrap();
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "consumer.php"])
            .current_dir(&root),
        &root,
        "native-vectors",
    );
}

fn php_literal(text: &str) -> String {
    // Test strings use noninterpolating PHP literals; neither PHP source nor schema code is parsed.
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[test]
#[ignore = "requires original tracked OpenRouter source and verified native Composer/PHPStan tools"]
fn composer_installed_actual_openrouter_five_operations() {
    use sha2::{Digest, Sha256};
    let path = PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT")
            .expect("set OPENROUTER_WEB_ROOT to the read-only original source"),
    )
    .join("projects/docs/openapi/openapi.yaml");
    let before = fs::read(&path).unwrap();
    let contract = load(&path);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<SourceId>>();
    assert_eq!(selected.len(), 5);
    let plan = plan_sdk(contract, &selected, config()).unwrap();
    let body_name = |operation: &str| {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.id == operation)
            .unwrap();
        let suspect_codegen::php_sdk::protocol::Payload::Schema(schema) = &op.body[0].payload
        else {
            panic!("expected source JSON request body")
        };
        plan.models().type_name(schema, false)
    };
    let root = install(&plan, "openrouter-");
    fs::write(root.join("source-provenance.json"),serde_json::to_string_pretty(&json!({"path":path,"bytes":before.len(),"sha256":format!("{:x}",Sha256::digest(&before)),"operationIds":wanted})).unwrap()).unwrap();
    let consumer = root.join("consumer");
    let script = include_str!("../src/php_sdk/native_openrouter.php")
        .replace("__CREATE__", &body_name("createKeys"))
        .replace("__UPDATE__", &body_name("updateKeys"));
    fs::write(consumer.join("positive.php"), script).unwrap();
    fs::write(consumer.join("phpstan.neon"),"parameters:\n    level: max\n    phpVersion: 80300\n    paths: [positive.php]\n    tmpDir: build/phpstan\n").unwrap();
    checked(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file=vendor/autoload.php",
            ])
            .current_dir(&consumer),
        &root,
        "positive-types",
    );
    let server = RecordingServer::new(true);
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .arg(&server.url)
            .current_dir(&consumer),
        &root,
        "native-five-operations",
    );
    let seen = server.finish();
    fs::write(
        root.join("wire-records.json"),
        serde_json::to_string_pretty(&seen).unwrap(),
    )
    .unwrap();
    let wire = seen
        .iter()
        .map(|r| (r.method.as_str(), r.target.as_str(), r.body.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        wire,
        vec![
            ("GET", "/api/v1/credits", ""),
            (
                "POST",
                "/api/v1/keys",
                "{\"limit\":50.25,\"limit_reset\":null,\"name\":\"Native Test Key\"}"
            ),
            (
                "PATCH",
                "/api/v1/keys/fixture-hash",
                "{\"disabled\":true,\"limit\":75.50,\"limit_reset\":null,\"name\":\"Updated Native Key\"}"
            ),
            (
                "GET",
                "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA",
                ""
            ),
            (
                "GET",
                "/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA",
                ""
            ),
        ]
    );
    for r in &seen {
        assert_eq!(
            r.headers.get("authorization").map(String::as_str),
            Some("Bearer test-key")
        );
        assert!(!r.headers.contains_key("cookie"));
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "tracked source was modified during native work"
    );
}

#[test]
fn unsupported_native_shapes_and_dialects_never_become_json_fallbacks() {
    use suspect_codegen::php_sdk::models::{Extras, Shape};
    use suspect_schema::{OwnedProgram, ProgramInstruction, ProgramType};

    for (schema, code, pointer) in [
        (
            json!({"type":"array","prefixItems":[{"type":"string"}]}),
            "php-tuple-native-unsupported",
            "/components/schemas/Root/prefixItems",
        ),
        (
            json!({"type":"object","allOf":[{"type":"object"},{"type":"object"}]}),
            "php-allof-native-unsupported",
            "/components/schemas/Root/allOf",
        ),
        (
            json!({"properties":{"typed":{"type":"string"}}}),
            "php-conditional-shape-unsupported",
            "/components/schemas/Root",
        ),
        (
            json!({"const":{"typed":1}}),
            "php-structural-literal-unsupported",
            "/components/schemas/Root",
        ),
        (
            json!({"type":"object","properties":{"secret":{"type":"string","writeOnly":true}}}),
            "php-protocol-representation",
            "/components/schemas/Root/properties/secret/writeOnly",
        ),
        (
            json!({"type":"object","patternProperties":{"[":{"type":"string"}}}),
            "php-schema-compilation",
            "/components/schemas/Root/patternProperties/[",
        ),
    ] {
        let errors =
            plan_document(&api_document(json!({"Root":schema}), "Root"), config()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == code
                && error.source.pointer() == pointer
                && !error.at.is_empty()),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|e| e.source.pointer().starts_with("/components/schemas/Root"))
        );
    }

    // Pattern properties are now a verified scoped profile. Keep this exact
    // former refusal as a positive native-object and source-codec obligation.
    let plan = plan_document(
        &api_document(
            json!({"Root":{"type":"object","patternProperties":{"^x":{"type":"string"}}}}),
            "Root",
        ),
        config(),
    )
    .unwrap();
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    assert_eq!(plan.program().profile, OwnedProgram::V2_PROFILE);
    assert_eq!(plan.program().check(), Ok(()));
    let root = plan
        .models()
        .nodes
        .values()
        .find(|node| node.source.pointer() == "/components/schemas/Root")
        .unwrap();
    let Shape::Object {
        fields,
        extras: Extras::Patterned(patterns),
    } = &root.shape
    else {
        panic!(
            "pattern properties must retain a native object and checked extras: {:?}",
            root.shape
        );
    };
    assert!(fields.is_empty());
    let pattern = root.source.child("patternProperties").child("^x");
    assert_eq!(patterns, &[("^x".into(), pattern.clone())]);
    assert_eq!(plan.models().type_name(&root.source, false), root.name);
    let constructor = root.constructor.as_ref().unwrap();
    assert!(constructor.parameters.is_empty());
    assert_eq!(constructor.extra_parameter.as_deref(), Some("extra"));
    assert_eq!(plan.models().type_name(&pattern, false), "string");
    let node = plan
        .program()
        .nodes
        .iter()
        .find(|node| node.source.pointer == root.source.pointer())
        .unwrap();
    let check = node
        .checks
        .iter()
        .find(|check| {
            matches!(
                check.instruction,
                ProgramInstruction::PatternProperties { .. }
            )
        })
        .unwrap();
    assert_eq!(
        check.source.pointer,
        "/components/schemas/Root/patternProperties"
    );
    let ProgramInstruction::PatternProperties { patterns } = &check.instruction else {
        unreachable!()
    };
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].0, "^x");
    let target = &plan.program().nodes[patterns[0].2];
    assert_eq!(target.source.pointer, pattern.pointer());
    assert!(target.checks.iter().any(|check| matches!(&check.instruction, ProgramInstruction::Type { types } if types == &[ProgramType::String])));

    let mut value = api_document(json!({"Root":{"type":"object"}}), "Root");
    value["openapi"] = json!("3.0.3");
    assert!(plan_document(&value, config()).is_err());
    for namespace in [
        "__NAMESPACE__",
        "class",
        "Unsafe\\Enum",
        "\\Leading",
        "Trailing\\",
    ] {
        assert!(
            plan_document(
                &api_document(json!({"Root":true}), "Root"),
                PhpConfig {
                    namespace: namespace.into(),
                    ..config()
                }
            )
            .is_err()
        );
    }
}

#[test]
#[ignore = "requires verified PHP/Composer/PHPStan; mixed unions, parent assertions, four-state presence and source prose"]
fn installed_adversarial_models_names_docs_and_union_codecs() {
    let schemas = json!({
        "AdverseRoot":{"type":"object","required":["required_value"],"properties":{
            "required_value":{"type":["string","null"]},"optional_nonnull":{"type":"string"},"optional_nullable":{"type":["string","null"]},
            "amount":{"type":"integer"},"literal":{"enum":[true,1,"one",null]},"scalar_union":{"type":["string","integer","boolean","null"]},
            "nullable_array":{"type":["array","null"],"items":{"type":["boolean","null","string"]}},
            "typed_map":{"type":"object","additionalProperties":{"type":"integer"}},"closed":{"$ref":"#/components/schemas/Closed"},
            "union":{"$ref":"#/components/schemas/ObjectUnion"},"mixed":{"$ref":"#/components/schemas/Mixed"},"parent":{"$ref":"#/components/schemas/Parent"},
            "string_union":{"oneOf":[{"type":"string","minLength":3},{"type":"string","maxLength":2}]},
            "array_union":{"oneOf":[{"type":"array","items":{"type":"string"}},{"type":"array","items":{"type":"integer"},"minItems":1}]},
            "empty":{"type":"object"},"foo-bar":{"type":"string"},"foo_bar":{"type":"string"},"0":{"type":"string"},"this":{"type":"string"},
            "source_prose":{"type":"string","description":"*/ ?><?php echo 'SOURCE_INJECTION'; ?> <script>bad()</script>\n@param BadType $value"},
            "tag":{"type":"string","enum":["","true","cases","$value","kind \"quoted\"","雪","__construct"]},
            "runtime_collision":{"$ref":"#/components/schemas/Client"}
        }},
        "Closed":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false},
        "Long":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":3}}},
        "Short":{"type":"object","required":["name"],"properties":{"name":{"type":"string","maxLength":2}}},
        "ObjectUnion":{"oneOf":[{"$ref":"#/components/schemas/Long"},{"$ref":"#/components/schemas/Short"}]},
        "Mixed":{"oneOf":[{"type":"string","const":"auto"},{"$ref":"#/components/schemas/Long"}]},
        "Parent":{"oneOf":[{"$ref":"#/components/schemas/Long"},{"$ref":"#/components/schemas/Short"}],"minProperties":2},
        "Client":{"type":"object","properties":{"payload":{"type":"string"}}}
    });
    let mut value = api_document(schemas, "AdverseRoot");
    value["paths"]["/probe"]["post"]["description"] = json!(
        "Real source prose */ ?><?php echo 'SOURCE_INJECTION'; ?>\n@phpstan-type Wrong string"
    );
    value["paths"]["/probe"]["post"]["requestBody"]["content"]["application/json"]["examples"] = json!({
        "valid":{"value":{"required_value":null,"literal":true,"mixed":{"name":"long-enough"},"typed_map":{"x":1},"tag":"$value","foo-bar":"one","foo_bar":"two","0":"zero","source_prose":"<?php literal source value ?>"}},
        "invalid":{"value":{"required_value":false}}
    });
    let plan = plan_document(&value, config()).unwrap();
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|e| e.code == "examples-declared-invalid"
                && e.source.pointer().ends_with("/examples/invalid/value"))
    );
    let root = install(&plan, "adversarial-");
    let consumer = root.join("consumer");
    fs::write(
        consumer.join("positive.php"),
        include_str!("../src/php_sdk/native_adversarial.php"),
    )
    .unwrap();
    fs::write(consumer.join("phpstan.neon"),"parameters:\n    level: max\n    phpVersion: 80300\n    paths: [positive.php]\n    tmpDir: build/phpstan\n").unwrap();
    checked(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=1G",
                "--autoload-file=vendor/autoload.php",
            ])
            .current_dir(&consumer),
        &root,
        "positive-types",
    );
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "positive.php"])
            .current_dir(&consumer),
        &root,
        "native-adversarial",
    );
    fs::write(consumer.join("negative.php"),"<?php\ndeclare(strict_types=1);\nnew FixtureSdk\\AdverseRoot();\nnew FixtureSdk\\AdverseRoot(requiredValue: FixtureSdk\\Absent::Value);\nnew FixtureSdk\\AdverseRoot(requiredValue: null, optionalNonnull: null);\n").unwrap();
    let output = Command::new(php())
        .arg("-n")
        .arg(phpstan())
        .args([
            "analyse",
            "--no-progress",
            "--autoload-file=vendor/autoload.php",
            "--error-format=json",
            "negative.php",
        ])
        .current_dir(&consumer)
        .output()
        .unwrap();
    fs::write(root.join("negative-presence.json"), &output.stdout).unwrap();
    assert!(!output.status.success());
    let results: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(results["totals"]["file_errors"], 3);
}

#[test]
#[ignore = "requires verified PHP; independent exact rational arithmetic and symbolic exponent cases"]
fn native_exact_numeric_math_and_json_boundaries() {
    let contract = schema_document(json!({"Root":true}));
    let compiled = suspect_schema::OwnedCompiler::new(config().validation)
        .compile(contract.clone(), contract.schema_roots())
        .unwrap();
    let root = root("exact-math-");
    write_files(
        suspect_codegen::php_sdk::emit_validation(&compiled.program(), &config()).unwrap(),
        &root,
    );
    let mut script = String::from(
        "<?php\ndeclare(strict_types=1);\nnamespace FixtureSdk;\nforeach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }\nfunction check(bool $ok): void { if (!$ok) { throw new \\RuntimeException('independent exact arithmetic mismatch'); } }\n$cases = [\n",
    );
    let mut state = 0x3678e9cabc1u64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let fraction = |coefficient: i128, exponent: i32| {
        if exponent >= 0 {
            (coefficient * 10i128.pow(exponent as u32), 1)
        } else {
            (coefficient, 10i128.pow((-exponent) as u32))
        }
    };
    for _ in 0..10_000 {
        let a = (next() % 2_000_001) as i128 - 1_000_000;
        let b = (next() % 1_000_000 + 1) as i128;
        let ae = (next() % 17) as i32 - 8;
        let be = (next() % 17) as i32 - 8;
        let at = format!(
            "{a}e{}{:0width$}",
            if ae < 0 { "-" } else { "+" },
            ae.unsigned_abs(),
            width = (next() % 32 + 1) as usize
        );
        let bt = format!(
            "{b}e{}{:0width$}",
            if be < 0 { "-" } else { "+" },
            be.unsigned_abs(),
            width = (next() % 32 + 1) as usize
        );
        let (an, ad) = fraction(a, ae);
        let (bn, bd) = fraction(b, be);
        let order = match (an * bd).cmp(&(bn * ad)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        script.push_str(&format!(
            "[{}, {}, {order}, {}, {}],\n",
            php_literal(&at),
            php_literal(&bt),
            an % ad == 0,
            (an * bd) % (ad * bn) == 0
        ));
    }
    script.push_str("];\nforeach ($cases as [$left, $right, $order, $integer, $multiple]) {\n    $a = JsonNumber::fromString($left); $b = JsonNumber::fromString($right);\n    check($a->compare($b) === $order); check($a->isInteger() === $integer);\n    $work = 100000;\n    check($a->multipleOf($b, static function (int $cost) use (&$work): void { $work -= $cost; if ($work < 0) { throw new \\RuntimeException('numeric budget'); } }) === $multiple);\n}\n");
    script.push_str(r#"
function limited(\Closure $action): void { try { $action(); } catch (JsonError $e) { check($e->kind === 'resource_limit'); return; } throw new \RuntimeException('resource ceiling bypassed'); }
check(JsonValue::fromNumber(JsonNumber::fromInt(8))->toJson(new JsonLimits(maxBytes: 1)) === '8');
check(JsonValue::fromString("\n")->toJson(new JsonLimits(maxBytes: 4)) === '"\\n"');
limited(static fn () => JsonValue::fromString("\n")->toJson(new JsonLimits(maxBytes: 3)));
check(JsonValue::parse('[0,1]', new JsonLimits(maxNodes: 3))->toJson() === '[0,1]');
limited(static fn () => JsonValue::parse('[0,1]', new JsonLimits(maxNodes: 2)));
limited(static fn () => JsonValue::parse('[0]', new JsonLimits(maxDepth: 1)));
check(JsonValue::parse('[]', new JsonLimits(maxDepth: 1))->toJson() === '[]');
check(JsonValue::parse('{"0":1,"00":2,"01":3,"-0":4,"9223372036854775808":5}')->toJson() === '{"-0":4,"0":1,"00":2,"01":3,"9223372036854775808":5}');
check(JsonValue::parse('"\uD834\uDD1E"')->asString() === '𝄞');
limited(static fn () => JsonNumber::fromString(str_repeat('9', 65537)));
$huge = JsonNumber::fromString('1e' . str_repeat('9', 1024));
check($huge->isInteger()); limited(static fn () => $huge->toDecimalString(32));
check(JsonNumber::fromString('-0e-' . str_repeat('9', 1024))->toInt() === 0);
echo '10000 independent exact rational comparisons/integrality/divisibility cases and JSON boundary checks passed', PHP_EOL;
"#);
    fs::write(root.join("consumer.php"), script).unwrap();
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "consumer.php"])
            .current_dir(&root),
        &root,
        "native-exact-math",
    );
}

#[test]
#[ignore = "requires verified PHP; source-compiled hostile composition and shared codec resource budgets"]
fn native_resource_failures_survive_branches_and_conversion_trials() {
    let root = root("budgets-");
    let mut script = String::from("<?php\ndeclare(strict_types=1);\n");
    for (index, schema, mut limits, input) in [
        (
            0,
            json!({"anyOf":[true,{"$ref":"#/components/schemas/Root"}]}),
            config().validation,
            "null",
        ),
        (
            1,
            json!({"not":{"type":"integer"}}),
            config().validation,
            "1e999999999999",
        ),
        (
            2,
            json!({"anyOf":[true,{"type":"integer"}]}),
            config().validation,
            "1e999999999999",
        ),
        (3, json!({"not":{"const":1}}), config().validation, "1"),
        (
            4,
            json!({"allOf":[false,{"anyOf":[true,true,true,true]}]}),
            config().validation,
            "null",
        ),
    ] {
        if index == 1 || index == 2 {
            limits.max_number_bytes = 8;
        }
        if index == 3 {
            limits.max_equality_steps = 0;
        }
        if index == 4 {
            limits.max_evaluation_steps = 5;
        }
        let contract = schema_document(json!({"Root":schema}));
        let compiled = suspect_schema::OwnedCompiler::new(limits)
            .compile(contract.clone(), contract.schema_roots())
            .unwrap();
        let program = compiled.program();
        let target = program.roots[0].target;
        let namespace = format!("Budget{index}");
        let config = PhpConfig {
            namespace: namespace.clone(),
            ..config()
        };
        let directory = root.join(format!("case{index}"));
        write_files(
            suspect_codegen::php_sdk::emit_validation(&program, &config).unwrap(),
            &directory,
        );
        script.push_str(&format!("foreach (glob(__DIR__ . '/case{index}/php/src/*.php') as $file) {{ require_once $file; }}\ntry {{ {namespace}\\Validator::validate({target}, {namespace}\\JsonValue::parse({})); throw new \\RuntimeException('incomplete evaluation became a result'); }} catch ({namespace}\\ValidationError $e) {{ if ($e->kind !== 'evaluation_failure' || !str_contains($e->source, '#/components/schemas/Root')) {{ throw $e; }} }}\n",php_literal(input)));
    }
    // Decided enum membership and duplicate detection do not visit later
    // operands. This is distinct from schema composition's failure dominance.
    for (index, schema, input, valid) in [
        (
            5,
            json!({"enum":[1,serde_json::from_str::<Value>("1e999999999999").unwrap()]}),
            "1",
            true,
        ),
        (
            6,
            json!({"uniqueItems":true}),
            "[1,1,1e999999999999]",
            false,
        ),
        (
            7,
            json!({"enum":[1,serde_json::from_str::<Value>(&format!("1e{}", "9".repeat(65_537))).unwrap()]}),
            "1",
            true,
        ),
    ] {
        let mut limits = config().validation;
        limits.max_number_bytes = 8;
        limits.max_equality_steps = 1;
        let contract = schema_document(json!({"Root":schema}));
        let compiled = suspect_schema::OwnedCompiler::new(limits)
            .compile(contract.clone(), contract.schema_roots())
            .unwrap();
        let program = compiled.program();
        let target = program.roots[0].target;
        let namespace = format!("Budget{index}");
        let configuration = PhpConfig {
            namespace: namespace.clone(),
            ..config()
        };
        write_files(
            suspect_codegen::php_sdk::emit_validation(&program, &configuration).unwrap(),
            &root.join(format!("case{index}")),
        );
        script.push_str(&format!("foreach (glob(__DIR__ . '/case{index}/php/src/*.php') as $file) {{ require_once $file; }}\n$valid = true; try {{ {namespace}\\Validator::validate({target}, {namespace}\\JsonValue::parse({})); }} catch ({namespace}\\ValidationError $error) {{ if ($error->kind !== 'invalid') {{ throw $error; }} $valid = false; }} if ($valid !== {}) {{ throw new \\RuntimeException('unvisited operand changed the outcome'); }}\n", php_literal(input), valid));
    }
    let document = api_document(
        json!({"Root":{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"generic":true}}}),
        "Root",
    );
    let bounded = plan_document(
        &document,
        PhpConfig {
            namespace: "SharedBudget".into(),
            max_conversion_bytes: 128,
            ..config()
        },
    )
    .unwrap();
    write_files(bounded.render(), &root.join("conversion"));
    script.push_str(r#"
foreach (glob(__DIR__ . '/conversion/php/src/*.php') as $file) { require_once $file; }
$part = SharedBudget\JsonValue::fromString(str_repeat('x', 60));
$native = new SharedBudget\Root(name: 'ok', generic: SharedBudget\JsonValue::fromObject(['a' => $part, 'b' => $part, 'c' => $part]));
try { $native->toJson(); throw new \RuntimeException('generic JSON bypassed shared conversion bytes'); }
catch (SharedBudget\JsonError $error) { if ($error->kind !== 'resource_limit') { throw $error; } }
if ((new SharedBudget\Root(name: 'ok'))->toJson() !== '{"name":"ok"}') { throw new \RuntimeException('budget leaked between calls'); }
$native = new SharedBudget\Root(name: 'ok', extra: ['a' => $part, 'b' => $part, 'c' => $part]);
try { $native->toJson(); throw new \RuntimeException('JSON extras reset conversion bytes'); }
catch (SharedBudget\JsonError $error) { if ($error->kind !== 'resource_limit') { throw $error; } }
echo 'shared schema/numeric/equality/recursion and native conversion budget cases passed', PHP_EOL;
"#);
    fs::write(root.join("consumer.php"), script).unwrap();
    checked(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "consumer.php"])
            .current_dir(&root),
        &root,
        "native-budgets",
    );
}
