//! Emitted-only OAuth runtime for the TypeScript HTTP backend: the generated
//! `typescript/oauth.ts` module, its operations.ts/index re-exports, strict
//! compilation, and native node behavior against a local `node:http` fake
//! authorization server.

#![cfg(feature = "http-protocol")]

use std::{collections::BTreeMap, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.oauth.test/openapi.json";

/// The document is served at a fixed logical URI, so two generations are
/// byte-comparable (emitted source references are stable).
fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

/// One client-credentials scheme and one device-authorization scheme, each used
/// by an operation, with declared token and refresh URLs.
fn oauth_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
            }},
            "/devices": {"get": {
                "operationId": "listDevices",
                "security": [{"deviceOAuth": []}],
                "responses": page
            }}
        },
        "components": {"securitySchemes": {
            "serviceOAuth": {"type": "oauth2", "flows": {
                "authorizationCode": {
                    "authorizationUrl": "https://auth.oauth.test/authorize",
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access", "write": "Write access"}
                },
                "clientCredentials": {
                    "tokenUrl": "https://auth.oauth.test/token",
                    "refreshUrl": "https://auth.oauth.test/token-refresh",
                    "scopes": {"read": "Read access"}
                }
            }},
            "deviceOAuth": {"type": "oauth2", "flows": {"deviceAuthorization": {
                "deviceAuthorizationUrl": "https://auth.oauth.test/device",
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {}
            }}}
        }}
    })
}

/// The same operations with every credential removed: the no-OAuth control.
fn plain_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Plain", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": page}},
            "/devices": {"get": {"operationId": "listDevices", "responses": page}}
        }
    })
}

/// One OpenID Connect scheme used by an operation: its flows are defined by
/// the discovery document at runtime, so every endpoint the emitted runtime
/// resolves comes from discovery.
fn discovery_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.1.0",
        "info": {"title": "OAuth discovery", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"identityOAuth": []}],
                "responses": page
            }},
            "/gadgets": {"get": {
                "operationId": "listGadgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
            }}
        },
        "components": {"securitySchemes": {
            "identityOAuth": {
                "type": "openIdConnect",
                "openIdConnectUrl": "https://authority.oauth.test/.well-known/openid-configuration"
            },
            "serviceOAuth": {"type": "oauth2", "flows": {"clientCredentials": {
                "tokenUrl": "https://auth.oauth.test/token",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@oauth/fixture".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    }
}

fn generate(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn oauth_config() -> OAuthDefaults {
    OAuthDefaults {
        schemes: BTreeMap::from([
            (
                "serviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_CLIENT_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_CLIENT_SECRET".into()),
                    revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
            (
                "deviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_DEVICE_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_DEVICE_SECRET".into()),
                    ..OAuthSchemeConfig::default()
                },
            ),
        ]),
        ..OAuthDefaults::default()
    }
}

fn oauth_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: oauth_config(),
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn off_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                mode: suspect_codegen::http_protocol::OAuthMode::Off,
                ..oauth_config()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn file<'a>(files: &'a [OutFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is emitted"))
        .content
        .as_str()
}

fn files_map(files: &[OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect()
}

#[test]
fn oauth_module_emits_exactly_for_executable_schemes() {
    let files = generate(oauth_document(), &oauth_options());
    let oauth = file(&files, "typescript/oauth.ts");
    for expected in [
        "export interface TokenSet {",
        "export interface TokenStore {",
        "export class MemoryTokenStore implements TokenStore {",
        "export function createClientCredentialsProvider(",
        "export function createRefreshProvider(",
        "export async function refreshToken(",
        "export async function beginAuthorization(",
        "export async function completeAuthorization(",
        "export async function beginDeviceAuthorization(",
        "export async function revoke(",
        "export const oauthSchemes = /* @__PURE__ */ Object.freeze(schemes);",
        "export function tokenStoreKey(",
        "export class AuthError extends Error {",
        // Frozen compiled descriptors carry the plan, including configuration
        // supplements and the compiled client authentication.
        "revocationEndpoint: \"https://auth.oauth.test/revoke\"",
        "clientAuth: \"client-secret-basic\"",
        "clientIdEnv: \"SUSPECT_OAUTH_CLIENT_ID\"",
        "clientSecretEnv: \"SUSPECT_OAUTH_CLIENT_SECRET\"",
        "refreshUrl: \"https://auth.oauth.test/token-refresh\"",
        "deviceAuthorizationUrl: \"https://auth.oauth.test/device\"",
        "refreshSkewSeconds: 30",
        "grant_type: 'client_credentials'",
        "grant_type: 'authorization_code'",
        "code_challenge_method', 'S256'",
        "grant_type: 'refresh_token'",
        "urn:ietf:params:oauth:grant-type:device_code",
    ] {
        assert!(oauth.contains(expected), "oauth.ts lacks {expected}");
    }
    // The operations module re-exports the conditional surface exactly.
    let operations = file(&files, "typescript/operations.ts");
    assert!(operations.contains("export { MemoryTokenStore, AuthError, isAuthError, oauthSchemes, tokenStoreKey, createClientCredentialsProvider, createRefreshProvider, refreshToken, beginAuthorization, completeAuthorization, revoke, beginDeviceAuthorization, createReplayingCredentialsProvider } from './oauth.js';"));
    assert!(operations.contains("export type { TokenSet, TokenStore, AuthErrorKind, CompiledFlowKind, CompiledFlow, CompiledScheme, AuthorizationTransaction, AuthorizationBegin, ClientCredentialsProviderOptions, RefreshProviderOptions, RefreshOptions, BeginAuthorizationOptions, CompleteAuthorizationOptions, RevokeOptions, BeginDeviceAuthorizationOptions, ReplayingCredential } from './oauth.js';"));
    // No introspection endpoint was configured, so introspection never emits.
    assert!(!oauth.contains("export async function introspect("));
    assert!(!operations.contains("IntrospectOptions"));
    // No scheme carries a discovery URL, so the discovery engine never emits.
    assert!(!oauth.contains("'discovery-failed'"));
    assert!(!oauth.contains("function discover("));
    assert!(!oauth.contains("resolveEndpoint("));
    // The package index and exports pick the module up exactly when emitted.
    assert!(
        file(&files, "typescript/source/index.ts")
            .contains("export * as oauth from '../oauth.js';")
    );
    assert!(file(&files, "typescript/package.json").contains("\"./oauth\""));

    // Control A: the same options against a document without OAuth schemes
    // keep the pre-OAuth bytes identical, with no new file and no re-exports.
    let mut unbound = oauth_options();
    unbound.sdk_defaults = Some(SdkDefaults::v1());
    let control = files_map(&generate(plain_document(), &unbound));
    let baseline = files_map(&generate(plain_document(), &GenerationOptions::default()));
    let differing: Vec<&String> = control
        .keys()
        .chain(baseline.keys())
        .filter(|path| control.get(*path) != baseline.get(*path))
        .collect();
    assert!(
        differing.is_empty(),
        "no OAuth usable: configured client defaults changed {differing:?}"
    );
    assert!(!control.contains_key("typescript/oauth.ts"));
    assert!(!control["typescript/operations.ts"].contains("./oauth.js"));
    assert!(!control["typescript/source/index.ts"].contains("oauth.js"));

    // Control B: oauth mode off emits nothing and stays byte-identical to the
    // pre-OAuth generation of the same document.
    let off = files_map(&generate(oauth_document(), &off_options()));
    let baseline_off = files_map(&generate(oauth_document(), &GenerationOptions::default()));
    let differing_off: Vec<&String> = off
        .keys()
        .chain(baseline_off.keys())
        .filter(|path| off.get(*path) != baseline_off.get(*path))
        .collect();
    assert!(
        differing_off.is_empty(),
        "oauth off changed {differing_off:?}"
    );
    assert!(!off.contains_key("typescript/oauth.ts"));
    assert!(!off["typescript/operations.ts"].contains("./oauth.js"));
}

/// Discovery options: a confidential OpenID Connect client whose endpoints
/// the discovery document defines at runtime, plus an OAuth2 scheme whose
/// auxiliary endpoints are configured (compiled endpoints win there).
fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "identityOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_DISCOVERY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_DISCOVERY_CLIENT_SECRET".into()),
                            revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                            introspection_endpoint: Some(
                                "https://auth.oauth.test/introspect".into(),
                            ),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

#[test]
fn discovery_schemes_emit_the_discovery_engine_and_cache() {
    let files = generate(discovery_document(), &discovery_options());
    let oauth = file(&files, "typescript/oauth.ts");
    // The discovery engine, its typed failure kind and the per-provider cache
    // emit exactly when a scheme compiles a discovery URL.
    for expected in [
        "'discovery-failed'",
        "interface DiscoveredEndpoints {",
        "const DISCOVERY_MAX_BYTES = 1048576;",
        "function discoveredEndpoint(",
        "function discoveryDocument(",
        "type DiscoveryCache = Map<string, Promise<DiscoveredEndpoints>>;",
        "async function discover(",
        "async function resolveEndpoint(",
        "function discoveryClientAuth(",
        "discovery: \"https://authority.oauth.test/.well-known/openid-configuration\"",
        // The documented precedence and issuer rule.
        "explicit compiled endpoint always wins",
        "issuer does not share the discovery URL origin",
        // The discovery-aware providers replaced the compiled-only ones.
        "export function createClientCredentialsProvider(",
        "export function createRefreshProvider(",
    ] {
        assert!(oauth.contains(expected), "oauth.ts lacks {expected}");
    }
    // The operations re-export surface is unchanged by discovery: the engine
    // is internal. The configured OAuth2 scheme emits the auxiliary
    // endpoints, so revoke/introspect join the surface as before.
    let operations = file(&files, "typescript/operations.ts");
    assert!(operations.contains("export { MemoryTokenStore, AuthError, isAuthError, oauthSchemes, tokenStoreKey, createClientCredentialsProvider, createRefreshProvider, refreshToken, beginAuthorization, completeAuthorization, revoke, introspect, createReplayingCredentialsProvider } from './oauth.js';"));
    // The configured OAuth2 scheme keeps its compiled endpoints (they win
    // over discovery); the discovery engine only fills the gaps.
    assert!(oauth.contains("revocationEndpoint: \"https://auth.oauth.test/revoke\""));
    assert!(oauth.contains("introspectionEndpoint: \"https://auth.oauth.test/introspect\""));
    // Control: without a discovery URL the engine is absent entirely.
    let plain = generate(oauth_document(), &oauth_options());
    let plain_oauth = file(&plain, "typescript/oauth.ts");
    assert!(!plain_oauth.contains("'discovery-failed'"));
    assert!(!plain_oauth.contains("function discover("));
    assert!(!plain_oauth.contains("resolveEndpoint("));
    // An OIDC-only document without usable compiled flows now emits, because
    // its discovery URL defines the endpoints at runtime.
    assert!(plain_oauth.contains("export function tokenStoreKey("));
}

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

#[test]
fn generated_oauth_module_compiles_strictly_with_the_package() {
    if !tool_available("tsc") {
        eprintln!("tsc is not on PATH; skipping the strict OAuth compile check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(oauth_document(), &oauth_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "operations.ts",
            "oauth.ts",
            "--noEmit",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
}

#[test]
fn oauth_lifecycle_behaves_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the behavioral OAuth check");
        return;
    }
    let fetch_available = Command::new("node")
        .arg("-e")
        .arg("process.exit(typeof fetch === 'function' ? 0 : 1)")
        .output()
        .is_ok_and(|output| output.status.success());
    if !fetch_available {
        eprintln!("node lacks native fetch; skipping the behavioral OAuth check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(oauth_document(), &oauth_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("driver.mjs")
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

#[test]
fn discovery_lifecycle_behaves_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the discovery behavioral check");
        return;
    }
    let fetch_available = Command::new("node")
        .arg("-e")
        .arg("process.exit(typeof fetch === 'function' ? 0 : 1)")
        .output()
        .is_ok_and(|output| output.status.success());
    if !fetch_available {
        eprintln!("node lacks native fetch; skipping the discovery behavioral check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(discovery_document(), &discovery_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), DISCOVERY_DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("driver.mjs")
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

const DISCOVERY_DRIVER: &str = r#"
import assert from 'node:assert/strict';
import http from 'node:http';
import {
  AuthError,
  createClientCredentialsProvider,
  createRefreshProvider,
  introspect,
  isAuthError,
  listWidgets,
  revoke,
  tokenStoreKey,
} from './dist/operations.js';

// The fake OpenID Connect provider: the discovery document, the discovered
// token endpoint, revocation and introspection. Every hit is recorded with
// its method, accept header and Authorization header.
const hits = [];
let accessTokenCounter = 0;
let mode = 'ok';
const server = http.createServer((request, response) => {
  const chunks = [];
  request.on('data', chunk => chunks.push(chunk));
  request.on('end', () => {
    const hit = {
      path: new URL(request.url, 'http://x').pathname,
      method: request.method,
      accept: request.headers.accept ?? '',
      authorization: request.headers.authorization ?? null,
      body: new URLSearchParams(Buffer.concat(chunks).toString('utf8')),
    };
    hits.push(hit);
    const json = (value, status = 200) => {
      response.writeHead(status, { 'content-type': 'application/json' });
      response.end(JSON.stringify(value));
    };
    if (hit.path === '/.well-known/openid-configuration' && hit.method === 'GET') {
      assert.equal(hit.accept, 'application/json', 'discovery requests advertise JSON');
      if (mode === 'http-500') return json({ error: 'boom' }, 500);
      if (mode === 'issuer-mismatch') {
        return json({
          issuer: 'https://elsewhere.oauth.test',
          token_endpoint: 'https://authority.oauth.test/oauth/token',
        });
      }
      return json({
        issuer: 'https://authority.oauth.test',
        token_endpoint: 'https://authority.oauth.test/oauth/token',
        revocation_endpoint: 'https://authority.oauth.test/oauth/revoke',
        introspection_endpoint: 'https://authority.oauth.test/oauth/introspect',
        unknown_member: { nested: true },
      });
    }
    if (hit.path === '/oauth/token' && hit.method === 'POST') {
      const basic = Buffer.from((hit.authorization ?? '').slice(6), 'base64').toString('utf8');
      if (basic !== 'discovery-client:discovery-secret') return json({ error: 'invalid_client' }, 401);
      accessTokenCounter += 1;
      return json({ access_token: `discovered-access-${accessTokenCounter}`, token_type: 'Bearer', expires_in: 3600 });
    }
    if (hit.path === '/oauth/revoke' && hit.method === 'POST') {
      assert.equal(hit.body.get('token'), 'token-to-revoke');
      response.writeHead(200);
      response.end();
      return;
    }
    if (hit.path === '/oauth/introspect' && hit.method === 'POST') {
      return json({ active: true, scope: 'read' });
    }
    if (hit.path === '/revoke' && hit.method === 'POST') {
      assert.equal(hit.body.get('token'), 'token-to-revoke');
      response.writeHead(200);
      response.end();
      return;
    }
    if (hit.path === '/introspect' && hit.method === 'POST') {
      return json({ active: true, scope: 'read' });
    }
    if (hit.path === '/api/widgets') return json({ ok: true });
    response.writeHead(404, { 'content-type': 'text/plain' });
    response.end(`missing path ${hit.path}`);
  });
});

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
const transport = async (input, init) => {
  const target = new URL(String(input instanceof Request ? input.url : input));
  if (target.hostname === 'authority.oauth.test' || target.hostname === 'api.oauth.test' || target.hostname === 'auth.oauth.test') {
    if (target.hostname === 'api.oauth.test') target.pathname = `/api${target.pathname.slice(3)}`;
    target.protocol = 'http:';
    target.hostname = '127.0.0.1';
    target.port = String(port);
  }
  return fetch(target.toString(), init);
};

const context = { signal: new AbortController().signal };
let now = 1_000_000;
const clock = () => now;

// Client credentials resolve through the DISCOVERED token endpoint: the
// discovery document is fetched exactly once and cached on the provider, so
// the second attach re-fetches nothing.
hits.length = 0;
const provider = createClientCredentialsProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
const first = await provider(context);
assert.equal(first.authorization, 'Bearer discovered-access-1');
const discoveryHits = () => hits.filter(hit => hit.path === '/.well-known/openid-configuration');
const tokenHits = () => hits.filter(hit => hit.path === '/oauth/token');
assert.equal(discoveryHits().length, 1, 'exactly one discovery fetch');
assert.equal(discoveryHits()[0].method, 'GET');
assert.equal(tokenHits().length, 1);
assert.match(tokenHits()[0].authorization, /^Basic /);
assert.equal(
  Buffer.from(tokenHits()[0].authorization.slice(6), 'base64').toString('utf8'),
  'discovery-client:discovery-secret',
);
assert.ok(!hits.some(hit => hit.path === '/token'), 'the compiled fallback URL was never contacted');
const second = await provider(context);
assert.equal(second.authorization, 'Bearer discovered-access-1');
assert.equal(discoveryHits().length, 1, 'the discovery document is cached per provider');
assert.equal(tokenHits().length, 1, 'the cached token set is reused');

// Two concurrent attaches share one discovery fetch and one acquisition.
hits.length = 0;
now = 10_000_000;
const flightProvider = createClientCredentialsProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
const [sharedOne, sharedTwo] = await Promise.all([flightProvider(context), flightProvider(context)]);
assert.equal(sharedOne.authorization, sharedTwo.authorization);
assert.equal(discoveryHits().length, 1, 'concurrent attaches single-flight the discovery fetch');
assert.equal(tokenHits().length, 1, 'concurrent attaches single-flight the acquisition');

// Issuer mismatch is a typed discovery failure whose message carries no body
// text; after the server is fixed the next call retries and succeeds.
hits.length = 0;
mode = 'issuer-mismatch';
const mismatchProvider = createClientCredentialsProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
await assert.rejects(
  () => mismatchProvider(context),
  (error) => {
    assert.ok(isAuthError(error), 'failures are typed AuthError');
    assert.equal(error.kind, 'discovery-failed');
    assert.equal(error.scheme, 'identityOAuth');
    assert.ok(!String(error.message).includes('elsewhere'));
    return true;
  },
);
mode = 'ok';
const recovered = await mismatchProvider(context);
assert.equal(recovered.authorization, 'Bearer discovered-access-3', 'a failed fetch is retried on the next call');
assert.equal(discoveryHits().length, 2, 'the failed fetch was not cached');

// A failing discovery request is a typed failure that the next call retries.
hits.length = 0;
mode = 'http-500';
const failingProvider = createClientCredentialsProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
await assert.rejects(
  () => failingProvider(context),
  (error) => isAuthError(error) && error.kind === 'discovery-failed' && error.status === 500,
);
mode = 'ok';
const retried = await failingProvider(context);
assert.equal(retried.authorization, 'Bearer discovered-access-4');
assert.equal(discoveryHits().length, 2, 'the failed fetch was retried');

// End to end through the generated operation runtime.
hits.length = 0;
now = 20_000_000;
const clientProvider = createClientCredentialsProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
const result = await listWidgets(
  { serverURL: 'https://api.oauth.test/v1', auth: { identityOAuth: clientProvider }, fetch: transport },
  {},
);
assert.equal(result.status, 200);
assert.equal(result.data.ok, true);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 1);
assert.equal(hits.filter(hit => hit.path === '/oauth/token').length, 1);

// Revocation and introspection resolve through the compiled precedence: the
// configured OAuth2 scheme posts to its compiled endpoints, while the
// discovery-only OpenID Connect scheme resolves both through the discovery
// document.
hits.length = 0;
await revoke({
  scheme: 'serviceOAuth', token: 'token-to-revoke', tokenTypeHint: 'access_token',
  clientId: 'discovery-client', clientSecret: 'discovery-secret', fetch: transport,
});
assert.equal(hits.filter(hit => hit.path === '/revoke').length, 1, 'the configured endpoint wins over discovery');
await revoke({
  scheme: 'identityOAuth', token: 'token-to-revoke', tokenTypeHint: 'access_token',
  clientId: 'discovery-client', clientSecret: 'discovery-secret', fetch: transport,
});
assert.equal(hits.filter(hit => hit.path === '/oauth/revoke').length, 1, 'revocation resolved through discovery');
assert.equal(discoveryHits().length, 1, 'the one-shot helper fetched discovery for the scheme without a configured endpoint');
const claims = await introspect({
  scheme: 'identityOAuth', token: 'token-to-inspect',
  clientId: 'discovery-client', clientSecret: 'discovery-secret', fetch: transport,
});
assert.equal(claims.active, true);

// The refresh provider refreshes through the discovered token endpoint.
hits.length = 0;
now = 30_000_000;
const refreshProvider = createRefreshProvider({
  scheme: 'identityOAuth', clientId: 'discovery-client', clientSecret: 'discovery-secret', clock, fetch: transport,
});
await assert.rejects(
  () => refreshProvider(context),
  (error) => isAuthError(error) && error.kind === 'missing-credential',
  'no stored token exists before the first acquisition',
);
await new Promise(resolve => server.close(resolve));
console.log('discovery driver ok');
"#;

const DRIVER: &str = r#"
import assert from 'node:assert/strict';
import http from 'node:http';
import {
  MemoryTokenStore,
  beginAuthorization,
  beginDeviceAuthorization,
  completeAuthorization,
  createClientCredentialsProvider,
  isAuthError,
  listWidgets,
  refreshToken,
  revoke,
  tokenStoreKey,
} from './dist/operations.js';

// The fake authorization server: node:http on a loopback port. Every hit is
// recorded with its Authorization header and decoded form body.
const hits = [];
let accessTokenCounter = 0;
let devicePolls = 0;
const server = http.createServer((request, response) => {
  const chunks = [];
  request.on('data', chunk => chunks.push(chunk));
  request.on('end', () => {
    const url = new URL(request.url, 'http://x');
    const hit = {
      path: url.pathname,
      method: request.method,
      authorization: request.headers.authorization ?? null,
      body: new URLSearchParams(Buffer.concat(chunks).toString('utf8')),
    };
    hits.push(hit);
    respond(hit, response);
  });
});

function respond(hit, response) {
  const json = (value, status = 200) => {
    response.writeHead(status, { 'content-type': 'application/json' });
    response.end(JSON.stringify(value));
  };
  const basicUser = () => {
    const header = hit.authorization ?? '';
    if (!header.startsWith('Basic ')) return null;
    return Buffer.from(header.slice(6), 'base64').toString('utf8');
  };
  const unauthorized = () => json({ error: 'invalid_client' }, 401);
  if (hit.path === '/token' && hit.method === 'POST') {
    const grant = hit.body.get('grant_type');
    if (grant === 'urn:ietf:params:oauth:grant-type:device_code') {
      if (basicUser() !== 'device-id:device-secret') return unauthorized();
      devicePolls += 1;
      if (devicePolls === 1) return json({ error: 'authorization_pending' }, 400);
      return json({ access_token: 'device-access', token_type: 'Bearer', expires_in: 3600 });
    }
    if (basicUser() !== 'client-id:client-secret') return unauthorized();
    if (grant === 'authorization_code') {
      assert.equal(typeof hit.body.get('code_verifier'), 'string');
      assert.ok(hit.body.get('code_verifier').length >= 43, 'PKCE verifier length');
      return json({ access_token: 'code-access', token_type: 'Bearer', expires_in: 3600 });
    }
    assert.equal(grant, 'client_credentials');
    accessTokenCounter += 1;
    return json({ access_token: `access-token-${accessTokenCounter}`, token_type: 'Bearer', expires_in: 3600 });
  }
  if (hit.path === '/token-refresh' && hit.method === 'POST') {
    if (basicUser() !== 'client-id:client-secret') return unauthorized();
    const presented = hit.body.get('refresh_token');
    if (presented === 'stale') return json({ access_token: 'refreshed-access', token_type: 'bearer', refresh_token: 'rotated' });
    if (presented === 'rotated') return json({ access_token: 'second-refresh-access', token_type: 'bearer' });
    return json({ error: 'invalid_grant' }, 400);
  }
  if (hit.path === '/device' && hit.method === 'POST') {
    return json({ device_code: 'device-code-1', user_code: 'ABCD-EFGH', verification_uri: 'https://auth.oauth.test/activate', expires_in: 30, interval: 0 });
  }
  if (hit.path === '/revoke' && hit.method === 'POST') {
    response.writeHead(200);
    response.end();
    return;
  }
  if (hit.path === '/api/widgets' || hit.path === '/api/devices') return json({ ok: true });
  response.writeHead(404);
  response.end();
}

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
const transport = async (input, init) => {
  const source = String(input instanceof Request ? input.url : input);
  const target = new URL(source);
  if (target.hostname === 'auth.oauth.test') {
    target.protocol = 'http:';
    target.hostname = '127.0.0.1';
    target.port = String(port);
  } else if (target.hostname === 'api.oauth.test') {
    target.protocol = 'http:';
    target.hostname = '127.0.0.1';
    target.port = String(port);
    target.pathname = `/api${target.pathname.slice(3)}`;
  }
  return fetch(target.toString(), init);
};

let now = 1_000_000;
const clock = () => now;
const context = { signal: new AbortController().signal };

// First attach acquires with the compiled client-secret-basic authentication
// and the form-encoded grant; the second attach reuses the stored token.
hits.length = 0;
const store = new MemoryTokenStore();
const provider = createClientCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', store, clock, fetch: transport,
});
const first = await provider(context);
assert.equal(first.authorization, 'Bearer access-token-1');
let tokenHits = hits.filter(hit => hit.path === '/token');
assert.equal(tokenHits.length, 1);
assert.equal(tokenHits[0].body.get('grant_type'), 'client_credentials');
assert.match(tokenHits[0].authorization, /^Basic /);
assert.equal(Buffer.from(tokenHits[0].authorization.slice(6), 'base64').toString('utf8'), 'client-id:client-secret');
const second = await provider(context);
assert.equal(second.authorization, 'Bearer access-token-1');
assert.equal(hits.filter(hit => hit.path === '/token').length, 1, 'second attach reuses the stored token');

// Past expiry (beyond the compiled skew) the next attach re-acquires.
now += 4_000_000;
const third = await provider(context);
assert.equal(third.authorization, 'Bearer access-token-2');
assert.equal(hits.filter(hit => hit.path === '/token').length, 2, 'an expired token re-acquires');

// Concurrent attaches share exactly one in-flight acquisition.
hits.length = 0;
now = 10_000_000;
const flightProvider = createClientCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', store: new MemoryTokenStore(), clock, fetch: transport,
});
const [sharedOne, sharedTwo] = await Promise.all([flightProvider(context), flightProvider(context)]);
assert.equal(sharedOne.authorization, sharedTwo.authorization);
assert.equal(hits.filter(hit => hit.path === '/token').length, 1, 'concurrent attaches single-flight');

// Explicit refresh: the rotated refresh token is adopted and stored; a later
// response without one retains the stored refresh token.
hits.length = 0;
const refreshStore = new MemoryTokenStore();
const refreshed = await refreshToken({
  scheme: 'serviceOAuth', refreshToken: 'stale', store: refreshStore, clientId: 'client-id', clientSecret: 'client-secret', fetch: transport,
});
assert.equal(refreshed.refreshToken, 'rotated');
const key = tokenStoreKey('serviceOAuth', 'https://auth.oauth.test/token-refresh', 'client-id');
assert.equal((await refreshStore.load(key)).refreshToken, 'rotated');
const retained = await refreshToken({
  scheme: 'serviceOAuth', refreshToken: 'rotated', store: refreshStore, clientId: 'client-id', clientSecret: 'client-secret', fetch: transport,
});
assert.equal(retained.accessToken, 'second-refresh-access');
assert.equal(retained.refreshToken, 'rotated', 'a response without a refresh token retains the stored one');
assert.equal((await refreshStore.load(key)).refreshToken, 'rotated');
const refreshHits = hits.filter(hit => hit.path === '/token-refresh');
assert.equal(refreshHits.length, 2, 'refresh posts to the declared refresh URL');
assert.equal(refreshHits[0].body.get('grant_type'), 'refresh_token');
assert.match(refreshHits[0].authorization, /^Basic /);

// Wrong credentials produce the typed error without any secret in the message.
hits.length = 0;
const badProvider = createClientCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'super-secret-value', fetch: transport,
});
await assert.rejects(
  () => badProvider(context),
  (error) => {
    assert.ok(isAuthError(error), 'failures are typed AuthError');
    assert.equal(error.kind, 'invalid-client');
    assert.equal(error.status, 401);
    assert.equal(error.scheme, 'serviceOAuth');
    assert.ok(!String(error.message).includes('super-secret-value'), 'no secret in the message');
    assert.ok(!String(error.message).includes('client-secret'), 'no secret fragment in the message');
    assert.ok(!String(error.message).includes('access-token'), 'no token in the message');
    return true;
  },
);

// Revocation posts to the configured endpoint with the compiled authentication.
hits.length = 0;
await revoke({
  scheme: 'serviceOAuth', token: 'token-to-revoke', tokenTypeHint: 'access_token', clientId: 'client-id', clientSecret: 'client-secret', fetch: transport,
});
const revokeHits = hits.filter(hit => hit.path === '/revoke');
assert.equal(revokeHits.length, 1, 'revocation posts to the configured endpoint');
assert.equal(revokeHits[0].body.get('token'), 'token-to-revoke');
assert.equal(revokeHits[0].body.get('token_type_hint'), 'access_token');
assert.match(revokeHits[0].authorization, /^Basic /);

// Authorization code + PKCE: the begin URL carries the S256 challenge and
// random state; state mismatch is a typed error; the transaction is consumed
// exactly once; the happy path exchanges the code with the stored verifier.
hits.length = 0;
const begin = await beginAuthorization({
  scheme: 'serviceOAuth', redirectUri: 'https://app.oauth.test/callback', clientId: 'client-id', scopes: ['read'],
});
const authorizationUrl = new URL(begin.authorizationUrl);
assert.equal(authorizationUrl.searchParams.get('response_type'), 'code');
assert.equal(authorizationUrl.searchParams.get('client_id'), 'client-id');
assert.equal(authorizationUrl.searchParams.get('redirect_uri'), 'https://app.oauth.test/callback');
assert.equal(authorizationUrl.searchParams.get('code_challenge_method'), 'S256');
assert.ok(begin.state.length >= 22, 'random state');
assert.ok(authorizationUrl.searchParams.get('code_challenge').length >= 43, 'S256 challenge');
assert.equal(authorizationUrl.searchParams.get('scope'), 'read');
await assert.rejects(
  () => completeAuthorization({
    transaction: begin.transaction, code: 'the-code', state: 'wrong', clientId: 'client-id', clientSecret: 'client-secret', fetch: transport,
  }),
  (error) => isAuthError(error) && error.kind === 'state-mismatch',
);
await assert.rejects(
  () => completeAuthorization({
    transaction: begin.transaction, code: 'the-code', state: begin.state, clientId: 'client-id', clientSecret: 'client-secret', fetch: transport,
  }),
  (error) => isAuthError(error) && error.kind === 'transaction-consumed',
);
const retry = await beginAuthorization({
  scheme: 'serviceOAuth', redirectUri: 'https://app.oauth.test/callback', clientId: 'client-id',
});
const codeStore = new MemoryTokenStore();
const codeTokens = await completeAuthorization({
  transaction: retry.transaction, code: 'the-code', state: retry.state, clientId: 'client-id', clientSecret: 'client-secret', store: codeStore, fetch: transport,
});
assert.equal(codeTokens.accessToken, 'code-access');
const codeKey = tokenStoreKey('serviceOAuth', 'https://auth.oauth.test/token', 'client-id');
assert.equal((await codeStore.load(codeKey)).accessToken, 'code-access');
const codeHits = hits.filter(hit => hit.path === '/token' && hit.body.get('grant_type') === 'authorization_code');
assert.equal(codeHits.length, 1);

// Device flow: authorization_pending polls again, then the typed token set
// arrives and lands in the store under the compiled identity.
hits.length = 0;
devicePolls = 0;
const deviceTokens = await beginDeviceAuthorization({
  scheme: 'deviceOAuth', clientId: 'device-id', clientSecret: 'device-secret', store: new MemoryTokenStore(), clock, fetch: transport,
});
assert.equal(deviceTokens.accessToken, 'device-access');
assert.equal(hits.filter(hit => hit.path === '/device').length, 1);
assert.equal(
  hits.filter(hit => hit.path === '/token' && hit.body.get('grant_type') === 'urn:ietf:params:oauth:grant-type:device_code').length,
  2,
  'authorization_pending polls again',
);

// End to end through the generated operation runtime: the provider is an
// ordinary credential callback and the API request carries the acquired token.
hits.length = 0;
now = 20_000_000;
const clientProvider = createClientCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', store: new MemoryTokenStore(), clock, fetch: transport,
});
const result = await listWidgets(
  { serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: clientProvider }, fetch: transport },
  {},
);
assert.equal(result.status, 200);
assert.equal(result.data.ok, true);
const apiHits = hits.filter(hit => hit.path === '/api/widgets');
assert.equal(apiHits.length, 1);
assert.equal(apiHits[0].authorization, 'Bearer access-token-4');
assert.equal(hits.filter(hit => hit.path === '/token').length, 1, 'the runtime call acquired exactly one token');

await new Promise(resolve => server.close(resolve));
console.log('oauth driver ok');
"#;

/// The replay fixture: one client-credentials scheme over a JSON operation
/// and one over a streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    let page = json!({
        "200": {"description": "Page", "content": {"application/json": {"schema": {
            "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": false
        }}}}}
    );
    json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth replay", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"serviceOAuth": ["read"]}],
                "responses": page
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
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_REPLAY_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_REPLAY_CLIENT_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "feedOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_REPLAY_FEED_ID".into()),
                            client_secret_env: Some("SUSPECT_REPLAY_FEED_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(replay_document(), &replay_options());
    let oauth = file(&files, "typescript/oauth.ts");
    for expected in [
        "export interface ReplayingCredential {",
        "export function createReplayingCredentialsProvider(options: ClientCredentialsProviderOptions): ReplayingCredential {",
        "triggers exactly one coordinated refresh",
        "delivered stream data prevents a transparent restart",
        "const noReplayRequirements: Readonly<Record<string, ReadonlySet<string>>> = {",
        // The streaming operation's requirement pointer is compiled into the
        // feed scheme's no-replay set.
        "\"/paths/~1events/get/security/0/feedOAuth\"",
    ] {
        assert!(oauth.contains(expected), "oauth.ts lacks {expected}");
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("~1widgets/get/security/0/serviceOAuth"));
    // The replay machinery stays out of the operations re-exports of a
    // package without any executable client-credentials flow: an
    // authorization-code-only scheme compiles exactly the pre-replay bytes.
    let code_only_document = json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {"/widgets": {"get": {
            "operationId": "listWidgets",
            "security": [{"userOAuth": ["read"]}],
            "responses": {"200": {"description": "Ok"}}
        }}},
        "components": {"securitySchemes": {"userOAuth": {"type": "oauth2", "flows": {"authorizationCode": {
            "authorizationUrl": "https://auth.oauth.test/authorize",
            "tokenUrl": "https://auth.oauth.test/token",
            "scopes": {"read": "Read access"}
        }}}}}
    });
    let code_only_options = GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([(
                    "userOAuth".to_owned(),
                    OAuthSchemeConfig {
                        client_id_env: Some("SUSPECT_CODE_ONLY_ID".into()),
                        client_secret_env: Some("SUSPECT_CODE_ONLY_SECRET".into()),
                        ..OAuthSchemeConfig::default()
                    },
                )]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    };
    let code_only = generate(code_only_document, &code_only_options);
    let plain_oauth = file(&code_only, "typescript/oauth.ts");
    assert!(!plain_oauth.contains("createReplayingCredentialsProvider"));
    assert!(!plain_oauth.contains("noReplayRequirements"));
}

#[test]
fn replay_lifecycle_behaves_in_node() {
    if !tool_available("tsc") || !tool_available("node") {
        eprintln!("tsc/node are not on PATH; skipping the replay behavioral check");
        return;
    }
    let fetch_available = Command::new("node")
        .arg("-e")
        .arg("process.exit(typeof fetch === 'function' ? 0 : 1)")
        .output()
        .is_ok_and(|output| output.status.success());
    if !fetch_available {
        eprintln!("node lacks native fetch; skipping the replay behavioral check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &generate(replay_document(), &replay_options()),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("driver.mjs"), REPLAY_DRIVER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "--outDir",
            "dist",
            "operations.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("driver.mjs")
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

const REPLAY_DRIVER: &str = r#"
import assert from 'node:assert/strict';
import http from 'node:http';
import {
  createClientCredentialsProvider,
  createReplayingCredentialsProvider,
  isAuthError,
  listWidgets,
  streamEvents,
} from './dist/operations.js';

// The fake authorization and API server. The API answers 401 for the first
// token of a scheme and 200 afterwards, so the driver can count exactly one
// refresh and one replay; `apiMode` and `failRefresh` flip per scenario.
const hits = [];
let serviceTokens = 0;
let feedTokens = 0;
let apiMode = 'ok';
let failRefresh = false;
let failFrom = Number.MAX_SAFE_INTEGER;
let staleToken = null;
let staleNext = false;
const server = http.createServer((request, response) => {
  const chunks = [];
  request.on('data', chunk => chunks.push(chunk));
  request.on('end', () => {
    const hit = {
      path: new URL(request.url, 'http://x').pathname,
      method: request.method,
      authorization: request.headers.authorization ?? null,
    };
    hits.push(hit);
    const json = (value, status = 200) => {
      response.writeHead(status, { 'content-type': 'application/json' });
      response.end(JSON.stringify(value));
    };
    if (hit.path === '/token' && hit.method === 'POST') {
      serviceTokens += 1;
      if (serviceTokens >= failFrom) return json({ error: 'server_error' }, 500);
      const token = `svc-${serviceTokens}`;
      if (staleNext) { staleToken = token; staleNext = false; }
      return json({ access_token: token, token_type: 'Bearer', expires_in: 3600 });
    }
    if (hit.path === '/feed-token' && hit.method === 'POST') {
      feedTokens += 1;
      return json({ access_token: `feed-${feedTokens}`, token_type: 'Bearer', expires_in: 3600 });
    }
    if (hit.path === '/api/widgets') {
      if (apiMode === 'always-401') return json({ error: 'unauthorized' }, 401);
      if (staleToken !== null && hit.authorization === `Bearer ${staleToken}`) return json({ error: 'stale' }, 401);
      return json({ ok: true });
    }
    if (hit.path === '/api/events') return json({ error: 'stream-denied' }, 401);
    response.writeHead(404);
    response.end();
  });
});

await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
const transport = async (input, init) => {
  const target = new URL(String(input instanceof Request ? input.url : input));
  if (target.hostname === 'auth.oauth.test') {
    target.protocol = 'http:';
    target.hostname = '127.0.0.1';
    target.port = String(port);
  } else if (target.hostname === 'api.oauth.test') {
    target.protocol = 'http:';
    target.hostname = '127.0.0.1';
    target.port = String(port);
    target.pathname = `/api${target.pathname.slice(3)}`;
  }
  return fetch(target.toString(), init);
};

let now = 1_000_000;
const clock = () => now;

// (a) 401 then success: one refresh, one replay, and the caller sees 200.
hits.length = 0;
apiMode = 'first-401';
staleNext = true;
const replaying = createReplayingCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', clock, fetch: transport,
});
const ok = await listWidgets(
  { serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: replaying }, fetch: replaying.fetch(transport) },
  {},
);
assert.equal(ok.status, 200);
assert.equal(ok.data.ok, true);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 2, 'exactly one replay');
assert.equal(hits.filter(hit => hit.path === '/token').length, 2, 'exactly one token refresh');
assert.equal(hits.filter(hit => hit.path === '/feed-token').length, 0);

// (b) 401 then 401: the second 401 surfaces and exactly one refresh ran.
hits.length = 0;
apiMode = 'always-401';
await assert.rejects(
  () => listWidgets(
    { serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: replaying }, fetch: replaying.fetch(transport) },
    {},
  ),
  (error) => {
    assert.equal(error.status, 401);
    return true;
  },
);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 2, 'one replay, no loops');
assert.equal(hits.filter(hit => hit.path === '/token').length, 1, 'exactly one refresh');

// (c) concurrent 401s: ONE refresh, two replays.
hits.length = 0;
apiMode = 'first-401';
staleNext = true;
const shared = createReplayingCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', clock, fetch: transport,
});
const [first, second] = await Promise.all([
  listWidgets({ serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: shared }, fetch: shared.fetch(transport) }, {}),
  listWidgets({ serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: shared }, fetch: shared.fetch(transport) }, {}),
]);
assert.equal(first.status, 200);
assert.equal(second.status, 200);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 4, 'two replays');
assert.equal(hits.filter(hit => hit.path === '/token').length, 2, 'one shared refresh');

// (d) a streaming operation is never replayed: the typed 401 surfaces.
hits.length = 0;
apiMode = 'ok';
const feed = createReplayingCredentialsProvider({
  scheme: 'feedOAuth', clientId: 'feed-id', clientSecret: 'feed-secret', clock, fetch: transport,
});
await assert.rejects(
  () => streamEvents(
    { serverURL: 'https://api.oauth.test/v1', auth: { feedOAuth: feed }, fetch: feed.fetch(transport) },
    {},
  ),
  (error) => {
    assert.equal(error.status, 401);
    return true;
  },
);
assert.equal(hits.filter(hit => hit.path === '/api/events').length, 1, 'no replay for the streaming operation');
assert.equal(hits.filter(hit => hit.path === '/feed-token').length, 1, 'no refresh for the streaming operation');

// (e) replay disabled by default: the plain provider surfaces the 401
// without any refresh.
hits.length = 0;
apiMode = 'first-401';
staleNext = true;
const plain = createClientCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', clock, fetch: transport,
});
await assert.rejects(
  () => listWidgets(
    { serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: plain }, fetch: transport },
    {},
  ),
  (error) => {
    assert.equal(error.status, 401);
    return true;
  },
);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 1, 'no replay');
assert.equal(hits.filter(hit => hit.path === '/token').length, 1, 'no refresh');

// (f) refresh failure: the typed auth error surfaces instead of a replay.
hits.length = 0;
apiMode = 'first-401';
staleNext = true;
failRefresh = true;
failFrom = serviceTokens + 2;
const failing = createReplayingCredentialsProvider({
  scheme: 'serviceOAuth', clientId: 'client-id', clientSecret: 'client-secret', clock, fetch: transport,
});
await assert.rejects(
  () => listWidgets(
    { serverURL: 'https://api.oauth.test/v1', auth: { serviceOAuth: failing }, fetch: failing.fetch(transport) },
    {},
  ),
  (error) => {
    assert.equal(error.kind, 'transport', 'the operation wraps the typed auth failure');
    assert.ok(isAuthError(error.cause), 'the typed AuthError is the cause');
    assert.equal(error.cause.kind, 'server-error');
    assert.equal(error.cause.status, 500);
    return true;
  },
);
assert.equal(hits.filter(hit => hit.path === '/api/widgets').length, 1, 'no replay after a failed refresh');
assert.equal(hits.filter(hit => hit.path === '/token').length, 2, 'the refresh was attempted exactly once');

await new Promise(resolve => server.close(resolve));
console.log('replay driver ok');
"#;
