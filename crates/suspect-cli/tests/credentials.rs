//! Credentials for the testing flows, end to end through the real binary:
//! secure loading from a credentials file, OAuth grants against a token
//! endpoint, `${VAR}` interpolation from the environment, and injection
//! into operations that declare security requirements.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::Value;

const API: &str = "openapi: 3.1.0
info: {title: Entry, version: '1'}
security: [{tokenAuth: []}]
components:
  securitySchemes:
    tokenAuth: {type: http, scheme: bearer}
paths:
  /models:
    get:
      operationId: listModels
      responses:
        '200': {description: ok}
";

const CHECK: &str = "arazzo: 1.0.0
info: {title: Check, version: '1'}
sourceDescriptions:
  - name: api
    type: openapi
    url: api.yaml
workflows:
  - workflowId: list
    steps:
      - stepId: models
        operationId: listModels
        successCriteria:
          - condition: '{$statusCode} == 200'
";

/// The pattern real workflows use (the Plex suite among them): an explicit
/// header parameter sourced from a required input that no run supplies.
const EXPLICIT_TOKEN: &str = "arazzo: 1.0.0
info: {title: Check, version: '1'}
sourceDescriptions:
  - name: api
    type: openapi
    url: api.yaml
workflows:
  - workflowId: list
    steps:
      - stepId: models
        operationId: listModels
        parameters:
          - name: X-Plex-Token
            in: header
            value: $inputs.plexToken
        successCriteria:
          - condition: '{$statusCode} == 200'
";

/// What the fake servers saw: bearer headers by arrival order, token grant
/// bodies by arrival order.
#[derive(Default)]
struct Saw {
    bearer_headers: Mutex<Vec<String>>,
    token_bodies: Mutex<Vec<String>>,
}

/// One request per connection: read headers, read `content-length` bytes of
/// body, route by path, answer and close.
fn serve(mut stream: TcpStream, saw: Arc<Saw>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let content_length = head
        .lines()
        .find_map(|l| {
            l.split_once(':').and_then(|(k, v)| {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
        })
        .unwrap_or(0);
    while buf.len() < header_end + 4 + content_length {
        let Ok(n) = stream.read(&mut chunk) else {
            break;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = buf[header_end + 4..].to_vec();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    let authorization = head
        .lines()
        .skip(1)
        .find_map(|l| {
            l.split_once(':').and_then(|(k, v)| {
                let k = k.trim().to_ascii_lowercase();
                if k == "authorization" || k == "x-plex-token" {
                    Some(v.trim().to_owned())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    if path == "/token" {
        saw.token_bodies
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&body).into_owned());
        respond(
            &mut stream,
            200,
            br#"{"access_token":"tok-123","expires_in":3600}"#,
        );
    } else if path == "/models" {
        saw.bearer_headers.lock().unwrap().push(authorization);
        respond(&mut stream, 200, br#"[{"id":1}]"#);
    } else {
        respond(&mut stream, 404, br#"{}"#);
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Spins the token + API server on an ephemeral port, returning its base
/// URL and what it has seen.
fn spawn_server() -> (String, Arc<Saw>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let saw = Arc::new(Saw::default());
    let state = saw.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            std::thread::spawn(move || serve(stream, state));
        }
    });
    (url, saw)
}

/// Writes the credentials file with owner-only permissions.
fn write_credentials(dir: &Path, json: Value) -> PathBuf {
    let path = dir.join(".suspect/credentials.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, serde_json::to_string(&json).expect("serializes")).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    path
}

/// Writes the secured OpenAPI + workflow pair.
fn write_suite(dir: &Path) {
    std::fs::write(dir.join("api.yaml"), API).expect("api");
    std::fs::write(dir.join("check.yaml"), CHECK).expect("check");
}

fn suspect() -> Command {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
}

#[test]
fn auth_check_verifies_oauth_grants_without_echoing_secrets() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    // A secret containing `&` and `=` must arrive percent-encoded or the
    // grant is corrupted; it must never reach the terminal.
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {
                "kind": "clientCredentials",
                "token_url": format!("{url}/token"),
                "client_id": "cid",
                "client_secret": "p&a=ss",
            }}}
        }),
    );
    let output = suspect()
        .args(["auth", "check", "--credentials"])
        .arg(&path)
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "auth check must pass: {stdout} {stderr}"
    );
    assert!(
        stdout.contains("all schemes resolve"),
        "the summary names the outcome: {stdout}"
    );
    assert!(
        stdout.contains("value not shown"),
        "placement is reported without the credential: {stdout}"
    );
    assert!(
        !stdout.contains("p&a=ss") && !stderr.contains("p&a=ss"),
        "no secret value ever reaches the output"
    );
    assert_eq!(
        saw.token_bodies.lock().unwrap().clone(),
        ["grant_type=client_credentials&client_id=cid&client_secret=p%26a%3Dss".to_owned()],
        "the grant is a correctly-encoded form body"
    );
}

#[test]
fn test_injects_credentials_into_secured_operations() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "tok-123"}}}
        }),
    );
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url, "--credentials"])
        .arg(&path)
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the secured workflow passes: {stdout} {stderr}"
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["Bearer tok-123".to_owned()],
        "the request arrives carrying the credential"
    );
}

#[test]
fn credentials_interpolate_environment_variables() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "${SUSPECT_TOKEN_VALUE}"}}}
        }),
    );
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url, "--credentials"])
        .arg(&path)
        .env("SUSPECT_TOKEN_VALUE", "envtok")
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the interpolated credential works: {stdout} {stderr}"
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["Bearer envtok".to_owned()],
        "the environment supplied the value, not the file"
    );
}

#[test]
fn missing_environment_references_are_named_in_errors() {
    let (url, _saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "${SUSPECT_UNSET_VAR_9183}"}}}
        }),
    );
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url, "--credentials"])
        .arg(&path)
        .env_remove("SUSPECT_UNSET_VAR_9183")
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    assert!(
        !output.status.success(),
        "an unset reference fails the run instead of sending a literal ${{...}} header"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("SUSPECT_UNSET_VAR_9183"),
        "the error names the variable: {stderr}"
    );
    assert!(
        stderr.contains("not set in the environment"),
        "the error says what happened: {stderr}"
    );
}

#[test]
fn credentials_are_discovered_without_a_flag() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "tok-123"}}}
        }),
    );
    // No --credentials, no env var: discovery walks up from the Arazzo
    // document and finds `.suspect/credentials.json` at the workspace root.
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url])
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "discovery supplies the credentials: {stdout} {stderr}"
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["Bearer tok-123".to_owned()],
        "the discovered file's credential reached the server"
    );
}

#[test]
fn suspect_credentials_env_var_points_at_the_file() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    // The file sits outside the workspace: only the env var finds it.
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let path = write_credentials(
        elsewhere.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "tok-123"}}}
        }),
    );
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url])
        .env("SUSPECT_CREDENTIALS", &path)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "the env var overrides discovery: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["Bearer tok-123".to_owned()]
    );
}

#[test]
#[cfg(unix)]
fn loose_credential_permissions_warn() {
    use std::os::unix::fs::PermissionsExt;
    let (_url, _saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "bearer", "token": "tok"}}}
        }),
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("loosen");
    let output = suspect()
        .args(["auth", "check", "--credentials"])
        .arg(&path)
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "loose permissions warn, they do not fail: {stderr}"
    );
    assert!(
        stderr.contains("chmod 600"),
        "the warning tells you how to fix it: {stderr}"
    );
}

#[test]
fn injection_fills_a_credential_parameter_left_empty_by_a_missing_input() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    // The workflow sets X-Plex-Token explicitly from $inputs.plexToken, but
    // no run supplies the input: the parameter resolves empty and the
    // credential fills it, so real suites work unchanged.
    std::fs::write(dir.path().join("check.yaml"), EXPLICIT_TOKEN).expect("check");
    let path = write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {
                "kind": "apiKey",
                "name": "X-Plex-Token",
                "value": "${PLEX_TOKEN}",
            }}}
        }),
    );
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url, "--credentials"])
        .arg(&path)
        .env("PLEX_TOKEN", "plextok")
        .env_remove("SUSPECT_CREDENTIALS")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the injected credential carries the step: {stdout} {stderr}"
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["plextok".to_owned()],
        "the credential fills the empty parameter; the header carries no Bearer prefix"
    );
}

#[test]
fn env_file_supplies_the_env_values_the_credentials_reference() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    // The credentials reference an env var; the `.suspect/.env` file
    // supplies it; the process environment does NOT — proving the file
    // is the source.
    write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "apiKey", "name": "X-Plex-Token", "value": "${FILE_SUPPLIED_TOKEN}"}}}
        }),
    );
    std::fs::write(
        dir.path().join(".suspect/.env"),
        "FILE_SUPPLIED_TOKEN=env-file-secret\n",
    )
    .expect("env file");
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url])
        .env_remove("FILE_SUPPLIED_TOKEN")
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "the env file supplies the credential: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["env-file-secret".to_owned()],
        "the value came from the env file"
    );
}

#[test]
fn repo_root_env_file_also_supplies_credentials() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "apiKey", "name": "X-Plex-Token", "value": "${REPO_ENV_TOKEN}"}}}
        }),
    );
    // The universal convention: repo-root .env, NOT inside .suspect/.
    std::fs::write(dir.path().join(".env"), "REPO_ENV_TOKEN=repo-root-secret\n").expect("env file");
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url])
        .env_remove("REPO_ENV_TOKEN")
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["repo-root-secret".to_owned()],
        "the value came from the repo-root .env file"
    );
}

#[test]
fn suspect_env_file_is_more_specific_than_the_repo_root_one() {
    let (url, saw) = spawn_server();
    let dir = tempfile::tempdir().expect("tempdir");
    write_suite(dir.path());
    write_credentials(
        dir.path(),
        serde_json::json!({
            "auth": {"schemes": {"tokenAuth": {"kind": "apiKey", "name": "X-Plex-Token", "value": "${LAYERED_TOKEN}"}}}
        }),
    );
    // Both files carry the same variable: the more specific one wins.
    std::fs::write(
        dir.path().join(".suspect/.env"),
        "LAYERED_TOKEN=beside-credentials\n",
    )
    .expect("specific");
    std::fs::write(dir.path().join(".env"), "LAYERED_TOKEN=repo-root\n").expect("repo-root");
    let output = suspect()
        .current_dir(dir.path())
        .arg("test")
        .arg("check.yaml")
        .args(["--base-url", &url])
        .env_remove("LAYERED_TOKEN")
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        saw.bearer_headers.lock().unwrap().clone(),
        ["beside-credentials".to_owned()],
        ".suspect/.env (beside the credentials) wins over the repo-root .env"
    );
}
