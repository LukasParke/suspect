//! Independent install/tool/socket helpers for the focused Dart v2 gates.
#![allow(dead_code)] // Different gates use different parts of this fixture seam.
use super::codegen;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

pub fn repository() -> PathBuf {
    std::env::var_os("SUSPECT_DART_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
        })
}
pub fn root(prefix: &str) -> PathBuf {
    let path = std::env::var_os("SUSPECT_DART_GATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-v2-native"));
    std::fs::create_dir_all(&path).unwrap();
    let root = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(path)
        .unwrap()
        .keep();
    println!("DART_V2_GATE_ROOT={}", root.display());
    root
}
pub fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
pub fn output(command: &mut Command, root: &Path, name: &str) -> Output {
    let result = command.output().unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    std::fs::write(
        root.join(format!("logs/{name}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .unwrap();
    result
}
pub fn check(command: &mut Command, root: &Path, name: &str) {
    let result = output(command, root, name);
    assert!(
        result.status.success(),
        "{} {command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
pub fn dart(root: &Path) -> Command {
    let binary = std::env::var_os("SUSPECT_DART_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository().join("target/sdk-dart-tools/dart-sdk/bin/dart"));
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .env("HOME", repository().join("target/sdk-dart-tools/home"))
        .env("PUB_CACHE", root.join("pub-cache"))
        .env("CI", "true")
        .env("DART_SUPPRESS_ANALYTICS", "true");
    command
}
pub fn node(root: &Path, script: &str, name: &str) {
    check(
        Command::new("node")
            .args(["-e", "globalThis.self=globalThis;require(process.argv[1]);"])
            .arg(root.join(script)),
        root,
        name,
    );
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub struct Server {
    pub url: String,
    pub records: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(
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
                    Err(error) => panic!("{error}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            url,
            records,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
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
pub fn reply(mut stream: std::net::TcpStream, status: u16, media: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

/// Serve exactly the emitted archive through an independent hosted pub endpoint,
/// then prove offline installation and compare every installed file's bytes.
pub fn install(root: &Path, files: &[codegen::OutFile]) -> PathBuf {
    install_package(root, files, "generated_sdk", "0.0.0")
}

pub fn install_package(
    root: &Path,
    files: &[codegen::OutFile],
    name: &str,
    version: &str,
) -> PathBuf {
    codegen::write_files(files, root).unwrap();
    check(
        Command::new("tar")
            .arg("-czf")
            .arg(root.join("package.tar.gz"))
            .arg("-C")
            .arg(root.join("dart"))
            .arg("."),
        root,
        "archive",
    );
    let bytes = std::fs::read(root.join("package.tar.gz")).unwrap();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let package_name = name.to_owned();
    let package_version = version.to_owned();
    let hosted = Server::start(move |stream, request, url| {
        if request.path == format!("/api/packages/{package_name}") {
            let version = json!({"version":package_version,"pubspec":{"name":package_name,"version":package_version,"environment":{"sdk":">=3.9.4 <4.0.0"}},"archive_url":format!("{url}/archive"),"archive_sha256":sha,"published":"2026-09-10T00:00:00Z"});
            reply(
                stream,
                200,
                "application/json",
                json!({"name":package_name,"latest":version,"versions":[version]})
                    .to_string()
                    .as_bytes(),
            );
        } else if request.path == "/archive" {
            reply(stream, 200, "application/octet-stream", &bytes);
        } else {
            reply(stream, 200, "application/json", br#"{"advisories":[]}"#);
        }
    });
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("bin")).unwrap();
    std::fs::write(consumer.join("pubspec.yaml"),format!("name: dart_v2_consumer\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\ndependencies:\n  {name}:\n    hosted: {}\n    version: {version}\n",hosted.url)).unwrap();
    std::fs::copy(
        root.join("dart/analysis_options.yaml"),
        consumer.join("analysis_options.yaml"),
    )
    .unwrap();
    check(dart(root).args(["--version"]), root, "toolchain");
    check(
        dart(root).args(["pub", "get"]).current_dir(&consumer),
        root,
        "install",
    );
    std::fs::write(
        root.join("hosted-requests.json"),
        serde_json::to_vec_pretty(&*hosted.records.lock().unwrap()).unwrap(),
    )
    .unwrap();
    drop(hosted);
    check(
        dart(root)
            .args(["pub", "get", "--offline"])
            .current_dir(&consumer),
        root,
        "install-offline",
    );
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(consumer.join(".dart_tool/package_config.json")).unwrap(),
    )
    .unwrap();
    let uri = config["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap()["rootUri"]
        .as_str()
        .unwrap();
    let installed = url::Url::from_directory_path(consumer.join(".dart_tool"))
        .unwrap()
        .join(uri)
        .unwrap()
        .to_file_path()
        .unwrap();
    let mut inventory = Vec::new();
    for file in files {
        let relative = file.path.strip_prefix("dart/").unwrap();
        let actual = std::fs::read(installed.join(relative)).unwrap();
        assert_eq!(actual, file.content.as_bytes(), "installed {relative}");
        inventory.push(json!({"path":relative,"sha256":format!("{:x}",Sha256::digest(&actual))}));
    }
    std::fs::write(
        root.join("installed-files.json"),
        serde_json::to_vec_pretty(&json!({"root":installed,"files":inventory})).unwrap(),
    )
    .unwrap();
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
    installed
}
