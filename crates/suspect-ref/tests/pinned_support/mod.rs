#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use suspect_ref::{acquire::AcquireOptions, sha256_digest};
use suspect_source::Uri;

pub fn tempdir() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-acquire-fixtures");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(root)
        .unwrap()
}

pub fn pin(uri: &Uri, bytes: &[u8]) -> Value {
    json!({
        "requested_uri": uri.as_str(), "effective_uri": uri.as_str(),
        "digest": sha256_digest(bytes), "media_type": "application/json",
        "via": if uri.scheme() == "file" { "local" } else { "direct" },
        "redirects": [], "retrieved_at": "2026-09-09T00:00:00Z",
        "attempts": if uri.scheme() == "file" { 0 } else { 1 }
    })
}

pub fn write_manifest(root: &Path, entry: &Uri, resources: Vec<Value>) -> PathBuf {
    let path = root.join("pins.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "manifest_version": 1, "entry": entry.as_str(), "resources": resources
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

pub fn options(root: &Path, server: &Server) -> AcquireOptions {
    AcquireOptions {
        cache_dir: root.join("cache"),
        insecure_test_origins: vec![server.origin.clone()],
        timeout: Duration::from_secs(3),
        ..AcquireOptions::default()
    }
}

pub fn redirected_pin(
    requested: &Uri,
    effective: &Uri,
    bytes: &[u8],
    hops: &[(&Uri, &Uri, u16)],
) -> Value {
    let mut resource = pin(requested, bytes);
    resource["effective_uri"] = json!(effective.as_str());
    resource["via"] = json!("redirect");
    resource["attempts"] = json!(hops.len() + 1);
    resource["redirects"] = json!(
        hops.iter()
            .map(|(from, to, status)| json!({
                "from_uri": from.as_str(), "to_uri": to.as_str(), "status": status
            }))
            .collect::<Vec<_>>()
    );
    resource
}

#[derive(Debug, Clone)]
pub struct Request {
    pub path: String,
    pub headers: BTreeMap<String, String>,
}

pub enum Action {
    Bytes(Vec<u8>),
    Pause(Duration),
}

pub struct ChildGuard(pub std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<Action> {
    let mut bytes = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(body);
    vec![Action::Bytes(bytes)]
}

pub struct Server {
    pub origin: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn new(reply: impl Fn(&Request) -> Vec<Action> + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = stop.clone();
        let thread_requests = requests.clone();
        let worker = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Darwin can inherit O_NONBLOCK from the listener.
                        // This peer uses blocking, timeout-bounded HTTP I/O;
                        // WouldBlock must not be mistaken for a closed request.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut chunk = [0; 1024];
                        while bytes.len() < 65536
                            && !bytes.windows(4).any(|part| part == b"\r\n\r\n")
                        {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                            }
                        }
                        if bytes.is_empty() {
                            continue;
                        }
                        let text = String::from_utf8_lossy(&bytes);
                        let mut lines = text.split("\r\n");
                        let path = lines
                            .next()
                            .unwrap_or_default()
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or_default()
                            .to_owned();
                        let headers = lines
                            .filter_map(|line| line.split_once(':'))
                            .map(|(name, value)| {
                                (name.to_ascii_lowercase(), value.trim().to_owned())
                            })
                            .collect();
                        let request = Request { path, headers };
                        thread_requests.lock().unwrap().push(request.clone());
                        for action in reply(&request) {
                            match action {
                                Action::Bytes(bytes) => {
                                    if stream.write_all(&bytes).is_err() {
                                        break;
                                    }
                                }
                                Action::Pause(duration) => thread::sleep(duration),
                            }
                        }
                        // Complete the response with FIN and wait for the
                        // client to close. Dropping a socket with unread peer
                        // bytes can send RST and discard the response on macOS,
                        // racing the policy error this independent peer tests.
                        let _ = stream.shutdown(Shutdown::Write);
                        let mut drained = 0;
                        while drained < 65536 {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(read) => drained += read,
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            origin: format!("http://{address}"),
            requests,
            stop,
            worker: Some(worker),
        }
    }

    pub fn uri(&self, path: &str) -> Uri {
        Uri::parse(&format!("{}{path}", self.origin)).unwrap()
    }
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.origin.trim_start_matches("http://"));
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
