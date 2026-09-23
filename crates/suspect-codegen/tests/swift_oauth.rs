//! Emitted first-party OAuth lifecycle for the Swift package: conditional
//! generation plus native behavior.
//!
//! The configured policy adds exactly one file (`OAuth.swift`) with the
//! compiled scheme descriptors; generation without the policy — or a scheme
//! whose only declared flows are the deprecated implicit and password grants —
//! adds nothing, so no-policy output stays byte-identical. The native test
//! builds the emitted package and drives the lifecycle through a scripted
//! transport, because token requests use the client's own transport.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    swift_sdk,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn service_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth service","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"service":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"service":{
            "type":"oauth2",
            "flows":{
                "clientCredentials":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "refreshUrl":"https://auth.oauth.test/token-refresh",
                    "scopes":{"read":"Read access"}
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
        "openapi":"3.1.0","info":{"title":"OAuth deprecated","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"legacy":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"legacy":{
            "type":"oauth2",
            "flows":{
                "implicit":{
                    "authorizationUrl":"https://auth.oauth.test/implicit-authorize",
                    "scopes":{"read":"Read access"}
                },
                "password":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }
            }
        }}}
    })
}

/// Authorization-code, client-credentials, device and implicit flows on one
/// scheme. The device flow is an OAS 3.2 declaration, so this document is 3.2.
fn interactive_document() -> Value {
    json!({
        "openapi":"3.2.0","info":{"title":"OAuth interactive","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"userAuth":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{"userAuth":{
            "type":"oauth2",
            "flows":{
                "authorizationCode":{
                    "authorizationUrl":"https://auth.oauth.test/authorize",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "refreshUrl":"https://auth.oauth.test/token-refresh",
                    "scopes":{"read":"Read access"}
                },
                "clientCredentials":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                },
                "deviceAuthorization":{
                    "deviceAuthorizationUrl":"https://auth.oauth.test/device",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                },
                "implicit":{
                    "authorizationUrl":"https://auth.oauth.test/implicit-authorize",
                    "scopes":{"read":"Read access"}
                }
            }
        }}}
    })
}

/// Two schemes over one document: an OpenID Connect scheme whose endpoints
/// the discovery document defines at runtime, and an OAuth2 scheme whose
/// compiled client-credentials endpoints coexist with a configured discovery
/// URL and a compiled revocation endpoint.
fn discovery_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth discovery","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"oidc":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }},
            "/metrics":{"get":{
                "operationId":"listMetrics",
                "security":[{"hybrid":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.oauth.test/.well-known/openid-configuration"},
            "hybrid":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/token",
                "refreshUrl":"https://auth.oauth.test/token-refresh",
                "scopes":{"read":"Read access"}
            }}}
        }}
    })
}

fn contract(document: &Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.oauth.test/openapi.json").unwrap();
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

fn generate(document: &Value, options: &GenerationOptions) -> Vec<OutFile> {
    let selected = {
        let contract = contract(document);
        contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>()
    };
    generate_with_options(
        contract(document),
        &selected,
        &TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "OAuthSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"service":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                    "revocation_endpoint":"https://auth.oauth.test/revoke",
                    "introspection_endpoint":"https://auth.oauth.test/introspect"
                }}}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn interactive_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"userAuth":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                    "revocation_endpoint":"https://auth.oauth.test/revoke",
                    "introspection_endpoint":"https://auth.oauth.test/introspect"
                }}}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn deprecated_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"legacy":{"client_id_env":"OAUTH_SDK_CLIENT_ID"}}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The compiled discovery policy: the OpenID Connect scheme carries its
/// declared metadata URL; the OAuth2 scheme gets a configured discovery URL
/// plus a compiled revocation endpoint, so the compiled endpoint must win
/// over the discovered one.
fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "hybrid":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                        "discovery_url":"https://auth.oauth.test/.well-known/oauth-authorization-server",
                        "revocation_endpoint":"https://auth.oauth.test/revoke",
                        "introspection_endpoint":"https://auth.oauth.test/introspect"
                    },
                    "oidc":{"client_id_env":"OAUTH_SDK_CLIENT_ID"}
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

fn oauth_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "swift/Sources/OAuthSDK/OAuth.swift")
        .expect("generated OAuth.swift")
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn configured_policy_emits_only_oauth_swift() {
    let configured = generate(&service_document(), &configured_options());
    let plain = generate(&service_document(), &GenerationOptions::default());
    let mut control = generate(&control_document(), &GenerationOptions::default());
    assert!(
        !plain.iter().any(|file| file.path.ends_with("OAuth.swift")),
        "no-policy output must not carry the OAuth lifecycle runtime"
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
    let oauth = oauth_file(&configured).content.clone();
    for expected in [
        // The compiled descriptor catalog: never parsed, never invented.
        "struct OAuthSchemeDescriptor: Sendable",
        "enum OAuthCatalog",
        "static let schemes: [String: OAuthSchemeDescriptor]",
        "\"service\": OAuthSchemeDescriptor(",
        "clientAuth: \"client-secret-basic\"",
        "clientIDEnv: \"OAUTH_SDK_CLIENT_ID\"",
        "clientSecretEnv: \"OAUTH_SDK_CLIENT_SECRET\"",
        "skew: 30",
        "tokenURL: \"https://auth.oauth.test/token\"",
        "refreshURL: \"https://auth.oauth.test/token-refresh\"",
        "clientCredentialsURL: \"https://auth.oauth.test/token\"",
        // The typed error, the token set and the store protocol.
        "public struct AuthError: Error, Sendable, CustomStringConvertible",
        "public struct TokenSet: Sendable",
        "public var authorization: String",
        "public protocol TokenStore: Sendable",
        "public final class MemoryTokenStore: TokenStore, @unchecked Sendable",
        // Skew-aware acquisition with the store, refresh, and the session.
        "public func clientCredentialsToken(scheme: String, store: any TokenStore",
        "stored.expired(skew: descriptor.skew)",
        "(\"grant_type\", \"client_credentials\")",
        "public func refreshToken(scheme: String, set: TokenSet)",
        "(\"grant_type\", \"refresh_token\"), (\"refresh_token\", refresh)",
        "public actor OAuthSession",
        "public nonisolated func authorizationProvider(_ scheme: String",
        // Conditional sections: the configured endpoints only.
        "public func revokeToken(scheme: String, tokenValue: String)",
        "descriptor.revocationURL",
        "public struct Introspection: Sendable, Equatable",
        "public func introspectToken(scheme: String, tokenValue: String)",
        "descriptor.introspectionURL",
        // Identity comes from the environment at call time, never embedded.
        "ProcessInfo.processInfo.environment[descriptor.clientIDEnv]",
        "ProcessInfo.processInfo.environment[descriptor.clientSecretEnv]",
        // Lifecycle requests ride the client's own transport.
        "try await send(request, source: oauthSource)",
        // Error text never carries secrets: the Basic header is the only place
        // a secret appears, and it never reaches a description.
        "public var description: String",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.swift is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    sorted(&mut control);
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("OAuth.swift")),
        "the no-scheme control must not carry the lifecycle runtime"
    );
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn deprecated_only_schemes_and_unconfigured_policies_emit_nothing() {
    let deprecated = generate(&deprecated_only_document(), &deprecated_options());
    let plain = generate(&deprecated_only_document(), &GenerationOptions::default());
    assert_eq!(
        deprecated.len(),
        plain.len(),
        "schemes with only implicit/password flows must emit nothing"
    );
    for (deprecated, plain) in deprecated.iter().zip(plain.iter()) {
        assert_eq!(deprecated.path, plain.path);
        assert_eq!(deprecated.content, plain.content);
    }
    // A policy whose schemes key binds no used scheme still fails planning
    // with the shared diagnostics, never a silent empty emission.
    let error = generate_with_options(
        contract(&control_document()),
        &contract(&control_document())
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>(),
        &TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "OAuthSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &configured_options(),
    )
    .unwrap_err();
    assert!(
        error
            .iter()
            .any(|item| item.code == "sdk-oauth-config"
                && item.message.contains("binds no used OAuth2")),
        "{error:?}"
    );
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn interactive_flows_compile_pkce_and_device_polling() {
    let files = generate(&interactive_document(), &interactive_options());
    let oauth = oauth_file(&files).content.clone();
    for expected in [
        "authorizationURL: \"https://auth.oauth.test/authorize\"",
        "codeTokenURL: \"https://auth.oauth.test/token\"",
        "deviceURL: \"https://auth.oauth.test/device\"",
        "deviceTokenURL: \"https://auth.oauth.test/token\"",
        // Authorization code with PKCE S256 over the system CSPRNG.
        "public final class AuthorizationTransaction: @unchecked Sendable",
        "public func beginAuthorization(scheme: String, redirectURI: String",
        "(\"code_challenge_method\", \"S256\")",
        "SHA256.hash(data: Data(verifier.utf8))",
        "SymmetricKey(size: .bits256)",
        "public func completeAuthorization(_ transaction: AuthorizationTransaction",
        "(\"code_verifier\", bound.verifier)",
        "throw AuthError(.stateConsumed, scheme: transaction.scheme)",
        "throw AuthError(.stateMismatch, scheme: transaction.scheme)",
        // RFC 8628 device authorization with interval, pending and slow_down.
        "public struct DeviceAuthorization: Sendable",
        "public func beginDeviceAuthorization(scheme: String)",
        "public func token() async throws -> TokenSet",
        "(\"grant_type\", oauthDeviceGrant), (\"device_code\", deviceCode)",
        "case \"authorization_pending\":",
        "case \"slow_down\":",
        "throw AuthError(.expired, scheme: scheme)",
        // The compiled table carries only the executable flows: the declared
        // implicit flow contributes nothing, and the configured supplemental
        // endpoints compile exactly once.
        "clientCredentialsURL: \"https://auth.oauth.test/token\"",
        "revocationURL: \"https://auth.oauth.test/revoke\"",
        "introspectionURL: \"https://auth.oauth.test/introspect\"",
        "Compiled source schemes: userAuth.",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.swift is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn plan_carries_the_oauth_outcome_only_when_usable() {
    let service_contract = contract(&service_document());
    let selected = service_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = swift_sdk::plan_sdk(
        service_contract.clone(),
        &selected,
        swift_sdk::SwiftConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"service":{
                    "client_id_env":"OAUTH_SDK_CLIENT_ID",
                    "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                    "revocation_endpoint":"https://auth.oauth.test/revoke"
                }}}}
                ))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = configured.oauth().expect("configured policy is carried");
    assert_eq!(oauth.schemes.len(), 1);
    let scheme = &oauth.schemes[0];
    assert_eq!(scheme.name, "service");
    assert_eq!(scheme.client_id_env.as_deref(), Some("OAUTH_SDK_CLIENT_ID"));
    assert_eq!(
        scheme.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(scheme.introspection_endpoint, None);
    assert_eq!(scheme.refresh_skew_seconds, 30);
    let control = swift_sdk::plan_sdk(
        service_contract,
        &selected,
        swift_sdk::SwiftConfig::default(),
    )
    .unwrap();
    assert!(control.oauth().is_none());
    // The deprecated-only scheme compiles but yields no emission.
    let deprecated_contract = contract(&deprecated_only_document());
    let deprecated_selected = deprecated_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let deprecated = swift_sdk::plan_sdk(
        deprecated_contract,
        &deprecated_selected,
        swift_sdk::SwiftConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version":"v1",
                    "oauth":{"schemes":{"legacy":{"client_id_env":"OAUTH_SDK_CLIENT_ID"}}}
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        deprecated.oauth().is_none(),
        "deprecated-only schemes must not carry or emit the lifecycle runtime"
    );
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn discovery_emits_only_when_a_scheme_compiles_a_discovery_url() {
    let discovered = generate(&discovery_document(), &discovery_options());
    let oauth = oauth_file(&discovered).content.clone();
    for expected in [
        // The compiled descriptor catalog carries each scheme's discovery URL.
        "discoveryURL: \"https://auth.oauth.test/.well-known/openid-configuration\"",
        "discoveryURL: \"https://auth.oauth.test/.well-known/oauth-authorization-server\"",
        // The discovery-aware failure kind and the discovery engine.
        "case discoveryFailed = \"discovery-failed\"",
        "struct DiscoveredEndpoints: Sendable",
        "static func endpointOrigin(_ value: String) -> String?",
        "static func discoveredEndpoint(_ object: JsonObject<JsonValue>, _ member: String,",
        "static func discoveryDocument(_ body: Data, scheme: String, url: String) throws -> DiscoveredEndpoints",
        "func discoveryDocument(_ descriptor: OAuthSchemeDescriptor) async throws -> DiscoveredEndpoints",
        "func resolveEndpoint(_ descriptor: OAuthSchemeDescriptor, compiled: String,",
        "member: \\.revocationEndpoint",
        "member: \\.introspectionEndpoint",
        "let endpoint = discovered.tokenEndpoint",
        // The session owns the per-scheme cache and single-flight.
        "private var discovered: [String: DiscoveredEndpoints]",
        "private var discoveryFlights: [String: Task<DiscoveredEndpoints, any Error>]",
        // A discovery-defined scheme acquires from the discovered endpoint.
        "guard !descriptor.clientCredentialsURL.isEmpty || !descriptor.discoveryURL.isEmpty else {",
        // The issuer-origin rule is compiled and never carries body text.
        "equals the discovery URL's origin",
        "Compiled source schemes: hybrid, oidc.",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.swift is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // Without a discovery URL the plain lifecycle is emitted: no discovery
    // machinery at all.
    let plain = generate(&interactive_document(), &interactive_options());
    let oauth = oauth_file(&plain).content.clone();
    for absent in [
        "discoveryFailed",
        "DiscoveredEndpoints",
        "discoveryDocument",
        "endpointOrigin",
        "discoveryFlights",
        "resolveEndpoint",
        "discoveryURL.isEmpty",
    ] {
        assert!(
            !oauth.contains(absent),
            "the plain lifecycle must not carry {absent}\n{oauth}"
        );
    }
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn plan_carries_the_compiled_discovery_urls() {
    let discovery_contract = contract(&discovery_document());
    let selected = discovery_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = swift_sdk::plan_sdk(
        discovery_contract.clone(),
        &selected,
        swift_sdk::SwiftConfig {
            sdk_defaults: Some(
                serde_json::from_value(json!({
                    "version": "v1",
                    "oauth": {
                        "schemes": {
                            "hybrid": {
                                "client_id_env": "OAUTH_SDK_CLIENT_ID",
                                "client_secret_env": "OAUTH_SDK_CLIENT_SECRET",
                                "discovery_url": "https://auth.oauth.test/.well-known/oauth-authorization-server",
                                "revocation_endpoint": "https://auth.oauth.test/revoke"
                            },
                            "oidc": {"client_id_env": "OAUTH_SDK_CLIENT_ID"}
                        }
                    }
                }))
                .unwrap(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let oauth = configured.oauth().expect("discovery policy is carried");
    assert_eq!(oauth.schemes.len(), 2);
    let hybrid = oauth.schemes.iter().find(|s| s.name == "hybrid").unwrap();
    assert_eq!(
        hybrid.discovery.as_deref(),
        Some("https://auth.oauth.test/.well-known/oauth-authorization-server")
    );
    assert_eq!(
        hybrid.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    let oidc = oauth.schemes.iter().find(|s| s.name == "oidc").unwrap();
    assert!(oidc.flows.is_empty(), "OpenID Connect compiles no flows");
    assert_eq!(
        oidc.discovery.as_deref(),
        Some("https://auth.oauth.test/.well-known/openid-configuration")
    );
    // Without a configured policy nothing is carried, as before.
    let plain = swift_sdk::plan_sdk(
        discovery_contract,
        &selected,
        swift_sdk::SwiftConfig::default(),
    )
    .unwrap();
    assert!(plain.oauth().is_none());
}

const BEHAVIOR: &str = r##"import Foundation
import XCTest
import OAuthSDK

/// Scripted transport: records requests and replays responses matched by URL
/// prefix. Token requests ride the same transport as operations.
actor ScriptedTransport: HTTPTransport {
    private var responses: [(String, Int, String)]
    private var requests: [(url: String, authorization: String?, body: Data)] = []

    init(responses: [(String, Int, String)]) {
        self.responses = responses
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let url = request.url.absoluteString
        requests.append((url, request.headers.first { $0.name.lowercased() == "authorization" }?.value, request.body ?? Data()))
        // A short pause makes single-flight races observable.
        try await Task.sleep(nanoseconds: 20_000_000)
        guard let index = responses.firstIndex(where: { url.hasPrefix($0.0) }) else {
            return HTTPResponse(status: 404, headers: [], body: Data())
        }
        let match = responses.remove(at: index)
        return HTTPResponse(
            status: match.1,
            headers: [HTTPHeader("Content-Type", "application/json")],
            body: Data(match.2.utf8)
        )
    }

    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        throw TransportError.exactMethodUnavailable
    }

    func recorded() -> [(url: String, authorization: String?, body: Data)] {
        requests
    }
}

final class OAuthLifecycleTests: XCTestCase {
    private func client(_ transport: ScriptedTransport) -> Client {
        Client(transport: transport)
    }

    private func tokenBody(_ access: String, expiresIn: Int? = 3600) -> String {
        var fields = "\"access_token\":\"\(access)\",\"token_type\":\"Bearer\""
        if let expiresIn { fields += ",\"expires_in\":\(expiresIn)" }
        return "{\(fields)}"
    }

    func testAcquireThenCacheHitIssuesOneRequest() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("t1")),
        ])
        let session = OAuthSession(client: client(transport))
        let first = try await session.clientCredentialsToken(scheme: "userAuth",
            clientID: "consumer-client", clientSecret: "consumer-secret")
        XCTAssertEqual(first.accessToken, "t1")
        XCTAssertEqual(first.authorization, "Bearer t1")
        let second = try await session.clientCredentialsToken(scheme: "userAuth",
            clientID: "consumer-client", clientSecret: "consumer-secret")
        XCTAssertEqual(second.accessToken, "t1")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1, "the cached set must be reused: \(requests.map(\.url))")
        let header = try XCTUnwrap(requests[0].authorization)
        XCTAssertTrue(header.hasPrefix("Basic "), header)
        XCTAssertEqual(
            String(decoding: Data(base64Encoded: String(header.dropFirst(6))) ?? Data(), as: UTF8.self),
            "consumer-client:consumer-secret")
        let form = String(decoding: requests[0].body, as: UTF8.self)
        XCTAssertEqual(form, "grant_type=client_credentials")
    }

    func testConcurrentCallersShareOneAcquisition() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("t1")),
        ])
        let session = OAuthSession(client: client(transport))
        let results = try await withThrowingTaskGroup(of: TokenSet.self) { group in
            for _ in 0..<5 {
                group.addTask {
                    try await session.clientCredentialsToken(scheme: "userAuth",
                        clientID: "consumer-client", clientSecret: "consumer-secret")
                }
            }
            var sets: [TokenSet] = []
            for try await set in group { sets.append(set) }
            return sets
        }
        XCTAssertEqual(results.count, 5)
        XCTAssertTrue(results.allSatisfy { $0.accessToken == "t1" })
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1, "single-flight must share one acquisition")
    }

    func testExpiryBeyondSkewReacquires() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("t1", expiresIn: 60)),
            ("https://auth.oauth.test/token", 200, tokenBody("t2")),
        ])
        // The client's compiled skew is 30s; a set expiring in 60s stays fresh.
        let session = OAuthSession(client: client(transport))
        _ = try await session.clientCredentialsToken(scheme: "userAuth",
            clientID: "c", clientSecret: "s")
        _ = try await session.clientCredentialsToken(scheme: "userAuth",
            clientID: "c", clientSecret: "s")
        let cached = await transport.recorded()
        XCTAssertEqual(cached.count, 1)
        // A distinct client identity partitions the store and acquires again.
        _ = try await session.clientCredentialsToken(scheme: "userAuth",
            clientID: "other", clientSecret: "s")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2)
    }

    func testRefreshAdoptsRotatedTokens() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token-refresh", 200, "{\"access_token\":\"a2\",\"token_type\":\"bearer\"}"),
            ("https://auth.oauth.test/token-refresh", 200, "{\"access_token\":\"a3\",\"refresh_token\":\"r3\"}"),
        ])
        let client = client(transport)
        let set = TokenSet(accessToken: "a1", refreshToken: "r1")
        let rotated = try await client.refreshToken(scheme: "userAuth", set: set)
        XCTAssertEqual(rotated.accessToken, "a2")
        XCTAssertTrue(rotated.hasRefresh, "a missing rotated token retains the previous one")
        XCTAssertEqual(rotated.refreshToken, "r1")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests[0].url, "https://auth.oauth.test/token-refresh")
        let form = String(decoding: requests[0].body, as: UTF8.self)
        XCTAssertEqual(form, "grant_type=refresh_token&refresh_token=r1")

        let adopted = try await client.refreshToken(scheme: "userAuth", set: rotated)
        XCTAssertEqual(adopted.accessToken, "a3")
        XCTAssertEqual(adopted.refreshToken, "r3")
        XCTAssertTrue(adopted.hasRefresh)
    }

    func testRefreshWithoutTokenOrEndpointFailsTyped() async throws {
        let transport = ScriptedTransport(responses: [])
        let client = client(transport)
        do {
            _ = try await client.refreshToken(scheme: "userAuth", set: TokenSet(accessToken: "a1"))
            XCTFail("a set without a refresh token must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .requestValidation)
            XCTAssertEqual(error.scheme, "userAuth")
        }
        // The compiled refresh endpoint is unreachable here: the scripted
        // transport answers 404 with a non-JSON body.
        do {
            _ = try await client.refreshToken(scheme: "userAuth", set: TokenSet(accessToken: "a1", refreshToken: "r1"))
            XCTFail("an unreachable refresh endpoint must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .invalidResponse)
            XCTAssertEqual(error.status, 404)
        }
        do {
            _ = try await client.refreshToken(scheme: "missing", set: TokenSet(accessToken: "a1", refreshToken: "r1"))
            XCTFail("unknown schemes must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .unknownScheme)
            XCTAssertEqual(error.scheme, "missing")
        }
    }

    func testProviderAttachesToOperations() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("t1")),
            ("https://api.oauth.test/v1/widgets", 200, "\"ok\""),
        ])
        let session = OAuthSession(client: Client(transport: transport))
        let client = Client(
            credentials: Credentials(userAuth: session.authorizationProvider("userAuth",
                clientID: "c", clientSecret: "s")),
            transport: transport)
        let response = try await client.listWidgets()
        XCTAssertEqual(response.data, "ok")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2)
        XCTAssertEqual(requests[0].url, "https://auth.oauth.test/token")
        XCTAssertNotNil(requests[0].authorization, "the token request carries client auth")
        XCTAssertEqual(requests[1].authorization, "Bearer t1")
    }

    func testEnvironmentIdentityIsReadAtCallTime() async throws {
        setenv("OAUTH_SDK_CLIENT_ID", "env-client", 1)
        setenv("OAUTH_SDK_CLIENT_SECRET", "env-secret", 1)
        defer {
            unsetenv("OAUTH_SDK_CLIENT_ID")
            unsetenv("OAUTH_SDK_CLIENT_SECRET")
        }
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("t1")),
        ])
        let session = OAuthSession(client: client(transport))
        _ = try await session.clientCredentialsToken(scheme: "userAuth")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1)
        let header = try XCTUnwrap(requests[0].authorization)
        XCTAssertEqual(
            String(decoding: Data(base64Encoded: String(header.dropFirst(6))) ?? Data(), as: UTF8.self),
            "env-client:env-secret",
            "the compiled variable names are read at call time")
    }

    func testServerErrorsNeverCarrySecrets() async throws {
        let secret = "super-secret-value-9f3a"
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 400, "{\"error\":\"invalid_client\",\"error_description\":\"bad secret \(secret)\"}"),
        ])
        let session = OAuthSession(client: client(transport))
        do {
            _ = try await session.clientCredentialsToken(scheme: "userAuth",
                clientID: "c", clientSecret: secret)
            XCTFail("a server-declared error must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .authorizationError)
            XCTAssertEqual(error.code, "invalid_client")
            let text = String(describing: error)
            XCTAssertFalse(text.contains(secret), "error text leaked a secret: \(text)")
        }
        // A non-2xx without a declared error is a typed server failure.
        let failing = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 503, "boom"),
        ])
        let other = OAuthSession(client: client(failing))
        do {
            _ = try await other.clientCredentialsToken(scheme: "userAuth",
                clientID: "c", clientSecret: secret)
            XCTFail("a server failure must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .invalidResponse)
            XCTAssertEqual(error.status, 503)
            XCTAssertFalse(String(describing: error).contains(secret))
        }
    }

    func testAuthorizationCodeTransactionBindsAndConsumesOnce() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/token", 200, tokenBody("code-token")),
            ("https://auth.oauth.test/token", 200, tokenBody("code-token")),
        ])
        let client = client(transport)
        let transaction = try client.beginAuthorization(scheme: "userAuth",
            redirectURI: "https://callback.test/done", clientID: "c")
        let url = transaction.authorizationURL
        XCTAssertTrue(url.hasPrefix("https://auth.oauth.test/authorize?"), url)
        XCTAssertTrue(url.contains("response_type=code"), url)
        XCTAssertTrue(url.contains("client_id=c"), url)
        XCTAssertTrue(url.contains("redirect_uri=https%3A%2F%2Fcallback.test%2Fdone"), url)
        XCTAssertTrue(url.contains("code_challenge_method=S256"), url)
        XCTAssertTrue(url.contains("code_challenge="), url)
        let state = transaction.state
        XCTAssertFalse(state.isEmpty)
        do {
            _ = try await client.completeAuthorization(transaction,
                callbackParameters: ["state": "wrong", "code": "abc"])
            XCTFail("a state mismatch must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .stateMismatch)
        }
        do {
            _ = try await client.completeAuthorization(transaction,
                callbackParameters: ["state": state, "code": "abc"])
            XCTFail("the mismatch already consumed the transaction")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .stateConsumed)
        }
        let second = try client.beginAuthorization(scheme: "userAuth",
            redirectURI: "https://callback.test/done", clientID: "c")
        let set = try await client.completeAuthorization(second,
            callbackParameters: ["state": second.state, "code": "abc"])
        XCTAssertEqual(set.accessToken, "code-token")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1)
        let form = String(decoding: requests[0].body, as: UTF8.self)
        XCTAssertTrue(form.contains("grant_type=authorization_code"), form)
        XCTAssertTrue(form.contains("code=abc"), form)
        XCTAssertTrue(form.contains("code_verifier="), form)
        XCTAssertTrue(form.contains("redirect_uri=https%3A%2F%2Fcallback.test%2Fdone"), form)
    }

    func testRevocationAndIntrospectionPostTheTokenValue() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/revoke", 200, "{}"),
            ("https://auth.oauth.test/introspect", 200, "{\"active\":true,\"scope\":\"read\",\"sub\":\"user-1\",\"exp\":4102444800,\"aud\":[\"api\"],\"iss\":\"https://auth.oauth.test\"}"),
        ])
        let client = client(transport)
        try await client.revokeToken(scheme: "userAuth", tokenValue: "rt-1")
        let introspection = try await client.introspectToken(scheme: "userAuth", tokenValue: "rt-1")
        XCTAssertTrue(introspection.active)
        XCTAssertEqual(introspection.scope, "read")
        XCTAssertEqual(introspection.subject, "user-1")
        XCTAssertEqual(introspection.audience, ["api"])
        XCTAssertEqual(introspection.issuer, "https://auth.oauth.test")
        XCTAssertNotNil(introspection.expiresAt)
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2)
        XCTAssertEqual(requests[0].url, "https://auth.oauth.test/revoke")
        XCTAssertEqual(String(decoding: requests[0].body, as: UTF8.self), "token=rt-1")
        XCTAssertEqual(requests[1].url, "https://auth.oauth.test/introspect")
        XCTAssertEqual(String(decoding: requests[1].body, as: UTF8.self), "token=rt-1")
    }

    func testDeviceAuthorizationPollsUntilGranted() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/device", 200, "{\"device_code\":\"dc1\",\"user_code\":\"ABCD-EFGH\",\"verification_uri\":\"https://auth.oauth.test/activate\",\"interval\":1,\"expires_in\":120}"),
            ("https://auth.oauth.test/token", 400, "{\"error\":\"authorization_pending\"}"),
            ("https://auth.oauth.test/token", 400, "{\"error\":\"slow_down\"}"),
            ("https://auth.oauth.test/token", 200, tokenBody("device-token")),
        ])
        let client = client(transport)
        let device = try await client.beginDeviceAuthorization(scheme: "userAuth", clientID: "c")
        XCTAssertEqual(device.userCode, "ABCD-EFGH")
        XCTAssertEqual(device.verificationURI, "https://auth.oauth.test/activate")
        XCTAssertEqual(device.interval, 1)
        let set = try await device.token()
        XCTAssertEqual(set.accessToken, "device-token")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 4, "\(requests.map(\.url))")
        XCTAssertTrue(requests[0].url.hasPrefix("https://auth.oauth.test/device"))
        for poll in requests.dropFirst() {
            XCTAssertTrue(poll.url.hasPrefix("https://auth.oauth.test/token"), poll.url)
            let form = String(decoding: poll.body, as: UTF8.self)
            XCTAssertTrue(form.contains("grant_type="), form)
            XCTAssertTrue(form.contains("device_code=dc1"), form)
        }
    }
}
"##;

/// The replay fixture: one client-credentials scheme over a JSON operation
/// and one over a streaming operation, each with its own token endpoint.
fn replay_document() -> Value {
    json!({
        "openapi":"3.2.0",
        "info":{"title":"OAuth replay","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "security":[{"serviceOAuth":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }},
            "/events":{"get":{
                "operationId":"streamEvents",
                "security":[{"feedOAuth":["read"]}],
                "responses":{"200":{"description":"Events","content":{"text/event-stream":{
                    "itemSchema":{"type":"object","properties":{"data":{"type":"string"}}}
                }}}}
            }}
        },
        "components":{"securitySchemes":{
            "serviceOAuth":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/token",
                "scopes":{"read":"Read access"}
            }}},
            "feedOAuth":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/feed-token",
                "scopes":{"read":"Read access"}
            }}}
        }}
    })
}

fn replay_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "serviceOAuth":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET"
                    },
                    "feedOAuth":{
                        "client_id_env":"OAUTH_SDK_FEED_ID",
                        "client_secret_env":"OAUTH_SDK_FEED_SECRET"
                    }
                }}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// An authorization-code-only scheme: the control for the replay emission
/// gate, which must compile exactly the pre-replay bytes.
fn code_only_document() -> Value {
    json!({
        "openapi":"3.2.0",
        "info":{"title":"OAuth code only","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "paths":{"/widgets":{"get":{
            "operationId":"listWidgets",
            "security":[{"userOAuth":["read"]}],
            "responses":{"200":{"description":"Ok"}}
        }}},
        "components":{"securitySchemes":{"userOAuth":{"type":"oauth2","flows":{"authorizationCode":{
            "authorizationUrl":"https://auth.oauth.test/authorize",
            "tokenUrl":"https://auth.oauth.test/token",
            "scopes":{"read":"Read access"}
        }}}}}
    })
}

fn code_only_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{"userOAuth":{
                    "client_id_env":"OAUTH_SDK_CODE_ID",
                    "client_secret_env":"OAUTH_SDK_CODE_SECRET"
                }}}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// The replaying credential wrapper is compiled only with an executable
/// client-credentials flow, wraps exactly that provider, and compiles the
/// stream-protection pointers of its scheme's operations.
#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(&replay_document(), &replay_options());
    let oauth = oauth_file(&files).content.clone();
    for expected in [
        "enum OAuthReplayProtection",
        "\"feedOAuth\": [\"/paths/~1events/get/security/0/feedOAuth\"],",
        // The plain JSON operation's requirement is absent: its attaches replay.
        "public final class OAuthReplayCredentials: @unchecked Sendable",
        "public nonisolated func replayingAuthorizationProvider(_ scheme: String",
        "public var provider: HTTPAuthorizationProvider",
        "public func transport(_ inner: any HTTPTransport) -> any HTTPTransport",
        "triggers exactly one coordinated refresh",
        "delivered stream data prevents a transparent restart",
        "a newer stored set wins over a stale",
        "served.count > 8",
        "func replayRefreshRound(scheme: String, key: String, presented: String",
    ] {
        assert!(
            oauth.contains(expected),
            "OAuth.swift is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    assert!(
        !oauth.contains("~1widgets/get/security/0/serviceOAuth"),
        "the JSON operation's requirement must stay replayable\n{oauth}"
    );

    // A package without any executable client-credentials flow compiles
    // exactly the pre-replay bytes: no wrapper, no protection table.
    let code_only = generate(&code_only_document(), &code_only_options());
    let plain_oauth = oauth_file(&code_only).content.clone();
    assert!(!plain_oauth.contains("OAuthReplayCredentials"));
    assert!(!plain_oauth.contains("OAuthReplayProtection"));
    assert!(!plain_oauth.contains("replayingAuthorizationProvider"));
}

const REPLAY_BEHAVIOR: &str = r##"import Foundation
import XCTest
import OAuthSDK

/// Scripted API and token endpoints. The API answers 401 for the armed stale
/// token and 200 afterwards, so the driver counts exactly one refresh and one
/// replay per scenario; `mode`, `failRefresh` and `armStale` flip per
/// scenario. Token requests ride the same transport as operations.
actor ReplayScriptedTransport: HTTPTransport {
    private var mode = "ok"
    private var failFrom = Int.max
    private var staleNext = false
    private var staleToken: String?
    private var serviceTokens = 0
    private var feedTokens = 0
    private var log: [(url: String, authorization: String?)] = []

    func setMode(_ value: String) { mode = value }
    func armStale() { staleNext = true }
    /// The token request after the next one is refused, so the attach
    /// succeeds and the coordinated refresh fails.
    func armRefreshFailure() { failFrom = serviceTokens + 2 }

    func hits(_ prefix: String) -> Int { log.filter { $0.url.hasPrefix(prefix) }.count }
    func authorizations(_ prefix: String) -> [String?] {
        log.filter { $0.url.hasPrefix(prefix) }.map(\.authorization)
    }
    func reset() { log.removeAll() }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let url = request.url.absoluteString
        let authorization = request.headers.first { $0.name.lowercased() == "authorization" }?.value
        log.append((url, authorization))
        // A short pause makes single-flight races observable.
        try await Task.sleep(nanoseconds: 20_000_000)
        func json(_ body: String, _ status: Int) -> HTTPResponse {
            HTTPResponse(status: status,
                         headers: [HTTPHeader("Content-Type", "application/json")],
                         body: Data(body.utf8))
        }
        if url == "https://auth.oauth.test/token" {
            serviceTokens += 1
            if serviceTokens >= failFrom { return json(#"{"error":"server_error"}"#, 500) }
            let token = "svc-\(serviceTokens)"
            if staleNext { staleToken = token; staleNext = false }
            return json("{\"access_token\":\"\(token)\",\"token_type\":\"Bearer\",\"expires_in\":3600}", 200)
        }
        if url == "https://auth.oauth.test/feed-token" {
            feedTokens += 1
            return json("{\"access_token\":\"feed-\(feedTokens)\",\"token_type\":\"Bearer\",\"expires_in\":3600}", 200)
        }
        if url == "https://api.oauth.test/v1/widgets" {
            if mode == "always-401" { return json(#"{"error":"unauthorized"}"#, 401) }
            if let staleToken, authorization == "Bearer \(staleToken)" {
                return json(#"{"error":"stale"}"#, 401)
            }
            return json("\"ok\"", 200)
        }
        if url == "https://api.oauth.test/v1/events" { return json(#"{"error":"stream-denied"}"#, 401) }
        return HTTPResponse(status: 404, headers: [], body: Data())
    }

    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        let url = request.url.absoluteString
        log.append((url, request.headers.first { $0.name.lowercased() == "authorization" }?.value))
        guard url == "https://api.oauth.test/v1/events" else {
            throw TransportError.exactMethodUnavailable
        }
        return HTTPStreamResponse(
            status: 401,
            headers: [HTTPHeader("Content-Type", "application/json")],
            body: HTTPByteStream(next: { nil }, cancel: {}))
    }
}

final class ReplayTests: XCTestCase {
    /// (a)-(f) in one sequence: each scenario arms exactly the state it
    /// needs, and the shared provider of (a)/(b) shows the budget on a warm
    /// store exactly like the reference drivers.
    func testReplayLifecycleBudgetsAndProtections() async throws {
        let transport = ReplayScriptedTransport()

        // (a) 401 then success: one refresh, one replay, and the caller sees
        // 200 with the fresh token.
        await transport.armStale()
        let session = OAuthSession(client: Client(transport: transport))
        let replay = try session.replayingAuthorizationProvider("serviceOAuth",
                                                                clientID: "c", clientSecret: "s")
        let client = Client(credentials: Credentials(serviceOAuth: replay.provider),
                            transport: replay.transport(transport))
        let ok = try await client.listWidgets()
        XCTAssertEqual(ok.data, "ok")
        let okWidgets = await transport.hits("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(okWidgets, 2, "exactly one replay")
        let okTokens = await transport.hits("https://auth.oauth.test/token")
        XCTAssertEqual(okTokens, 2, "exactly one refresh")
        let okFeedTokens = await transport.hits("https://auth.oauth.test/feed-token")
        XCTAssertEqual(okFeedTokens, 0, "no cross-scheme refresh")
        let widgetValues = await transport.authorizations("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(widgetValues[0], "Bearer svc-1")
        XCTAssertEqual(widgetValues[1], "Bearer svc-2", "the replay carried the fresh token")

        // (b) 401 then 401: the second 401 surfaces and exactly one refresh
        // ran; the stored set from (a) serves the attach.
        await transport.setMode("always-401")
        await transport.reset()
        do {
            _ = try await client.listWidgets()
            XCTFail("the second 401 must surface")
        } catch let error as SDKError {
            XCTAssertEqual(error.status, 401)
        }
        let loopWidgets = await transport.hits("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(loopWidgets, 2, "one replay, no loops")
        let loopTokens = await transport.hits("https://auth.oauth.test/token")
        XCTAssertEqual(loopTokens, 1, "exactly one refresh")

        // (c) concurrent 401s: ONE refresh, two replays.
        await transport.setMode("ok")
        await transport.reset()
        await transport.armStale()
        let sharedSession = OAuthSession(client: Client(transport: transport))
        let shared = try sharedSession.replayingAuthorizationProvider("serviceOAuth",
                                                                      clientID: "c", clientSecret: "s")
        let sharedClient = Client(credentials: Credentials(serviceOAuth: shared.provider),
                                  transport: shared.transport(transport))
        async let firstResult = sharedClient.listWidgets()
        async let secondResult = sharedClient.listWidgets()
        _ = try await firstResult
        _ = try await secondResult
        let sharedWidgets = await transport.hits("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(sharedWidgets, 4, "two replays")
        let sharedTokens = await transport.hits("https://auth.oauth.test/token")
        XCTAssertEqual(sharedTokens, 2, "one shared refresh")

        // (d) a streaming operation is never replayed: the typed 401
        // surfaces and the feed token is never refreshed.
        await transport.reset()
        let feedSession = OAuthSession(client: Client(transport: transport))
        let feed = try feedSession.replayingAuthorizationProvider("feedOAuth",
                                                                  clientID: "f", clientSecret: "s")
        let feedClient = Client(credentials: Credentials(feedOAuth: feed.provider),
                                transport: feed.transport(transport))
        do {
            _ = try await feedClient.streamEvents()
            XCTFail("a stream-protected 401 must surface without a replay")
        } catch let error as SDKError {
            XCTAssertEqual(error.status, 401)
        }
        let streamEventsHits = await transport.hits("https://api.oauth.test/v1/events")
        XCTAssertEqual(streamEventsHits, 1, "no replay for the streaming operation")
        let feedTokenHits = await transport.hits("https://auth.oauth.test/feed-token")
        XCTAssertEqual(feedTokenHits, 1, "no refresh for the streaming operation")

        // (e) replay disabled by default: the plain provider surfaces the 401
        // without any refresh.
        await transport.reset()
        await transport.armStale()
        let plainSession = OAuthSession(client: Client(transport: transport))
        let plainClient = Client(
            credentials: Credentials(serviceOAuth: plainSession.authorizationProvider("serviceOAuth",
                                                                                      clientID: "c", clientSecret: "s")),
            transport: transport)
        do {
            _ = try await plainClient.listWidgets()
            XCTFail("the plain provider must surface the 401")
        } catch let error as SDKError {
            XCTAssertEqual(error.status, 401)
        }
        let plainWidgets = await transport.hits("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(plainWidgets, 1, "no replay")
        let plainTokens = await transport.hits("https://auth.oauth.test/token")
        XCTAssertEqual(plainTokens, 1, "no refresh")

        // (f) refresh failure: the typed auth failure surfaces instead of a
        // replay.
        await transport.reset()
        await transport.armStale()
        await transport.armRefreshFailure()
        let failingSession = OAuthSession(client: Client(transport: transport))
        let failing = try failingSession.replayingAuthorizationProvider("serviceOAuth",
                                                                        clientID: "c", clientSecret: "s")
        let failingClient = Client(credentials: Credentials(serviceOAuth: failing.provider),
                                   transport: failing.transport(transport))
        do {
            _ = try await failingClient.listWidgets()
            XCTFail("a failed refresh must surface instead of a replay")
        } catch let error as SDKError {
            XCTAssertEqual(error.kind, .transport, "the operation maps the typed auth failure onto the transport kind")
        }
        let failedWidgets = await transport.hits("https://api.oauth.test/v1/widgets")
        XCTAssertEqual(failedWidgets, 1, "no replay after a failed refresh")
        let failedTokens = await transport.hits("https://auth.oauth.test/token")
        XCTAssertEqual(failedTokens, 2, "the refresh was attempted exactly once")
    }
}
"##;

fn swiftc() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_SWIFTC_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let found = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .ok()?;
    if !found.status.success() {
        return None;
    }
    let text = String::from_utf8(found.stdout).ok()?;
    let path = std::path::PathBuf::from(text.trim());
    Command::new(&path).arg("--version").output().ok()?;
    Some(path)
}

fn swift() -> std::path::PathBuf {
    // Manifest resolution (`SUSPECT_SWIFT_BIN`, then `PATH`), with the
    // historical /usr/bin fallback preserved.
    suspect_codegen::toolchain::resolve_path("swift")
        .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin/swift"))
}

fn swift_command(root: &Path, action: &str) -> Command {
    let mut command = Command::new(swift());
    command.arg(action);
    if action == "test" {
        command.arg("--disable-swift-testing");
    }
    if let Some(sdkroot) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("--sdk").arg(&sdkroot).env("SDKROOT", sdkroot);
    }
    command.env("SWIFT_EXEC", swiftc().unwrap_or_else(|| "swiftc".into()));
    command.current_dir(root);
    command
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn native_lifecycle_counts_requests_and_protects_secrets() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_oauth: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_oauth: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    // The interactive package compiles every executable flow plus the
    // configured revocation and introspection endpoints, so one scripted
    // consumer covers the whole lifecycle.
    let files = generate(&interactive_document(), &interactive_options());
    suspect_codegen::write_files(&files, &root.path().join("sdk")).unwrap();

    // The generated package must build cleanly with warnings as errors.
    let build = swift_command(&root.path().join("sdk/swift"), "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/sdk"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "generated package build failed\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Behavioral consumer: a scripted transport drives acquire → cache-hit,
    // single-flight, skew-aware reacquisition, refresh rotation, the
    // credential-attach integration, revocation, introspection, PKCE
    // transactions and device polling.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/OAuthConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"OAuthConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"OAuthConsumer\", dependencies: [.product(name: \"OAuthSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/OAuthConsumer/OAuthTests.swift"),
        BEHAVIOR,
    )
    .unwrap();
    let test = swift_command(&consumer, "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "native OAuth behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_oauth: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}

const DISCOVERY_BEHAVIOR: &str = r##"import Foundation
import XCTest
import OAuthSDK

/// Scripted transport: records requests and replays responses matched by URL
/// prefix. Discovery and token requests ride the same transport as operations.
actor ScriptedTransport: HTTPTransport {
    private var responses: [(String, Int, String)]
    private var requests: [(url: String, method: String, accept: String?, body: Data)] = []

    init(responses: [(String, Int, String)]) {
        self.responses = responses
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let url = request.url.absoluteString
        requests.append((
            url,
            request.method,
            request.headers.first { $0.name.lowercased() == "accept" }?.value,
            request.body ?? Data()))
        // A short pause makes single-flight races observable.
        try await Task.sleep(nanoseconds: 20_000_000)
        guard let index = responses.firstIndex(where: { url.hasPrefix($0.0) }) else {
            return HTTPResponse(status: 404, headers: [], body: Data())
        }
        let match = responses.remove(at: index)
        return HTTPResponse(
            status: match.1,
            headers: [HTTPHeader("Content-Type", "application/json")],
            body: Data(match.2.utf8)
        )
    }

    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        throw TransportError.exactMethodUnavailable
    }

    func recorded() -> [(url: String, method: String, accept: String?, body: Data)] {
        requests
    }
}

final class DiscoveryTests: XCTestCase {
    private func client(_ transport: ScriptedTransport) -> Client {
        Client(transport: transport)
    }

    private let oidcDiscovery = #"{"issuer":"https://auth.oauth.test","token_endpoint":"https://auth.oauth.test/token","revocation_endpoint":"https://auth.oauth.test/revoke-discovered","introspection_endpoint":"https://auth.oauth.test/introspect-discovered"}"#

    private func tokenBody(_ access: String) -> String {
        "{\"access_token\":\"\(access)\",\"token_type\":\"Bearer\",\"expires_in\":3600}"
    }

    private func discoveryURLs(
        _ requests: [(url: String, method: String, accept: String?, body: Data)]
    ) -> String {
        requests.map(\.url).joined(separator: ", ")
    }

    /// A discovery-defined scheme acquires from the DISCOVERED token endpoint,
    /// with the public client identity in the form.
    func testAcquisitionUsesTheDiscoveredTokenEndpoint() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/token", 200, tokenBody("d1")),
        ])
        let session = OAuthSession(client: client(transport))
        let set = try await session.clientCredentialsToken(scheme: "oidc", clientID: "mobile-client")
        XCTAssertEqual(set.accessToken, "d1")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2, discoveryURLs(requests))
        XCTAssertEqual(requests[0].url, "https://auth.oauth.test/.well-known/openid-configuration")
        XCTAssertEqual(requests[0].method, "GET")
        XCTAssertEqual(requests[0].accept, "application/json")
        XCTAssertEqual(requests[1].url, "https://auth.oauth.test/token")
        XCTAssertEqual(
            String(decoding: requests[1].body, as: UTF8.self),
            "grant_type=client_credentials&client_id=mobile-client")
    }

    /// The discovery document is cached per session and the acquisition itself
    /// is single-flighted: concurrent callers share one fetch and one request.
    func testDiscoveryIsCachedAndSingleFlight() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/token", 200, tokenBody("d1")),
        ])
        let session = OAuthSession(client: client(transport))
        let results = try await withThrowingTaskGroup(of: TokenSet.self) { group in
            for _ in 0..<5 {
                group.addTask {
                    try await session.clientCredentialsToken(scheme: "oidc", clientID: "c")
                }
            }
            var sets: [TokenSet] = []
            for try await set in group { sets.append(set) }
            return sets
        }
        XCTAssertEqual(results.count, 5)
        XCTAssertTrue(results.allSatisfy { $0.accessToken == "d1" })
        let requests = await transport.recorded()
        XCTAssertEqual(
            requests.count, 2,
            "one discovery fetch and one token request: \(discoveryURLs(requests))")
    }

    /// An issuer that does not share the discovery URL's origin is a typed
    /// discovery failure that never carries response body text.
    func testIssuerMismatchFailsTyped() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200,
             #"{"issuer":"https://evil.test","token_endpoint":"https://evil.test/token"}"#),
        ])
        let session = OAuthSession(client: client(transport))
        do {
            _ = try await session.clientCredentialsToken(scheme: "oidc", clientID: "c")
            XCTFail("an issuer mismatch must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .discoveryFailed)
            XCTAssertEqual(error.scheme, "oidc")
            let text = String(describing: error)
            XCTAssertFalse(text.contains("evil"), "the failure carried body text: \(text)")
        }
        // A missing issuer claim is tolerated.
        let tolerated = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200,
             #"{"token_endpoint":"https://auth.oauth.test/token"}"#),
            ("https://auth.oauth.test/token", 200, tokenBody("d2")),
        ])
        let other = OAuthSession(client: client(tolerated))
        let set = try await other.clientCredentialsToken(scheme: "oidc", clientID: "c")
        XCTAssertEqual(set.accessToken, "d2")
    }

    /// An unusable discovered endpoint value is a typed discovery failure.
    func testUnusableDiscoveredEndpointsFailTyped() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200,
             #"{"issuer":"https://auth.oauth.test","token_endpoint":""}"#),
        ])
        let session = OAuthSession(client: client(transport))
        do {
            _ = try await session.clientCredentialsToken(scheme: "oidc", clientID: "c")
            XCTFail("an unusable endpoint value must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .discoveryFailed)
        }
        let malformed = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, "not json"),
        ])
        let other = OAuthSession(client: client(malformed))
        do {
            _ = try await other.clientCredentialsToken(scheme: "oidc", clientID: "c")
            XCTFail("an unreadable document must fail")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .discoveryFailed)
        }
    }

    /// A failed discovery fetch is a typed failure that is never cached: the
    /// next call retries and succeeds.
    func testFetchFailureIsTypedAndRetried() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 503, "boom"),
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/token", 200, tokenBody("d2")),
        ])
        let session = OAuthSession(client: client(transport))
        do {
            _ = try await session.clientCredentialsToken(scheme: "oidc", clientID: "c")
            XCTFail("a failed discovery fetch must fail typed")
        } catch let error as AuthError {
            XCTAssertEqual(error.kind, .discoveryFailed)
            XCTAssertEqual(error.status, 503)
        }
        let set = try await session.clientCredentialsToken(scheme: "oidc", clientID: "c")
        XCTAssertEqual(set.accessToken, "d2", "a failed fetch is retried on the next call")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 3, discoveryURLs(requests))
    }

    /// A compiled endpoint always wins over the discovery document: no
    /// discovery fetch happens at all.
    func testCompiledEndpointsWinOverDiscovery() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/revoke", 200, "{}"),
        ])
        let client = client(transport)
        try await client.revokeToken(scheme: "hybrid", tokenValue: "rt-1")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 1, discoveryURLs(requests))
        XCTAssertEqual(requests[0].url, "https://auth.oauth.test/revoke")
    }

    /// Missing compiled endpoints fall back to the discovery document, which
    /// the session caches for its lifetime.
    func testDiscoveredEndpointsServeRevokeAndIntrospect() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/revoke-discovered", 200, "{}"),
            ("https://auth.oauth.test/introspect-discovered", 200, #"{"active":true,"sub":"user-1"}"#),
            ("https://auth.oauth.test/introspect-discovered", 200, #"{"active":true,"sub":"user-1"}"#),
        ])
        let session = OAuthSession(client: client(transport))
        try await session.revokeToken(scheme: "oidc", tokenValue: "rt-9")
        let info = try await session.introspectToken(scheme: "oidc", tokenValue: "rt-9")
        XCTAssertTrue(info.active)
        XCTAssertEqual(info.subject, "user-1")
        var requests = await transport.recorded()
        XCTAssertEqual(requests.count, 3, discoveryURLs(requests))
        XCTAssertEqual(requests[1].url, "https://auth.oauth.test/revoke-discovered")
        XCTAssertEqual(requests[2].url, "https://auth.oauth.test/introspect-discovered")
        // Repeated calls reuse the cached document: only the endpoint call is
        // issued again.
        _ = try await session.introspectToken(scheme: "oidc", tokenValue: "rt-9")
        requests = await transport.recorded()
        XCTAssertEqual(requests.count, 4, discoveryURLs(requests))
        XCTAssertEqual(requests[3].url, "https://auth.oauth.test/introspect-discovered")
    }

    /// One-shot helpers fetch discovery per call and keep no cache.
    func testOneShotHelpersFetchDiscoveryPerCall() async throws {
        let transport = ScriptedTransport(responses: [
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/revoke-discovered", 200, "{}"),
            ("https://auth.oauth.test/.well-known/openid-configuration", 200, oidcDiscovery),
            ("https://auth.oauth.test/revoke-discovered", 200, "{}"),
        ])
        let client = client(transport)
        try await client.revokeToken(scheme: "oidc", tokenValue: "rt-1")
        try await client.revokeToken(scheme: "oidc", tokenValue: "rt-2")
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 4, discoveryURLs(requests))
    }
}
"##;

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn native_discovery_resolves_caches_and_validates_documents() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_oauth discovery: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_oauth discovery: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    // The discovery package compiles the discovery-aware lifecycle for both
    // schemes: the discovery-defined OpenID Connect scheme and the hybrid
    // scheme whose compiled endpoints coexist with discovery.
    let files = generate(&discovery_document(), &discovery_options());
    suspect_codegen::write_files(&files, &root.path().join("sdk")).unwrap();

    // The generated package must build cleanly with warnings as errors.
    let build = swift_command(&root.path().join("sdk/swift"), "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/sdk"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "discovery package build failed\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Behavioral consumer: a scripted transport serves discovery documents
    // and token/revocation/introspection endpoints.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/DiscoveryConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"DiscoveryConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"DiscoveryConsumer\", dependencies: [.product(name: \"OAuthSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/DiscoveryConsumer/DiscoveryTests.swift"),
        DISCOVERY_BEHAVIOR,
    )
    .unwrap();
    let test = swift_command(&consumer, "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "native discovery behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_oauth discovery: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn native_replay_lifecycle_budgets_one_refresh_and_one_replay() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_oauth replay: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_oauth replay: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    // The replay package compiles two client-credentials schemes: the JSON
    // operation's scheme stays replayable and the streaming operation's
    // scheme compiles its stream-protection pointer.
    let files = generate(&replay_document(), &replay_options());
    suspect_codegen::write_files(&files, &root.path().join("sdk")).unwrap();

    // The generated package must build cleanly with warnings as errors.
    let build = swift_command(&root.path().join("sdk/swift"), "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/sdk"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "replay package build failed\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Behavioral consumer: the scripted transport drives scenarios (a)-(f)
    // through the generated operation runtime.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/ReplayConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"ReplayConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"ReplayConsumer\", dependencies: [.product(name: \"OAuthSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/ReplayConsumer/ReplayTests.swift"),
        REPLAY_BEHAVIOR,
    )
    .unwrap();
    let test = swift_command(&consumer, "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "native replay behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_oauth replay: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}
