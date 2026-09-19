//! Emitted-only OAuth 2.0 lifecycle for the PHP HTTP backend: one generated
//! `src/OAuth.php` with compiled scheme descriptors, a `TokenSet` value, the
//! caller-implementable `TokenStore` plus an instance-owned
//! `MemoryTokenStore`, client-credentials acquisition with skew-aware caching
//! and single-flight, explicit refresh, PKCE authorization-code, device
//! polling, revocation and introspection. Static runtime files are never
//! modified; plans without a configured policy — or with only the deprecated
//! implicit and password flows — emit nothing at all.
#![cfg(all(feature = "php-sdk", feature = "http-protocol"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const NAMESPACE: &str = "OAuthFixture";

/// One scheme carrying every executable flow kind, plus configured
/// supplemental endpoints: one emission covers the whole conditional surface.
/// The device-authorization flow is an OAS 3.2 declaration.
fn service_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"service": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {"service": {
            "type": "oauth2",
            "flows": {
                "clientCredentials": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access"}
                },
                "authorizationCode": {
                    "authorizationUrl": "https://auth.oauth.test/authorize",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                },
                "deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.oauth.test/device",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                }
            }
        }}}
    })
}

/// The same shape without the OAuth scheme: the no-scheme control.
fn control_document() -> Value {
    let mut document = service_document();
    document["paths"]["/widgets"]["get"]
        .as_object_mut()
        .unwrap()
        .remove("security");
    document["components"]
        .as_object_mut()
        .unwrap()
        .remove("securitySchemes");
    document
}

/// A scheme whose only declared flows are the deprecated implicit and password
/// grants: represented by the plan, never executed.
fn deprecated_only_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth deprecated", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"legacy": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {"legacy": {
            "type": "oauth2",
            "flows": {
                "implicit": {
                    "authorizationUrl": "https://auth.oauth.test/implicit-authorize",
                    "scopes": {"read": "Read access"}
                },
                "password": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "scopes": {"read": "Read access"}
                }
            }
        }}}
    })
}

/// One OpenID Connect scheme whose endpoints the discovery document defines
/// at runtime, plus one OAuth2 scheme with a compiled token endpoint and
/// configured supplemental endpoints: one emission covers both shapes. Each
/// scheme is used by an operation.
fn discovery_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"identity": ["openid"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }},
            "/gadgets": {"get": {
                "operationId": "listGadgets",
                "security": [{"service": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {"type": "string"}}}}}
            }}
        },
        "components": {"securitySchemes": {
            "identity": {"type": "openIdConnect", "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"},
            "service": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

/// The discovery-aware policy: env names compiled, discovery-defined identity
/// scheme left without configured supplemental endpoints so they resolve
/// through the document.
fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {
                    "identity": {
                        "client_id_env": "PHP_OAUTH_CLIENT_ID",
                        "client_secret_env": "PHP_OAUTH_CLIENT_SECRET"
                    },
                    "service": {
                        "client_id_env": "PHP_OAUTH_CLIENT_ID",
                        "client_secret_env": "PHP_OAUTH_CLIENT_SECRET",
                        "refresh_skew_seconds": 30,
                        "revocation_endpoint": "https://auth.oauth.test/revoke",
                        "introspection_endpoint": "https://auth.oauth.test/introspect"
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn contract(document: &Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.oauth.test/php-oauth.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(document).unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::PhpHttp,
        package_name: "oauth/php-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some(NAMESPACE.into()),
    }
}

fn generate(document: &Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {"service": {
                    "client_id_env": "PHP_OAUTH_CLIENT_ID",
                    "client_secret_env": "PHP_OAUTH_CLIENT_SECRET",
                    "refresh_skew_seconds": 30,
                    "revocation_endpoint": "https://auth.oauth.test/revoke",
                    "introspection_endpoint": "https://auth.oauth.test/introspect"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn oauth_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "php/src/OAuth.php")
        .expect("generated OAuth.php")
}

fn sorted(files: &[OutFile]) -> std::collections::BTreeSet<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn oauth_file_emits_only_under_a_usable_scheme() {
    let configured = generate(&service_document(), &configured());
    let plain = generate(&service_document(), &GenerationOptions::default());
    let control = generate(&control_document(), &GenerationOptions::default());
    assert!(
        !plain.iter().any(|file| file.path == "php/src/OAuth.php"),
        "no-policy output must not carry the OAuth lifecycle runtime"
    );
    assert!(
        !control.iter().any(|file| file.path == "php/src/OAuth.php"),
        "the control document must not carry the OAuth lifecycle runtime"
    );
    assert_eq!(
        plain
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        control
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        "the control document must not add or remove artifact paths"
    );
    assert_eq!(
        configured.len(),
        plain.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "final class OAuth",
        // Value types and the instance-owned, key-partitioned store.
        "final readonly class TokenSet",
        "interface TokenStore",
        "final class MemoryTokenStore implements TokenStore",
        "final class AuthException extends \\RuntimeException",
        // Compiled descriptors as constants: env names compiled, never values.
        "private const SCHEMES = [",
        "\"service\" => [",
        "'client_id_env' => \"PHP_OAUTH_CLIENT_ID\"",
        "'client_secret_env' => \"PHP_OAUTH_CLIENT_SECRET\"",
        "'revocation' => \"https://auth.oauth.test/revoke\"",
        "'introspection' => \"https://auth.oauth.test/introspect\"",
        "'token_url' => \"https://auth.oauth.test/token\"",
        "'refresh_url' => \"https://auth.oauth.test/token-refresh\"",
        "'device_authorization_url' => \"https://auth.oauth.test/device\"",
        "'client_auth' => \"client-secret-basic\"",
        "'scopes' => [\"read\" => \"Read access\"",
        // Lifecycle methods.
        "public function clientCredentials(",
        "public function credential(",
        "public function refresh(",
        "public function beginAuthorization(",
        "public function completeAuthorization(",
        "public function beginDeviceAuthorization(",
        "public function pollDeviceAuthorization(",
        "public function revoke(",
        "public function introspect(",
        // Traversal and acquisition semantics.
        "Re-check the store after acquiring the per-key gate",
        "'grant_type' => 'client_credentials'",
        "'grant_type' => 'refresh_token'",
        "'grant_type' => 'authorization_code'",
        "'grant_type' => 'urn:ietf:params:oauth:grant-type:device_code'",
        "'code_challenge_method' => 'S256'",
        "random_bytes(32)",
        "hash('sha256', $codeVerifier, true)",
        "getenv($variable)",
        "the device code expired before authorization completed",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.php is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }

    // A scheme with only the deprecated implicit and password flows emits
    // nothing: byte-identical to the no-policy package.
    let deprecated = generate(
        &deprecated_only_document(),
        &GenerationOptions {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version": "v1",
                    "oauth": {"schemes": {"legacy": {"client_id_env": "PHP_OAUTH_CLIENT_ID"}}}
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    );
    let deprecated_plain = generate(&deprecated_only_document(), &GenerationOptions::default());
    assert!(
        !deprecated
            .iter()
            .any(|file| file.path == "php/src/OAuth.php"),
        "deprecated-only flows must not emit OAuth.php"
    );
    assert_eq!(sorted(&deprecated), sorted(&deprecated_plain));
    for file in &deprecated_plain {
        let emitted = deprecated
            .iter()
            .find(|candidate| candidate.path == file.path)
            .expect("same file set");
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }

    // An explicit `off` policy emits nothing as well.
    let off = generate(
        &service_document(),
        &GenerationOptions {
            sdk_defaults: Some(
                serde_json::from_value(json!({"version": "v1", "oauth": "off"})).unwrap(),
            ),
            ..Default::default()
        },
    );
    assert!(!off.iter().any(|file| file.path == "php/src/OAuth.php"));
    assert_eq!(sorted(&off), sorted(&plain));
}

/// The repository's verified PHP 8.3 interpreter, when available.
fn php() -> Option<PathBuf> {
    let candidate = std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-tools/php-8.3.32/php")
        });
    Command::new(&candidate)
        .arg("-v")
        .output()
        .is_ok()
        .then_some(candidate)
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn emitted_oauth_php_lints() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; skipping the lint check");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&service_document(), &configured()), root.path())
        .unwrap();
    let oauth = root.path().join("php/src/OAuth.php");
    let output = Command::new(&php).arg("-l").arg(&oauth).output().unwrap();
    assert!(
        output.status.success(),
        "OAuth.php failed to lint:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn oauth_lifecycle_drives_stubbed_transport_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&service_document(), &configured());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    fs::write(root.path().join("behavior.php"), BEHAVIOR).unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("behavior.php"))
        // Compiled environment variable NAMES, real values supplied by the
        // environment at call time.
        .env("PHP_OAUTH_CLIENT_ID", "cli-id")
        .env("PHP_OAUTH_CLIENT_SECRET", "cli-secret")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("behavior.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("behavior.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn discovery_schemes_emit_the_discovery_engine_and_the_per_instance_cache() {
    let with_policy = generate(&discovery_document(), &discovery_options());
    let plain = generate(&discovery_document(), &GenerationOptions::default());
    // The discovery policy adds exactly one file and changes no other byte.
    assert_eq!(
        with_policy.len(),
        plain.len() + 1,
        "the discovery policy may add exactly one file ({} vs {})",
        with_policy.len(),
        plain.len()
    );
    for file in &plain {
        let emitted = with_policy
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the discovery policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let source = oauth_file(&with_policy).content.clone();
    for expected in [
        // The compiled discovery URL lands in the descriptor of the
        // discovery-defined scheme, whose flow table stays empty.
        "'discovery' => \"https://authority.oauth.test/.well-known/openid-configuration\"",
        // The compiled scheme keeps its endpoints; discovery only supplements.
        "\"service\" => [\n            'name' => \"service\",\n            'kind' => \"oauth2\",\n            'skew' => 30,\n            'discovery' => null,",
        "'revocation' => \"https://auth.oauth.test/revoke\"",
        "'introspection' => \"https://auth.oauth.test/introspect\"",
        // The typed engine: issuer rule, per-instance cache, single-flight
        // gates, endpoint-resolution precedence and the discovery-aware
        // client-authentication rule.
        "private const DISCOVERY_MAX_BYTES = 1048576;",
        "private array $discovered = [];",
        "private array $discoveryInflight = [];",
        "the discovery document issuer does not share the discovery URL origin",
        "the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery",
        "private function discover(string $scheme): array",
        "private function resolveEndpoint(string $scheme, ?string $compiled, string $member): string",
        "private function executableFlowOrNull(array $scheme, string $kind): ?array",
        "private function refreshFlowOrNull(array $scheme): ?array",
        "private function discoveryClientAuth(array $scheme, string $schemeName, ?string $clientId, ?string $clientSecret): ?string",
        "private function waitForDiscovery(string $scheme): void",
        "accept' => 'application/json'",
        "AuthException('discovery-failed'",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.php is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }

    // Control: a plan without any discovery URL emits no discovery engine at
    // all; every compiled endpoint stays a frozen constant.
    let undiscovered = oauth_file(&generate(&service_document(), &configured()))
        .content
        .clone();
    for absent in [
        "'discovery' =>",
        "discovery-failed",
        "DISCOVERY_MAX_BYTES",
        "discoveryInflight",
        "resolveEndpoint",
    ] {
        assert!(
            !undiscovered.contains(absent),
            "a plan without a discovery URL must not carry {absent}"
        );
    }
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn emitted_discovery_oauth_php_lints() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; skipping the lint check");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&discovery_document(), &discovery_options()),
        root.path(),
    )
    .unwrap();
    let oauth = root.path().join("php/src/OAuth.php");
    let output = Command::new(&php).arg("-l").arg(&oauth).output().unwrap();
    assert!(
        output.status.success(),
        "discovery OAuth.php failed to lint:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn discovery_lifecycle_drives_stubbed_transport_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&discovery_document(), &discovery_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    fs::write(
        root.path().join("discovery-behavior.php"),
        DISCOVERY_BEHAVIOR,
    )
    .unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("discovery-behavior.php"))
        .env("PHP_OAUTH_CLIENT_ID", "cli-id")
        .env("PHP_OAUTH_CLIENT_SECRET", "cli-secret")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("discovery.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("discovery.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const BEHAVIOR: &str = r#"<?php
declare(strict_types=1);

foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }

use OAuthFixture\AuthException;
use OAuthFixture\AuthorizationCredential;
use OAuthFixture\AuthorizationTransaction;
use OAuthFixture\DeviceGrant;
use OAuthFixture\HttpRequest;
use OAuthFixture\HttpResponse;
use OAuthFixture\OAuth;
use OAuthFixture\TokenSet;
use OAuthFixture\TokenStore;
use OAuthFixture\Transport;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

final class OAuthStub implements Transport
{
    /** @var list<HttpRequest> */
    public array $requests = [];
    /** @param array<string, HttpResponse|list<HttpResponse>> $responses A list answers in request order, then repeats its last entry. */
    public function __construct(public array $responses) {}
    public function send(HttpRequest $request): HttpResponse
    {
        $this->requests[] = $request;
        $response = $this->responses[$request->url] ?? throw new LogicException('unexpected request: ' . $request->url);
        if (is_array($response)) {
            $index = 0;
            foreach ($this->requests as $past) { if ($past->url === $request->url) { $index += 1; } }
            $response = $response[min($index - 1, count($response) - 1)];
        }
        return $response;
    }
    /** @return list<array<string, string>> Decoded form bodies of every captured request to one URL. */
    public function bodies(string $url): array
    {
        $bodies = [];
        foreach ($this->requests as $request) {
            if ($request->url === $url) {
                parse_str((string) $request->body, $fields);
                $bodies[] = $fields;
            }
        }
        return $bodies;
    }
    /** Headers of the most recent captured request to one URL. */
    public function headers(string $url): array
    {
        $headers = null;
        foreach ($this->requests as $request) {
            if ($request->url === $url) { $headers = $request->headers; }
        }
        return $headers ?? throw new LogicException('no request to ' . $url);
    }
}

function json(int $status, array $payload): HttpResponse
{
    return new HttpResponse($status, ['content-type' => 'application/json'], json_encode($payload, JSON_THROW_ON_ERROR));
}

function base64Url(string $bytes): string
{
    return rtrim(strtr(base64_encode($bytes), '+/', '-_'), '=');
}

const TOKEN = 'https://auth.oauth.test/token';
const REFRESH = 'https://auth.oauth.test/token-refresh';
const REVOKE = 'https://auth.oauth.test/revoke';
const INTROSPECT = 'https://auth.oauth.test/introspect';
const DEVICE = 'https://auth.oauth.test/device';
const AUTHORIZE = 'https://auth.oauth.test/authorize';

// A controllable clock and a recorded sleeper.
$now = 1_000_000;
$clock = static function () use (&$now): int { return $now; };
$sleeps = [];
$sleep = static function (int $seconds) use (&$sleeps): void { $sleeps[] = $seconds; };

// ---- Client credentials: acquisition, caching, env identity, Basic auth ----
$transport = new OAuthStub([
    TOKEN => [
        json(200, ['access_token' => 'at-1', 'token_type' => 'Bearer', 'expires_in' => 3600, 'refresh_token' => 'rt-1', 'scope' => 'read']),
        json(200, ['access_token' => 'at-2', 'token_type' => 'Bearer', 'expires_in' => 3600, 'refresh_token' => 'rt-2']),
        json(200, ['access_token' => 'at-3', 'token_type' => 'bearer', 'expires_in' => 3600]),
        json(200, ['access_token' => 'at-4', 'expires_in' => 3600]),
    ],
]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$set = $oauth->clientCredentials('service');
check($set instanceof TokenSet, 'a token set is returned');
check($set->accessToken === 'at-1', 'acquired token: ' . $set->accessToken);
check($set->tokenType === 'Bearer', 'token type');
check($set->expiresAt === $now + 3600, 'expiry: ' . ($set->expiresAt ?? 'null'));
check($set->refreshToken === 'rt-1', 'rotated refresh token adopted on acquisition');
check($set->scope === 'read', 'scope');
check(count($transport->requests) === 1, 'exactly one token request: ' . count($transport->requests));
$headers = $transport->headers(TOKEN);
check(str_starts_with($headers['authorization'], 'Basic '), 'client-secret-basic sends HTTP Basic');
check(base64_decode(substr($headers['authorization'], 6), true) === 'cli-id:cli-secret', 'basic identity');
$bodies = $transport->bodies(TOKEN);
check(($bodies[0]['grant_type'] ?? '') === 'client_credentials', 'grant type');
check(!array_key_exists('client_id', $bodies[0]), 'confidential clients never send a body client id');
check(!isset($bodies[0]['scope']), 'no scope is inferred from operations');

// Cache hit: a fresh stored set is served without another request.
$cached = $oauth->clientCredentials('service');
check($cached->accessToken === 'at-1' && count($transport->requests) === 1, 'skew-aware cache hit');

// Expiry past the compiled skew triggers re-acquisition; the previous refresh
// token is retained when the response carries none.
$now += 3560; // Forty seconds before expiry: still fresh beyond the 30s skew.
check($oauth->clientCredentials('service')->accessToken === 'at-1', 'a token outliving the skew stays cached');
check(count($transport->requests) === 1, 'still cached');
$now += 11; // Twenty-nine seconds before expiry: inside the 30s skew, stale.
$rotated = $oauth->clientCredentials('service');
check($rotated->accessToken === 'at-2', 're-acquired past the skew');
check(count($transport->requests) === 2, 'one new request: ' . count($transport->requests));
check($rotated->refreshToken === 'rt-2', 'rotated refresh token adopted');
$now += 3601;
$transport->responses[TOKEN] = json(200, ['access_token' => 'at-3', 'token_type' => 'bearer', 'expires_in' => 3600]);
$retained = $oauth->clientCredentials('service');
check($retained->accessToken === 'at-3', 're-acquired again');
check($retained->refreshToken === 'rt-2', 'previous refresh token retained when none is issued');
check($retained->tokenType === 'bearer', 'declared token type is kept');

// Explicit client identity wins over the compiled environment variables, and
// partitions the store separately.
$transport->responses[TOKEN] = json(200, ['access_token' => 'at-5', 'expires_in' => 3600]);
$oauth->clientCredentials('service', clientId: 'explicit-id', clientSecret: 'explicit-secret');
check(base64_decode(substr($transport->headers(TOKEN)['authorization'], 6), true) === 'explicit-id:explicit-secret', 'explicit identity wins');

// Unknown scheme is a typed failure.
try {
    $oauth->clientCredentials('nope');
    throw new LogicException('expected an unknown-scheme failure');
} catch (AuthException $error) {
    check($error->kind === 'unknown-scheme', 'unknown scheme kind: ' . $error->kind);
}

// ---- Single-flight: a caller arriving on a pending gate re-checks the store ----
final class DoubleCheckStore implements TokenStore
{
    public function __construct(private OAuthFixture\TokenSet $fresh) {}
    public function load(string $key): ?OAuthFixture\TokenSet
    {
        static $first = true;
        if ($first) { $first = false; return null; }
        return $this->fresh;
    }
    public function replace(string $key, OAuthFixture\TokenSet $tokenSet): void {}
    public function clear(string $key): void {}
}
$transport = new OAuthStub([TOKEN => json(200, ['access_token' => 'never', 'expires_in' => 3600])]);
$oauth = new OAuth(new DoubleCheckStore(new TokenSet('double-checked')), $transport, $clock, $sleep);
$served = $oauth->clientCredentials('service');
check($served->accessToken === 'double-checked', 'the gate re-check serves the store without a duplicate request');
check(count($transport->requests) === 0, 'single-flight re-check issues no token request: ' . count($transport->requests));

// ---- Credential attach path ----
$transport = new OAuthStub([TOKEN => json(200, ['access_token' => 'at-hook', 'expires_in' => 3600])]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$credential = $oauth->credential('service');
check($credential instanceof Closure, 'the attach path returns a closure credential');
$authorization = $credential(new OAuthFixture\CredentialRequest('listWidgets', 'service', 'oauth2', ['read'], OAuthFixture\JsonValue::fromObject([])));
check($authorization instanceof AuthorizationCredential, 'the hook answers with an AuthorizationCredential');
check($authorization->value === 'Bearer at-hook', 'hook value: ' . $authorization->value);
check(!str_contains(print_r($authorization, true), 'at-hook'), 'credential debug output is redacted');

// ---- Explicit refresh: declared refresh URL, rotation and retention ----
$transport = new OAuthStub([
    REFRESH => json(200, ['access_token' => 'rt-at-1', 'expires_in' => 100, 'refresh_token' => 'rt-rotated']),
]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$refreshed = $oauth->refresh('service', new TokenSet('stale', refreshToken: 'rt-current'));
check($refreshed->accessToken === 'rt-at-1', 'refreshed access token');
check($refreshed->refreshToken === 'rt-rotated', 'rotated refresh token adopted');
check($transport->requests[0]->url === REFRESH, 'the declared refresh URL serves refreshes');
$bodies = $transport->bodies(REFRESH);
check(($bodies[0]['grant_type'] ?? '') === 'refresh_token' && ($bodies[0]['refresh_token'] ?? '') === 'rt-current', 'refresh form');
$transport->responses[REFRESH] = json(200, ['access_token' => 'rt-at-2', 'expires_in' => 100]);
$retained = $oauth->refresh('service', new TokenSet('stale', refreshToken: 'rt-keep'));
check($retained->refreshToken === 'rt-keep', 'current refresh token retained when none is issued');
try {
    $oauth->refresh('service', new TokenSet('no-refresh'));
    throw new LogicException('expected a no-refresh-token failure');
} catch (AuthException $error) {
    check($error->kind === 'no-refresh-token', 'refresh without a token is typed: ' . $error->kind);
}

// Store replacement: refresh replaces the store entry under the partition key.
$store = new OAuthFixture\MemoryTokenStore();
$transport->responses[REFRESH] = json(200, ['access_token' => 'rt-at-3', 'expires_in' => 100, 'refresh_token' => 'rt-3']);
$oauth->refresh('service', new TokenSet('stale', refreshToken: 'rt-x'), store: $store);
check($store->load('service|' . REFRESH . '|cli-id')?->accessToken === 'rt-at-3', 'store replacement on refresh');

// ---- Token endpoint failure mapping: typed, secret-free ----
$transport = new OAuthStub([TOKEN => json(400, ['error' => 'invalid_client', 'error_description' => 'server says: cli-secret'])]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
try {
    $oauth->clientCredentials('service');
    throw new LogicException('expected a typed failure');
} catch (AuthException $error) {
    check($error->kind === 'invalid-client', 'server code maps to a kind: ' . $error->kind);
    check($error->status === 400, 'status metadata');
    check($error->serverError === 'invalid_client', 'server error metadata');
    check(!str_contains($error->getMessage(), 'cli-secret') && !str_contains($error->getMessage(), 'server says'), 'message carries no server description or secret: ' . $error->getMessage());
}
$transport->responses[TOKEN] = json(500, ['boom' => true]);
try {
    $oauth->clientCredentials('service');
    throw new LogicException('expected a typed failure');
} catch (AuthException $error) {
    check($error->kind === 'server-error' && $error->status === 500, 'unmapped server errors stay typed: ' . $error->kind);
}

// ---- Authorization code + PKCE S256 ----
$transport = new OAuthStub([TOKEN => json(200, ['access_token' => 'code-at', 'expires_in' => 3600])]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$transaction = $oauth->beginAuthorization('service', 'https://app.test/callback', ['read']);
check($transaction instanceof AuthorizationTransaction, 'a transaction is returned');
check(str_starts_with($transaction->authorizationUrl, AUTHORIZE . '?'), 'redirect target: ' . $transaction->authorizationUrl);
$query = [];
parse_str((string) parse_url($transaction->authorizationUrl, PHP_URL_QUERY), $query);
check(($query['response_type'] ?? '') === 'code', 'response type');
check(($query['client_id'] ?? '') === 'cli-id', 'client id from the compiled environment variable');
check(($query['redirect_uri'] ?? '') === 'https://app.test/callback', 'redirect uri');
check(($query['code_challenge_method'] ?? '') === 'S256', 'S256 challenge method');
check($query['code_challenge'] === base64Url(hash('sha256', $transaction->codeVerifier, true)), 'the challenge is the S256 hash of the verifier');
check(count($transport->requests) === 0, 'beginning an authorization makes no request');
try {
    $oauth->beginAuthorization('service', 'https://app.test/callback#fragment');
    throw new LogicException('expected an invalid-request failure');
} catch (AuthException $error) {
    check($error->kind === 'invalid-request', 'redirect fragments are refused');
}
// State mismatch is a typed failure that consumes the transaction.
try {
    $oauth->completeAuthorization($transaction, 'the-code', 'wrong-state');
    throw new LogicException('expected a state-mismatch failure');
} catch (AuthException $error) {
    check($error->kind === 'state-mismatch', 'state mismatch kind');
}
try {
    $oauth->completeAuthorization($transaction, 'the-code', $transaction->state);
    throw new LogicException('expected a transaction-consumed failure');
} catch (AuthException $error) {
    check($error->kind === 'transaction-consumed', 'consumed by any attempt: ' . $error->kind);
}
$transaction = $oauth->beginAuthorization('service', 'https://app.test/callback');
$completed = $oauth->completeAuthorization($transaction, 'the-code', $transaction->state, clientSecret: 'cli-secret');
check($completed->accessToken === 'code-at', 'code exchanged');
$exchange = $transport->bodies(TOKEN)[0];
check(($exchange['grant_type'] ?? '') === 'authorization_code', 'code grant');
check(($exchange['code'] ?? '') === 'the-code', 'code');
check(($exchange['redirect_uri'] ?? '') === 'https://app.test/callback', 'redirect uri');
check(base64Url(hash('sha256', $transaction->codeVerifier, true)) === $transaction->codeChallenge, 'the exchanged verifier matches the bound challenge');
check(count($transport->requests) === 1, 'one exchange request');

// ---- Device authorization with polling, slow-down and expiry ----
$transport = new OAuthStub([
    DEVICE => json(200, ['device_code' => 'dev-1', 'user_code' => 'ABCD-EFGH', 'verification_uri' => 'https://auth.oauth.test/activate', 'verification_uri_complete' => 'https://auth.oauth.test/activate?code=ABCD-EFGH', 'expires_in' => 300, 'interval' => 1]),
]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$grant = $oauth->beginDeviceAuthorization('service');
check($grant instanceof DeviceGrant, 'a device grant is returned');
check($grant->deviceCode === 'dev-1' && $grant->userCode === 'ABCD-EFGH', 'grant fields');
check($grant->expiresAt === $now + 300 && $grant->intervalSeconds === 1, 'grant policy fields');
$deviceHeaders = $transport->headers(DEVICE);
check(str_starts_with($deviceHeaders['authorization'], 'Basic '), 'the confidential device request authenticates with Basic');
// authorization_pending waits the declared interval, then success.
$pendingTransport = new class implements Transport {
    public array $requests = [];
    public int $pending = 1;
    public function send(HttpRequest $request): HttpResponse
    {
        $this->requests[] = $request;
        if ($this->pending > 0) { $this->pending -= 1; return json(400, ['error' => 'authorization_pending']); }
        return json(200, ['access_token' => 'device-at', 'expires_in' => 3600]);
    }
};
$oauth = new OAuth(null, $pendingTransport, $clock, $sleep);
$store = new OAuthFixture\MemoryTokenStore();
$token = $oauth->pollDeviceAuthorization($grant, store: $store);
check($token->accessToken === 'device-at', 'device polling resolves a token set');
check($store->load('service|' . TOKEN . '|cli-id')?->accessToken === 'device-at', 'device token stored under the partition key');
check($sleeps === [1], 'authorization_pending waits the declared interval: ' . json_encode($sleeps));
// slow_down grows the interval by five seconds.
$sleeps = [];
$slowTransport = new class implements Transport {
    public int $calls = 0;
    public function send(HttpRequest $request): HttpResponse
    {
        $this->calls += 1;
        if ($this->calls === 1) { return json(400, ['error' => 'slow_down']); }
        return json(200, ['access_token' => 'device-at-2', 'expires_in' => 3600]);
    }
};
$oauth = new OAuth(null, $slowTransport, $clock, $sleep);
$token = $oauth->pollDeviceAuthorization($grant);
check($token->accessToken === 'device-at-2', 'slow-down polling resolves');
check($sleeps === [6], 'slow_down backs off five extra seconds: ' . json_encode($sleeps));
// Expiry ends polling with a typed failure.
$expired = new DeviceGrant('service', 'dev-2', 'ABCD-EFGH', 'https://auth.oauth.test/activate', null, $now, 1);
$now += 1;
try {
    $oauth->pollDeviceAuthorization($expired);
    throw new LogicException('expected a device-code-expired failure');
} catch (AuthException $error) {
    check($error->kind === 'device-code-expired', 'expiry kind: ' . $error->kind);
}
// A terminal server refusal is typed and stops polling.
$deniedTransport = new class implements Transport {
    public function send(HttpRequest $request): HttpResponse { return json(400, ['error' => 'access_denied']); }
};
$oauth = new OAuth(null, $deniedTransport, $clock, $sleep);
try {
    $oauth->pollDeviceAuthorization($grant);
    throw new LogicException('expected a typed refusal');
} catch (AuthException $error) {
    check($error->kind === 'server-error' && $error->serverError === 'access_denied', 'terminal refusal: ' . $error->kind);
}

// ---- Revocation and introspection over the configured endpoints ----
$transport = new OAuthStub([
    REVOKE => json(200, []),
    INTROSPECT => json(200, ['active' => true, 'scope' => 'read']),
]);
$oauth = new OAuth(null, $transport, $clock, $sleep);
$oauth->revoke('service', 'revoke-me', 'access_token');
$bodies = $transport->bodies(REVOKE);
check(($bodies[0]['token'] ?? '') === 'revoke-me' && ($bodies[0]['token_type_hint'] ?? '') === 'access_token', 'revocation form');
$active = $oauth->introspect('service', 'peek-me');
check($active->asObject()['active']->asBool() === true, 'introspection response');
$bodies = $transport->bodies(INTROSPECT);
check(($bodies[0]['token'] ?? '') === 'peek-me', 'introspection form');

// ---- Store ownership: instances never share a global store ----
$transport = new OAuthStub([TOKEN => json(200, ['access_token' => 'at-shared', 'expires_in' => 3600])]);
$left = new OAuth(null, $transport, $clock, $sleep);
$right = new OAuth(null, $transport, $clock, $sleep);
$left->clientCredentials('service');
check($right->clientCredentials('service') !== null && count($transport->requests) === 2, 'two instances acquire independently: ' . count($transport->requests));

// ---- TokenSet never leaks values through debug output ----
$set = new TokenSet('secret-access', refreshToken: 'secret-refresh');
$rendered = print_r($set, true);
check(!str_contains($rendered, 'secret-access') && !str_contains($rendered, 'secret-refresh'), 'token set debug output is redacted: ' . $rendered);

echo 'oauth behavior verified', PHP_EOL;
"#;

const DISCOVERY_BEHAVIOR: &str = r#"<?php
declare(strict_types=1);

foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }

use OAuthFixture\AuthException;
use OAuthFixture\HttpRequest;
use OAuthFixture\HttpResponse;
use OAuthFixture\MemoryTokenStore;
use OAuthFixture\OAuth;
use OAuthFixture\TokenSet;
use OAuthFixture\Transport;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

function json(int $status, array $payload): HttpResponse
{
    return new HttpResponse($status, ['content-type' => 'application/json'], json_encode($payload, JSON_THROW_ON_ERROR));
}

const DISCOVERY = 'https://authority.oauth.test/.well-known/openid-configuration';
const DISCOVERED_TOKEN = 'https://authority.oauth.test/oauth/token';
const DISCOVERED_REVOKE = 'https://authority.oauth.test/oauth/revoke';
const DISCOVERED_INTROSPECT = 'https://authority.oauth.test/oauth/introspect';
const COMPILED_TOKEN = 'https://auth.oauth.test/token';

/** A stubbed transport serving one discovery document plus token plumbing. */
final class DiscoveryStub implements Transport
{
    /** @var list<HttpRequest> */
    public array $requests = [];
    public string $issuer = 'https://authority.oauth.test';
    public int $failStatus = 0;
    public int $discoveryFetches = 0;
    public function send(HttpRequest $request): HttpResponse
    {
        $this->requests[] = $request;
        if ($request->url === DISCOVERY) {
            $this->discoveryFetches += 1;
            check($request->method === 'GET', 'discovery requests are GETs');
            check(($request->headers['accept'] ?? '') === 'application/json', 'discovery carries accept: application/json');
            check($request->maxResponseBytes === 1048576, 'the discovery response is bounded at the compiled ceiling: ' . $request->maxResponseBytes);
            if ($this->failStatus !== 0) { return json($this->failStatus, ['error' => 'boom']); }
            $document = ['issuer' => $this->issuer, 'token_endpoint' => DISCOVERED_TOKEN, 'revocation_endpoint' => DISCOVERED_REVOKE, 'introspection_endpoint' => DISCOVERED_INTROSPECT, 'unknown_member' => ['nested' => true]];
            return json(200, $document);
        }
        if ($request->url === DISCOVERED_TOKEN || $request->url === COMPILED_TOKEN) {
            parse_str((string) $request->body, $fields);
            if (($fields['grant_type'] ?? '') === 'refresh_token') { return json(200, ['access_token' => 'refreshed-access', 'expires_in' => 3600]); }
            return json(200, ['access_token' => 'discovered-1', 'token_type' => 'Bearer', 'expires_in' => 3600, 'refresh_token' => 'rotated-1']);
        }
        if ($request->url === DISCOVERED_REVOKE) { return json(200, []); }
        if ($request->url === DISCOVERED_INTROSPECT) { return json(200, ['active' => true, 'scope' => 'read']); }
        throw new LogicException('unexpected request: ' . $request->url);
    }
    /** @return list<HttpRequest> */
    public function requestsTo(string $url): array
    {
        return array_values(array_filter($this->requests, static fn (HttpRequest $request): bool => $request->url === $url));
    }
}

// A controllable clock and a recorded sleeper.
$now = 1_000_000;
$clock = static function () use (&$now): int { return $now; };
$sleeps = [];
$sleep = static function (int $seconds) use (&$sleeps): void { $sleeps[] = $seconds; };

// ---- 1. The discovered token endpoint serves acquisition ----
$stub = new DiscoveryStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$set = $oauth->clientCredentials('identity');
check($set instanceof TokenSet && $set->accessToken === 'discovered-1', 'the discovered token endpoint issued the set: ' . $set->accessToken);
check($stub->discoveryFetches === 1, 'exactly one discovery fetch: ' . $stub->discoveryFetches);
$discoveryRequests = $stub->requestsTo(DISCOVERY);
check(count($discoveryRequests) === 1, 'one discovery request');
$tokenRequests = $stub->requestsTo(DISCOVERED_TOKEN);
check(count($tokenRequests) === 1, 'one acquisition against the discovered endpoint: ' . count($tokenRequests));
$basic = $tokenRequests[0]->headers['authorization'] ?? '';
check(str_starts_with($basic, 'Basic ') && base64_decode(substr($basic, 6), true) === 'cli-id:cli-secret', 'the discovery-defined client authenticates with basic');
parse_str((string) $tokenRequests[0]->body, $fields);
check(($fields['grant_type'] ?? '') === 'client_credentials', 'discovery-defined grant');
check($stub->requestsTo(COMPILED_TOKEN) === [], 'the compiled fallback endpoint was never contacted');

// ---- 2. The document is cached per instance: no re-fetch, no re-acquire ----
$cached = $oauth->clientCredentials('identity');
check($cached->accessToken === 'discovered-1', 'cache hit serves the stored set');
check($stub->discoveryFetches === 1 && count($stub->requestsTo(DISCOVERED_TOKEN)) === 1, 'the cached document and token are reused: ' . $stub->discoveryFetches . ' fetches, ' . count($stub->requestsTo(DISCOVERED_TOKEN)) . ' acquisitions');

// ---- 3. Single-flight: a caller arriving on a pending gate waits, then
// serves whatever the in-flight fetch cached ----
$stub = new DiscoveryStub();
$sleeps = [];
$holder = null;
$sleep = static function (int $seconds) use (&$sleeps, &$holder): void {
    $sleeps[] = $seconds;
    // The "other flight" completes while this caller waits: the document is
    // cached and the gate is released.
    $reflection = new ReflectionClass($holder);
    $reflection->getProperty('discovered')->setValue($holder, ['identity' => ['token_endpoint' => DISCOVERED_TOKEN, 'revocation_endpoint' => DISCOVERED_REVOKE, 'introspection_endpoint' => DISCOVERED_INTROSPECT]]);
    $reflection->getProperty('discoveryInflight')->setValue($holder, []);
};
$oauth = new OAuth(null, $stub, $clock, $sleep);
$holder = $oauth;
$reflection = new ReflectionClass($oauth);
$reflection->getProperty('discoveryInflight')->setValue($oauth, ['identity' => true]);
$served = $oauth->clientCredentials('identity');
check($served->accessToken === 'discovered-1', 'the single-flight waiter serves the cached document');
check($stub->discoveryFetches === 0, 'the waiter never duplicates the in-flight fetch: ' . $stub->discoveryFetches);
check(count($stub->requestsTo(DISCOVERED_TOKEN)) === 1, 'exactly one acquisition');
check($sleeps === [1], 'the waiter polls through the injected sleeper: ' . json_encode($sleeps));
$sleep = static function (int $seconds) use (&$sleeps): void { $sleeps[] = $seconds; };

// ---- 4. A mismatching issuer origin is a typed failure carrying no body
// text, and the failed fetch is retried on the next call ----
$stub = new DiscoveryStub();
$stub->issuer = 'https://elsewhere.oauth.test';
$oauth = new OAuth(null, $stub, $clock, $sleep);
try {
    $oauth->clientCredentials('identity');
    throw new LogicException('expected an issuer-mismatch failure');
} catch (AuthException $error) {
    check($error->kind === 'discovery-failed' && $error->scheme === 'identity', 'typed issuer mismatch: ' . $error->kind);
    check(!str_contains($error->getMessage(), 'elsewhere') && !str_contains($error->getMessage(), 'boom'), 'the failure carries no response text: ' . $error->getMessage());
}
$stub->issuer = 'https://authority.oauth.test';
$retried = $oauth->clientCredentials('identity');
check($retried->accessToken === 'discovered-1', 'the issuer retry succeeds');
check($stub->discoveryFetches === 2, 'the failed fetch was not cached: ' . $stub->discoveryFetches);

// ---- 5. A failing discovery request is typed with its status, and the same
// instance recovers when the endpoint heals ----
$stub = new DiscoveryStub();
$stub->failStatus = 500;
$oauth = new OAuth(null, $stub, $clock, $sleep);
try {
    $oauth->clientCredentials('identity');
    throw new LogicException('expected a failed-discovery failure');
} catch (AuthException $error) {
    check($error->kind === 'discovery-failed' && $error->status === 500, 'typed transport refusal: ' . $error->kind . ' ' . ($error->status ?? 'null'));
}
$stub->failStatus = 0;
check($oauth->clientCredentials('identity')->accessToken === 'discovered-1', 'the same instance retries after a failed fetch');
check($stub->discoveryFetches === 2, 'the failed fetch is retried, never cached: ' . $stub->discoveryFetches);

// ---- 6. Compiled precedence: the compiled token URL wins, no discovery ----
$stub = new DiscoveryStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$compiled = $oauth->clientCredentials('service');
check($compiled->accessToken === 'discovered-1', 'the compiled scheme acquires');
check($stub->requestsTo(COMPILED_TOKEN) !== [] && $stub->discoveryFetches === 0, 'the compiled scheme never fetches discovery: ' . $stub->discoveryFetches);

// ---- 7. Revocation and introspection resolve through the document for the
// discovery-defined scheme ----
$stub = new DiscoveryStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$oauth->revoke('identity', 'revoke-me');
check(count($stub->requestsTo(DISCOVERED_REVOKE)) === 1, 'revocation resolved through the document');
$bodies = [];
parse_str((string) $stub->requestsTo(DISCOVERED_REVOKE)[0]->body, $bodies);
check(($bodies['token'] ?? '') === 'revoke-me', 'revocation form');
$active = $oauth->introspect('identity', 'peek-me');
check($active->asObject()['active']->asBool() === true, 'introspection resolved through the document');

// ---- 8. Explicit refresh resolves the discovered token endpoint and stores
// under the discovered issuer key ----
$stub = new DiscoveryStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$store = new MemoryTokenStore();
$seeded = $oauth->clientCredentials('identity', store: $store);
check($seeded->refreshToken === 'rotated-1', 'the seeded set carries its refresh token');
$refreshed = $oauth->refresh('identity', new TokenSet('stale', refreshToken: 'rt-current'), store: $store);
check($refreshed->accessToken === 'refreshed-access', 'the refresh exchange resolved the discovered endpoint');
$refreshRequests = array_values(array_filter($stub->requestsTo(DISCOVERED_TOKEN), static function (HttpRequest $request): bool { parse_str((string) $request->body, $f); return ($f['grant_type'] ?? '') === 'refresh_token'; }));
check(count($refreshRequests) === 1, 'one refresh exchange against the discovered endpoint: ' . count($refreshRequests));
check($store->load('identity|' . DISCOVERED_TOKEN . '|cli-id')?->accessToken === 'refreshed-access', 'the refreshed set is stored under the discovered issuer key');

echo 'discovery behavior verified', PHP_EOL;
"#;

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": {
                    "200": {"description": "Ok"},
                    "401": {"description": "Denied"}
                }
            }},
            "/events": {"get": {
                "operationId": "streamEvents",
                "security": [{"feedOAuth": ["read"]}],
                "responses": {
                    "200": {"description": "Events", "content": {"text/event-stream": {
                        "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
                    }}},
                    "401": {"description": "Denied"}
                }
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
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {
                    "serviceOAuth": {
                        "client_id_env": "PHP_OAUTH_REPLAY_CLIENT_ID",
                        "client_secret_env": "PHP_OAUTH_REPLAY_CLIENT_SECRET"
                    },
                    "feedOAuth": {
                        "client_id_env": "PHP_OAUTH_REPLAY_FEED_ID",
                        "client_secret_env": "PHP_OAUTH_REPLAY_FEED_SECRET"
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// An authorization-code-only scheme (no client-credentials flow): the
/// control for the replay emission gate.
fn code_only_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "security": [{"userOAuth": ["read"]}],
        "paths": {"/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "Ok"}}}}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    })
}

fn code_only_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1",
                "oauth": {"schemes": {"userOAuth": {
                    "client_id_env": "PHP_OAUTH_CODE_ONLY_ID",
                    "client_secret_env": "PHP_OAUTH_CODE_ONLY_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The replaying credential wrapper is emitted only with an executable
/// client-credentials flow, wraps exactly that attach path, and compiles the
/// stream-protection pointers of its scheme's operations.
#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let configured = generate(&replay_document(), &replay_options());
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "public function replayingCredential(",
        "public function replayTransport(",
        "final class ReplayTransport implements StreamTransport",
        "private const REPLAY_NO_REPLAY = [",
        "\"/paths/~1events/get/security/0/feedOAuth\"",
        "one coordinated refresh",
        "stream-protected requirements are never replayed",
        "one refresh plus one replay",
    ] {
        assert!(
            source.contains(expected),
            "OAuth.php is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!source.contains("~1widgets/get/security/0/serviceOAuth"));

    // A package without any executable client-credentials flow compiles
    // exactly the pre-replay bytes: no wrapper, no stream-protection table,
    // and no other artifact changes a byte.
    let code_only = generate(&code_only_document(), &code_only_options());
    let plain = generate(&code_only_document(), &GenerationOptions::default());
    assert_eq!(
        code_only.len(),
        plain.len() + 1,
        "the code-only policy may add exactly one file"
    );
    for file in &plain {
        let emitted = code_only
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let plain_oauth = oauth_file(&code_only).content.clone();
    for absent in ["replayingCredential", "ReplayTransport", "REPLAY_NO_REPLAY"] {
        assert!(
            !plain_oauth.contains(absent),
            "an authorization-code-only plan must not carry {absent}"
        );
    }
}

/// The discovery variant resolves the wrapped token endpoint through the
/// compiled precedence and adds the compiled discovery URL to the
/// lifecycle-endpoint guard.
#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn replaying_credentials_resolve_their_discovery_variant() {
    let configured = generate(&discovery_document(), &discovery_options());
    let source = oauth_file(&configured).content.clone();
    for expected in [
        "public function replayingCredential(",
        "The client-credentials token endpoint through the compiled precedence",
        "the compiled discovery URL joins the compiled endpoints",
        "private const REPLAY_NO_REPLAY = [",
    ] {
        assert!(
            source.contains(expected),
            "the discovery OAuth.php is missing:\n{expected}\n--- emitted: ---\n{source}"
        );
    }
    // The plain variant's exact-compiled-endpoint guard never appears here.
    assert!(
        !source.contains("and the exact-target guard is defense in depth against replay loops")
    );
}

#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn emitted_replay_oauth_php_lints() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; skipping the lint check");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(&replay_document(), &replay_options()),
        root.path(),
    )
    .unwrap();
    let oauth = root.path().join("php/src/OAuth.php");
    let output = Command::new(&php).arg("-l").arg(&oauth).output().unwrap();
    assert!(
        output.status.success(),
        "replay OAuth.php failed to lint:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The replay lifecycle (a)–(f) over a stubbed stream transport, mirroring the
/// TypeScript, Python, Go and Rust acceptance scenarios.
#[ignore = "requires the PHP CLI on the test host"]
#[test]
fn replay_lifecycle_drives_stubbed_transport_in_php() {
    let Some(php) = php() else {
        eprintln!("no PHP interpreter available; static emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&replay_document(), &replay_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();
    fs::write(root.path().join("replay-behavior.php"), REPLAY_BEHAVIOR).unwrap();
    let output = Command::new(&php)
        .arg("-d")
        .arg("error_reporting=-1")
        .arg(root.path().join("replay-behavior.php"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("replay.stdout.log"), &output.stdout).unwrap();
    fs::write(root.path().join("replay.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const REPLAY_BEHAVIOR: &str = r#"<?php
declare(strict_types=1);

foreach (glob(__DIR__ . '/php/src/*.php') as $file) { require_once $file; }

use OAuthFixture\ApiError;
use OAuthFixture\AuthException;
use OAuthFixture\BodyReader;
use OAuthFixture\CallControl;
use OAuthFixture\Client;
use OAuthFixture\ClientOptions;
use OAuthFixture\Credentials;
use OAuthFixture\HttpRequest;
use OAuthFixture\HttpResponse;
use OAuthFixture\OAuth;
use OAuthFixture\SdkError;
use OAuthFixture\StreamResponse;
use OAuthFixture\StreamTransport;
use OAuthFixture\TokenSet;
use OAuthFixture\Transport;

function check(bool $condition, string $message): void { if (!$condition) { throw new LogicException($message); } }

/** The first typed auth failure in an error's cause chain. */
function authCause(Throwable $error): ?AuthException
{
    for ($cause = $error->getPrevious(); $cause !== null; $cause = $cause->getPrevious()) {
        if ($cause instanceof AuthException) { return $cause; }
    }
    return null;
}

function answer(int $status): HttpResponse
{
    return new HttpResponse($status, [], '');
}

const WIDGETS = 'https://api.oauth.test/v1/widgets';
const EVENTS = 'https://api.oauth.test/v1/events';
const TOKEN = 'https://auth.oauth.test/token';
const FEED_TOKEN = 'https://auth.oauth.test/feed-token';

/** Fake API and token endpoints. armStale() makes the next issued token answer 401 on the API, so the driver counts exactly one refresh. */
final class ReplayStub implements StreamTransport
{
    /** @var list<array{url: string, authorization: ?string, body: ?string}> */
    public array $requests = [];
    public int $serviceTokens = 0;
    public int $feedTokens = 0;
    public string $mode = 'ok';
    public bool $staleNext = false;
    public ?string $staleToken = null;
    public ?int $failFrom = null;

    public function send(HttpRequest $request): HttpResponse
    {
        $this->requests[] = ['url' => $request->url, 'authorization' => self::header($request, 'authorization'), 'body' => $request->body];
        $presented = self::header($request, 'authorization');
        if ($request->url === TOKEN) {
            $this->serviceTokens += 1;
            $token = 'svc-' . $this->serviceTokens;
            if ($this->staleNext) { $this->staleNext = false; $this->staleToken = $token; }
            if ($this->failFrom !== null && $this->serviceTokens >= $this->failFrom) { return new HttpResponse(500, ['content-type' => 'application/json'], json_encode(['error' => 'server_error'], JSON_THROW_ON_ERROR)); }
            return new HttpResponse(200, ['content-type' => 'application/json'], json_encode(['access_token' => $token, 'token_type' => 'Bearer', 'expires_in' => 3600], JSON_THROW_ON_ERROR));
        }
        if ($request->url === FEED_TOKEN) {
            $this->feedTokens += 1;
            return new HttpResponse(200, ['content-type' => 'application/json'], json_encode(['access_token' => 'feed-' . $this->feedTokens, 'token_type' => 'Bearer', 'expires_in' => 3600], JSON_THROW_ON_ERROR));
        }
        if ($request->url === WIDGETS) {
            if ($this->mode === 'always-401' || ($this->staleToken !== null && $presented === 'Bearer ' . $this->staleToken)) { return answer(401); }
            return answer(200);
        }
        if ($request->url === EVENTS) { return answer(401); }
        throw new LogicException('unexpected request: ' . $request->url);
    }

    public function open(HttpRequest $request): StreamResponse
    {
        $this->requests[] = ['url' => $request->url, 'authorization' => self::header($request, 'authorization'), 'body' => $request->body];
        return new StreamResponse(401, ['content-type' => ['text/event-stream']], new ReplayReader());
    }

    private static function header(HttpRequest $request, string $name): ?string
    {
        foreach ($request->headers as $key => $value) {
            if (strtolower((string) $key) === $name) { return $value; }
        }
        return null;
    }

    /** @return list<array{url: string, authorization: ?string, body: ?string}> */
    public function hits(string $url): array
    {
        return array_values(array_filter($this->requests, static fn (array $request): bool => $request['url'] === $url));
    }
}

final class ReplayReader implements BodyReader
{
    public function read(): ?string { return null; }
    public function close(): void {}
}

// A controllable clock and a recorded sleeper.
$now = 1_000_000;
$clock = static function () use (&$now): int { return $now; };
$sleeps = [];
$sleep = static function (int $seconds) use (&$sleeps): void { $sleeps[] = $seconds; };

/** One wired replaying client over a fresh stub. */
function replayClient(ReplayStub $stub, OAuth $oauth, string $scheme): Client
{
    $credential = $oauth->replayingCredential($scheme, 'cid', 'csecret');
    return new Client(new Credentials([$scheme => $credential]), $oauth->replayTransport($stub), new ClientOptions());
}

// ---- (a) 401 then success: one refresh, one replay, and the caller sees 200 ----
$stub = new ReplayStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$stub->staleNext = true;
replayClient($stub, $oauth, 'serviceOAuth')->listWidgets();
check(count($stub->hits(WIDGETS)) === 2, '(a) one refresh, one replay: ' . count($stub->hits(WIDGETS)));
check(count($stub->hits(TOKEN)) === 2, '(a) exactly one forced acquisition: ' . count($stub->hits(TOKEN)));
$widgets = $stub->hits(WIDGETS);
check($widgets[0]['authorization'] === 'Bearer svc-1', '(a) the original attach carried the stale token');
check($widgets[1]['authorization'] === 'Bearer svc-2', '(a) the replay carried the fresh token');
foreach ($stub->hits(TOKEN) as $request) {
    parse_str((string) $request['body'], $fields);
    check(($fields['grant_type'] ?? '') === 'client_credentials', '(a) the refresh re-acquires through the same client-credentials endpoint');
}

// ---- (b) 401 then 401: the second 401 surfaces and exactly one refresh ran ----
$stub = new ReplayStub();
$stub->mode = 'always-401';
$oauth = new OAuth(null, $stub, $clock, $sleep);
try {
    replayClient($stub, $oauth, 'serviceOAuth')->listWidgets();
    throw new LogicException('(b) expected the second 401 to surface');
} catch (ApiError $error) {
    check($error->status === 401, '(b) the second 401 surfaces as the declared error: ' . get_class($error));
}
check(count($stub->hits(WIDGETS)) === 2, '(b) one replay, no loops: ' . count($stub->hits(WIDGETS)));
check(count($stub->hits(TOKEN)) === 2, '(b) exactly one refresh: ' . count($stub->hits(TOKEN)));

// ---- (c) a newer stored set wins over a stale re-refresh, and a caller
// arriving on a pending round gate waits through the sleeper ----
$stub = new ReplayStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
$stub->staleNext = true;
replayClient($stub, $oauth, 'serviceOAuth')->listWidgets();
$stale = 'Bearer ' . $stub->staleToken;
$wrapper = $oauth->replayTransport($stub);
$wrapper->send(new HttpRequest('GET', WIDGETS, ['authorization' => $stale, 'accept' => 'application/json'], null, 30000, 1048576, 65536, 4096, new CallControl(static function (): void {})));
$widgets = $stub->hits(WIDGETS);
check(count($widgets) === 4, '(c) two originals and two replays: ' . count($widgets));
check($widgets[2]['authorization'] === $stale && $widgets[3]['authorization'] === 'Bearer svc-2', '(c) the second replay carried the newer stored set');
check(count($stub->hits(TOKEN)) === 2, '(c) the newer stored set won without a re-acquisition: ' . count($stub->hits(TOKEN)));

// The single-flight round gate: a caller arriving on a pending round waits
// through the injected sleeper and serves whatever the round stored, without
// duplicating the token request.
$stub = new ReplayStub();
$stub->staleNext = true;
$holder = null;
$key = 'serviceOAuth|' . TOKEN . '|cid';
$roundSleeps = [];
$roundSleep = static function (int $seconds) use (&$roundSleeps, &$holder, $key): void {
    $roundSleeps[] = $seconds;
    $reflection = new ReflectionClass($holder);
    $reflection->getProperty('replayRounds')->setValue($holder, []);
    $store = $reflection->getProperty('store')->getValue($holder);
    $store->replace($key, new TokenSet('round-winner'));
};
$oauth = new OAuth(null, $stub, $clock, $roundSleep);
$holder = $oauth;
$reflection = new ReflectionClass($oauth);
$reflection->getProperty('replayRounds')->setValue($oauth, [$key => true]);
replayClient($stub, $oauth, 'serviceOAuth')->listWidgets();
check($roundSleeps === [1], '(c) the round waiter polls through the injected sleeper: ' . json_encode($roundSleeps));
check(count($stub->hits(TOKEN)) === 1, '(c) the waiter never duplicates the in-flight round: ' . count($stub->hits(TOKEN)));
$widgets = $stub->hits(WIDGETS);
check(count($widgets) === 2 && $widgets[1]['authorization'] === 'Bearer round-winner', '(c) the replay carried the round winner');

// ---- (d) a stream-protected operation surfaces the typed 401 without any
// replay or refresh ----
$stub = new ReplayStub();
$oauth = new OAuth(null, $stub, $clock, $sleep);
try {
    replayClient($stub, $oauth, 'feedOAuth')->streamEvents();
    throw new LogicException('(d) expected the stream 401 to surface');
} catch (ApiError $error) {
    check($error->status === 401, '(d) the stream 401 surfaces typed: ' . get_class($error));
}
check(count($stub->hits(EVENTS)) === 1, '(d) no replay for the streaming operation: ' . count($stub->hits(EVENTS)));
check(count($stub->hits(FEED_TOKEN)) === 1, '(d) no refresh for the streaming operation: ' . count($stub->hits(FEED_TOKEN)));

// ---- (e) replay disabled by default: the plain attach path surfaces the 401
// without any refresh ----
$stub = new ReplayStub();
$stub->staleNext = true;
$oauth = new OAuth(null, $stub, $clock, $sleep);
$plain = new Client(new Credentials(['serviceOAuth' => $oauth->credential('serviceOAuth', 'cid', 'csecret')]), $stub, new ClientOptions());
try {
    $plain->listWidgets();
    throw new LogicException('(e) expected the 401 to surface');
} catch (ApiError $error) {
    check($error->status === 401, '(e) the plain attach path surfaces the 401');
}
check(count($stub->hits(WIDGETS)) === 1, '(e) no replay: ' . count($stub->hits(WIDGETS)));
check(count($stub->hits(TOKEN)) === 1, '(e) no refresh: ' . count($stub->hits(TOKEN)));

// ---- (f) refresh failure: the typed auth failure surfaces instead of a replay ----
$stub = new ReplayStub();
$stub->staleNext = true;
$stub->failFrom = 2;
$oauth = new OAuth(null, $stub, $clock, $sleep);
try {
    replayClient($stub, $oauth, 'serviceOAuth')->listWidgets();
    throw new LogicException('(f) expected the refresh failure to surface');
} catch (SdkError $error) {
    $auth = authCause($error);
    check($auth !== null && $auth->status === 500, '(f) the refresh failure surfaces typed: ' . $error);
}
check(count($stub->hits(WIDGETS)) === 1, '(f) no replay after a failed refresh: ' . count($stub->hits(WIDGETS)));
check(count($stub->hits(TOKEN)) === 2, '(f) the refresh was attempted exactly once: ' . count($stub->hits(TOKEN)));

echo 'replay behavior verified', PHP_EOL;
"#;
