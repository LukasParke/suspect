//! The journal and gateway surfaces wired from the audit: `suspect why`,
//! the scenario gateway mode, `--journal <file>` sinks, and the redaction
//! denylist flags.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

const SPEC: &str = "openapi: 3.1.0
info:
  title: Journal wiring
  version: \"1\"
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200': {description: ok}
";

const CHECK: &str = "arazzo: 1.0.0
info: {title: C, version: '1'}
sourceDescriptions:
  - name: api
    type: openapi
    url: api.yaml
workflows:
  - workflowId: w
    steps:
      - stepId: s
        operationId: ping
        successCriteria:
          - condition: '{$statusCode} == 200'
";

fn suspect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
}

/// A minimal HTTP server: one request per connection, always `200 {}`.
struct TinyServer {
    port: u16,
}

impl TinyServer {
    fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let body = br#"{}"#;
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    );
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                });
            }
        });
        Self { port }
    }
}

/// One raw HTTP GET over a throwaway connection, returning the status line.
fn raw_get(port: u16, path: &str, extra_header: Option<(&str, &str)>) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let mut request = format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1\r\n");
    if let Some((name, value)) = extra_header {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    // `connection: close` servers may RST instead of half-closing once
    // done (Linux routinely does once unread input remains): the
    // response has already arrived, so a reset ends the read instead of
    // failing it.
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(e) => panic!("read: {e}"),
        }
    }
    let head = String::from_utf8_lossy(&response[..response.len().min(512)]).into_owned();
    head.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// Spawns the gateway child on a free port; caller kills it.
struct GatewayChild {
    child: Child,
    port: u16,
}

impl GatewayChild {
    /// Starts `suspect gateway --port 0` and returns once it is serving.
    ///
    /// The gateway binds the port itself and announces the bound address
    /// on stderr only once it can serve. Reserving a port here instead
    /// (bind `:0`, drop, pass the number) is racy: a child spawned
    /// concurrently by another test inherits the reservation socket
    /// (until exec on Linux, for its whole life on macOS), so the
    /// gateway's own bind can fail and early connections land in a
    /// backlog nobody serves, then get reset.
    fn spec(root: &Path, args: &[&str]) -> Self {
        let spec = root.join("api.yaml");
        let mut command = suspect();
        command
            .arg("gateway")
            .arg(&spec)
            .args(["--port", "0"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("gateway spawn");
        let stderr = child.stderr.take().expect("piped stderr");
        let (ready, port) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut seen = String::new();
            let mut lines = BufReader::new(stderr).lines();
            for line in lines.by_ref().map_while(Result::ok) {
                let bound = line
                    .split_once("listening on 127.0.0.1:")
                    .and_then(|(_, rest)| rest.split_whitespace().next())
                    .and_then(|port| port.parse::<u16>().ok());
                if let Some(port) = bound {
                    let _ = ready.send(Ok(port));
                    // Keep draining so the child never blocks on a full pipe.
                    for _ in lines {}
                    return;
                }
                seen.push_str(&line);
                seen.push('\n');
            }
            let _ = ready.send(Err(seen));
        });
        match port.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(port)) => Self { child, port },
            Ok(Err(stderr)) => {
                let _ = child.kill();
                panic!("gateway exited before listening: {stderr}");
            }
            Err(e) => {
                let _ = child.kill();
                panic!("gateway never announced its address: {e}");
            }
        }
    }
}

impl Drop for GatewayChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_workspace(root: &Path) {
    std::fs::write(root.join("api.yaml"), SPEC).expect("api");
    std::fs::write(root.join("check.yaml"), CHECK).expect("check");
}

#[test]
fn why_traces_a_failure_to_its_source_location() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_workspace(root);
    let output = suspect()
        .args(["why", "type: string expected", "--spec"])
        .arg(root.join("api.yaml"))
        .args(["--why-line", "3", "--why-col", "9"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "why must succeed: {stdout} {stderr}"
    );
    assert!(
        stdout.contains("Constraint fired: type: string expected"),
        "the trace names the constraint: {stdout}"
    );
    assert!(
        stdout.contains("line 3"),
        "the editor location resolves to the source line: {stdout}"
    );

    let output = suspect()
        .args(["why", "anything", "--spec"])
        .arg(root.join("api.yaml"))
        .args(["--format", "json"])
        .output()
        .expect("run");
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json format parses");
    assert!(
        parsed["steps"].as_array().is_some_and(|s| !s.is_empty()),
        "json output carries the trace: {parsed}"
    );
}

#[test]
fn scenario_mode_serves_the_script_and_rejects_deviation() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    let scenario = root.join("scenario.json");
    std::fs::write(
        &scenario,
        r#"{"steps": [
            {"method": "GET", "path_suffix": "/first", "status": 200, "body": {"ok": true}},
            {"method": "GET", "path_suffix": "/second", "status": 201, "body": {"done": true}}
        ]}"#,
    )
    .expect("scenario");
    let gateway = GatewayChild::spec(
        root,
        &[
            "--mode",
            "scenario",
            "--scenario",
            scenario.to_str().expect("path"),
        ],
    );
    assert_eq!(raw_get(gateway.port, "/first", None), 200);
    assert_eq!(
        raw_get(gateway.port, "/nope", None),
        400,
        "a request off the script is rejected with expected-versus-got"
    );
    assert_eq!(raw_get(gateway.port, "/second", None), 201);
    assert_eq!(
        raw_get(gateway.port, "/anything", None),
        410,
        "requests after the last step are gone"
    );
}

#[test]
fn a_journal_file_receives_the_run_summary() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_workspace(root);
    let server = TinyServer::spawn();
    let journal = root.join("runs.jsonl");
    let output = suspect()
        .current_dir(root)
        .arg("test")
        .arg("check.yaml")
        .args([
            "--base-url",
            &format!("http://127.0.0.1:{}", server.port),
            "--journal",
        ])
        .arg(&journal)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines = std::fs::read_to_string(&journal).expect("journal written");
    assert!(
        lines.contains("run_summary"),
        "the run summary lands in the journal file: {lines}"
    );
    assert!(
        lines.contains("\"run_kind\":\"test\""),
        "the record names its kind: {lines}"
    );
}

#[test]
fn redact_flags_extend_the_journal_denylist() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    write_workspace(root);
    let journal = root.join("gw.jsonl");
    let journal_arg = journal.to_str().expect("path").to_owned();
    let gateway = GatewayChild::spec(
        root,
        &[
            "--mode",
            "mock",
            "--journal",
            &journal_arg,
            "--redact-header",
            "X-Internal-Secret",
        ],
    );
    let code = raw_get(
        gateway.port,
        "/ping",
        Some(("X-Internal-Secret", "WireUpCanarySecret7291")),
    );
    assert_eq!(code, 200, "the mock serves the request");
    drop(gateway);
    // The child buffers writes; give the file a moment to land.
    let text = {
        let mut text = std::fs::read_to_string(&journal).unwrap_or_default();
        let mut tries = 0;
        while text.is_empty() && tries < 50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            text = std::fs::read_to_string(&journal).unwrap_or_default();
            tries += 1;
        }
        text
    };
    assert!(!text.is_empty(), "the gateway journal landed in the file");
    assert!(
        !text.contains("WireUpCanarySecret7291"),
        "the denylisted header value never reaches the journal: {text}"
    );
    assert!(
        text.contains("[redacted]"),
        "the denylisted header is redacted, not dropped: {text}"
    );
}
