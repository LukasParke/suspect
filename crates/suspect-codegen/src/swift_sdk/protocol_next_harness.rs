//! Shared native gate driver; expected wire values are literal protocol fixtures.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn checked(c: &mut Command, root: &Path) {
    let o = c.output().expect("native tool required");
    assert!(
        o.status.success(),
        "{}\n{c:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    if !o.stdout.is_empty() {
        println!("{}", String::from_utf8_lossy(&o.stdout));
    }
}
fn driver() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/bin/swift".into())
}
fn compiler() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFTC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if driver() != Path::new("/usr/bin/swift") {
                return driver().with_file_name("swiftc");
            }
            let o = Command::new("xcrun")
                .args(["--find", "swiftc"])
                .output()
                .unwrap();
            assert!(o.status.success());
            PathBuf::from(String::from_utf8(o.stdout).unwrap().trim())
        })
}
fn sdk() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_SDKROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let o = Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .unwrap();
            assert!(o.status.success());
            PathBuf::from(String::from_utf8(o.stdout).unwrap().trim())
        })
}
fn swift(action: &str) -> Command {
    let mut c = Command::new(driver());
    c.arg(action)
        .env("SWIFT_EXEC", compiler())
        .arg("--sdk")
        .arg(sdk())
        .env("SDKROOT", sdk());
    if action == "test" {
        c.arg("--disable-swift-testing");
    }
    c
}
fn directory(root: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()? {
        let p = e.ok()?.path();
        if p.is_dir() {
            if p.file_name()?.to_str()? == name {
                return Some(p);
            }
            if let Some(d) = directory(&p, name) {
                return Some(d);
            }
        }
    }
    None
}

pub fn run(write_sdk: impl FnOnce(&Path), native: &str, label: &str) -> PathBuf {
    let base = std::env::var_os("SUSPECT_SWIFT_NEXT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-next-gates"));
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap();
    write_sdk(&root.join("sdk"));
    checked(
        swift("test")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    std::fs::create_dir_all(root.join("consumer/Tests/NextConsumer")).unwrap();
    std::fs::write(root.join("consumer/Package.swift"),"// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"NextConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"NextConsumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(
        root.join("consumer/Tests/NextConsumer/NextTests.swift"),
        native,
    )
    .unwrap();
    let server = Server::start(&root);
    checked(
        Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-keyout",
            ])
            .arg(root.join("key.pem"))
            .arg("-out")
            .arg(root.join("cert.pem")),
        &root,
    );
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut tls = Command::new("openssl")
        .args(["s_server", "-quiet", "-www", "-accept"])
        .arg(format!("127.0.0.1:{port}"))
        .arg("-key")
        .arg(root.join("key.pem"))
        .arg("-cert")
        .arg(root.join("cert.pem"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(root.join("tls-server.log")).unwrap())
        .spawn()
        .unwrap();
    let address = format!("127.0.0.1:{port}").parse().unwrap();
    let mut ready = false;
    for _ in 0..100 {
        if TcpStream::connect_timeout(&address, Duration::from_millis(20)).is_ok() {
            ready = true;
            break;
        }
        assert!(
            tls.try_wait().unwrap().is_none(),
            "TLS fixture exited: {}",
            std::fs::read_to_string(root.join("tls-server.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready, "TLS fixture did not listen");
    let output = swift("test")
        .arg("--package-path")
        .arg(root.join("consumer"))
        .arg("--scratch-path")
        .arg(root.join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .env("SUSPECT_SWIFT_NEXT_BASE", &server.base)
        .env(
            "SUSPECT_SWIFT_NEXT_TLS",
            format!("https://localhost:{port}"),
        )
        .env("SUSPECT_SWIFT_NEXT_MARKERS", &root)
        .output()
        .unwrap();
    let _ = tls.kill();
    let _ = tls.wait();
    assert!(
        output.status.success(),
        "{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    for name in [
        "method-COPY",
        "method-MiXeD",
        "method-x-PING",
        "method-get",
        "method-GeT",
        "method-head",
        "method-pOsT",
        "json-query",
        "text-query",
        "form-query",
        "ordered",
        "ordered-form",
        "empty",
        "barrier",
        "extended",
        "reset",
        "exact-chunked",
        "exact-close",
        "exact-continue",
        "exact-cancel",
        "stream-break",
        "stream-cancel",
    ] {
        assert!(root.join(name).is_file(), "missing wire witness {name}");
    }
    typechecks(&root);
    docs(&root);
    println!(
        "Swift remaining standard protocol gate passed: {}",
        root.display()
    );
    root
}
fn docs(root: &Path) {
    checked(
        swift("package")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["dump-symbol-graph", "--minimum-access-level", "public"]),
        root,
    );
    let mut docc = if let Some(binary) = std::env::var_os("SUSPECT_SWIFT_DOCC_BIN") {
        Command::new(binary)
    } else {
        let mut c = Command::new("xcrun");
        c.arg("docc");
        c
    };
    checked(
        docc.arg("convert")
            .arg(root.join("sdk/Sources/GeneratedSDK/GeneratedSDK.docc"))
            .arg("--additional-symbol-graph-dir")
            .arg(directory(&root.join("build/sdk"), "symbolgraph").unwrap())
            .arg("--output-path")
            .arg(root.join("GeneratedSDK.doccarchive"))
            .arg("--warnings-as-errors"),
        root,
    );
    assert!(root.join("GeneratedSDK.doccarchive/index.html").is_file());
}
fn typechecks(root: &Path) {
    let modules = directory(&root.join("build/sdk"), "Modules").unwrap();
    let common = |path: &Path| {
        let mut c = Command::new(compiler());
        c.args(["-typecheck", "-swift-version", "6", "-sdk"])
            .arg(sdk())
            .arg("-module-cache-path")
            .arg(root.join("frontend-cache"))
            .arg("-I")
            .arg(&modules)
            .arg(path);
        c
    };
    let path = root.join("positive.swift");
    std::fs::write(&path,"import Foundation\nimport GeneratedSDK\nfunc probe(_ client: Client) async throws { let _: String = try await client.lowerHead().data.value; let _ = WholeJSONInput(criteria: Query(q: \"x\"), id: \"x\"); let _ = WholeFormInput(id: \"x\", fields: FormFields(bar: true, foo: \"x\")); let _ = EmptyPartsMultipartBody(); let _ = BarrierMultipartBody(part1: .value(HTTPPart(\"x\"))) }\n").unwrap();
    checked(common(&path).arg("-warnings-as-errors"), root);
    for(i,text)in[
        "let _ = WholeJSONInput(criteria: \"raw JSON\", id: \"x\")",
        "let _ = WholeFormInput(id: \"x\", fields: \"foo=a&bar=true\")",
        "let _ = SendOrderedMultipartBody()",
        "let _ = SendOrderedMultipartBody(part1: HTTPPart(\"untyped\"), part2: HTTPPart(Data()))",
        "let _ = BarrierMultipartBody(items: [HTTPPart(1)])",
        "let _ = EmptyPartsMultipartBody(part1: HTTPPart(Data()))",
        "func fail(_ client: Client) async throws { let _: String = try await client.sendOrdered(SendOrderedInput(body: .init())).data.part2.value }",
    ].iter().enumerate(){let path=root.join(format!("negative-{i}.swift"));std::fs::write(&path,format!("import Foundation\nimport GeneratedSDK\n{text}\n")).unwrap();let o=common(&path).output().unwrap();assert!(!o.status.success(),"negative compiled: {text}");let e=String::from_utf8_lossy(&o.stderr);assert!(!e.contains("no such module")&&!e.contains("unable to load standard library"),"{e}");}
}
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start(root: &Path) -> Self {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        l.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let root = root.to_owned();
        let join = thread::spawn(move || {
            let mut tasks = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((s, _)) => {
                        let root = root.clone();
                        tasks.push(thread::spawn(move || serve(s, &root)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for t in tasks {
                t.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            join: Some(join),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let r = join.join();
            if !thread::panicking() {
                r.unwrap();
            }
        }
    }
}
fn serve(mut socket: TcpStream, root: &Path) {
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).ok() != Some(1) {
            return;
        }
        head.push(byte[0]);
        assert!(head.len() <= 65_536);
    }
    let text = String::from_utf8(head).unwrap();
    let mut first = text.lines().next().unwrap().split(' ');
    let method = first.next().unwrap();
    let target = first.next().unwrap();
    let header = |name: &str| {
        text.lines()
            .skip(1)
            .filter_map(|s| s.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    };
    let count = header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(count <= 32_768);
    let mut body = vec![0; count];
    socket.read_exact(&mut body).unwrap();
    let mark = |name: &str| std::fs::write(root.join(name), target).unwrap();
    if target == "/base/case" {
        assert!(["COPY", "MiXeD", "x-PING", "get", "GeT", "head", "pOsT"].contains(&method));
        if method == "pOsT" {
            assert_eq!(body, br#"{"q":"payload"}"#);
        }
        mark(&format!("method-{method}"));
        reply(socket, method);
        return;
    }
    if target.starts_with("/base/query/") {
        assert!(["GET", "QUERY"].contains(&method));
        assert!(header("content-type").is_none());
        mark(if target.contains("/json/") {
            "json-query"
        } else if target.contains("/form/") {
            "form-query"
        } else {
            "text-query"
        });
        reply(socket, target);
        return;
    }
    if [
        "/base/ordered",
        "/base/ordered-form",
        "/base/empty",
        "/base/barrier",
        "/base/extended",
    ]
    .contains(&target)
    {
        assert_eq!(method, "POST");
        mark(target.rsplit('/').next().unwrap());
        let media = header("content-type").unwrap();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).unwrap();
        socket.write_all(&body).unwrap();
        return;
    }
    if target == "/base/reset" {
        assert_eq!(method, "get");
        mark("reset");
        socket.write_all(b"HTTP/1.1 205 Reset Content\r\nX-Reset: 1\r\nContent-Length: 9999999\r\nConnection: close\r\n\r\n").unwrap();
        return;
    }
    assert_eq!(method, "get");
    if target.starts_with("/base/exact-events/") {
        let mode = target.rsplit('/').next().unwrap();
        assert!(["break", "cancel"].contains(&mode));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        let value = if mode == "break" {
            "data: first\n\n"
        } else {
            ": ready\n\n"
        };
        chunk(&mut socket, value.as_bytes());
        closed(&mut socket);
        mark(&format!("stream-{mode}"));
        return;
    }
    let mode = target.strip_prefix("/base/exact/").unwrap();
    match mode {
        "chunked" => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            chunk(&mut socket, b"{\"value\":");
            chunk(&mut socket, b"\"chunked\"}");
            socket.write_all(b"0\r\nX-Final: yes\r\n\r\n").unwrap();
            mark("exact-chunked");
        }
        "close" => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"value\":\"close\"}").unwrap();
            mark("exact-close");
        }
        "continue" => {
            socket.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
            mark("exact-continue");
            reply(socket, "continue");
        }
        "redirect" => {
            socket
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: /never\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        }
        "large" => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 9999999\r\n\r\n").unwrap();
        }
        "ambiguous" => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n").unwrap();
        }
        "timeout" | "cancel" => {
            closed(&mut socket);
            mark(&format!("exact-{mode}"));
        }
        _ => panic!("unexpected request: {method} {target}"),
    }
}
fn reply(mut s: TcpStream, value: &str) {
    let body = serde_json::json!({"value":value}).to_string();
    s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
}
fn chunk(s: &mut TcpStream, value: &[u8]) {
    s.write_all(format!("{:x}\r\n", value.len()).as_bytes())
        .unwrap();
    s.write_all(value).unwrap();
    s.write_all(b"\r\n").unwrap();
}
fn closed(s: &mut TcpStream) {
    let mut b = [0];
    match s.read(&mut b) {
        Ok(0) => (),
        Err(e)
            if [
                std::io::ErrorKind::ConnectionReset,
                std::io::ErrorKind::BrokenPipe,
            ]
            .contains(&e.kind()) => {}
        other => panic!("connection not released: {other:?}"),
    }
}
