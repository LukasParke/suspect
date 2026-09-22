//! Emitted-only OAuth runtime for the native Dart HTTP backend: the generated
//! `lib/src/oauth.dart` part with its conditional transport pair, no-policy
//! byte-identity, and strict static assertions of the emitted behavior. No
//! Dart toolchain is installed, so the behavioral checks review the emitted
//! source directly; the static runtime files are never modified.

#![cfg(feature = "dart-sdk")]

use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.oauth.test/dart-openapi.json";

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

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::DartHttp,
        package_name: "oauth_sdk".into(),
        package_version: "0.1.0".into(),
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
        schemes: [
            (
                "serviceOAuth".to_owned(),
                OAuthSchemeConfig {
                    client_id_env: Some("SUSPECT_OAUTH_CLIENT_ID".into()),
                    client_secret_env: Some("SUSPECT_OAUTH_CLIENT_SECRET".into()),
                    revocation_endpoint: Some("https://auth.oauth.test/revoke".into()),
                    introspection_endpoint: Some("https://auth.oauth.test/introspect".into()),
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
        ]
        .into_iter()
        .collect(),
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

fn source(files: &[OutFile], path: &str) -> String {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} missing"))
        .content
        .clone()
}

fn files_map(files: &[OutFile]) -> std::collections::BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect()
}

fn assert_unchanged(
    configured: &std::collections::BTreeMap<String, String>,
    baseline: &std::collections::BTreeMap<String, String>,
    label: &str,
) {
    let differing: Vec<&String> = configured
        .keys()
        .chain(baseline.keys())
        .filter(|path| configured.get(*path) != baseline.get(*path))
        .collect();
    assert!(differing.is_empty(), "{label} changed {differing:?}");
}

#[test]
fn oauth_part_emits_exactly_for_executable_schemes() {
    let files = generate(oauth_document(), &oauth_options());
    let oauth = source(&files, "dart/lib/src/oauth.dart");
    for expected in [
        // The token set, the instance-owned store contract and typed error.
        "final class TokenSet {",
        "abstract interface class TokenStore {",
        "final class MemoryTokenStore implements TokenStore {",
        "final class AuthException implements Exception {",
        // Frozen compiled descriptors carry the plan, including configuration
        // supplements and the compiled client authentication.
        "\"serviceOAuth\": CompiledScheme(\"serviceOAuth\", \"oauth2\", 30, null, \"https://auth.oauth.test/revoke\", \"https://auth.oauth.test/introspect\", \"SUSPECT_OAUTH_CLIENT_ID\", \"SUSPECT_OAUTH_CLIENT_SECRET\", <CompiledFlow>[",
        "\"deviceOAuth\": CompiledScheme(\"deviceOAuth\", \"oauth2\", 30, null, null, null, \"SUSPECT_OAUTH_DEVICE_ID\", \"SUSPECT_OAUTH_DEVICE_SECRET\", <CompiledFlow>[",
        "CompiledFlow(\"client-credentials\", null, \"https://auth.oauth.test/token\", \"https://auth.oauth.test/token-refresh\", null, \"client-secret-basic\", false, const <String, String>{\"read\": \"Read access\"",
        "CompiledFlow(\"device-authorization\", null, \"https://auth.oauth.test/token\", null, \"https://auth.oauth.test/device\", \"client-secret-basic\", false, const <String, String>{})",
        // Acquisition, refresh, providers and the authorization-code flow.
        "String tokenStoreKey(String scheme, String issuer, String? clientId)",
        "Future<TokenSet> clientCredentialsToken(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport})",
        "CredentialProvider createClientCredentialsProvider(String scheme,",
        "CredentialProvider createRefreshProvider(String scheme,",
        "Future<TokenSet> refreshToken(String scheme, {required String refreshToken,",
        "AuthorizationBegin beginAuthorization(String scheme, {required String redirectUri,",
        "Future<TokenSet> completeAuthorization(AuthorizationTransaction transaction, {required String code, required String state,",
        "Future<TokenSet> beginDeviceAuthorization(String scheme, {String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, Future<void> Function(int milliseconds)? delay})",
        "Future<void> revokeToken(String scheme, {required String token,",
        "Future<Map<String, dynamic>> introspectToken(String scheme, {required String token,",
        // Skew-aware freshness, single-flight and atomic store replacement.
        "bool _isFresh(TokenSet tokenSet, int skewMilliseconds, int now) => tokenSet.expiresAt - skewMilliseconds > now;",
        "scheme.refreshSkewSeconds * 1000",
        "final Map<String, Future<TokenSet>> _inflight",
        "final pending = _inflight[key];",
        "store?.replace(",
        // Compiled grant constants, never OpenAPI parsing.
        "'grant_type': 'client_credentials'",
        "'grant_type': 'authorization_code'",
        "'grant_type': 'refresh_token'",
        "'grant_type': 'urn:ietf:params:oauth:grant-type:device_code'",
        "'code_challenge_method': 'S256'",
        // Device polling honors pending/slow-down and the injectable delay.
        "error.serverError == 'authorization_pending'",
        "intervalMilliseconds += 5000",
        "Future<void>.delayed(Duration(milliseconds: milliseconds))",
        // Typed errors stay credential-free and the dependency-free SHA-256
        // backs the PKCE challenge without a crypto package.
        "secrets never appear in error messages",
        "const List<int> _sha256K",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.dart lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // Errors never interpolate credentials; the message constructor arguments
    // are the compiled metadata only.
    assert!(
        !oauth.contains("toString() => 'AuthException($kind, $scheme, $message)'\n"),
        "sanity"
    );
    for forbidden in ["accessToken:',", "clientSecret:',"] {
        assert!(
            !oauth.contains(forbidden),
            "oauth.dart may not interpolate {forbidden}"
        );
    }

    // The library registers the part and the platform transport pair, and the
    // pubspec gains no dependency (PKCE uses the embedded SHA-256).
    let library = source(&files, "dart/lib/oauth_sdk.dart");
    assert!(library.contains("part 'src/oauth.dart';"), "{library}");
    assert!(
        library.contains("import 'src/oauth_transport_stub.dart' if (dart.library.io) 'src/oauth_transport_io.dart' as _oauth_transport;"),
        "{library}"
    );
    assert!(
        library.contains("import 'dart:math' show Random;"),
        "{library}"
    );
    let io = source(&files, "dart/lib/src/oauth_transport_io.dart");
    assert!(io.contains("Future<OAuthEndpointResponse> oauthEndpointPost(Uri url, Map<String, String> headers, String body) async {"), "{io}");
    assert!(io.contains("HttpClient()"), "{io}");
    let stub = source(&files, "dart/lib/src/oauth_transport_stub.dart");
    assert!(stub.contains("UnsupportedError"), "{stub}");
    // The compiled environment-variable names resolve through the shared
    // conditional platform pair even without a credential-env policy.
    source(&files, "dart/lib/src/environment_io.dart");
    source(&files, "dart/lib/src/environment_stub.dart");
    let pubspec = source(&files, "dart/pubspec.yaml");
    assert!(
        !pubspec.contains("dependencies:"),
        "the OAuth emission must not add packages: {pubspec}"
    );

    // Control A: the same options against a document without OAuth schemes
    // keep the pre-OAuth bytes identical, with no new files.
    let mut unbound = oauth_options();
    unbound.sdk_defaults = Some(SdkDefaults::v1());
    let control = files_map(&generate(plain_document(), &unbound));
    let baseline = files_map(&generate(plain_document(), &GenerationOptions::default()));
    assert_unchanged(
        &control,
        &baseline,
        "no OAuth usable: configured client defaults",
    );
    assert!(!control.contains_key("dart/lib/src/oauth.dart"));
    assert!(!control.contains_key("dart/lib/src/oauth_transport_io.dart"));
    assert!(!control["dart/lib/oauth_sdk.dart"].contains("oauth.dart"));

    // Control B: oauth mode off emits nothing and stays byte-identical to the
    // pre-OAuth generation of the same document.
    let off = files_map(&generate(oauth_document(), &off_options()));
    let baseline_off = files_map(&generate(oauth_document(), &GenerationOptions::default()));
    assert_unchanged(&off, &baseline_off, "oauth off");
    assert!(!off.contains_key("dart/lib/src/oauth.dart"));

    // Control C: a scheme carrying only a deprecated implicit flow emits
    // nothing, mirroring the planner's refusal to execute implicit grants.
    let implicit_only = json!({
        "openapi": "3.2.0",
        "info": {"title": "Implicit", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "security": [{"implicitOAuth": ["read"]}],
                "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {
                    "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}
                }}}}}
            }}
        },
        "components": {"securitySchemes": {
            "implicitOAuth": {"type": "oauth2", "flows": {"implicit": {
                "authorizationUrl": "https://auth.oauth.test/authorize",
                "scopes": {"read": "Read access"}
            }}}
        }}
    });
    let implicit_files = files_map(&generate(implicit_only.clone(), &unbound));
    let implicit_baseline = files_map(&generate(implicit_only, &GenerationOptions::default()));
    assert_unchanged(&implicit_files, &implicit_baseline, "implicit-only scheme");
    assert!(!implicit_files.contains_key("dart/lib/src/oauth.dart"));
}

#[test]
fn plan_carries_the_oauth_plan_only_when_configured() {
    use suspect_codegen::dart_sdk::{self, DartConfig, PackageConfig};
    let contract = contract_with_document(oauth_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = dart_sdk::plan_sdk_with_profiles(
        contract.clone(),
        &selected,
        DartConfig {
            package: PackageConfig {
                name: "oauth_sdk".into(),
                version: "0.1.0".into(),
            },
            sdk_defaults: Some(SdkDefaults {
                oauth: oauth_config(),
                ..SdkDefaults::v1()
            }),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let plan = configured.oauth().expect("configured policy is carried");
    assert_eq!(plan.schemes.len(), 2);
    let service = plan
        .schemes
        .iter()
        .find(|scheme| scheme.name == "serviceOAuth")
        .expect("client-credentials scheme");
    assert_eq!(
        service.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(service.refresh_skew_seconds, 30);
    assert_eq!(
        service.client_id_env.as_deref(),
        Some("SUSPECT_OAUTH_CLIENT_ID")
    );
    let control = dart_sdk::plan_sdk_with_profiles(
        contract,
        &selected,
        DartConfig {
            package: PackageConfig {
                name: "oauth_sdk".into(),
                version: "0.1.0".into(),
            },
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    assert!(control.oauth().is_none());
}

/// The no-policy pin: a configured-but-unusable scheme set, and a generation
/// without any sdk_defaults, must produce identical file lists.
#[test]
fn no_policy_output_stays_byte_identical() {
    let shared = contract_with_document(oauth_document());
    let mut configured = generate(oauth_document(), &GenerationOptions::default());
    let mut baseline = {
        let contract = contract_with_document(oauth_document());
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        generate_with_options(
            contract,
            &selected,
            &TargetConfig {
                backend: Backend::DartHttp,
                package_name: "oauth_sdk".into(),
                package_version: "0.1.0".into(),
                import_name: None,
            },
            &GenerationOptions::default(),
        )
        .unwrap()
    };
    assert_eq!(configured.len(), baseline.len());
    configured.sort_by(|left, right| left.path.cmp(&right.path));
    baseline.sort_by(|left, right| left.path.cmp(&right.path));
    for (configured, baseline) in configured.iter().zip(baseline.iter()) {
        assert_eq!(configured.path, baseline.path);
        assert_eq!(configured.content, baseline.content);
    }
    let _ = shared;
}

/// One OpenID Connect scheme used by an operation — its flows are defined by
/// the discovery document at runtime, so every endpoint the emitted runtime
/// resolves comes from discovery — beside the same client-credentials scheme
/// as the plain document, whose compiled endpoints win over discovery and
/// whose auxiliary endpoints stay configured.
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
                "refreshUrl": "https://auth.oauth.test/token-refresh",
                "scopes": {"read": "Read access"}
            }}}
        }}
    })
}

/// Discovery options: a confidential OpenID Connect client whose endpoints the
/// discovery document defines at runtime, plus the OAuth2 scheme whose
/// compiled endpoints win over discovery.
fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: [
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
                ]
                .into_iter()
                .collect(),
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
    let oauth = source(&files, "dart/lib/src/oauth.dart");
    for expected in [
        // The discovery engine, its typed failure kind, the exact issuer rule
        // and the ~1 MiB ceiling emit exactly when a scheme compiles a
        // discovery URL.
        "'discovery-failed'",
        "final class _DiscoveredEndpoints {",
        "const int _discoveryMaxBytes = 1048576;",
        "enum _DiscoveryMember {",
        "String? _discoveredEndpoint(String scheme, Map<String, dynamic> document, String member) {",
        "_DiscoveredEndpoints _discoveryDocument(CompiledScheme scheme, String url, String body) {",
        "the discovery document issuer does not share the discovery URL origin",
        "Future<_DiscoveredEndpoints> _discover(CompiledScheme scheme, OAuthGetTransport transport, Map<String, Future<_DiscoveredEndpoints>> cache) async {",
        "Future<String> _resolveEndpoint(CompiledScheme scheme, String? compiledEndpoint, _DiscoveryMember member,",
        "(bool, String?, String?) _discoveryAuth(CompiledScheme scheme, (String?, String?) identity)",
        // Single-flight through the in-flight-future map: a failed fetch is
        // never cached, a success is cached for the instance lifetime.
        "final discoveryCache = <String, Future<_DiscoveredEndpoints>>{};",
        "cache.remove(scheme.name);",
        "the discovery document exceeds the compiled response ceiling",
        "utf8.encode(body).length > _discoveryMaxBytes",
        // The compiled precedence: an explicit compiled endpoint always wins.
        "client-credentials flow's token URL always wins",
        "the discovery document's",
        "grant?.refreshUrl ?? grant?.tokenUrl",
        // The discovery-aware providers replace the compiled-only halves and
        // keep the single-flight and atomic-store discipline.
        "CredentialProvider createClientCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) {",
        "CredentialProvider createRefreshProvider(String scheme, {String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) {",
        "Future<TokenSet> clientCredentialsToken(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {",
        "Future<TokenSet> refreshToken(String scheme, {required String refreshToken, String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {",
        "Future<void> revokeToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {",
        "Future<Map<String, dynamic>> introspectToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {",
        "and the discovery document declares none",
        // The discovery-defined client authenticates basic when the compiled
        // configuration carries a client secret variable.
        "(scheme.clientSecretEnv != null, identity.$1, scheme.clientSecretEnv != null ? identity.$2 : null)",
        // The OpenID Connect scheme compiles no executable flows: its descriptor
        // carries the discovery URL that defines the endpoints at runtime.
        "\"identityOAuth\": CompiledScheme(\"identityOAuth\", \"open-id-connect\", 30, \"https://authority.oauth.test/.well-known/openid-configuration\", null, null, \"SUSPECT_DISCOVERY_CLIENT_ID\", \"SUSPECT_DISCOVERY_CLIENT_SECRET\", <CompiledFlow>[",
        // The configured OAuth2 scheme keeps its compiled endpoints (they win
        // over discovery); the engine only fills the gaps.
        "\"serviceOAuth\": CompiledScheme(\"serviceOAuth\", \"oauth2\", 30, null, \"https://auth.oauth.test/revoke\", \"https://auth.oauth.test/introspect\", \"SUSPECT_DISCOVERY_CLIENT_ID\", \"SUSPECT_DISCOVERY_CLIENT_SECRET\", <CompiledFlow>[",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.dart lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // The conditional transport pair gains the document GET exactly when
    // discovery participates.
    let io = source(&files, "dart/lib/src/oauth_transport_io.dart");
    let stub = source(&files, "dart/lib/src/oauth_transport_stub.dart");
    for pair in [&io, &stub] {
        assert!(
            pair.contains("Future<OAuthEndpointResponse> oauthEndpointGet(Uri url, Map<String, String> headers) async {"),
            "the discovery transport pair lacks the document GET: {pair}"
        );
    }
    assert!(io.contains("client.openUrl('GET', url);"));
    assert!(stub.contains("UnsupportedError"));
    // The library import block is unchanged by discovery.
    let library = source(&files, "dart/lib/oauth_sdk.dart");
    assert!(library.contains("import 'src/oauth_transport_stub.dart' if (dart.library.io) 'src/oauth_transport_io.dart' as _oauth_transport;"));

    // Control: without a discovery URL the engine is absent entirely and the
    // plain part keeps its exact pre-discovery shape.
    let plain = generate(oauth_document(), &oauth_options());
    let plain_oauth = source(&plain, "dart/lib/src/oauth.dart");
    for absent in [
        "'discovery-failed'",
        "_discoveryMaxBytes",
        "_DiscoveredEndpoints",
        "_resolveEndpoint",
        "discoveryTransport",
        "OAuthGetTransport",
        "oauthEndpointGet",
    ] {
        assert!(
            !plain_oauth.contains(absent),
            "the discovery-less oauth part must not carry {absent}"
        );
    }
    // The transport pair stays POST-only without discovery.
    let plain_io = source(&plain, "dart/lib/src/oauth_transport_io.dart");
    assert!(!plain_io.contains("oauthEndpointGet"));
    let plain_stub = source(&plain, "dart/lib/src/oauth_transport_stub.dart");
    assert!(!plain_stub.contains("oauthEndpointGet"));
}

#[test]
fn discovery_plan_carries_the_compiled_discovery_urls() {
    use suspect_codegen::dart_sdk::{self, DartConfig, PackageConfig};
    let contract = contract_with_document(discovery_document());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk_with_profiles(
        contract,
        &selected,
        DartConfig {
            package: PackageConfig {
                name: "oauth_sdk".into(),
                version: "0.1.0".into(),
            },
            sdk_defaults: Some(SdkDefaults {
                oauth: discovery_options()
                    .sdk_defaults
                    .expect("discovery defaults")
                    .oauth,
                ..SdkDefaults::v1()
            }),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let oauth_plan = plan.oauth().expect("configured policy is carried");
    let identity = oauth_plan
        .schemes
        .iter()
        .find(|scheme| scheme.name == "identityOAuth")
        .expect("the OpenID Connect scheme");
    assert_eq!(
        identity.kind,
        suspect_codegen::http_protocol::OAuthSchemeKind::OpenIdConnect
    );
    assert_eq!(
        identity.discovery.as_deref(),
        Some("https://authority.oauth.test/.well-known/openid-configuration")
    );
    assert!(identity.flows.is_empty());
}

/// Static verification notes (no Dart toolchain installed): the emitted
/// discovery runtime keeps the OAuth part's invariants.///
/// 1. Discovery fetches through the conditional `_oauth_transport` pair: a
///    VM build uses the default GET transport, a portable build fails until a
///    caller passes `discoveryTransport:`, and the injected transports stay
///    caller-owned exactly like the POST path.
/// 2. The per-instance cache is keyed by scheme name and holds futures:
///    concurrent attaches share the one in-flight fetch through the cached
///    future (single-flight, the existing `_inflight` idiom), successes stay
///    cached for the provider's lifetime and failures remove the entry so the
///    next call retries.
/// 3. `AuthException('discovery-failed', ...)` never carries response body
///    text: failures carry the safe status/metadata label only, and the
///    document decode reports unusable members by name, never by value.
/// 4. The issuer rule: a declared `issuer` claim must share the discovery
///    URL's origin (scheme, host and the port with the scheme default made
///    explicit); a missing claim is tolerated.
/// 5. Precedence: compiled endpoints win, then the cached discovery document,
///    then the typed endpoint-unavailable refusal the compiled plan alone
///    would produce.
#[test]
fn discovery_emission_is_static_and_the_plain_part_is_unchanged() {
    let files = generate(discovery_document(), &discovery_options());
    let oauth = source(&files, "dart/lib/src/oauth.dart");
    // The discovery engine performs no OpenAPI parsing and keeps the frozen
    // descriptor discipline; discovery supplies only omitted endpoints.
    assert!(oauth.contains("The runtime never parses OpenAPI."));
    // The plain (no discovery URL) part keeps its exact pre-discovery bytes:
    // the plain header paragraph and the plain frozen-descriptor sentence.
    let plain = generate(oauth_document(), &oauth_options());
    let plain_oauth = source(&plain, "dart/lib/src/oauth.dart");
    assert!(
        plain_oauth.contains("// OpenAPI and never fetches discovery documents."),
        "the plain header paragraph must keep the pre-discovery bytes"
    );
    assert!(
        plain_oauth.contains(
            "OpenID Connect schemes compile no executable flows in v1 and emit nothing. The runtime never parses OpenAPI and never fetches discovery documents."
        ),
        "the plain descriptor sentence must keep the pre-discovery bytes"
    );
    // And the whole plain generation is deterministic.
    let again = generate(oauth_document(), &oauth_options());
    for (plain, other) in plain.iter().zip(again.iter()) {
        assert_eq!(plain.path, other.path);
        assert_eq!(plain.content, other.content);
    }
}

/// The replay fixture: one client-credentials scheme over a JSON operation and
/// one over a streaming operation, each with its own token endpoint.
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

/// The replaying credential wrapper is compiled only with a non-deprecated
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations in the full
/// document-and-pointer form the attach request resolves by operation prefix.
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(replay_document(), &replay_options());
    let oauth = source(&files, "dart/lib/src/oauth.dart");
    for expected in [
        // The opt-in factory and the wrapper it returns.
        "ReplayingCredentials createReplayingCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport}) {",
        "final class ReplayingCredentials {",
        "HttpTransport replayTransport(HttpTransport inner) => _ReplayTransport(this, inner);",
        "Future<AuthorizationCredential> call(CredentialRequest request) async {",
        "triggers exactly one coordinated refresh",
        "delivered stream data prevents a transparent restart",
        "surfaces as the typed AuthException instead of a replay",
        // The coordinated refresh: one round per store key, a newer stored set
        // wins, a failed round fails every waiter exactly once.
        "final Map<String, Future<AuthorizationCredential>> _rounds = <String, Future<AuthorizationCredential>>{};",
        "if (current.value != presented) {",
        "await _tokenStore.clear(key);",
        // The one replay: a fresh Authorization header through the same inner
        // transport, and the second response surfaced whatever it is.
        "<String, String>{...request.headers, 'authorization': fresh.value},",
        "return _inner.send(replayed);",
        // The compiled stream-protection table, in the form the attach request
        // resolves: the streaming operation's requirement pointer is compiled
        // into the feed scheme's no-replay set.
        "final Map<String, Set<String>> _noReplayRequirements = <String, Set<String>>{",
        "\"https://source.oauth.test/dart-openapi.json#/paths/~1events/get/security/0/feedOAuth\"",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.dart lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(!oauth.contains("~1widgets/get/security/0/serviceOAuth"));

    // The replay machinery stays out of a package without any non-deprecated
    // client-credentials flow: an authorization-code-only scheme compiles
    // exactly the pre-replay bytes, and the plain provider keeps today's
    // attach-only semantics.
    let code_only_document = json!({
        "openapi": "3.2.0",
        "info": {"title": "OAuth code only", "version": "1"},
        "servers": [{"url": "https://api.oauth.test/v1"}],
        "paths": {"/widgets": {"get": {
            "operationId": "listWidgets",
            "security": [{"userOAuth": ["read"]}],
            "responses": {"200": {"description": "Ok", "content": {"application/json": {"schema": {
                "type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}
            }}}}}
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
    let plain_oauth = source(&code_only, "dart/lib/src/oauth.dart");
    assert!(!plain_oauth.contains("ReplayingCredentials"));
    assert!(!plain_oauth.contains("_noReplayRequirements"));
    assert!(!plain_oauth.contains("createReplayingCredentialsProvider"));
    assert!(!plain_oauth.contains("_ReplayTransport"));
}

/// Static verification notes (no Dart toolchain installed): the emitted
/// replay runtime keeps the OAuth part's invariants, scenario by scenario.
///
/// (a) 401 then success: the attach records the served Authorization value;
///     the wrapped transport answers the 401 with exactly one `_refresh`
///     round — the store is cleared and the plain provider re-acquires
///     through the same single-flight store gate — and exactly one replay
///     carrying `fresh.value` through the same inner transport. The replayed
///     response is returned directly, so the caller sees the replay.
/// (b) 401 then 401: the replay's response is returned without a second
///     interception (`return _inner.send(replayed);`), so a second 401
///     surfaces to the caller and the budget stays one refresh plus one
///     replay, never nested.
/// (c) concurrent 401s: `_rounds` shares one store round per stale value —
///     a waiter returns the leader's future (`return pending;`), a newer
///     stored set wins over a stale re-refresh, and a failed round fails
///     every waiter exactly once (the shared future rejects for each
///     awaiter). Dart's single-event-loop execution makes the round
///     check-and-set atomic between awaits.
/// (d) a streaming operation is never replayed: the compiled
///     stream-protected requirement pointer makes the feed attach
///     ineligible, so `_servedEntry` finds nothing, the 401 passes through
///     untouched and no refresh runs. Lifecycle endpoint requests are never
///     replayed either: they carry no bearer token of this provider, and the
///     exact-target guard is defense in depth against loops.
/// (e) replay disabled by default: the plain provider keeps today's
///     attach-only semantics — no served record, no transport wrapper, and
///     the plain generation's bytes are untouched by the opt-in section.
/// (f) refresh failure: `_refreshRound` rethrows the plain provider's typed
///     AuthException instead of replaying; the budget still holds because
///     the replayed request is never sent after a failed refresh.
#[test]
fn replay_lifecycle_is_static_and_the_plain_part_is_unchanged() {
    let files = generate(replay_document(), &replay_options());
    let oauth = source(&files, "dart/lib/src/oauth.dart");
    for (label, expected) in [
        (
            "(a) the wrapped transport intercepts 401 only",
            "if (response.status != 401) {",
        ),
        (
            "(a) exactly one refresh per qualifying 401",
            "final fresh = await _provider._refresh(presented!, attachRequest);",
        ),
        (
            "(a) one replay through the same inner transport",
            "return _inner.send(replayed);",
        ),
        (
            "(c) concurrent 401s share one store round",
            "final pending = _rounds[key];",
        ),
        ("(c) a waiter returns the leader's round", "return pending;"),
        (
            "(c) a newer stored set wins over a stale re-refresh",
            "if (current.value != presented) {",
        ),
        (
            "(d) only a served, eligible attach replays",
            "if (_provider._servedEntry(presented) == null || _provider._lifecycle(request)) {",
        ),
        (
            "(d) the stream-protected prefix resolves the attach",
            "final prefix = '${request.operation.document}#${request.operation.pointer}/security/';",
        ),
        (
            "(f) the fresh header replaces the stale one",
            "'authorization': fresh.value",
        ),
        (
            "(f) the failed round rethrows the typed failure",
            "return (await _plain(request))!;",
        ),
        (
            "lifecycle endpoint requests never replay",
            "bool _lifecycle(TransportRequest request) => _lifecycleUrl != null && request.url.toString() == _lifecycleUrl;",
        ),
        (
            "the served record keeps only safe metadata and stays bounded",
            "if (_served.length > 8) {",
        ),
    ] {
        assert!(
            oauth.contains(expected),
            "{label}: oauth.dart lacks {expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // (e) The plain provider keeps today's attach-only semantics, and the
    // whole plain generation is deterministic.
    let plain = generate(oauth_document(), &oauth_options());
    let plain_oauth = source(&plain, "dart/lib/src/oauth.dart");
    assert!(
        plain_oauth.contains("CredentialProvider createClientCredentialsProvider(String scheme,")
    );
    assert!(plain_oauth.contains("the SDK itself never retries."));
    let again = generate(oauth_document(), &oauth_options());
    for (plain, other) in plain.iter().zip(again.iter()) {
        assert_eq!(plain.path, other.path);
        assert_eq!(plain.content, other.content);
    }
}
