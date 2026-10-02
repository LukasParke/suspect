//! A language-server client, written the way an editor behaves.
//!
//! Every other test in this crate that touches the server answers a request
//! and moves on. That is not what an editor does, and it is why a server
//! with three separate deadlocks passed its whole test suite: each handler
//! was exercised in isolation, and every one of the deadlocks needed a
//! *second* party — a queued writer, a warming index — to be in flight at
//! the same time.
//!
//! So this client is deliberately faithful about the parts that matter:
//!
//! - it frames `Content-Length` messages over real pipes to the real binary,
//! - it sends many requests before waiting for any of them,
//! - it sends notifications with **no** id, because tower-lsp silently
//!   drops a notification method that arrives with one,
//! - it answers the server's own requests, or the server waits forever on
//!   `initialized`'s `workspace/configuration`,
//! - it cancels requests, because an editor does.
//!
//! It answers nothing else, and it asserts nothing by itself — see
//! `lsp_session.rs` for what the server is required to return.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long any request (or whole batch) may take before the test calls it a
/// wedge.
///
/// Generous for a cold index on a large document — the slowest legitimate
/// request measured here is well under two seconds — but not so generous
/// that a wedged server costs minutes before it is reported.
pub const BUDGET: Duration = Duration::from_secs(30);

/// One measured request.
#[derive(Debug)]
pub struct Timed {
    /// The request method.
    pub method: String,
    /// Whether it was answered, and with what.
    pub answer: Result<serde_json::Value, NoReply>,
    /// Wall-clock time from writing the frame to filing this response.
    pub elapsed: Duration,
    /// Wall-clock time from the start of the batch, for the caller's own
    /// batch-level figures. Meaningful on the last entry of a batch.
    pub batch: Duration,
}

/// A request the server has not answered yet.
#[derive(Debug)]
pub struct NoReply(pub String);

impl std::fmt::Display for NoReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} never answered", self.0)
    }
}

impl std::error::Error for NoReply {}

#[derive(Default)]
struct Inbox {
    answers: HashMap<i64, serde_json::Value>,
    /// When each answer was filed. Without this, a batch measures every
    /// request from the batch's start, so each one inherits the wait for
    /// everything queued ahead of it — which made a 52ms hover look like
    /// 4.2 seconds.
    arrived_at: HashMap<i64, Instant>,
    errors: HashMap<i64, String>,
    notifications: Vec<(String, serde_json::Value)>,
    /// Server requests we have seen, so a test can assert the server asked.
    server_requests: Vec<(String, serde_json::Value)>,
    /// Set if the server ever closed its output.
    closed: bool,
}

/// A live `suspect lsp` over stdio, driven like an editor.
pub struct Editor {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
    next_id: AtomicI64,
    running: Arc<AtomicBool>,
    /// Root the server was told about.
    pub root: std::path::PathBuf,
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Closing stdin is how a client asks a well-behaved server to stop.
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = stdin.flush();
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// The harness surface is deliberately wider than any single test uses: each
// scenario needs a different part of it, and trimming it to one would make
// the next scenario reach into another file.
#[allow(dead_code)]
impl Editor {
    /// Starts `suspect lsp` and completes the handshake, exactly as an
    /// editor does: `initialize`, wait for the result, then `initialized`.
    ///
    /// `capabilities` is the client's own advertised capability set; pass
    /// [`editor_capabilities`] for a realistic one.
    pub fn start(root: &Path, capabilities: serde_json::Value) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_suspect"));
        command
            .arg("lsp")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("suspect lsp must start");
        let stdin = Arc::new(Mutex::new(child.stdin.take().expect("stdin")));
        let stdout = child.stdout.take().expect("stdout");

        let inbox: Arc<(Mutex<Inbox>, Condvar)> =
            Arc::new((Mutex::new(Inbox::default()), Condvar::new()));
        let running = Arc::new(AtomicBool::new(true));

        {
            // The reader thread is the whole client side of the protocol: it
            // answers what the server asks and files what it sends back.
            let inbox_for_thread = Arc::clone(&inbox);
            let running = Arc::clone(&running);
            let writer = Arc::clone(&stdin);
            std::thread::spawn(move || {
                let (lock, condvar) = &*inbox_for_thread;
                let mut reader = BufReader::new(stdout);
                while running.load(Ordering::SeqCst) {
                    let Ok(Some(frame)) = read_frame(&mut reader) else {
                        let mut inbox = lock.lock().expect("inbox");
                        inbox.closed = true;
                        condvar.notify_all();
                        return;
                    };
                    let Ok(message) = serde_json::from_slice::<serde_json::Value>(&frame) else {
                        // A frame we cannot parse is not fatal: skip it and
                        // keep reading, or one bad message would look like
                        // a dead server.
                        continue;
                    };
                    let method = message
                        .get("method")
                        .and_then(|m| m.as_str())
                        .map(str::to_owned);
                    let id = message.get("id").and_then(|i| i.as_i64());
                    match (method, id) {
                        // Server -> client request: answer it, or the server
                        // waits on us forever.
                        (Some(method), Some(id)) => {
                            let params = message.get("params").cloned().unwrap_or_default();
                            let result = client_answer(&method, &params);
                            {
                                let mut inbox = lock.lock().expect("inbox");
                                inbox.server_requests.push((method, params));
                            }
                            condvar.notify_all();
                            let reply = serde_json::json!({
                                "jsonrpc": "2.0", "id": id, "result": result
                            });
                            let mut writer = writer.lock().expect("stdin");
                            write_frame(&mut *writer, &reply);
                        }
                        (Some(method), None) => {
                            let mut inbox = lock.lock().expect("inbox");
                            inbox
                                .notifications
                                .push((method, message.get("params").cloned().unwrap_or_default()));
                            condvar.notify_all();
                        }
                        (None, Some(id)) => {
                            let mut inbox = lock.lock().expect("inbox");
                            inbox.arrived_at.insert(id, Instant::now());
                            if let Some(error) = message.get("error") {
                                inbox.errors.insert(
                                    id,
                                    error
                                        .get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("error")
                                        .to_owned(),
                                );
                            }
                            inbox
                                .answers
                                .insert(id, message.get("result").cloned().unwrap_or_default());
                            condvar.notify_all();
                        }
                        _ => {}
                    }
                }
            });
        }

        let editor = Self {
            child: Mutex::new(child),
            stdin,
            inbox,
            next_id: AtomicI64::new(1),
            running,
            root: root.to_path_buf(),
        };
        editor.handshake(capabilities);
        editor
    }

    /// A client that advertises nothing beyond configuration. Most features
    /// behave differently with real capabilities, so this is for tests that
    /// want the server's defaults rather than an editor's.
    pub fn start_bare(root: &Path) -> Self {
        Self::start(
            root,
            serde_json::json!({"workspace": {"configuration": true}}),
        )
    }

    fn handshake(&self, capabilities: serde_json::Value) {
        let root_uri = url_of(&self.root);
        let value = self
            .request(
                "initialize",
                serde_json::json!({
                    "processId": serde_json::Value::Null,
                    "rootUri": root_uri,
                    "workspaceFolders": [{
                        "uri": root_uri, "name": self.root.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "root".to_owned()),
                    }],
                    "clientInfo": {"name": "suspect-lsp-tests", "version": "1"},
                    "capabilities": capabilities,
                }),
            )
            .expect("initialize must be answered");
        assert!(
            value.get("capabilities").is_some(),
            "initialize must return capabilities, got {value}"
        );
        self.notify("initialized", serde_json::json!({}));
    }

    fn write(&self, message: &serde_json::Value) {
        let mut stdin = self.stdin.lock().expect("stdin");
        write_frame(&mut *stdin, message);
    }

    /// Sends a notification: **no id**, because tower-lsp drops a
    /// notification method that arrives carrying one.
    pub fn notify(&self, method: &str, params: serde_json::Value) {
        self.write(&serde_json::json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }));
    }

    /// Opens a document the way an editor does, as a notification.
    pub fn open(&self, path: &Path, text: &str) -> String {
        let uri = url_of(path);
        self.notify(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": uri, "languageId": language_of(path),
                    "version": 1, "text": text,
                }
            }),
        );
        uri
    }

    /// Sends a request and returns its id without waiting, so a test can
    /// cancel that exact request while others are in flight.
    pub fn issue(&self, method: &str, params: serde_json::Value) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.write(&serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }));
        id
    }

    fn await_one(&self, id: i64, method: &str) -> Result<serde_json::Value, NoReply> {
        self.await_until(id, method, Instant::now() + BUDGET)
    }

    #[allow(dead_code)]
    fn await_one_unused(&self, id: i64, method: &str) -> Result<serde_json::Value, NoReply> {
        let deadline = Instant::now() + BUDGET;
        let (lock, condvar) = &*self.inbox;
        let mut inbox = lock.lock().expect("inbox");
        loop {
            if let Some(value) = inbox.answers.get(&id) {
                return Ok(value.clone());
            }
            if let Some(message) = inbox.errors.get(&id) {
                // An error is still an answer: the server replied.
                return Err(NoReply(format!(
                    "{method} answered with an error: {message}"
                )));
            }
            if inbox.closed {
                return Err(NoReply(format!(
                    "{method} never answered: the server closed its output"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(NoReply(method.to_owned()));
            }
            let (guard, _) = condvar
                .wait_timeout(inbox, deadline - now)
                .expect("condvar");
            inbox = guard;
        }
    }

    /// Issues one request and waits for it.
    pub fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, NoReply> {
        let id = self.issue(method, params);
        self.await_one(id, method)
    }

    /// Issues one request and reports how long the answer took.
    ///
    /// Measured from the moment the frame is written to the moment the
    /// matching response is filed, so it includes the server's own work and
    /// the wait behind anything already in flight — which is what an editor
    /// experiences. A request issued into a queue behind others is slow for
    /// the user even when the server itself is fast.
    pub fn request_timed(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> (Result<serde_json::Value, NoReply>, Duration) {
        let started = Instant::now();
        let id = self.issue(method, params);
        let answer = self.await_one(id, method);
        (answer, started.elapsed())
    }

    /// Issues a batch before awaiting any, timing each answer from when that
    /// answer actually arrived.
    ///
    /// Timing from the batch start instead folds the queue wait into every
    /// sample, so a 52ms hover inherits the wait for everything issued ahead
    /// of it and reads as four seconds. [`Timed::batch`] carries the batch's
    /// own wall time for the figure that genuinely is a batch.
    pub fn request_all_timed(&self, calls: &[(String, serde_json::Value)]) -> Vec<Timed> {
        let started = Instant::now();
        let issued: Vec<(String, i64)> = calls
            .iter()
            .map(|(method, params)| (method.clone(), self.issue(method, params.clone())))
            .collect();
        issued
            .into_iter()
            .map(|(method, id)| {
                let answer = self.await_until(id, &method, started + BUDGET);
                let elapsed = self
                    .inbox
                    .0
                    .lock()
                    .expect("inbox")
                    .arrived_at
                    .get(&id)
                    .map_or_else(|| started.elapsed(), |at| at.duration_since(started));
                let batch = started.elapsed();
                Timed {
                    method,
                    answer,
                    elapsed,
                    batch,
                }
            })
            .collect()
    }

    /// Waits for an id this caller issued itself, for the bench to time.
    pub fn await_for_bench(&self, id: i64, method: &str) -> Result<serde_json::Value, NoReply> {
        self.await_until(id, method, Instant::now() + BUDGET)
    }

    /// Notifications carry no reply, so they are timed by issuing them.
    pub fn notify_timed(&self, method: &str, params: serde_json::Value) -> Duration {
        let started = Instant::now();
        self.notify(method, params);
        started.elapsed()
    }

    /// Issues every request **before** waiting for any of them.
    ///
    /// This is the whole point of this client. Awaiting each in turn would
    /// serialise the session and hide every ordering bug in the server.
    /// The whole batch shares one deadline.
    ///
    /// A per-request budget multiplies: a wedged server would cost
    /// `BUDGET × requests`, which turned one failing test into half an hour.
    /// One deadline for the batch makes a wedge fail in seconds and leaves a
    /// genuinely slow — but alive — server the full budget to answer.
    pub fn request_all(
        &self,
        calls: &[(String, serde_json::Value)],
    ) -> Vec<(String, Result<serde_json::Value, NoReply>)> {
        let deadline = Instant::now() + BUDGET;
        let issued: Vec<(String, i64)> = calls
            .iter()
            .map(|(method, params)| (method.clone(), self.issue(method, params.clone())))
            .collect();
        issued
            .into_iter()
            .map(|(method, id)| {
                let answer = self.await_until(id, &method, deadline);
                (method, answer)
            })
            .collect()
    }

    /// Waits for one id against a caller-supplied deadline.
    fn await_until(
        &self,
        id: i64,
        method: &str,
        deadline: Instant,
    ) -> Result<serde_json::Value, NoReply> {
        let (lock, condvar) = &*self.inbox;
        let mut inbox = lock.lock().expect("inbox");
        loop {
            if let Some(value) = inbox.answers.get(&id) {
                return Ok(value.clone());
            }
            if let Some(message) = inbox.errors.get(&id) {
                return Err(NoReply(format!(
                    "{method} answered with an error: {message}"
                )));
            }
            if inbox.closed {
                return Err(NoReply(format!(
                    "{method} never answered: the server closed its output"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(NoReply(method.to_owned()));
            }
            let (guard, _) = condvar
                .wait_timeout(inbox, deadline - now)
                .expect("condvar");
            inbox = guard;
        }
    }

    /// Cancels a request by id, as an editor does when the cursor moves.
    pub fn cancel(&self, id: i64) {
        self.notify("$/cancelRequest", serde_json::json!({"id": id}));
    }

    /// Notifications the server pushed, in order.
    #[allow(dead_code)]
    pub fn notifications(&self) -> Vec<(String, serde_json::Value)> {
        self.inbox.0.lock().expect("inbox").notifications.clone()
    }

    /// Requests the server has made of us.
    pub fn server_requests(&self) -> Vec<(String, serde_json::Value)> {
        self.inbox.0.lock().expect("inbox").server_requests.clone()
    }

    /// Waits for the server to push something, or gives up.
    pub fn wait_for_notification(&self, method: &str, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let (lock, condvar) = &*self.inbox;
        let mut inbox = lock.lock().expect("inbox");
        loop {
            if inbox.notifications.iter().any(|(m, _)| m == method) {
                return true;
            }
            if inbox.closed || Instant::now() >= deadline {
                return false;
            }
            let (guard, _) = condvar.wait_timeout(inbox, budget).expect("condvar");
            inbox = guard;
        }
    }

    /// `textDocument/hover` at a 1-based line and column, as a human counts.
    pub fn hover(&self, uri: &str, line: usize, column: usize) -> Result<String, NoReply> {
        let value = self.request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": {"line": line - 1, "character": column - 1},
            }),
        )?;
        Ok(hover_text(&value))
    }

    /// The line a definition points at, 1-based, or `None` for no definition.
    pub fn definition_line(
        &self,
        uri: &str,
        line: usize,
        column: usize,
    ) -> Result<Option<usize>, NoReply> {
        let value = self.request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": uri},
                "position": {"line": line - 1, "character": column - 1},
            }),
        )?;
        Ok(definition_target(&value))
    }
}

/// The markdown a hover response carries.
#[must_use]
pub fn hover_text(value: &serde_json::Value) -> String {
    let Some(contents) = value.get("contents") else {
        return String::new();
    };
    if let Some(text) = contents.get("value").and_then(|v| v.as_str()) {
        return text.to_owned();
    }
    if let Some(items) = contents.as_array() {
        return items
            .iter()
            .filter_map(|item| item.get("value").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

/// The 1-based line a definition/definition-list response points at.
#[must_use]
pub fn definition_target(value: &serde_json::Value) -> Option<usize> {
    let entry = match value {
        serde_json::Value::Array(items) => items.first()?,
        serde_json::Value::Null => return None,
        other => other,
    };
    let range = entry.get("range").or_else(|| entry.get("targetRange"))?;
    Some(range.get("start")?.get("line")?.as_u64()? as usize + 1)
}

// ---------- wire framing ----------

fn write_frame(writer: &mut impl Write, message: &serde_json::Value) {
    let body = serde_json::to_vec(message).expect("serialisable frame");
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).expect("header");
    writer.write_all(&body).expect("body");
    writer.flush().expect("flush");
}

/// Reads one `Content-Length` frame. `None` at end of stream.
fn read_frame(reader: &mut BufReader<impl Read>) -> std::io::Result<Option<Vec<u8>>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = rest.trim().parse().ok();
        }
    }
    let Some(length) = length else {
        return Ok(None);
    };
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

/// What a real editor answers to the server's own requests.
fn client_answer(method: &str, params: &serde_json::Value) -> serde_json::Value {
    match method {
        "workspace/configuration" => {
            // One null per requested item: "I have no opinion", which is
            // what a client with no settings for these sections returns.
            let count = params
                .get("items")
                .and_then(|i| i.as_array())
                .map_or(0, Vec::len);
            serde_json::Value::Array(vec![serde_json::Value::Null; count])
        }
        "workspace/applyEdit" => serde_json::json!({"applied": true}),
        // Progress, capability registration, dynamic watcher registration:
        // all notifications in effect, all accepted.
        _ => serde_json::Value::Null,
    }
}

/// A `file://` URI for a path, as an editor would send.
#[must_use]
pub fn url_of(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// The language id VS Code assigns by extension.
#[must_use]
pub fn language_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "json",
        Some("md" | "markdown") => "markdown",
        _ => "yaml",
    }
}

/// A realistic VS Code client capability set.
///
/// Read verbatim from `fixtures/vscode-capabilities.json`, which is the
/// `capabilities` object from a captured live `initialize`. Servers that gate
/// behaviour on client capabilities — this one gates dynamic registration,
/// watched-file registration and diagnostic refresh on them — behave
/// differently when the client advertises nothing, so a bare harness tests a
/// server nobody runs. A hand-written set also drifts: the first attempt here
/// declared `semanticTokens.tokenTypes` as integers, and the server rejected
/// the entire handshake with "invalid type: number, expected a string".
#[must_use]
pub fn editor_capabilities() -> serde_json::Value {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/vscode-capabilities.json"))
            .expect("the captured capability set must parse");
    fixture
        .get("capabilities")
        .cloned()
        .expect("the captured capability set must carry `capabilities`")
}
