//! Contract testing end to end: `suspect test run`, `stateful` and `fuzz`
//! against a scripted server and the gateway, through the real binary.
//!
//! Each test pins one way a run used to pass (or fail) for the wrong
//! reason: a schema the runner never compared, a recording replayed out
//! of order, a report that named no test.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

const PETS: &str = r##"openapi: 3.1.0
info:
  title: Pets
  version: "1"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: a page, declared inline
          content:
            application/json:
              schema:
                type: object
                required: [items]
                properties:
                  items:
                    type: array
                    items: { $ref: '#/components/schemas/Pet' }
    post:
      operationId: createPet
      requestBody:
        required: true
        content:
          application/json:
            schema: { $ref: '#/components/schemas/NewPet' }
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
  /pets/{petId}:
    get:
      operationId: showPet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: string } }
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
        '404':
          $ref: '#/components/responses/NotFound'
    delete:
      operationId: deletePet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: string } }
      responses:
        '204': { description: gone }
components:
  responses:
    NotFound:
      description: no such pet
      content:
        application/problem+json:
          schema: { $ref: '#/components/schemas/Problem' }
  schemas:
    NewPet:
      type: object
      required: [name]
      properties:
        name: { type: string }
    Pet:
      type: object
      required: [id, name]
      properties:
        id: { type: string, example: pet_42 }
        name: { type: string, example: Rex }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string }
"##;

const FLOWS: &str = r#"arazzo: 1.0.0
info: { title: Pet journeys, version: "1" }
sourceDescriptions:
  - { name: pets, url: pets.openapi.yaml, type: openapi }
workflows:
  - workflowId: browse
    steps:
      - stepId: list
        operationId: listPets
        outputs:
          firstId: $response.body#/items/0/id
      - stepId: open
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: $steps.list.outputs.firstId }
  - workflowId: lifecycle
    steps:
      - stepId: create
        operationId: createPet
        requestBody: { name: Rex }
        successCriteria:
          - condition: '{$statusCode} == 201'
        outputs:
          petId: $response.body#/id
      - stepId: read-back
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: $steps.create.outputs.petId }
      - stepId: remove
        operationId: deletePet
        parameters:
          - { name: petId, in: path, value: $steps.create.outputs.petId }
"#;

const LIST_ONLY: &str = r#"arazzo: 1.0.0
info: { title: List, version: "1" }
sourceDescriptions:
  - { name: pets, url: pets.openapi.yaml, type: openapi }
workflows:
  - workflowId: list
    steps:
      - stepId: list
        operationId: listPets
"#;

const SHOW_ONLY: &str = r#"arazzo: 1.0.0
info: { title: Show, version: "1" }
sourceDescriptions:
  - { name: pets, url: ../pets.openapi.yaml, type: openapi }
workflows:
  - workflowId: show
    steps:
      - stepId: show
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: pet_42 }
"#;

fn suspect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
}

/// One scripted reply: `(method, path, status, content type, body)`.
type Route = (&'static str, &'static str, u16, &'static str, &'static str);

/// A scripted HTTP server: each request is answered by the first route
/// with its method and path (the query is ignored); anything else is a
/// `404` problem document.
struct Server {
    port: u16,
}

impl Server {
    fn spawn(routes: Vec<Route>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                std::thread::spawn(move || serve(stream, &routes));
            }
        });
        Self { port }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn serve(mut stream: std::net::TcpStream, routes: &[Route]) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let path = target.split('?').next().unwrap_or_default().to_owned();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    let (status, content_type, reply) = routes
        .iter()
        .find(|(m, p, ..)| *m == method && *p == path)
        .map_or(
            (
                404,
                "application/problem+json",
                r#"{"title":"not scripted"}"#,
            ),
            |(_, _, status, content_type, body)| (*status, *content_type, *body),
        );
    let mut head = format!("HTTP/1.1 {status} X\r\nconnection: close\r\n");
    if !content_type.is_empty() {
        head.push_str(&format!("content-type: {content_type}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\n\r\n", reply.len()));
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
}

const JSON: &str = "application/json";
const GOOD_PET: &str = r#"{"id":"pet_42","name":"Rex"}"#;

/// A server every reply of which is the one `PETS` declares.
fn conforming_server() -> Server {
    Server::spawn(vec![
        (
            "GET",
            "/pets",
            200,
            JSON,
            r#"{"items":[{"id":"pet_42","name":"Rex"}]}"#,
        ),
        ("POST", "/pets", 201, JSON, GOOD_PET),
        ("GET", "/pets/pet_42", 200, JSON, GOOD_PET),
        ("DELETE", "/pets/pet_42", 204, "", ""),
    ])
}

/// A project directory holding the description and the suites.
fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path();
    std::fs::write(root.join("pets.openapi.yaml"), PETS).expect("pets");
    std::fs::write(root.join("flows.arazzo.yaml"), FLOWS).expect("flows");
    std::fs::write(root.join("list.arazzo.yaml"), LIST_ONLY).expect("list");
    std::fs::create_dir(root.join("suites")).expect("suites");
    std::fs::write(root.join("suites/show.arazzo.yaml"), SHOW_ONLY).expect("show");
    directory
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn finish(output: Output) -> Run {
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `suspect test run <suite> --base-url <base> <args>` inside `root`.
fn test_run(root: &Path, suite: &str, base: &str, args: &[&str]) -> Run {
    finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args(["test", "run", suite, "--base-url", base])
            .args(args)
            .output()
            .expect("run suspect"),
    )
}

/// Spawns `suspect traffic gateway` on a free port; killed on drop.
struct Gateway {
    child: Child,
    port: u16,
}

impl Gateway {
    fn spawn(root: &Path, spec: &str, args: &[&str]) -> Self {
        // One journal per gateway. The name is a number: the arguments
        // hold URLs and paths, which no file name can carry on Windows.
        static SPAWNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let journal = format!(
            "gateway-{}.jsonl",
            SPAWNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut child = suspect()
            .current_dir(root)
            .args(["traffic", "gateway", spec, "--port", "0", "--journal"])
            .arg(root.join(journal))
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("gateway spawn");
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
            Err(error) => {
                let _ = child.kill();
                panic!("gateway never announced its address: {error}");
            }
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Stops the gateway so everything it recorded is on disk.
    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ------------------------------------------------------- conformance

#[test]
fn a_conforming_server_passes_every_step() {
    let dir = workspace();
    let server = conforming_server();
    let run = test_run(dir.path(), "flows.arazzo.yaml", &server.base(), &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("5 passed, 0 failed, 0 skipped"),
        "{}",
        run.stdout
    );
}

#[test]
fn an_inline_schema_violation_fails_the_run_and_names_the_field() {
    let dir = workspace();
    // `id` must be a string and `name` is required.
    let server = Server::spawn(vec![(
        "GET",
        "/pets",
        200,
        JSON,
        r#"{"items":[{"id":42}]}"#,
    )]);
    let run = test_run(dir.path(), "list.arazzo.yaml", &server.base(), &[]);
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("FAIL response-schema")
            && run.stdout.contains("/items/0/id")
            && run.stdout.contains("name"),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("matches contract"),
        "a violating body is never reported as matching: {}",
        run.stdout
    );
}

#[test]
fn undeclared_statuses_and_content_types_fail_the_run() {
    let dir = workspace();
    let accepted = Server::spawn(vec![("GET", "/pets", 202, JSON, r#"{"items":[]}"#)]);
    let run = test_run(dir.path(), "list.arazzo.yaml", &accepted.base(), &[]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout
            .contains("FAIL response-status: expected a declared status (200), got 202"),
        "{}",
        run.stdout
    );

    let html = Server::spawn(vec![("GET", "/pets", 200, "text/html", "<html></html>")]);
    let run = test_run(dir.path(), "list.arazzo.yaml", &html.base(), &[]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout.contains(
            "FAIL response-content-type: expected a declared content type (application/json), got text/html"
        ),
        "{}",
        run.stdout
    );
}

#[test]
fn the_conformance_level_comes_from_the_flag_or_the_settings_file() {
    let dir = workspace();
    let accepted = Server::spawn(vec![("GET", "/pets", 202, JSON, r#"{"items":[]}"#)]);
    let base = accepted.base();

    let strict = test_run(dir.path(), "list.arazzo.yaml", &base, &[]);
    assert_eq!(strict.code, 1, "strict is the default: {}", strict.stdout);

    let by_flag = test_run(
        dir.path(),
        "list.arazzo.yaml",
        &base,
        &["--conformance", "schema"],
    );
    assert_eq!(by_flag.code, 0, "{}\n{}", by_flag.stdout, by_flag.stderr);

    std::fs::write(
        dir.path().join(".suspect.yaml"),
        "test:\n  conformance: schema\n",
    )
    .expect("settings");
    let by_file = test_run(dir.path(), "list.arazzo.yaml", &base, &[]);
    assert_eq!(by_file.code, 0, "{}\n{}", by_file.stdout, by_file.stderr);

    // The flag wins over the file.
    let flag_over_file = test_run(
        dir.path(),
        "list.arazzo.yaml",
        &base,
        &["--conformance", "strict"],
    );
    assert_eq!(flag_over_file.code, 1, "{}", flag_over_file.stdout);
}

#[test]
fn conformance_off_leaves_only_the_criteria() {
    let dir = workspace();
    let html = Server::spawn(vec![("GET", "/pets", 200, "text/html", "<html></html>")]);
    let run = test_run(
        dir.path(),
        "list.arazzo.yaml",
        &html.base(),
        &["--conformance", "off"],
    );
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
}

#[test]
fn an_openapi_30_description_is_read_in_its_own_dialect() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("api.yaml"),
        r##"openapi: 3.0.3
info: { title: Legacy, version: "1" }
paths:
  /pet:
    get:
      operationId: getPet
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
components:
  schemas:
    Pet:
      type: object
      required: [id, password]
      properties:
        id: { type: string }
        tag: { type: string, nullable: true }
        password: { type: string, writeOnly: true }
        price: { type: number, minimum: 0, exclusiveMinimum: true }
"##,
    )
    .expect("api");
    std::fs::write(
        dir.path().join("t.arazzo.yaml"),
        "arazzo: 1.0.0\ninfo: { title: t, version: '1' }\nsourceDescriptions:\n  - { name: api, url: api.yaml, type: openapi }\nworkflows:\n  - workflowId: get\n    steps:\n      - { stepId: get, operationId: getPet }\n",
    )
    .expect("arazzo");
    let server = Server::spawn(vec![(
        "GET",
        "/pet",
        200,
        "application/json; charset=utf-8",
        r#"{"id":"p1","tag":null,"price":5}"#,
    )]);
    let run = test_run(dir.path(), "t.arazzo.yaml", &server.base(), &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("matches contract"), "{}", run.stdout);
}

/// A description split across files is checked like a bundled one: the
/// schema, the shared response and the parameter each live in another
/// document, and the pet schema refers to itself.
#[test]
fn a_multi_file_description_is_checked_through_its_references() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let write = |name: &str, text: &str| {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    };
    write(
        "api.yaml",
        "openapi: 3.1.0\ninfo: { title: Multi, version: '1' }\npaths:\n  /pets/{petId}:\n    $ref: './paths/pet.yaml'\n",
    );
    write(
        "paths/pet.yaml",
        r#"get:
  operationId: showPet
  parameters:
    - $ref: '../params.yaml#/PetId'
  responses:
    '200':
      description: one pet
      content:
        application/json:
          schema:
            $ref: '../schemas/pet.yaml'
    '404':
      $ref: '../responses.yaml#/NotFound'
"#,
    );
    write(
        "params.yaml",
        "PetId:\n  name: petId\n  in: path\n  required: true\n  schema: { type: string }\n",
    );
    write(
        "responses.yaml",
        "NotFound:\n  description: no such pet\n  content:\n    application/problem+json:\n      schema:\n        $ref: './schemas/problem.yaml'\n",
    );
    write(
        "schemas/pet.yaml",
        "type: object\nrequired: [id]\nproperties:\n  id: { type: string }\n  owner:\n    $ref: './owner.yaml'\n  friend:\n    $ref: './pet.yaml'\n",
    );
    write(
        "schemas/owner.yaml",
        "type: object\nrequired: [email]\nproperties:\n  email: { type: string }\n",
    );
    write(
        "schemas/problem.yaml",
        "type: object\nrequired: [title]\nproperties:\n  title: { type: string }\n",
    );
    write(
        "t.arazzo.yaml",
        r#"arazzo: 1.0.0
info: { title: t, version: '1' }
sourceDescriptions:
  - { name: api, url: api.yaml, type: openapi }
workflows:
  - workflowId: found
    steps:
      - stepId: show
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: p1 }
  - workflowId: missing
    steps:
      - stepId: show
        operationId: showPet
        parameters:
          - { name: petId, in: path, value: nope }
        successCriteria:
          - condition: '{$statusCode} == 404'
"#,
    );

    let conforming = Server::spawn(vec![
        (
            "GET",
            "/pets/p1",
            200,
            JSON,
            r#"{"id":"p1","owner":{"email":"a@b.c"},"friend":{"id":"p2","friend":{"id":"p3"}}}"#,
        ),
        (
            "GET",
            "/pets/nope",
            404,
            "application/problem+json",
            r#"{"title":"no such pet"}"#,
        ),
    ]);
    let run = test_run(root, "t.arazzo.yaml", &conforming.base(), &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(
        run.stdout.matches("matches contract").count(),
        2,
        "both bodies were compared with schemas from other files: {}",
        run.stdout
    );

    // A violation two references deep, through the recursion.
    let violating = Server::spawn(vec![
        (
            "GET",
            "/pets/p1",
            200,
            JSON,
            r#"{"id":"p1","friend":{"id":"p2","owner":{"email":7}}}"#,
        ),
        ("GET", "/pets/nope", 404, "application/problem+json", "{}"),
    ]);
    let run = test_run(root, "t.arazzo.yaml", &violating.base(), &[]);
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("/friend/owner/email"), "{}", run.stdout);
    assert!(run.stdout.contains("title"), "{}", run.stdout);
}

// ----------------------------------------------------------- recordings

/// The documented flow: record a run through the gateway, then run the
/// same suite offline from the cassette.
#[test]
fn a_recorded_run_replays_offline_from_its_cassette() {
    let dir = workspace();
    let root = dir.path();
    let cassette = root.join("rec.scj");
    let upstream = Gateway::spawn(root, "pets.openapi.yaml", &["--mode", "mock"]);
    let recorder = Gateway::spawn(
        root,
        "pets.openapi.yaml",
        &[
            "--mode",
            "record",
            "--upstream",
            &upstream.base(),
            "--cassette",
            cassette.to_str().expect("path"),
        ],
    );
    let live = test_run(root, "flows.arazzo.yaml", &recorder.base(), &[]);
    assert_eq!(
        live.code, 0,
        "the recorded run: {}\n{}",
        live.stdout, live.stderr
    );
    recorder.stop();
    upstream.stop();

    // Workflows run concurrently, so the order requests arrive in varies
    // from run to run; every one of them has to replay.
    for attempt in 0..5 {
        let offline = finish(
            suspect()
                .current_dir(root)
                .env_remove("SUSPECT_BASE_URL")
                .args(["test", "run", "flows.arazzo.yaml", "--cassette"])
                .arg(&cassette)
                .output()
                .expect("run suspect"),
        );
        assert_eq!(
            offline.code, 0,
            "offline attempt {attempt}: {}\n{}",
            offline.stdout, offline.stderr
        );
        assert!(
            offline.stdout.contains("5 passed, 0 failed, 0 skipped"),
            "{}",
            offline.stdout
        );
    }
}

// -------------------------------------------------------------- reports

#[test]
fn junit_and_json_reports_describe_every_step() {
    let dir = workspace();
    let root = dir.path();
    // `create` answers 200 where 201 is declared and required.
    let server = Server::spawn(vec![
        (
            "GET",
            "/pets",
            200,
            JSON,
            r#"{"items":[{"id":"pet_42","name":"Rex"}]}"#,
        ),
        ("GET", "/pets/pet_42", 200, JSON, GOOD_PET),
        ("POST", "/pets", 200, JSON, GOOD_PET),
    ]);
    let run = test_run(
        root,
        "flows.arazzo.yaml",
        &server.base(),
        &[
            "--junit",
            "out/junit.xml",
            "--report-file",
            "out/report.json",
        ],
    );
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);

    let xml = std::fs::read_to_string(root.join("out/junit.xml")).expect("junit");
    assert!(
        xml.contains(
            r#"<testsuites name="flows.arazzo.yaml" tests="5" failures="1" errors="0" skipped="2""#
        ),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<testsuite name="browse" tests="2" failures="0""#),
        "{xml}"
    );
    assert!(
        xml.contains(
            r#"<testsuite name="lifecycle" tests="3" failures="1" errors="0" skipped="2""#
        ),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<testcase name="create" classname="lifecycle""#)
            && xml.contains(r#"type="response-status""#)
            && xml.contains("statusCode == 201: expected 201, got 200"),
        "{xml}"
    );
    assert_eq!(xml.matches("<testcase ").count(), 5, "{xml}");
    assert_eq!(xml.matches("<skipped ").count(), 2, "{xml}");

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("out/report.json")).expect("json"))
            .expect("valid json");
    assert_eq!(report["command"], "test");
    assert_eq!(
        (&report["passed"], &report["failed"], &report["skipped"]),
        (
            &serde_json::json!(2),
            &serde_json::json!(1),
            &serde_json::json!(2)
        )
    );
    let cases = report["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 5, "{report:#}");
    let create = cases
        .iter()
        .find(|case| case["name"] == "create")
        .expect("create case");
    assert_eq!(create["suite"], "lifecycle");
    assert_eq!(create["status"], "failed");
    assert_eq!(create["failures"][0]["check"], "response-status");
    assert!(
        create["requests"][0]
            .as_str()
            .is_some_and(|line| line.starts_with("POST ") && line.ends_with("-> 200")),
        "{create:#}"
    );
}

#[test]
fn human_output_carries_no_journal_record() {
    let dir = workspace();
    let server = conforming_server();
    let text = test_run(dir.path(), "list.arazzo.yaml", &server.base(), &[]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        !text.stdout.contains("\"kind\":\"run_summary\""),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout
            .lines()
            .last()
            .is_some_and(|line| line.starts_with("1 passed, 0 failed, 0 skipped in ")),
        "the totals line ends the report: {}",
        text.stdout
    );

    // The machine stream keeps it: one JSON object per line, summary last.
    let ndjson = test_run(
        dir.path(),
        "list.arazzo.yaml",
        &server.base(),
        &["--format", "ndjson"],
    );
    assert_eq!(ndjson.code, 0, "{}", ndjson.stderr);
    let lines: Vec<serde_json::Value> = ndjson
        .stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}")))
        .collect();
    assert_eq!(
        lines.last().expect("a line")["kind"],
        "run_summary",
        "{}",
        ndjson.stdout
    );
}

// ------------------------------------------------- several documents

#[test]
fn several_documents_and_directories_run_as_one_gate() {
    let dir = workspace();
    let root = dir.path();
    let server = conforming_server();
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "run",
                "list.arazzo.yaml",
                "suites",
                "--base-url",
                &server.base(),
                "--junit",
                "junit.xml",
            ])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("list.arazzo.yaml"), "{}", run.stdout);
    assert!(
        run.stdout.contains("show.arazzo.yaml"),
        "the directory's suite ran: {}",
        run.stdout
    );
    let xml = std::fs::read_to_string(root.join("junit.xml")).expect("junit");
    assert!(xml.contains(r#"tests="2" failures="0""#), "{xml}");
    assert!(
        xml.contains(r#"<testsuite name="list.arazzo.yaml: list""#)
            && xml.contains(r#"<testsuite name="suites/show.arazzo.yaml: show""#),
        "suites are told apart by their document: {xml}"
    );

    // One failing document fails the gate, and the others still run.
    let broken = Server::spawn(vec![
        ("GET", "/pets", 200, JSON, r#"{"items":[{"id":42}]}"#),
        ("GET", "/pets/pet_42", 200, JSON, GOOD_PET),
    ]);
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "run",
                "list.arazzo.yaml",
                "suites",
                "--base-url",
                &broken.base(),
            ])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stdout.contains("show ... PASS") || run.stdout.contains("show .. PASS"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_directory_without_suites_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = finish(
        suspect()
            .current_dir(dir.path())
            .args(["test", "run", "."])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 2, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("no Arazzo documents"), "{}", run.stderr);
}

// --------------------------------------------------- stateful and fuzz

#[test]
fn stateful_sequences_are_held_to_the_contract() {
    let dir = workspace();
    let root = dir.path();
    // Right status, wrong body: `id` is not a string.
    let server = Server::spawn(vec![
        ("POST", "/pets", 201, JSON, r#"{"id":7,"name":"Rex"}"#),
        ("DELETE", "/pets/7", 204, "", ""),
    ]);
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "stateful",
                "pets.openapi.yaml",
                "--base-url",
                &server.base(),
            ])
            .args(["--junit", "stateful.xml"])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("breaks its contract") && run.stdout.contains("/id"),
        "{}",
        run.stdout
    );
    let xml = std::fs::read_to_string(root.join("stateful.xml")).expect("junit");
    assert!(
        xml.contains(r#"<testcase name="createPet" classname="stateful""#)
            && xml.contains("<failure "),
        "{xml}"
    );

    let lenient = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "stateful",
                "pets.openapi.yaml",
                "--base-url",
                &server.base(),
            ])
            .args(["--conformance", "off"])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(lenient.code, 0, "{}\n{}", lenient.stdout, lenient.stderr);
}

#[test]
fn a_description_with_nothing_to_sequence_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("read-only.yaml"),
        "openapi: 3.1.0\ninfo: { title: r, version: '1' }\npaths:\n  /ping:\n    get:\n      operationId: ping\n      responses:\n        '200': { description: ok }\n",
    )
    .expect("spec");
    let run = finish(
        suspect()
            .current_dir(dir.path())
            .args(["test", "stateful", "read-only.yaml"])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 2, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("no operation that creates, changes or deletes a resource"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("None"), "{}", run.stderr);
}

#[test]
fn fuzz_reports_replies_that_break_the_contract() {
    let dir = workspace();
    let root = dir.path();
    // Every mutant of `showPet` gets a 200 whose body is not a `Pet`.
    let server = Server::spawn(vec![
        ("GET", "/pets", 200, JSON, r#"{"items":[]}"#),
        ("POST", "/pets", 201, JSON, GOOD_PET),
    ]);
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "fuzz",
                "pets.openapi.yaml",
                "--base-url",
                &server.base(),
            ])
            .args([
                "--runs",
                "6",
                "--filter",
                "createPet",
                "--junit",
                "fuzz.xml",
            ])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(
        run.code, 0,
        "a conforming operation passes: {}\n{}",
        run.stdout, run.stderr
    );
    assert!(run.stdout.contains("violations: 0"), "{}", run.stdout);

    let wrong = Server::spawn(vec![("POST", "/pets", 201, JSON, r#"{"id":7}"#)]);
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "fuzz",
                "pets.openapi.yaml",
                "--base-url",
                &wrong.base(),
            ])
            .args([
                "--runs",
                "6",
                "--filter",
                "createPet",
                "--junit",
                "fuzz.xml",
            ])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("violations: 6") && run.stdout.contains("/id"),
        "{}",
        run.stdout
    );
    let xml = std::fs::read_to_string(root.join("fuzz.xml")).expect("junit");
    assert!(
        xml.contains(r#"<testcase name="createPet" classname="fuzz""#)
            && xml.contains(r#"type="contract""#),
        "{xml}"
    );

    // Crashes are still the only finding when conformance is off.
    let run = finish(
        suspect()
            .current_dir(root)
            .env_remove("SUSPECT_BASE_URL")
            .args([
                "test",
                "fuzz",
                "pets.openapi.yaml",
                "--base-url",
                &wrong.base(),
            ])
            .args([
                "--runs",
                "6",
                "--filter",
                "createPet",
                "--conformance",
                "off",
            ])
            .output()
            .expect("run suspect"),
    );
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
}

// ---------------------------------------------------- bundled example

/// `examples/contract-testing/` is what the guide and the README point at:
/// every command they show has to work on it.
#[test]
fn the_bundled_example_runs_live_offline_stateful_and_fuzzed() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/contract-testing");
    let spec = example.join("petstore.openapi.yaml");
    let suite = example.join("flows.arazzo.yaml");
    let scratch = tempfile::tempdir().expect("tempdir");
    let cassette = scratch.path().join("rec.scj");

    // The description and the suite are themselves clean.
    let check = finish(
        suspect()
            .arg("check")
            .arg(&spec)
            .arg(&suite)
            .output()
            .expect("run suspect"),
    );
    assert_eq!(check.code, 0, "{}\n{}", check.stdout, check.stderr);

    let mock = Gateway::spawn(
        scratch.path(),
        spec.to_str().expect("path"),
        &["--mode", "mock", "--enforce"],
    );
    let suite_arg = suite.to_str().expect("path");
    let live = test_run(scratch.path(), suite_arg, &mock.base(), &[]);
    assert_eq!(live.code, 0, "{}\n{}", live.stdout, live.stderr);
    assert!(
        live.stdout.contains("6 passed, 0 failed, 0 skipped"),
        "{}",
        live.stdout
    );

    let recorder = Gateway::spawn(
        scratch.path(),
        spec.to_str().expect("path"),
        &[
            "--mode",
            "record",
            "--upstream",
            &mock.base(),
            "--cassette",
            cassette.to_str().expect("path"),
        ],
    );
    let recorded = test_run(scratch.path(), suite_arg, &recorder.base(), &[]);
    assert_eq!(recorded.code, 0, "{}\n{}", recorded.stdout, recorded.stderr);
    recorder.stop();
    let offline = finish(
        suspect()
            .current_dir(scratch.path())
            .env_remove("SUSPECT_BASE_URL")
            .args(["test", "run", suite_arg, "--cassette"])
            .arg(&cassette)
            .output()
            .expect("run suspect"),
    );
    assert_eq!(offline.code, 0, "{}\n{}", offline.stdout, offline.stderr);

    for command in ["stateful", "fuzz"] {
        let run = finish(
            suspect()
                .current_dir(scratch.path())
                .env_remove("SUSPECT_BASE_URL")
                .args(["test", command])
                .arg(&spec)
                .args(["--base-url", &mock.base()])
                .output()
                .expect("run suspect"),
        );
        assert_eq!(
            run.code, 0,
            "suspect test {command}: {}\n{}",
            run.stdout, run.stderr
        );
    }
}

/// Retries and the full condition grammar through the real binary.
#[test]
fn a_workflow_polls_until_its_condition_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("api.yaml"),
        "openapi: 3.1.0\ninfo: { title: Jobs, version: '1' }\npaths:\n  /job:\n    get:\n      operationId: showJob\n      responses:\n        '200': { description: the job }\n        '503': { description: busy }\n",
    )
    .expect("api");
    std::fs::write(
        dir.path().join("poll.arazzo.yaml"),
        r#"arazzo: 1.0.1
info: { title: poll, version: '1' }
sourceDescriptions:
  - { name: api, url: api.yaml, type: openapi }
workflows:
  - workflowId: poll
    steps:
      - stepId: wait
        operationId: showJob
        successCriteria:
          - condition: $statusCode == 200 && $response.header.X-State == 'done'
        onFailure:
          - { name: again, type: retry, retryAfter: 0, retryLimit: 2 }
"#,
    )
    .expect("suite");
    // The scripted server always answers the same, so the retries run out.
    let busy = Server::spawn(vec![("GET", "/job", 503, "", "")]);
    let run = test_run(dir.path(), "poll.arazzo.yaml", &busy.base(), &[]);
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.stdout.matches("retry ").count(), 2, "{}", run.stdout);
    assert!(
        run.stdout.contains("$statusCode = 503"),
        "the failure shows what the condition saw: {}",
        run.stdout
    );
}
