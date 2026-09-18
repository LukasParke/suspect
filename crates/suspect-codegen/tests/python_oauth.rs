//! Generated OAuth 2.0 / OpenID Connect token lifecycle: emission shape,
//! conditional-emission controls and native behavioral verification of the
//! emitted `_oauth.py` module over a stubbed httpx transport. Static runtime
//! files and the shared planner stay untouched; everything executable lives in
//! the generated module, and plans without usable schemes emit no new bytes.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::json;
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig},
    python_http::{self, HttpConfig},
    sdk_defaults::SdkDefaults,
    write_files,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const IMPORT: &str = "oauth_sdk";

/// One scheme with client-credentials and authorization-code flows plus one
/// public device-authorization scheme. `/widgets` uses the confidential
/// scheme at document level; `/devices` the device scheme at operation level.
fn fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth.json").unwrap();
    let document = json!({
        // OAS 3.2 admits the deviceAuthorization flow key used by the fixture.
        "openapi": "3.2.0",
        "info": {"title": "OAuth", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"serviceOAuth": ["read"]}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "responses": {"200": {"description": "Ok"}}
            }},
            "/devices": {"get": {
                "operationId": "listDevices",
                "security": [{"deviceOAuth": []}],
                "responses": {"200": {"description": "Ok"}}
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {
                "clientCredentials": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access", "write": "Write access"}
                },
                "authorizationCode": {
                    "authorizationUrl": "https://auth.oauth.test/authorize",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/refresh",
                    "scopes": {"read": "Read access"}
                }
            }},
            "deviceOAuth": {"type": "oauth2", "flows": {
                "deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.oauth.test/device",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                }
            }}
        }}
    });
    contract_with_document(uri, document)
}

fn implicit_only_fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth-implicit.json").unwrap();
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "Legacy", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"legacyOAuth": ["read"]}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}}},
        "components": {"securitySchemes": {"legacyOAuth": {"type": "oauth2", "flows": {
            "implicit": {"authorizationUrl": "https://auth.oauth.test/authorize", "scopes": {"read": "Read access"}}
        }}}}
    });
    contract_with_document(uri, document)
}

fn plain_fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth-plain.json").unwrap();
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "Plain", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}}}
    });
    contract_with_document(uri, document)
}

/// One OpenID Connect scheme (its endpoints the discovery document defines at
/// runtime) plus one OAuth2 scheme with configured auxiliary endpoints.
fn discovery_fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth-discovery.json").unwrap();
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"identityOAuth": []}],
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}},
            "/gadgets": {"get": {
                "operationId": "listGadgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok"}}
            }}
        },
        "components": {"securitySchemes": {
            "identityOAuth": {"type": "openIdConnect", "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"},
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    });
    contract_with_document(uri, document)
}

/// Discovery defaults: a confidential OpenID Connect client plus an OAuth2
/// scheme whose auxiliary endpoints are configured (compiled endpoints win).
fn discovery_defaults() -> SdkDefaults {
    serde_json::from_value(json!({
        "version": "v1",
        "oauth": {
            "mode": "auto",
            "storage": "memory",
            "refresh": "on-demand",
            "schemes": {
                "identityOAuth": {
                    "client_id_env": "OAUTH_DISCOVERY_CLIENT_ID",
                    "client_secret_env": "OAUTH_DISCOVERY_CLIENT_SECRET"
                },
                "serviceOAuth": {
                    "client_id_env": "OAUTH_DISCOVERY_CLIENT_ID",
                    "client_secret_env": "OAUTH_DISCOVERY_CLIENT_SECRET",
                    "revocation_endpoint": "https://auth.oauth.test/revoke",
                    "introspection_endpoint": "https://auth.oauth.test/introspect"
                }
            }
        }
    }))
    .unwrap()
}

fn contract_with_document(uri: Uri, document: serde_json::Value) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}

/// The configured client defaults under test: confidential credentials,
/// skew, and both auxiliary endpoints for the service scheme; a public
/// client id for the device scheme.
fn defaults() -> SdkDefaults {
    serde_json::from_value(json!({
        "version": "v1",
        "oauth": {
            "mode": "auto",
            "storage": "memory",
            "refresh": "on-demand",
            "schemes": {
                "serviceOAuth": {
                    "client_id_env": "OAUTH_CLIENT_ID",
                    "client_secret_env": "OAUTH_CLIENT_SECRET",
                    "refresh_skew_seconds": 30,
                    "revocation_endpoint": "https://auth.oauth.test/revoke",
                    "introspection_endpoint": "https://auth.oauth.test/introspect"
                },
                "deviceOAuth": {"client_id_env": "OAUTH_DEVICE_CLIENT_ID"}
            }
        }
    }))
    .unwrap()
}

fn oauth_off() -> SdkDefaults {
    serde_json::from_value(json!({"version": "v1", "oauth": "off"})).unwrap()
}

fn generate(contract: Arc<Contract>, defaults: Option<SdkDefaults>) -> Vec<OutFile> {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "oauth-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    };
    let options = GenerationOptions {
        sdk_defaults: defaults,
        ..Default::default()
    };
    suspect_codegen::backend::generate_with_options(contract, &selected, &target, &options).unwrap()
}

fn oauth_file(files: &[OutFile]) -> Option<&OutFile> {
    files
        .iter()
        .find(|file| file.path == format!("python/src/{IMPORT}/_oauth.py"))
}

fn fingerprint(files: &[OutFile]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for file in files {
        hash.update((file.path.len() as u64).to_le_bytes());
        hash.update(file.path.as_bytes());
        hash.update((file.content.len() as u64).to_le_bytes());
        hash.update(file.content.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn usable_schemes_emit_the_lifecycle_module_with_compiled_descriptors() {
    let files = generate(fixture(), Some(defaults()));
    let oauth = oauth_file(&files)
        .expect("usable schemes must emit _oauth.py")
        .content
        .clone();

    // The store, provider and typed error.
    for expected in [
        "class AuthError(Exception):",
        "@dataclasses.dataclass(frozen=True, kw_only=True)\nclass TokenSet:",
        "class TokenStore(Protocol):",
        "class MemoryTokenStore(TokenStore):",
        "def client_credential(scheme: str, *",
        "class _ClientCredential:",
        "def _running_loop() -> bool:",
        "asyncio.get_running_loop()",
        "threading.Lock()",
        "asyncio.Lock()",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // Explicit refresh with rotated-token adoption.
    for expected in [
        "def refresh_token_set(scheme: str, token: TokenSet, *",
        "async def refresh_token_set_async(",
        "'grant_type': 'refresh_token'",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // Compiled endpoints appear as frozen constants, never invented.
    for expected in [
        "'serviceOAuth': {",
        "'deviceOAuth': {",
        "'token_url': 'https://auth.oauth.test/token'",
        "'authorization_url': 'https://auth.oauth.test/authorize'",
        "'refresh_url': 'https://auth.oauth.test/refresh'",
        "'device_authorization_url': 'https://auth.oauth.test/device'",
        "'revocation': 'https://auth.oauth.test/revoke'",
        "'introspection': 'https://auth.oauth.test/introspect'",
        "'client_id_env': 'OAUTH_CLIENT_ID'",
        "'client_secret_env': 'OAUTH_CLIENT_SECRET'",
        "'client_id_env': 'OAUTH_DEVICE_CLIENT_ID'",
        "'skew': 30",
        "'client_auth': 'client-secret-basic'",
        "'client_auth': 'none'",
        "'storage': 'memory'",
        "'refresh': 'on-demand'",
        "'read': 'Read access'",
        "'write': 'Write access'",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // Device and auxiliary-endpoint sections exist for this compilation.
    for expected in [
        "def begin_authorization(",
        "def complete_authorization(",
        "async def complete_authorization_async(",
        "class AuthorizationTransaction:",
        "secrets.token_urlsafe",
        "hashlib.sha256",
        "'code_challenge_method': 'S256'",
        "def begin_device_authorization(",
        "def poll_device_authorization(",
        "async def poll_device_authorization_async(",
        "'urn:ietf:params:oauth:grant-type:device_code'",
        "def revoke(",
        "async def revoke_async(",
        "def introspect(",
        "async def introspect_async(",
        "__all__ = ['AuthError', 'TokenSet', 'TokenStore', 'MemoryTokenStore', 'client_credential',",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // No undeclared grant kinds are described or executed here.
    assert!(!oauth.contains("'implicit': {"));
    assert!(!oauth.contains("'password': {"));
    // Client identity resolves at call time; nothing credential-shaped is baked in.
    assert!(oauth.contains("def _environment(variable: str) -> str | None:"));
    assert!(oauth.contains("os.environ.get(variable)"));
}

#[test]
fn discovery_schemes_emit_the_discovery_engine() {
    let files = generate(discovery_fixture(), Some(discovery_defaults()));
    let oauth = oauth_file(&files)
        .expect("discovery schemes emit _oauth.py")
        .content
        .clone();
    // The discovery engine, its typed failure kind and the provider cache.
    for expected in [
        "_DISCOVERY_MAX_BYTES = 1 << 20",
        "def _discovery_document(",
        "def _discovered_endpoint(",
        "def _discover_sync(",
        "async def _discover_async(",
        "def _discovery_auth(",
        "self._discovered_document: dict[str, Any] | None = None",
        "'discovery-failed'",
        "'discovery': 'https://authority.oauth.test/.well-known/openid-configuration'",
        // The documented issuer rule and precedence.
        "sharing the discovery URL's origin",
        "explicitly\n        compiled endpoint always wins",
        // The discovery-aware provider replaced the compiled-only one.
        "def client_credential(scheme: str, *",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // Control: without a discovery URL the engine is absent entirely.
    let plain = generate(fixture(), Some(defaults()));
    let plain_oauth = oauth_file(&plain)
        .expect("the plain fixture still emits the lifecycle")
        .content
        .clone();
    assert!(!plain_oauth.contains("'discovery-failed'"));
    assert!(!plain_oauth.contains("def _discover_sync("));
    assert!(!plain_oauth.contains("_discovery_auth("));
}

#[test]
fn controls_emit_no_new_files_and_keep_no_policy_bytes_identical() {
    // The same OAuth-bearing document with OAuth planning off generates the
    // pre-OAuth bytes exactly: no new files, byte-identical output.
    let without_defaults = generate(fixture(), None);
    let with_off = generate(fixture(), Some(oauth_off()));
    assert!(oauth_file(&without_defaults).is_none());
    assert!(oauth_file(&with_off).is_none());
    assert_eq!(without_defaults.len(), with_off.len());
    for (left, right) in without_defaults.iter().zip(with_off.iter()) {
        assert_eq!(left.path, right.path);
        assert_eq!(left.content, right.content, "{}", left.path);
    }
    assert_eq!(fingerprint(&without_defaults), fingerprint(&with_off));

    // Auto mode without any OAuth scheme, and auto mode with only deprecated
    // flows, both stay at the pre-OAuth file set.
    assert!(oauth_file(&generate(plain_fixture(), Some(SdkDefaults::v1()))).is_none());
    // The deprecated-only document keeps its plain bytes even under auto mode
    // (its plan compiles the declaration but nothing executable).
    let implicit_files = generate(implicit_only_fixture(), Some(SdkDefaults::v1()));
    assert!(oauth_file(&implicit_files).is_none());
    let implicit_plain = generate(implicit_only_fixture(), None);
    assert_eq!(implicit_files.len(), implicit_plain.len());
    for (left, right) in implicit_files.iter().zip(implicit_plain.iter()) {
        assert_eq!(left.path, right.path);
        assert_eq!(left.content, right.content, "{}", left.path);
    }
}

#[test]
fn the_plan_carries_the_compiled_oauth_plan() {
    let contract = fixture();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(
        contract.clone(),
        &selected,
        HttpConfig {
            sdk_defaults: Some(defaults()),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = plan.oauth().expect("configured defaults compile a plan");
    assert_eq!(oauth.mode, suspect_codegen::http_protocol::OAuthMode::Auto);
    assert_eq!(oauth.schemes.len(), 2);
    assert_eq!(oauth.schemes[0].name, "deviceOAuth");
    assert_eq!(
        oauth.schemes[0].flows[0]
            .device_authorization_url
            .as_deref(),
        Some("https://auth.oauth.test/device")
    );
    assert_eq!(oauth.schemes[1].name, "serviceOAuth");
    assert_eq!(oauth.schemes[1].flows.len(), 2);
    assert_eq!(oauth.schemes[1].refresh_skew_seconds, 30);
    assert_eq!(
        oauth.schemes[1].revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );

    // Without client defaults no OAuth plan is computed at all.
    let plan = python_http::plan_http(contract, &selected, HttpConfig::default()).unwrap();
    assert!(plan.oauth().is_none());

    // Off mode yields the empty plan; deprecated-only schemes stay described.
    let implicit = implicit_only_fixture();
    let plan = python_http::plan_http(
        implicit.clone(),
        &selected_of(&implicit),
        HttpConfig {
            sdk_defaults: Some(SdkDefaults::v1()),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = plan.oauth().unwrap();
    assert_eq!(oauth.schemes.len(), 1);
    assert!(
        oauth.schemes[0]
            .flows
            .iter()
            .all(|flow| flow.deprecated_flow)
    );
}

fn selected_of(contract: &Arc<Contract>) -> Vec<suspect_ir::contract::SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

#[test]
fn oauth_configuration_errors_surface_as_generation_diagnostics() {
    let broken: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "oauth": {"schemes": {"ghost": {"client_id_env": "GHOST_ID"}}}
    }))
    .unwrap();
    let contract = fixture();
    let selected = selected_of(&contract);
    let target = TargetConfig {
        backend: Backend::PythonHttp,
        package_name: "oauth-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(IMPORT.into()),
    };
    let options = GenerationOptions {
        sdk_defaults: Some(broken),
        ..Default::default()
    };
    let error =
        suspect_codegen::backend::generate_with_options(contract, &selected, &target, &options)
            .expect_err("a configured scheme binding no used source scheme must fail generation");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "sdk-oauth-config"),
        "{error:?}"
    );
}

#[test]
fn every_emitted_python_file_compiles() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(fixture(), Some(defaults()));
    write_files(&files, root.path()).unwrap();
    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile",
            );
        }
    }
}

fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An interpreter able to import httpx: plain `python3` first, then this
/// repository's native Python tools environment, then nothing.
fn httpx_interpreter() -> Option<PathBuf> {
    fn imports_httpx(python: &Path) -> bool {
        Command::new(python)
            .arg("-c")
            .arg("import httpx")
            .output()
            .is_ok_and(|output| output.status.success())
    }
    if imports_httpx(Path::new("python3")) {
        return Some(PathBuf::from("python3"));
    }
    let candidate = std::env::var_os("SUSPECT_PYTHON_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    imports_httpx(&candidate).then_some(candidate)
}

const DISCOVERY_BEHAVIOR: &str = r##"import asyncio
import base64
import sys
import threading
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from oauth_sdk import AsyncClient, Client
from oauth_sdk import _oauth

IDENTITY = "identityOAuth"
SERVICE = "serviceOAuth"


class Stub(httpx.BaseTransport):
    def __init__(self, responder):
        self.requests = []
        self.responder = responder

    def handle_request(self, request):
        self.requests.append(request)
        return self.responder(request)


class AsyncStub(httpx.AsyncBaseTransport):
    def __init__(self, responder, gate=None, gate_path=None):
        self.requests = []
        self.responder = responder
        self.gate = gate
        self.gate_path = gate_path

    async def handle_async_request(self, request):
        self.requests.append(request)
        response = self.responder(request)
        if self.gate is not None and request.url.path == self.gate_path:
            gate = self.gate
            self.gate = None
            await gate.wait()
        return response


def discovery_document(issuer="https://authority.oauth.test"):
    return {
        "issuer": issuer,
        "token_endpoint": "https://authority.oauth.test/oauth/token",
        "revocation_endpoint": "https://authority.oauth.test/oauth/revoke",
        "introspection_endpoint": "https://authority.oauth.test/oauth/introspect",
        "unknown_member": {"nested": True},
    }


class Server:
    """The fake OpenID Connect provider: discovery, token, revoke, introspect."""

    def __init__(self, discovery=None, token_status=200):
        self.requests = []
        self.discovery = discovery
        self.token_status = token_status
        self.counter = 0

    def responder(self, request):
        self.requests.append(request)
        path = request.url.path
        if path == "/.well-known/openid-configuration":
            assert request.method == "GET", request.method
            assert request.headers["accept"] == "application/json", request.headers
            if self.token_status != 200:
                return httpx.Response(self.token_status)
            return httpx.Response(200, json=self.discovery or discovery_document())
        if path == "/oauth/token":
            self.counter += 1
            return httpx.Response(200, json={
                "access_token": "discovered-%d" % self.counter,
                "token_type": "Bearer",
                "expires_in": 3600,
            })
        if path == "/oauth/revoke":
            return httpx.Response(200)
        if path == "/oauth/introspect":
            return httpx.Response(200, json={"active": True, "scope": "read"})
        if path == "/token":
            return httpx.Response(200, json={"access_token": "compiled", "token_type": "Bearer", "expires_in": 3600})
        if path == "/revoke":
            return httpx.Response(200)
        if path == "/introspect":
            return httpx.Response(200, json={"active": True, "scope": "read"})
        return httpx.Response(200)

    def hits(self, path, method=None):
        return [r for r in self.requests if r.url.path == path and (method is None or r.method == method)]

    def form(self, request):
        return {k: v[0] for k, v in urllib.parse.parse_qs(request.content.decode()).items()}


def expect_auth_error(call, kind):
    try:
        call()
    except _oauth.AuthError as error:
        assert error.kind == kind, (error.kind, error.scheme, error.status)
        return error
    raise AssertionError("expected AuthError(%r)" % kind)


def sync_cc_via_discovered_endpoint_cached_and_single_flighted():
    server = Server()
    transport = Stub(server.responder)
    credential = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                          transport=transport)
    token = credential(None)
    assert token.value == "Bearer discovered-1", token
    assert len(server.hits("/.well-known/openid-configuration", "GET")) == 1, [str(r.url) for r in server.requests]
    acquire = server.hits("/oauth/token", "POST")[0]
    assert str(acquire.url) == "https://authority.oauth.test/oauth/token", str(acquire.url)
    assert server.form(acquire) == {"grant_type": "client_credentials"}, server.form(acquire)
    assert acquire.headers["authorization"] == "Basic " + base64.b64encode(b"disc-id:disc-secret").decode()
    # Cached discovery and cached token: the second attach fetches nothing.
    assert credential(None).value == "Bearer discovered-1"
    assert len(server.hits("/.well-known/openid-configuration")) == 1
    assert len(server.hits("/oauth/token")) == 1

    # Two concurrent callers share one discovery fetch and one acquisition.
    server.requests.clear()
    flight = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                      transport=transport)
    results = []
    lock = threading.Lock()

    def call():
        value = flight(None)
        with lock:
            results.append(value.value)

    threads = [threading.Thread(target=call) for _ in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert len(server.hits("/.well-known/openid-configuration")) == 1, len(server.hits("/.well-known/openid-configuration"))
    assert len(server.hits("/oauth/token")) == 1
    assert len({value for value in results}) == 1


def sync_end_to_end_via_the_generated_client():
    server = Server()
    transport = Stub(server.responder)
    credential = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                          transport=transport)
    with Client(auth={IDENTITY: credential}, transport=transport) as client:
        client.list_widgets()
    api = [r for r in server.requests if r.url.path.endswith("/widgets")]
    assert len(api) == 1 and api[0].headers["authorization"] == "Bearer discovered-1", [str(r.url) for r in server.requests]


def issuer_mismatch_is_typed_and_the_next_call_retries():
    server = Server(discovery=discovery_document(issuer="https://elsewhere.oauth.test"))
    transport = Stub(server.responder)
    credential = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                          transport=transport)
    error = expect_auth_error(lambda: credential(None), "discovery-failed")
    assert "elsewhere" not in str(error) and "elsewhere" not in repr(error), (str(error), repr(error))
    # The failed fetch is not cached: after the server is fixed the next call
    # retries and succeeds.
    server.discovery = discovery_document()
    assert credential(None).value == "Bearer discovered-1"
    assert len(server.hits("/.well-known/openid-configuration")) == 2


def failed_discovery_fetch_is_typed_and_retried():
    server = Server(token_status=500)
    transport = Stub(server.responder)
    credential = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                          transport=transport)
    error = expect_auth_error(lambda: credential(None), "discovery-failed")
    assert error.status == 500, error.status
    server.token_status = 200
    assert credential(None).value == "Bearer discovered-1"
    assert len(server.hits("/.well-known/openid-configuration")) == 2


def refresh_revocation_and_introspection_resolve_through_discovery():
    server = Server()
    transport = Stub(server.responder)
    refreshed = _oauth.refresh_token_set(IDENTITY, _oauth.TokenSet(access_token="t", refresh_token="rt-1"),
                                         client_id="disc-id", client_secret="disc-secret",
                                         transport=transport)
    refresh_request = server.hits("/oauth/token", "POST")[0]
    assert server.form(refresh_request) == {"grant_type": "refresh_token", "refresh_token": "rt-1"}, server.form(refresh_request)
    assert refreshed.access_token == "discovered-1"
    # Revocation and introspection resolve through discovery for the scheme
    # with no configured endpoints, and the compiled endpoint wins for the
    # scheme with configured ones.
    _oauth.revoke(IDENTITY, "tkn-live", client_id="disc-id", client_secret="disc-secret", transport=transport)
    assert str(server.hits("/oauth/revoke", "POST")[0].url) == "https://authority.oauth.test/oauth/revoke"
    _oauth.revoke(SERVICE, "tkn-live", client_id="disc-id", client_secret="disc-secret", transport=transport)
    assert str(server.hits("/revoke", "POST")[0].url) == "https://auth.oauth.test/revoke"
    claims = _oauth.introspect(IDENTITY, "tkn-live", client_id="disc-id", client_secret="disc-secret",
                               transport=transport)
    assert claims == {"active": True, "scope": "read"}, claims
    assert str(server.hits("/oauth/introspect", "POST")[0].url) == "https://authority.oauth.test/oauth/introspect"


async def async_checks():
    server = Server()
    transport = AsyncStub(server.responder)
    credential = _oauth.client_credential(IDENTITY, client_id="disc-id", client_secret="disc-secret",
                                          transport=transport)
    async with AsyncClient(auth={IDENTITY: credential}, transport=transport) as client:
        await client.list_widgets()
    assert len(server.hits("/.well-known/openid-configuration")) == 1, len(server.hits("/.well-known/openid-configuration"))
    first, second = await asyncio.gather(credential(None), credential(None))
    assert first.value == second.value
    assert len(server.hits("/.well-known/openid-configuration")) == 1
    assert len(server.hits("/oauth/token")) == 1
    # Async refresh resolves through discovery too.
    refreshed = await _oauth.refresh_token_set_async(IDENTITY, _oauth.TokenSet(access_token="t", refresh_token="rt-1"),
                                                     client_id="disc-id", client_secret="disc-secret",
                                                     transport=AsyncStub(server.responder))
    assert refreshed.access_token.startswith("discovered-")
    # Async revocation resolves through discovery.
    await _oauth.revoke_async(IDENTITY, refreshed, client_id="disc-id", client_secret="disc-secret",
                              transport=AsyncStub(server.responder))
    assert any(r.url.path == "/oauth/revoke" for r in server.requests)


def main():
    sync_cc_via_discovered_endpoint_cached_and_single_flighted()
    sync_end_to_end_via_the_generated_client()
    issuer_mismatch_is_typed_and_the_next_call_retries()
    failed_discovery_fetch_is_typed_and_retried()
    refresh_revocation_and_introspection_resolve_through_discovery()
    asyncio.run(async_checks())
    print("discovery behavior verified")


main()
"##;

#[test]
fn native_discovery_lifecycle_over_a_stubbed_transport() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(discovery_fixture(), Some(discovery_defaults()));
    write_files(&files, root.path()).unwrap();

    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile-discovery",
            );
        }
    }

    let Some(python) = httpx_interpreter() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(
        root.path().join("discovery_behavior.py"),
        DISCOVERY_BEHAVIOR,
    )
    .unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("discovery_behavior.py"))
            .current_dir(root.path()),
        root.path(),
        "discovery-behavior",
    );
}

const BEHAVIOR: &str = r##"import asyncio
import base64
import hashlib
import os
import sys
import threading
import time
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from oauth_sdk import AsyncClient, Client, SdkError
from oauth_sdk import _oauth

SERVICE_KEY = ("serviceOAuth", "https://auth.oauth.test", "cid")
DEVICE_KEY = ("deviceOAuth", "https://auth.oauth.test", "dev-cid")
API = "api.oauth.test"
AUTH = "auth.oauth.test"


def token_json(index, **extra):
    payload = {"access_token": "tkn-%d" % index, "token_type": "Bearer", "expires_in": 3600, "scope": "read"}
    payload.update(extra)
    return payload


def token_count(requests):
    return [r for r in requests if r.url.host == AUTH]


class Stub(httpx.BaseTransport):
    def __init__(self, responder):
        self.requests = []
        self.responder = responder

    def handle_request(self, request):
        self.requests.append(request)
        return self.responder(request)


class AsyncStub(httpx.AsyncBaseTransport):
    def __init__(self, responder, gate=None, gate_path=None):
        self.requests = []
        self.responder = responder
        self.gate = gate
        self.gate_path = gate_path

    async def handle_async_request(self, request):
        self.requests.append(request)
        response = self.responder(request)
        if self.gate is not None and request.url.path == self.gate_path:
            gate = self.gate
            self.gate = None
            await gate.wait()
        return response


def form(request):
    return {k: v[0] for k, v in urllib.parse.parse_qs(request.content.decode()).items()}


def expect_auth_error(call, kind, code=None):
    try:
        call()
    except _oauth.AuthError as error:
        assert error.kind == kind, (error.kind, error.scheme, error.code, error.status)
        if code is not None:
            assert error.code == code, (error.kind, error.code)
        return error
    raise AssertionError("expected AuthError(%r)" % kind)


def sync_full_client_acquire_cache_hit_and_reacquire():
    requests = []

    def responder(request):
        requests.append(request)
        if request.url.host != AUTH:
            return httpx.Response(200)
        index = len(token_count(requests)) - 1
        return httpx.Response(200, json=token_json(index))

    clock = [1000.0]
    transport = Stub(responder)
    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                          clock=lambda: clock[0], transport=transport)
    with Client(auth={"serviceOAuth": credential}, transport=transport) as client:
        client.list_widgets()
        client.list_widgets()
    token_requests = token_count(requests)
    assert len(token_requests) == 1, len(token_requests)
    request = token_requests[0]
    assert str(request.url) == "https://auth.oauth.test/token", str(request.url)
    assert form(request) == {"grant_type": "client_credentials"}, form(request)
    assert request.headers["authorization"] == "Basic " + base64.b64encode(b"cid:csecret").decode(), request.headers["authorization"]
    operations = [r for r in requests if r.url.host == API]
    assert len(operations) == 2, len(operations)
    assert operations[0].headers["authorization"] == "Bearer tkn-0", operations[0].headers["authorization"]
    assert operations[1].headers["authorization"] == "Bearer tkn-0"
    # 3600-30 seconds later the cached set is expired and the next attach reacquires.
    clock[0] = 1000.0 + 3600 - 30 + 5
    credential(None)
    assert len(token_count(requests)) == 2, len(token_count(requests))


def sync_single_flight_threads_share_one_token_request():
    requests = []

    def responder(request):
        requests.append(request)
        if request.url.host != AUTH:
            return httpx.Response(200)
        time.sleep(0.2)
        index = len(token_count(requests)) - 1
        return httpx.Response(200, json=token_json(index))

    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                          transport=Stub(responder))
    results = []
    lock = threading.Lock()

    def call():
        value = credential(None)
        with lock:
            results.append(value)

    threads = [threading.Thread(target=call) for _ in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert len(token_count(requests)) == 1, len(token_count(requests))
    assert len({value.value for value in results}) == 1, results


def sync_refresh_adopts_rotated_token_and_replaces_the_store():
    requests = []

    def responder(request):
        requests.append(request)
        return httpx.Response(200, json={"access_token": "tkn-2", "token_type": "Bearer",
                                         "expires_in": 100, "refresh_token": "rt-new"})

    store = _oauth.MemoryTokenStore()
    old = _oauth.TokenSet(access_token="tkn-1", refresh_token="rt-old", expires_at=2000)
    refreshed = _oauth.refresh_token_set("serviceOAuth", old, client_id="cid", client_secret="csecret",
                                         store=store, transport=Stub(responder))
    request = requests[0]
    assert str(request.url) == "https://auth.oauth.test/refresh", str(request.url)
    assert form(request) == {"grant_type": "refresh_token", "refresh_token": "rt-old"}, form(request)
    assert refreshed.refresh_token == "rt-new"
    assert refreshed.access_token == "tkn-2"
    assert store.load(SERVICE_KEY) == refreshed, store.load(SERVICE_KEY)
    # Without a rotated token the current refresh token is retained.
    requests.clear()

    def plain(request):
        requests.append(request)
        return httpx.Response(200, json={"access_token": "tkn-3", "token_type": "Bearer", "expires_in": 100})

    retained = _oauth.refresh_token_set("serviceOAuth", refreshed, client_id="cid", client_secret="csecret",
                                        transport=Stub(plain))
    assert retained.refresh_token == "rt-new", retained.refresh_token
    # Refreshing without a refresh token is a typed refusal.
    expect_auth_error(lambda: _oauth.refresh_token_set("serviceOAuth", _oauth.TokenSet(access_token="x")),
                      "no-refresh-token")


def sync_wrong_credentials_raise_typed_error_without_secrets():
    secret = "top-secret-value"
    requests = []

    def responder(request):
        requests.append(request)
        # The description deliberately echoes the secret; it must never surface.
        return httpx.Response(400, json={"error": "invalid_client", "error_description": "bad " + secret})

    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret=secret,
                                          transport=Stub(responder))
    error = expect_auth_error(lambda: credential(None), "token-request", "invalid_client")
    assert error.status == 400, error.status
    assert secret not in str(error), str(error)
    assert secret not in repr(error), repr(error)


def env_read_at_call_time_and_missing_credentials_refused():
    requests = []

    def responder(request):
        requests.append(request)
        return httpx.Response(200, json=token_json(0))

    os.environ["OAUTH_CLIENT_ID"] = "env-cid"
    try:
        credential = _oauth.client_credential("serviceOAuth", client_secret="env-secret",
                                              transport=Stub(responder))
        credential(None)
    finally:
        del os.environ["OAUTH_CLIENT_ID"]
    expected = "Basic " + base64.b64encode(b"env-cid:env-secret").decode()
    assert requests[0].headers["authorization"] == expected, requests[0].headers["authorization"]
    # Nothing configured and nothing resolvable: a typed refusal, no request.
    requests.clear()
    credential = _oauth.client_credential("serviceOAuth", transport=Stub(responder))
    expect_auth_error(lambda: credential(None), "missing-client-credentials")
    assert not requests


def sync_revocation_and_introspection():
    requests = []

    def responder(request):
        requests.append(request)
        if request.url.path == "/revoke":
            return httpx.Response(200)
        if request.url.path == "/introspect":
            return httpx.Response(200, json={"active": True, "scope": "read"})
        return httpx.Response(200, json=token_json(0))

    store = _oauth.MemoryTokenStore()
    transport = Stub(responder)
    store.replace(SERVICE_KEY, _oauth.TokenSet(access_token="tkn-live"))
    _oauth.revoke("serviceOAuth", "tkn-live", store=store, client_id="cid", client_secret="csecret",
                  transport=transport)
    revoke_request = requests[0]
    assert str(revoke_request.url) == "https://auth.oauth.test/revoke", str(revoke_request.url)
    assert form(revoke_request) == {"token": "tkn-live"}, form(revoke_request)
    assert store.load(SERVICE_KEY) is None, "revocation must clear the partition"
    claims = _oauth.introspect("serviceOAuth", "tkn-live", client_id="cid", client_secret="csecret",
                               transport=transport)
    assert claims == {"active": True, "scope": "read"}, claims
    introspect_request = requests[1]
    assert str(introspect_request.url) == "https://auth.oauth.test/introspect", str(introspect_request.url)
    assert form(introspect_request)["token"] == "tkn-live"
    # An explicit hint travels as the RFC 7009 hint field.
    requests.clear()
    _oauth.revoke("serviceOAuth", "tkn-live", token_type_hint="refresh_token", client_id="cid",
                  client_secret="csecret", transport=transport)
    assert form(requests[0]) == {"token": "tkn-live", "token_type_hint": "refresh_token"}, form(requests[0])
    # TokenSet inputs work, and a server refusal is typed without body leakage.
    error = expect_auth_error(
        lambda: _oauth.revoke("serviceOAuth", _oauth.TokenSet(access_token="tkn-x"), client_id="cid",
                              client_secret="csecret",
                              transport=Stub(lambda r: httpx.Response(400, json={"error": "unsupported_token_type"}))),
        "revocation", "unsupported_token_type")
    assert "tkn-x" not in str(error), str(error)


def pkce_authorization_url_and_single_use_completion():
    requests = []

    def responder(request):
        requests.append(request)
        return httpx.Response(200, json={"access_token": "tkn-auth", "token_type": "Bearer",
                                         "expires_in": 600, "refresh_token": "rt-auth"})

    store = _oauth.MemoryTokenStore()
    transaction = _oauth.begin_authorization("serviceOAuth", redirect_uri="http://127.0.0.1:8765/callback",
                                             scopes=("read",), client_id="cid")
    parts = urllib.parse.urlsplit(transaction.authorization_url)
    query = {k: v[0] for k, v in urllib.parse.parse_qs(parts.query).items()}
    assert parts.scheme == "https" and parts.netloc == AUTH and parts.path == "/authorize", parts
    assert query["response_type"] == "code" and query["client_id"] == "cid", query
    assert query["redirect_uri"] == "http://127.0.0.1:8765/callback", query
    assert query["scope"] == "read", query
    assert query["code_challenge_method"] == "S256", query
    digest = hashlib.sha256(transaction.code_verifier.encode("ascii")).digest()
    assert query["code_challenge"] == base64.urlsafe_b64encode(digest).decode().rstrip("=")
    assert 43 <= len(transaction.code_verifier) <= 128, len(transaction.code_verifier)
    assert transaction.code_verifier not in repr(transaction), repr(transaction)
    token = _oauth.complete_authorization(transaction, {"state": query["state"], "code": "abc"},
                                          store=store, client_id="cid", client_secret="csecret",
                                          transport=Stub(responder))
    assert token.access_token == "tkn-auth", token
    exchange = requests[0]
    assert str(exchange.url) == "https://auth.oauth.test/token", str(exchange.url)
    sent = form(exchange)
    assert sent["grant_type"] == "authorization_code" and sent["code"] == "abc", sent
    assert sent["code_verifier"] == transaction.code_verifier, sent
    assert sent["redirect_uri"] == "http://127.0.0.1:8765/callback", sent
    assert exchange.headers["authorization"] == "Basic " + base64.b64encode(b"cid:csecret").decode()
    assert store.load(SERVICE_KEY) == token
    # The transaction is bound and single-use.
    expect_auth_error(lambda: _oauth.complete_authorization(transaction, {"state": query["state"], "code": "abc"}),
                      "transaction-used")
    fresh = _oauth.begin_authorization("serviceOAuth", redirect_uri="http://127.0.0.1:8765/callback", client_id="cid")
    expect_auth_error(lambda: _oauth.complete_authorization(fresh, {"state": "other", "code": "abc"}),
                      "state-mismatch")
    denied = _oauth.begin_authorization("serviceOAuth", redirect_uri="http://127.0.0.1:8765/callback", client_id="cid")
    expect_auth_error(lambda: _oauth.complete_authorization(denied, {"state": denied.state, "error": "access_denied"}),
                      "authorization-denied", "access_denied")
    # Declared-only scope vocabulary.
    expect_auth_error(
        lambda: _oauth.begin_authorization("serviceOAuth", redirect_uri="http://127.0.0.1:8765/callback",
                                           scopes=("admin",), client_id="cid"),
        "unknown-scope")


def device_flow_begin_poll_and_expiry():
    requests = []
    pauses = []
    grants = iter(["pending", "pending", "slow", "done"])

    def responder(request):
        requests.append(request)
        if request.url.path == "/device":
            return httpx.Response(200, json={"device_code": "dev-1", "user_code": "ABCD-EFGH",
                                             "verification_uri": "https://auth.oauth.test/activate",
                                             "verification_uri_complete": "https://auth.oauth.test/activate?code=ABCD-EFGH",
                                             "expires_in": 1800, "interval": 2})
        which = next(grants)
        if which == "pending":
            return httpx.Response(400, json={"error": "authorization_pending"})
        if which == "slow":
            return httpx.Response(400, json={"error": "slow_down"})
        return httpx.Response(200, json={"access_token": "tkn-dev", "token_type": "Bearer", "expires_in": 900})

    transport = Stub(responder)
    clock = [5000.0]
    grant = _oauth.begin_device_authorization("deviceOAuth", client_id="dev-cid", clock=lambda: clock[0],
                                              transport=transport)
    assert grant.user_code == "ABCD-EFGH" and grant.verification_uri == "https://auth.oauth.test/activate"
    assert grant.interval == 2.0 and grant.expires_at == 6800, (grant.interval, grant.expires_at)
    assert "dev-1" not in repr(grant), repr(grant)
    begin_request = requests[0]
    assert str(begin_request.url) == "https://auth.oauth.test/device", str(begin_request.url)
    assert form(begin_request) == {"client_id": "dev-cid"}, form(begin_request)
    store = _oauth.MemoryTokenStore()
    token = _oauth.poll_device_authorization(grant, client_id="dev-cid", store=store, clock=lambda: clock[0],
                                             transport=transport, wait=lambda seconds: pauses.append(seconds))
    assert token.access_token == "tkn-dev", token
    assert pauses == [2.0, 2.0, 7.0], pauses
    poll_requests = [r for r in requests if r.url.path == "/token"]
    assert len(poll_requests) == 4, len(poll_requests)
    assert form(poll_requests[0]) == {"grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                                      "device_code": "dev-1", "client_id": "dev-cid"}, form(poll_requests[0])
    assert store.load(DEVICE_KEY) == token
    clock[0] = 6801.0
    expect_auth_error(
        lambda: _oauth.poll_device_authorization(grant, client_id="dev-cid", clock=lambda: clock[0],
                                                 transport=transport, wait=lambda seconds: None),
        "device-flow-expired")
    # Any other server refusal is typed and stays free of token values.
    refusal = _oauth.DeviceAuthorization(scheme="deviceOAuth", device_code="dev-2", user_code="X",
                                         verification_uri="https://auth.oauth.test/activate",
                                         expires_at=9000, interval=1.0)
    error = expect_auth_error(
        lambda: _oauth.poll_device_authorization(refusal, client_id="dev-cid", clock=lambda: 1000.0,
                                                 transport=Stub(lambda r: httpx.Response(400, json={"error": "access_denied"})),
                                                 wait=lambda seconds: None),
        "token-request", "access_denied")
    assert "dev-2" not in str(error), str(error)


def sync_client_wraps_credential_failures_as_authentication():
    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret="s",
                                          transport=Stub(lambda r: httpx.Response(400, json={"error": "invalid_client"})))
    with Client(auth={"serviceOAuth": credential}, transport=Stub(lambda r: httpx.Response(200))) as client:
        try:
            client.list_widgets()
        except SdkError as error:
            assert error.kind == "authentication", error.kind
            assert isinstance(error.cause, _oauth.AuthError), error.cause
        else:
            raise AssertionError("expected the attach path to wrap AuthError")


async def async_checks():
    # Acquire, cache hit, single flight and expiry under asyncio.
    requests = []

    def responder(request):
        requests.append(request)
        if request.url.host != AUTH:
            return httpx.Response(200)
        index = len(token_count(requests)) - 1
        return httpx.Response(200, json=token_json(index))

    transport = AsyncStub(responder)
    clock = [1000.0]
    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                          clock=lambda: clock[0], transport=transport)
    async with AsyncClient(auth={"serviceOAuth": credential}, transport=transport) as client:
        await client.list_widgets()
        await client.list_widgets()
    assert len(token_count(requests)) == 1, len(token_count(requests))
    first, second = await asyncio.gather(credential(None), credential(None))
    assert first.value == second.value
    assert len(token_count(requests)) == 1, len(token_count(requests))
    clock[0] = 1000.0 + 3600 - 30 + 5
    await credential(None)
    assert len(token_count(requests)) == 2, len(token_count(requests))
    # Async refresh adopts the rotated token and replaces the store.
    requests.clear()

    def refresh_responder(request):
        requests.append(request)
        return httpx.Response(200, json={"access_token": "tkn-2", "token_type": "Bearer", "refresh_token": "rt-new"})

    store = _oauth.MemoryTokenStore()
    refreshed = await _oauth.refresh_token_set_async("serviceOAuth", _oauth.TokenSet(access_token="t", refresh_token="rt-old"),
                                                     client_id="cid", client_secret="csecret", store=store,
                                                     transport=AsyncStub(refresh_responder))
    assert refreshed.refresh_token == "rt-new", refreshed
    assert store.load(SERVICE_KEY) == refreshed
    await _oauth.revoke_async("serviceOAuth", refreshed, store=store, client_id="cid", client_secret="csecret",
                              transport=AsyncStub(lambda r: httpx.Response(200)))
    assert store.load(SERVICE_KEY) is None
    claims = await _oauth.introspect_async("serviceOAuth", "tkn-1", client_id="cid", client_secret="csecret",
                                           transport=AsyncStub(lambda r: httpx.Response(200, json={"active": False})))
    assert claims == {"active": False}, claims
    # Async authorization-code completion stays single-use.
    transaction = _oauth.begin_authorization("serviceOAuth", redirect_uri="http://127.0.0.1:8765/callback", client_id="cid")
    completed = await _oauth.complete_authorization_async(transaction, {"state": transaction.state, "code": "abc"},
                                                          client_id="cid", client_secret="csecret",
                                                          transport=AsyncStub(lambda r: httpx.Response(200, json={"access_token": "tkn-ac", "token_type": "Bearer"})))
    assert completed.access_token == "tkn-ac", completed
    expect_auth_error(lambda: _oauth.complete_authorization(transaction, {"state": transaction.state, "code": "abc"}),
                      "transaction-used")
    # Async device polling honors the interval via the injected no-op wait.
    pauses = []
    grants = iter(["pending", "done"])

    def poll_responder(request):
        which = next(grants)
        if which == "pending":
            return httpx.Response(400, json={"error": "authorization_pending"})
        return httpx.Response(200, json={"access_token": "tkn-dev2", "token_type": "Bearer"})

    grant = _oauth.begin_device_authorization("deviceOAuth", client_id="dev-cid",
                                              transport=Stub(lambda r: httpx.Response(200, json={
                                                  "device_code": "dev-3", "user_code": "ZZ",
                                                  "verification_uri": "https://auth.oauth.test/activate",
                                                  "expires_in": 600, "interval": 3})))
    token = await _oauth.poll_device_authorization_async(grant, client_id="dev-cid", transport=AsyncStub(poll_responder),
                                                         wait=lambda seconds: pauses.append(seconds))
    assert token.access_token == "tkn-dev2", token
    assert pauses == [3.0], pauses


def main():
    sync_full_client_acquire_cache_hit_and_reacquire()
    sync_single_flight_threads_share_one_token_request()
    sync_refresh_adopts_rotated_token_and_replaces_the_store()
    sync_wrong_credentials_raise_typed_error_without_secrets()
    env_read_at_call_time_and_missing_credentials_refused()
    sync_revocation_and_introspection()
    pkce_authorization_url_and_single_use_completion()
    device_flow_begin_poll_and_expiry()
    sync_client_wraps_credential_failures_as_authentication()
    asyncio.run(async_checks())
    print("oauth behavior verified")


main()
"##;

#[test]
fn native_lifecycle_over_a_stubbed_transport() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(fixture(), Some(defaults()));
    write_files(&files, root.path()).unwrap();

    // Every emitted Python file must at least be valid bytecode.
    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile",
            );
        }
    }

    let Some(python) = httpx_interpreter() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(root.path().join("behavior.py"), BEHAVIOR).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("behavior.py"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
}

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint.
fn replay_fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth-replay.json").unwrap();
    let document = json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok"}}
            }},
            "/events": {"get": {
                "operationId": "streamEvents",
                "security": [{"feedOAuth": ["read"]}],
                "responses": {"200": {"description": "Events", "content": {"text/event-stream": {
                    "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                }}}}
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}},
            "feedOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/feed-token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    });
    contract_with_document(uri, document)
}

fn replay_defaults() -> SdkDefaults {
    serde_json::from_value(json!({
        "version": "v1",
        "oauth": {
            "mode": "auto",
            "storage": "memory",
            "refresh": "on-demand",
            "schemes": {
                "serviceOAuth": {
                    "client_id_env": "OAUTH_REPLAY_CLIENT_ID",
                    "client_secret_env": "OAUTH_REPLAY_CLIENT_SECRET"
                },
                "feedOAuth": {
                    "client_id_env": "OAUTH_REPLAY_FEED_ID",
                    "client_secret_env": "OAUTH_REPLAY_FEED_SECRET"
                }
            }
        }
    }))
    .unwrap()
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(replay_fixture(), Some(replay_defaults()));
    let oauth = oauth_file(&files)
        .expect("the replay fixture emits _oauth.py")
        .content
        .clone();
    for expected in [
        "def replaying_credential(scheme: str, *",
        "class _ReplayingCredential:",
        "class _ReplayTransport(httpx.BaseTransport):",
        "_NO_REPLAY_REQUIREMENTS: dict[str, frozenset[str]] = {",
        "'/paths/~1events/get/security/0/feedOAuth'",
        "'replaying_credential'",
    ] {
        assert!(oauth.contains(expected), "missing: {expected}");
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("~1widgets/get/security/0/serviceOAuth"));
    // A package without any executable client-credentials flow compiles
    // exactly the pre-replay bytes.
    let code_only = generate(interactive_code_fixture(), Some(code_only_defaults()));
    let plain_oauth = oauth_file(&code_only)
        .expect("the code-only fixture still emits")
        .content
        .clone();
    assert!(!plain_oauth.contains("replaying_credential"));
    assert!(!plain_oauth.contains("_NO_REPLAY_REQUIREMENTS"));
}

/// An authorization-code-only scheme (no client-credentials flow): the
/// control for the replay emission gate.
fn interactive_code_fixture() -> Arc<Contract> {
    let uri = Uri::parse("https://source.oauth.test/python-oauth-code.json").unwrap();
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"userOAuth": ["read"]}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    });
    contract_with_document(uri, document)
}

fn code_only_defaults() -> SdkDefaults {
    serde_json::from_value(json!({
        "version": "v1",
        "oauth": {"schemes": {"userOAuth": {
            "client_id_env": "OAUTH_CODE_ONLY_ID",
            "client_secret_env": "OAUTH_CODE_ONLY_SECRET"
        }}}
    }))
    .unwrap()
}

const REPLAY_BEHAVIOR: &str = r##"import asyncio
import base64
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "python" / "src"))

import httpx

from oauth_sdk import AsyncClient, Client, SdkError
from oauth_sdk import _oauth

WIDGETS = "/v1/widgets"
EVENTS = "/v1/events"


class Stub(httpx.BaseTransport):
    def __init__(self, responder):
        self.requests = []
        self.responder = responder

    def handle_request(self, request):
        self.requests.append(request)
        return self.responder(request)


class AsyncStub(httpx.AsyncBaseTransport):
    def __init__(self, responder, gate=None, gate_path=None):
        self.requests = []
        self.responder = responder
        self.gate = gate
        self.gate_path = gate_path

    async def handle_async_request(self, request):
        self.requests.append(request)
        response = self.responder(request)
        if self.gate is not None and request.url.path == self.gate_path:
            gate = self.gate
            self.gate = None
            await gate.wait()
        return response


class Server:
    """Fake API and token endpoints. ``arm_stale`` makes the next issued
    token answer 401 on the API, so the driver counts exactly one refresh."""

    def __init__(self):
        self.requests = []
        self.service_tokens = 0
        self.feed_tokens = 0
        self.mode = "ok"
        self.stale_next = False
        self.stale_token = None
        self.fail_from = None
        self.lock = threading.Lock()

    def arm_stale(self):
        with self.lock:
            self.stale_next = True
            self.mode = "ok"

    def always_401(self):
        with self.lock:
            self.mode = "always-401"

    def responder(self, request):
        with self.lock:
            self.requests.append(request)
            path = request.url.path
            if path == "/token":
                self.service_tokens += 1
                index = self.service_tokens
                stale_next = self.stale_next
                if stale_next:
                    self.stale_next = False
                    self.stale_token = "svc-%d" % index
                if self.fail_from is not None and index >= self.fail_from:
                    return httpx.Response(500, json={"error": "server_error"})
                return httpx.Response(200, json={
                    "access_token": "svc-%d" % index, "token_type": "Bearer", "expires_in": 3600,
                })
            if path == "/feed-token":
                self.feed_tokens += 1
                return httpx.Response(200, json={
                    "access_token": "feed-%d" % self.feed_tokens, "token_type": "Bearer", "expires_in": 3600,
                })
            if path == WIDGETS:
                stale = self.stale_token
                mode = self.mode
                presented = request.headers.get("authorization")
                if mode == "always-401" or (stale is not None and presented == "Bearer " + stale):
                    return httpx.Response(401, json={"error": "stale"})
                return httpx.Response(200, json={"ok": True})
            if path == EVENTS:
                return httpx.Response(401, json={"error": "stream-denied"})
            return httpx.Response(404)

    def hits(self, path):
        with self.lock:
            return [r for r in self.requests if r.url.path == path]


def count_stale_and_replay(server, transport, async_mode=False):
    """(a) 401 then success: one refresh, one replay, 200 surfaced."""
    server.requests.clear()
    server.arm_stale()
    clock = [1000.0]
    if async_mode:
        stub = AsyncStub(server.responder)
        credential = _oauth.replaying_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                                 clock=lambda: clock[0], transport=stub)
        async def run():
            async with AsyncClient(auth={"serviceOAuth": credential},
                                   transport=credential.replay_transport(stub)) as client:
                return await client.list_widgets()
        asyncio.run(run())
    else:
        stub = Stub(server.responder)
        credential = _oauth.replaying_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                                 clock=lambda: clock[0], transport=stub)
        with Client(auth={"serviceOAuth": credential},
                    transport=credential.replay_transport(stub)) as client:
            client.list_widgets()
    assert len(server.hits(WIDGETS)) == 2, len(server.hits(WIDGETS))
    assert len(server.hits("/token")) == 2, len(server.hits("/token"))
    replayed = server.hits(WIDGETS)[1]
    first = server.hits(WIDGETS)[0]
    assert replayed.headers["authorization"] != first.headers["authorization"], "the replay carried the fresh token"
    assert first.headers["authorization"] == "Bearer " + server.stale_token, first.headers["authorization"]


def count_401_then_401(server):
    """(b) the second 401 surfaces; exactly one refresh."""
    server.requests.clear()
    server.always_401()
    clock = [1000.0]
    stub = Stub(server.responder)
    credential = _oauth.replaying_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                             clock=lambda: clock[0], transport=stub)
    with Client(auth={"serviceOAuth": credential},
                transport=credential.replay_transport(stub)) as client:
        try:
            client.list_widgets()
        except SdkError as error:
            assert error.status == 401, (error.kind, error.status)
        else:
            raise AssertionError("expected the second 401 to surface")
    assert len(server.hits(WIDGETS)) == 2, "one replay, no loops"
    assert len(server.hits("/token")) == 2, "exactly one refresh"


def concurrent_401s_share_one_refresh():
    """(c) two concurrent 401s across two tasks: ONE refresh, two replays."""
    server = Server()
    server.arm_stale()
    stub = AsyncStub(server.responder)
    credential = _oauth.replaying_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                             transport=stub)
    wrapper = credential.replay_transport(stub)

    async def run():
        first = await credential(None)
        second = await credential(None)
        assert first.value == second.value, "both attaches serve the same token"
        async def send():
            request = httpx.Request("GET", "https://api.oauth.test/v1/widgets")
            request.headers["authorization"] = first.value
            return await wrapper.handle_async_request(request)
        return await asyncio.gather(send(), send())

    responses = asyncio.run(run())
    assert all(response.status_code == 200 for response in responses), [r.status_code for r in responses]
    stale_value = "Bearer " + server.stale_token
    originals = [r for r in server.hits(WIDGETS) if r.headers["authorization"] == stale_value]
    replays = [r for r in server.hits(WIDGETS) if r.headers["authorization"] != stale_value]
    assert len(originals) == 2 and len(replays) == 2, (len(originals), len(replays))
    assert len({r.headers["authorization"] for r in replays}) == 1, "both replays carried the fresh token"
    assert len(server.hits("/token")) == 2, "one shared refresh: %d" % len(server.hits("/token"))


def streaming_operation_never_replays(server):
    """(d) a stream-protected operation surfaces the typed 401 without any
    replay or refresh."""
    server.requests.clear()
    stub = Stub(server.responder)
    credential = _oauth.replaying_credential("feedOAuth", client_id="fid", client_secret="fsecret",
                                             transport=stub)
    with Client(auth={"feedOAuth": credential},
                transport=credential.replay_transport(stub)) as client:
        try:
            client.stream_events()
        except SdkError as error:
            assert error.status == 401, (error.kind, error.status)
        else:
            raise AssertionError("expected the stream 401 to surface")
    assert len(server.hits(EVENTS)) == 1, "no replay for the streaming operation"
    assert len(server.hits("/feed-token")) == 1, "no refresh for the streaming operation"


def replay_disabled_by_default(server):
    """(e) the plain provider surfaces the 401 without any refresh."""
    server.requests.clear()
    server.arm_stale()
    stub = Stub(server.responder)
    credential = _oauth.client_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                          transport=stub)
    with Client(auth={"serviceOAuth": credential}, transport=stub) as client:
        try:
            client.list_widgets()
        except SdkError as error:
            assert error.status == 401, (error.kind, error.status)
        else:
            raise AssertionError("expected the 401 to surface")
    assert len(server.hits(WIDGETS)) == 1, "no replay"
    assert len(server.hits("/token")) == 1, "no refresh"


def refresh_failure_is_typed_and_never_replays(server):
    """(f) a failed refresh surfaces the typed auth error, with no replay."""
    server.requests.clear()
    server.arm_stale()
    server.fail_from = server.service_tokens + 2
    stub = Stub(server.responder)
    credential = _oauth.replaying_credential("serviceOAuth", client_id="cid", client_secret="csecret",
                                             transport=stub)
    with Client(auth={"serviceOAuth": credential},
                transport=credential.replay_transport(stub)) as client:
        try:
            client.list_widgets()
        except SdkError as error:
            assert error.kind == "transport", error.kind
            assert isinstance(error.cause, _oauth.AuthError), error.cause
        else:
            raise AssertionError("expected the refresh failure to surface")
    assert len(server.hits(WIDGETS)) == 1, "no replay after a failed refresh"
    assert len(server.hits("/token")) == 2, "the refresh was attempted exactly once"


def main():
    server = Server()
    count_stale_and_replay(server, None)
    count_stale_and_replay(Server(), None, async_mode=True)
    count_401_then_401(Server())
    concurrent_401s_share_one_refresh()
    streaming_operation_never_replays(Server())
    replay_disabled_by_default(Server())
    refresh_failure_is_typed_and_never_replays(Server())
    print("replay behavior verified")


main()
"##;

#[test]
fn native_replay_lifecycle_over_a_stubbed_transport() {
    let root = tempfile::tempdir().unwrap();
    let files = generate(replay_fixture(), Some(replay_defaults()));
    write_files(&files, root.path()).unwrap();

    for file in &files {
        if file.path.ends_with(".py") {
            checked(
                Command::new("python3")
                    .args(["-m", "py_compile"])
                    .arg(root.path().join(&file.path)),
                root.path(),
                "compile-replay",
            );
        }
    }

    let Some(python) = httpx_interpreter() else {
        eprintln!(
            "no interpreter with httpx available; degraded to static emission and py_compile checks"
        );
        return;
    };
    fs::write(root.path().join("replay_behavior.py"), REPLAY_BEHAVIOR).unwrap();
    checked(
        Command::new(&python)
            .arg(root.path().join("replay_behavior.py"))
            .current_dir(root.path()),
        root.path(),
        "replay-behavior",
    );
}
