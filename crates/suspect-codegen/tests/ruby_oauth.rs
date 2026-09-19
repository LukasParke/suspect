//! Emitted first-party OAuth lifecycle for the Ruby gem: emission shape,
//! plan carriage, syntax validity and native behavioral verification of the
//! generated `oauth.rb` over a scripted transport. Static runtime files are
//! never modified; the lifecycle lives entirely in the generated module.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    ruby_sdk,
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
            backend: Backend::RubyHttp,
            package_name: "oauth-sdk".into(),
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

/// One OpenID Connect scheme (its endpoints the discovery document defines at
/// runtime) beside an OAuth2 scheme with configured auxiliary endpoints.
fn discovery_document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"OAuth discovery","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"identityOAuth":[]}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }},
            "/gadgets":{"get":{
                "operationId":"listGadgets",
                "security":[{"service":["read"]}],
                "responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"string"}}}}}
            }}
        },
        "components":{"securitySchemes":{
            "identityOAuth":{"type":"openIdConnect","openIdConnectUrl":"https://authority.oauth.test/.well-known/openid-configuration"},
            "service":{"type":"oauth2","flows":{"clientCredentials":{
                "tokenUrl":"https://auth.oauth.test/token",
                "scopes":{"read":"Read access"}
            }}}
        }}
    })
}

fn discovery_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "oauth":{"schemes":{
                    "identityOAuth":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET"
                    },
                    "service":{
                        "client_id_env":"OAUTH_SDK_CLIENT_ID",
                        "client_secret_env":"OAUTH_SDK_CLIENT_SECRET",
                        "revocation_endpoint":"https://auth.oauth.test/revoke",
                        "introspection_endpoint":"https://auth.oauth.test/introspect"
                    }
                }}}
            ))
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn content<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn configured_policy_emits_only_the_oauth_module_and_entry_require() {
    let configured = generate(&service_document(), &configured_options());
    let plain = generate(&service_document(), &GenerationOptions::default());
    assert!(
        !plain.iter().any(|file| file.path.ends_with("oauth.rb")),
        "no-policy output must not carry the OAuth lifecycle module"
    );
    let mut changed = Vec::new();
    for file in &plain {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if emitted.content != file.content {
            changed.push(file.path.clone());
        }
    }
    changed.sort();
    assert_eq!(
        changed,
        vec![
            "ruby/lib/oauth_sdk.rb".to_owned(),
            "ruby/sig/oauth_sdk.rbs".to_owned()
        ],
        "only the generated entry and its signatures may change; the module itself is new"
    );
    assert_eq!(configured.len(), plain.len() + 1);
    let oauth = content(&configured, "ruby/lib/oauth_sdk/oauth.rb");
    for expected in [
        // The compiled descriptor table: never parsed, never invented.
        "SCHEMES = {",
        "\"service\" => {",
        "client_auth: \"client-secret-basic\"",
        "client_id_env: \"OAUTH_SDK_CLIENT_ID\"",
        "client_secret_env: \"OAUTH_SDK_CLIENT_SECRET\"",
        "skew: 30",
        "token_url: \"https://auth.oauth.test/token\"",
        "refresh_url: \"https://auth.oauth.test/token-refresh\"",
        "client_credentials: \"https://auth.oauth.test/token\"",
        "revocation: \"https://auth.oauth.test/revoke\"",
        "introspection: \"https://auth.oauth.test/introspect\"",
        // The typed error, the frozen token set and the store.
        "class AuthError < SdkError",
        "class TokenSet",
        "def authorization_credential",
        "module TokenStore",
        "class MemoryTokenStore",
        "@lock = ::Mutex.new",
        // Acquisition with the store, the provider, and explicit refresh.
        "def client_credentials_token(scheme, store:",
        "class ClientCredentialProvider",
        "def acquire",
        "def client_credential(scheme,",
        "def refresh_token_set(scheme, token,",
        "['grant_type', 'refresh_token'], ['refresh_token', token.refresh_token]",
        // Conditional sections for the configured endpoints only.
        "def revoke(scheme, token_value,",
        "def introspect(scheme, token_value,",
        // Identity comes from the environment at call time, never embedded.
        "::ENV[variable]",
        // Lifecycle requests ride the caller's transport.
        "transport.exchange(request: request, context: context)",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.rb is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The entry requires the module exactly when the file is emitted, after
    // the runtime it integrates with.
    let entry = content(&configured, "ruby/lib/oauth_sdk.rb");
    assert!(
        entry.contains("require_relative \"oauth_sdk/http\""),
        "{entry}"
    );
    assert!(
        entry.contains("require_relative \"oauth_sdk/oauth\""),
        "{entry}"
    );
    assert!(
        entry.find("require_relative \"oauth_sdk/oauth\"").unwrap()
            > entry.find("require_relative \"oauth_sdk/http\"").unwrap(),
        "oauth.rb must be required after http.rb"
    );
    // RBS signatures cover the emitted module.
    let signatures = content(&configured, "sig/oauth_sdk.rbs");
    for expected in [
        "module OAuth",
        "class AuthError < SdkError",
        "class TokenSet",
        "class MemoryTokenStore",
        "include TokenStore",
        "class ClientCredentialProvider",
        "def self.client_credential:",
        "def self.refresh_token_set:",
        "def self.revoke:",
        "def self.introspect:",
    ] {
        assert!(
            signatures.contains(expected),
            "signatures are missing:\n{expected}\n--- emitted: ---\n{signatures}"
        );
    }
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn interactive_flows_compile_pkce_and_device_polling() {
    let files = generate(&interactive_document(), &interactive_options());
    let oauth = content(&files, "ruby/lib/oauth_sdk/oauth.rb");
    for expected in [
        "authorization_url: \"https://auth.oauth.test/authorize\"",
        "code_token_url: \"https://auth.oauth.test/token\"",
        "device_url: \"https://auth.oauth.test/device\"",
        "device_token_url: \"https://auth.oauth.test/token\"",
        "client_credentials: \"https://auth.oauth.test/token\"",
        // Authorization code with PKCE S256 over the system CSPRNG.
        "class AuthorizationTransaction",
        "def begin_authorization(scheme, redirect_uri:",
        "['code_challenge_method', 'S256']",
        "::SecureRandom.urlsafe_base64(48, padding: false)",
        "::OpenSSL::Digest::SHA256.digest(verifier)",
        "def complete_authorization(transaction, callback_params,",
        "['code_verifier', bound[0]]",
        "auth_kind: 'transaction-used'",
        "auth_kind: 'state-mismatch'",
        // RFC 8628 device authorization with interval, pending and slow_down.
        "class DeviceAuthorization",
        "def begin_device_authorization(scheme,",
        "def poll_device_authorization(device,",
        "['grant_type', DEVICE_GRANT], ['device_code', device.device_code]",
        "when 'authorization_pending' then next",
        "when 'slow_down' then interval += 5",
        "auth_kind: 'device-flow-expired'",
        // The compiled table carries only executable flows.
        "Compiled source schemes: userAuth.",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.rb is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn discovery_schemes_emit_the_discovery_engine_and_byte_identical_plain_output() {
    let files = generate(&discovery_document(), &discovery_options());
    let oauth = content(&files, "ruby/lib/oauth_sdk/oauth.rb");
    for expected in [
        // The compiled descriptor carries the discovery URL; the scheme with
        // no discovery URL carries none.
        "discovery: \"https://authority.oauth.test/.well-known/openid-configuration\"",
        // The discovery engine: one GET through the caller's transport, the
        // typed decode with the documented issuer rule, the ~1 MiB bound, and
        // the endpoint-resolution precedence.
        "DISCOVERY_MAX_BYTES = 1 << 20",
        "def discovery_request(transport:, scheme:, url:)",
        "def discovery_document(scheme, url, body, status)",
        "def discovered_endpoint(scheme, payload, member)",
        "def discovery_payload(scheme, descriptor, transport)",
        "def resolve_endpoint(scheme, descriptor, compiled, member, transport)",
        "def discovery_client_auth(descriptor, client_id: nil, client_secret: nil)",
        "auth_kind: 'discovery-failed'",
        "sharing the discovery URL's origin",
        // The provider cache: per instance, keyed by scheme, Mutex
        // single-flight, failed fetches retried on the next call.
        "@discovery_lock = ::Mutex.new",
        "def discovered",
        "@discovered[@scheme] = fetched",
        // The discovery-aware provider resolves the token endpoint through the
        // compiled precedence.
        "def token_endpoint",
        "def refresh_endpoint",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.rb is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }

    // The plain fixture (no discovery URL anywhere) emits the pre-discovery
    // bytes exactly: no engine, no discovery-aware sections, no failures.
    let plain = generate(&service_document(), &configured_options());
    let plain_oauth = content(&plain, "ruby/lib/oauth_sdk/oauth.rb");
    assert!(!plain_oauth.contains("'discovery-failed'"));
    assert!(!plain_oauth.contains("def discovery_document("));
    assert!(!plain_oauth.contains("@discovery_lock"));
    assert!(!plain_oauth.contains("discovery: \""));
    assert!(!plain_oauth.contains("DISCOVERY_MAX_BYTES"));
}

#[ignore = "requires the Ruby interpreter on the test host"]
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
    let selected = {
        let contract = contract(&control_document());
        contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>()
    };
    let error = generate_with_options(
        contract(&control_document()),
        &selected,
        &TargetConfig {
            backend: Backend::RubyHttp,
            package_name: "oauth-sdk".into(),
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

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn plan_carries_the_oauth_outcome_only_when_usable() {
    let service_contract = contract(&service_document());
    let selected = service_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = ruby_sdk::plan_sdk(
        service_contract.clone(),
        &selected,
        ruby_sdk::RubyConfig {
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
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
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
    let control = ruby_sdk::plan_sdk(service_contract, &selected, ruby_sdk::RubyConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(control.oauth().is_none());
    // The deprecated-only scheme compiles but yields no emission.
    let deprecated_contract = contract(&deprecated_only_document());
    let deprecated_selected = deprecated_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let deprecated = ruby_sdk::plan_sdk(
        deprecated_contract,
        &deprecated_selected,
        ruby_sdk::RubyConfig {
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
    .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(
        deprecated.oauth().is_none(),
        "deprecated-only schemes must not carry or emit the lifecycle module"
    );
}

const BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'oauth_sdk'

def form_of(body)
  URI.decode_www_form(body || '').to_h
end

# Records requests and replays responses matched by URL prefix. Token requests
# ride the same transport as operations.
class ScriptedTransport
  attr_reader :requests

  def initialize(responses)
    @responses = responses
    @requests = []
    @lock = Mutex.new
  end

  def exchange(request:, context:)
    context.check!
    @lock.synchronize do
      @requests << [request.url, request.headers['Authorization'], request.body]
      sleep(0.02)
      match_index = @responses.index { |entry| request.url.start_with?(entry[0]) }
      raise "unexpected request #{request.url}" unless match_index
      _prefix, status, body = @responses.delete_at(match_index)
      yield OauthSdk::WireResponse.new(
        status: status,
        headers: { 'Content-Type' => 'application/json' },
        body: body
      )
    end
  end
end

# acquire → cache-hit: the stored set is reused; the Basic header carries the
# RFC 6749 2.3.1 form-encoded identity and only grant_type travels.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 't1', 'token_type' => 'Bearer', 'expires_in' => 3600 }.to_json],
])
provider = OauthSdk::OAuth.client_credential('userAuth', client_id: 'consumer-client',
                                             client_secret: 'consumer-secret', transport: transport)
first = provider.acquire
raise 'acquire changed' unless first.access_token == 't1' && first.token_type == 'Bearer'
remaining = first.expires_at - ::Process.clock_gettime(::Process::CLOCK_REALTIME)
raise 'the compiled skew was not applied' if first.expires_at.nil? || remaining > 3600 - 30 || remaining <= 3600 - 90
second = provider.acquire
raise 'the cached set must be reused' unless second.access_token == 't1'
raise 'cache hit issued a request' unless transport.requests.length == 1
url, authorization, body = transport.requests[0]
raise 'token request URL changed' unless url == 'https://auth.oauth.test/token'
raise 'Basic auth missing' unless authorization.to_s.start_with?('Basic ')
require 'base64'
raise 'Basic identity changed' unless Base64.decode64(authorization.split(' ', 2)[1]) == 'consumer-client:consumer-secret'
raise 'grant form changed' unless form_of(body) == { 'grant_type' => 'client_credentials' }
credential = provider.call(nil)
raise 'attach path minted the wrong credential' unless credential.is_a?(OauthSdk::AuthorizationCredential) && credential.scheme == 'Bearer' && credential.token == 't1'

# Concurrent attaches are single-flighted per provider: one token request.
slow = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 'shared', 'expires_in' => 3600 }.to_json],
])
shared = OauthSdk::OAuth.client_credential('userAuth', client_id: 'c', client_secret: 's', transport: slow)
results = Array.new(8) { Thread.new { shared.acquire } }.map(&:value)
raise 'single-flight lost results' unless results.all? { |set| set.access_token == 'shared' }
raise 'single-flight issued multiple requests' unless slow.requests.length == 1

# A distinct client identity partitions the store and acquires again.
partitioned = OauthSdk::OAuth.client_credential('userAuth', client_id: 'other', client_secret: 's',
                                                transport: ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 't2', 'expires_in' => 3600 }.to_json],
]))
other = partitioned.acquire
raise 'store partition leaked' unless other.access_token == 't2'

# The compiled variable names are read at call time.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 'env', 'expires_in' => 3600 }.to_json],
])
ENV['OAUTH_SDK_CLIENT_ID'] = 'env-client'
ENV['OAUTH_SDK_CLIENT_SECRET'] = 'env-secret'
begin
  environment = OauthSdk::OAuth.client_credential('userAuth', transport: transport)
  raise 'environment identity changed' unless environment.acquire.access_token == 'env'
  _, authorization, = transport.requests[0]
  raise 'environment Basic changed' unless Base64.decode64(authorization.split(' ', 2)[1]) == 'env-client:env-secret'
ensure
  ENV.delete('OAUTH_SDK_CLIENT_ID')
  ENV.delete('OAUTH_SDK_CLIENT_SECRET')
end

# Explicit refresh adopts rotated tokens and retains the current one otherwise.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token-refresh', 200, { 'access_token' => 'a2', 'token_type' => 'bearer' }.to_json],
  ['https://auth.oauth.test/token-refresh', 200, { 'access_token' => 'a3', 'refresh_token' => 'r3' }.to_json],
])
client = OauthSdk::Client.new(transport: transport)
rotated = OauthSdk::OAuth.refresh_token_set('userAuth', OauthSdk::OAuth::TokenSet.new(access_token: 'a1', refresh_token: 'r1'), transport: transport)
raise 'refresh changed the access token' unless rotated.access_token == 'a2'
raise 'a missing rotated token must retain the previous one' if rotated.refresh_token != 'r1' || !rotated.refreshable?
url, _, body = transport.requests[0]
raise 'refresh endpoint changed' unless url == 'https://auth.oauth.test/token-refresh'
raise 'refresh form changed' unless form_of(body) == { 'grant_type' => 'refresh_token', 'refresh_token' => 'r1' }
adopted = OauthSdk::OAuth.refresh_token_set('userAuth', rotated, transport: transport)
raise 'rotation was not adopted' unless adopted.access_token == 'a3' && adopted.refresh_token == 'r3' && adopted.refreshable?

# Refresh without a refresh token, and unknown schemes, fail typed.
begin
  OauthSdk::OAuth.refresh_token_set('userAuth', OauthSdk::OAuth::TokenSet.new(access_token: 'a1'))
  raise 'a set without a refresh token must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong refresh failure' unless error.auth_kind == 'no-refresh-token'
end
begin
  OauthSdk::OAuth.refresh_token_set('missing', OauthSdk::OAuth::TokenSet.new(access_token: 'a1', refresh_token: 'r1'))
  raise 'unknown schemes must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong scheme failure' unless error.auth_kind == 'unknown-scheme' && error.auth_scheme == 'missing'
end

# The provider integrates with the generated client's credential attach path.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 't1', 'expires_in' => 3600 }.to_json],
  ['https://api.oauth.test/v1/widgets', 200, '"ok"'],
])
OauthSdk::Client.open(credential_provider: OauthSdk::OAuth.client_credential('userAuth',
                         client_id: 'c', client_secret: 's', transport: transport),
                      transport: transport) do |attached|
  response = attached.list_widgets
  raise 'attach integration changed' unless response.data == 'ok'
end
raise 'the attach path skipped the token request' unless transport.requests.length == 2
raise 'the operation skipped the bearer attachment' unless transport.requests[1][1] == 'Bearer t1'

# Server-declared errors and failures never carry secrets.
secret = 'super-secret-value-9f3a'
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 400, { 'error' => 'invalid_client', 'error_description' => "bad secret #{secret}" }.to_json],
])
failing = OauthSdk::OAuth.client_credential('userAuth', client_id: 'c', client_secret: secret, transport: transport)
begin
  failing.acquire
  raise 'a server-declared error must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong authorization failure' unless error.auth_kind == 'authorization-error' && error.server_code == 'invalid_client'
  raise 'error inspect leaked a secret' if error.inspect.include?(secret) || error.message.include?(secret)
end

# Revocation and introspection post the token value; any 2xx revocation wins.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/revoke', 200, '{}'],
  ['https://auth.oauth.test/introspect', 200, { 'active' => true, 'scope' => 'read', 'sub' => 'user-1', 'exp' => 4_102_444_800, 'aud' => ['api'], 'iss' => 'https://auth.oauth.test' }.to_json],
])
OauthSdk::OAuth.revoke('userAuth', 'rt-1', client_id: 'c', client_secret: 's', transport: transport)
introspection = OauthSdk::OAuth.introspect('userAuth', 'rt-1', client_id: 'c', client_secret: 's', transport: transport)
raise 'introspection changed' unless introspection['active'] == true && introspection['sub'] == 'user-1'
raise 'introspection body changed' unless form_of(transport.requests[1][2]) == { 'token' => 'rt-1' }

# Authorization code + PKCE S256: URL binding, state checks, one-time use.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/token', 200, { 'access_token' => 'code-token', 'expires_in' => 3600 }.to_json],
])
client = OauthSdk::Client.new(transport: transport)
transaction = OauthSdk::OAuth.begin_authorization('userAuth', redirect_uri: 'https://callback.test/done', client_id: 'c')
url = transaction.authorization_url
raise 'authorization URL changed' unless url.start_with?('https://auth.oauth.test/authorize?')
query = URI.decode_www_form(url.split('?', 2)[1]).to_h
raise 'response_type changed' unless query['response_type'] == 'code'
raise 'client id changed' unless query['client_id'] == 'c'
raise 'redirect uri changed' unless query['redirect_uri'] == 'https://callback.test/done'
raise 'challenge method changed' unless query['code_challenge_method'] == 'S256'
raise 'challenge missing' unless query['code_challenge'] && !query['code_challenge'].empty?
raise 'state missing' unless query['state'] == transaction.state
begin
  OauthSdk::OAuth.complete_authorization(transaction, { 'state' => 'wrong', 'code' => 'abc' })
  raise 'a state mismatch must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong state failure' unless error.auth_kind == 'state-mismatch'
end
# A failed validation does not consume the transaction: the correct callback
# still completes it, exactly once.
set = OauthSdk::OAuth.complete_authorization(transaction, { 'state' => transaction.state, 'code' => 'abc' },
                                             transport: transport)
raise 'code exchange changed' unless set.access_token == 'code-token'
begin
  OauthSdk::OAuth.complete_authorization(transaction, { 'state' => transaction.state, 'code' => 'abc' })
  raise 'the completion must consume the transaction'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong consumption failure' unless error.auth_kind == 'transaction-used'
end
url, _, body = transport.requests[0]
raise 'code token endpoint changed' unless url == 'https://auth.oauth.test/token'
form = form_of(body)
raise 'code form changed' unless form['grant_type'] == 'authorization_code' && form['code'] == 'abc'
raise 'the verifier was not retained' unless form['code_verifier'] && !form['code_verifier'].empty?
raise 'redirect uri lost' unless form['redirect_uri'] == 'https://callback.test/done'

# Device authorization polls through pending and slow_down until granted.
transport = ScriptedTransport.new([
  ['https://auth.oauth.test/device', 200, { 'device_code' => 'dc1', 'user_code' => 'ABCD-EFGH', 'verification_uri' => 'https://auth.oauth.test/activate', 'interval' => 1, 'expires_in' => 120 }.to_json],
  ['https://auth.oauth.test/token', 400, { 'error' => 'authorization_pending' }.to_json],
  ['https://auth.oauth.test/token', 400, { 'error' => 'slow_down' }.to_json],
  ['https://auth.oauth.test/token', 200, { 'access_token' => 'device-token', 'expires_in' => 3600 }.to_json],
])
client = OauthSdk::Client.new(transport: transport)
device = OauthSdk::OAuth.begin_device_authorization('userAuth', client_id: 'c', transport: transport)
raise 'device grant changed' unless device.user_code == 'ABCD-EFGH' && device.verification_uri == 'https://auth.oauth.test/activate' && device.interval == 1
waits = []
set = OauthSdk::OAuth.poll_device_authorization(device, client_id: 'c', client_secret: 's',
                                                transport: transport, wait: ->(seconds) { waits << seconds })
raise 'device token changed' unless set.access_token == 'device-token'
raise 'device polling changed' unless transport.requests.length == 4
raise 'slow_down did not extend the interval' unless waits == [1, 1, 6]
device_form = form_of(transport.requests[1][2])
raise 'device grant form changed' unless device_form['grant_type'] == 'urn:ietf:params:oauth:grant-type:device_code' && device_form['device_code'] == 'dc1'

puts 'oauth behavior verified'
"#;

const DISCOVERY_BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'oauth_sdk'

DISCOVERY_DOCUMENT = {
  'issuer' => 'https://authority.oauth.test',
  'token_endpoint' => 'https://authority.oauth.test/oauth/token',
  'revocation_endpoint' => 'https://authority.oauth.test/oauth/revoke',
  'introspection_endpoint' => 'https://authority.oauth.test/oauth/introspect',
  'unknown_member' => { 'nested' => true }
}.freeze

# The fake OpenID Connect provider: discovery, token, revoke, introspect and
# the plain API endpoint, all over one recorded transport.
class ScriptedTransport
  attr_reader :requests
  attr_accessor :discovery_status, :discovery_document

  def initialize
    @requests = []
    @lock = Mutex.new
    @discovery_status = 200
    @discovery_document = DISCOVERY_DOCUMENT
    @counter = 0
  end

  def exchange(request:, context:)
    context.check!
    @lock.synchronize do
      @requests << [request.method, request.url, request.headers['Authorization'],
                    request.headers['Accept'], request.body]
      url = request.url
      response = if url == 'https://authority.oauth.test/.well-known/openid-configuration'
        [@discovery_status, @discovery_document.to_json]
      elsif url == 'https://authority.oauth.test/oauth/token'
        @counter += 1
        [200, { 'access_token' => "discovered-#{@counter}", 'token_type' => 'Bearer', 'expires_in' => 3600 }.to_json]
      elsif url == 'https://authority.oauth.test/oauth/revoke'
        [200, '{}']
      elsif url == 'https://authority.oauth.test/oauth/introspect'
        [200, { 'active' => true, 'scope' => 'read' }.to_json]
      elsif url == 'https://api.oauth.test/v1/widgets'
        [200, '"ok"']
      elsif url == 'https://auth.oauth.test/revoke'
        [200, '{}']
      elsif url == 'https://auth.oauth.test/introspect'
        [200, { 'active' => true, 'scope' => 'read' }.to_json]
      elsif url == 'https://auth.oauth.test/token'
        [200, { 'access_token' => 'compiled', 'token_type' => 'Bearer', 'expires_in' => 3600 }.to_json]
      else
        raise "unexpected request #{url}"
      end
      yield OauthSdk::WireResponse.new(
        status: response[0],
        headers: { 'Content-Type' => 'application/json' },
        body: response[1]
      )
    end
  end

  def hits(path, method: nil)
    @requests.select { |entry| entry[1].include?(path) && (method.nil? || entry[0] == method) }
  end
end

def form_of(body)
  URI.decode_www_form(body || '').to_h
end

# The discovered token endpoint serves the acquisition; the discovery document
# is fetched once per provider and the token set is cached beside it.
transport = ScriptedTransport.new
provider = OauthSdk::OAuth.client_credential('identityOAuth', client_id: 'disc-id',
                                             client_secret: 'disc-secret', transport: transport)
set = provider.acquire
raise 'discovered token changed' unless set.access_token == 'discovered-1'
raise 'discovery method changed' unless transport.hits('.well-known/openid-configuration', method: 'GET').length == 1
discovery_hit = transport.hits('.well-known/openid-configuration').first
raise 'discovery accept changed' unless discovery_hit[3] == 'application/json'
token_hit = transport.hits('/oauth/token', method: 'POST').first
raise 'token endpoint changed' unless token_hit
raise 'token form changed' unless form_of(token_hit[4]) == { 'grant_type' => 'client_credentials' }
require 'base64'
raise 'token basic changed' unless token_hit[2] == 'Basic ' + Base64.encode64('disc-id:disc-secret').delete("\n")
second = provider.acquire
raise 'cache miss changed' unless second.access_token == 'discovered-1'
raise 'discovery was re-fetched' unless transport.hits('.well-known/openid-configuration').length == 1
raise 'token was re-requested' unless transport.hits('/oauth/token').length == 1

# Concurrent acquires are single-flighted per provider: one discovery fetch
# and one token request.
flight_transport = ScriptedTransport.new
flight = OauthSdk::OAuth.client_credential('identityOAuth', client_id: 'disc-id',
                                           client_secret: 'disc-secret', transport: flight_transport)
results = Array.new(4) { Thread.new { flight.acquire } }.map(&:value)
raise 'single-flight lost results' unless results.all? { |candidate| candidate.access_token == 'discovered-1' }
raise 'single-flight fetched discovery twice' unless flight_transport.hits('.well-known/openid-configuration').length == 1
raise 'single-flight requested tokens twice' unless flight_transport.hits('/oauth/token').length == 1

# An issuer that does not share the discovery URL origin fails typed and
# without the mismatching value; the failed fetch stays uncached, so the next
# call retries and succeeds.
mismatch = ScriptedTransport.new
mismatch.discovery_document = DISCOVERY_DOCUMENT.merge('issuer' => 'https://elsewhere.oauth.test/v2').freeze
failing = OauthSdk::OAuth.client_credential('identityOAuth', client_id: 'disc-id',
                                            client_secret: 'disc-secret', transport: mismatch)
begin
  failing.acquire
  raise 'an issuer mismatch must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong discovery failure' unless error.auth_kind == 'discovery-failed'
  raise 'the mismatching issuer leaked' if error.inspect.include?('elsewhere') || error.message.include?('elsewhere')
end
mismatch.discovery_document = DISCOVERY_DOCUMENT
raise 'the failed fetch was not retried' unless failing.acquire.access_token == 'discovered-1'
raise 'the retry count changed' unless mismatch.hits('.well-known/openid-configuration').length == 2

# A failed discovery fetch is typed, carries the status, and is retried on the
# next call.
broken = ScriptedTransport.new
broken.discovery_status = 500
unreachable = OauthSdk::OAuth.client_credential('identityOAuth', client_id: 'disc-id',
                                                client_secret: 'disc-secret', transport: broken)
begin
  unreachable.acquire
  raise 'a failed discovery fetch must fail'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong discovery failure' unless error.auth_kind == 'discovery-failed' && error.status == 500
end
broken.discovery_status = 200
raise 'retry changed' unless unreachable.acquire.access_token == 'discovered-1'
raise 'retry fetch count changed' unless broken.hits('.well-known/openid-configuration').length == 2

# Explicit refresh resolves the token endpoint through discovery; the scheme
# with compiled endpoints keeps them without fetching discovery.
refresh_transport = ScriptedTransport.new
refreshed = OauthSdk::OAuth.refresh_token_set('identityOAuth',
                                              OauthSdk::OAuth::TokenSet.new(access_token: 't', refresh_token: 'r1'),
                                              client_id: 'disc-id', client_secret: 'disc-secret',
                                              transport: refresh_transport)
raise 'refresh changed' unless refreshed.access_token == 'discovered-1'
raise 'refresh endpoint changed' unless refresh_transport.hits('/oauth/token', method: 'POST').length == 1
raise 'refresh form changed' unless form_of(refresh_transport.hits('/oauth/token', method: 'POST').first[4]) ==
  { 'grant_type' => 'refresh_token', 'refresh_token' => 'r1' }

# Revocation and introspection resolve through discovery for the scheme with
# no configured endpoints, and the compiled endpoints win for the scheme that
# declares them (which never fetches discovery).
OauthSdk::OAuth.revoke('identityOAuth', 'tkn-live', client_id: 'disc-id',
                       client_secret: 'disc-secret', transport: refresh_transport)
raise 'discovered revocation changed' unless refresh_transport.requests.any? { |entry|
  entry[1] == 'https://authority.oauth.test/oauth/revoke'
}
claims = OauthSdk::OAuth.introspect('identityOAuth', 'tkn-live', client_id: 'disc-id',
                                    client_secret: 'disc-secret', transport: refresh_transport)
raise 'discovered introspection changed' unless claims['active'] == true && claims['scope'] == 'read'
compiled = ScriptedTransport.new
OauthSdk::OAuth.revoke('service', 'tkn-live', client_id: 'c', client_secret: 's', transport: compiled)
raise 'compiled revocation lost' unless compiled.requests.map { |entry| entry[1] } == ['https://auth.oauth.test/revoke']
claims = OauthSdk::OAuth.introspect('service', 'tkn-live', client_id: 'c', client_secret: 's', transport: compiled)
raise 'compiled introspection lost' unless claims['active'] == true
raise 'the compiled scheme fetched discovery' unless compiled.hits('.well-known').empty?

# The discovery-aware provider integrates with the generated client's
# credential attach path end to end.
OauthSdk::Client.open(credential_provider: OauthSdk::OAuth.client_credential('identityOAuth',
                         client_id: 'disc-id', client_secret: 'disc-secret', transport: transport),
                      transport: transport) do |client|
  response = client.list_widgets
  raise 'attach integration changed' unless response.data == 'ok'
end
raise 'the attach path skipped the bearer attachment' unless transport.requests.any? { |entry|
  entry[1] == 'https://api.oauth.test/v1/widgets' && entry[2] == 'Bearer discovered-2'
}

puts 'discovery behavior verified'
"#;

/// One client-credentials scheme over a JSON operation and one over a
/// streaming operation, each with its own token endpoint: the replay fixture.
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
#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn replaying_credentials_emit_conditionally_with_stream_protection() {
    let files = generate(&replay_document(), &replay_options());
    let oauth = content(&files, "ruby/lib/oauth_sdk/oauth.rb");
    for expected in [
        "NO_REPLAY_REQUIREMENTS = {",
        "\"feedOAuth\" => [\"/paths/~1events/get/security/0/feedOAuth\"].freeze,",
        "class ReplayingCredentialProvider",
        "class ReplayTransport",
        "def replaying_credential(scheme, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)",
        "def replay_transport(inner)",
        "triggers exactly one coordinated refresh",
        "delivered stream data prevents a transparent restart",
        "a newer stored set wins over a stale",
        "concurrent 401s share one round",
        "@served.length > 8",
        "def refresh(presented)",
    ] {
        assert!(
            oauth.contains(expected),
            "oauth.rb is missing:\n{expected}\n--- emitted: ---\n{oauth}"
        );
    }
    // The plain JSON operation's requirement is absent: its attaches replay.
    assert!(
        !oauth.contains("~1widgets/get/security/0/serviceOAuth"),
        "the JSON operation's requirement must stay replayable\n{oauth}"
    );
    // The replay surface joins the RBS signatures exactly with the runtime.
    let signatures = content(&files, "sig/oauth_sdk.rbs");
    for expected in [
        "class ReplayingCredentialProvider",
        "def self.replaying_credential:",
    ] {
        assert!(
            signatures.contains(expected),
            "signatures are missing:\n{expected}\n--- emitted: ---\n{signatures}"
        );
    }

    // A package without any executable client-credentials flow compiles
    // exactly the pre-replay bytes: no wrapper, no protection table.
    let code_only = generate(&code_only_document(), &code_only_options());
    let plain_oauth = content(&code_only, "ruby/lib/oauth_sdk/oauth.rb");
    assert!(!plain_oauth.contains("ReplayingCredentialProvider"));
    assert!(!plain_oauth.contains("NO_REPLAY_REQUIREMENTS"));
    assert!(!plain_oauth.contains("replaying_credential"));
    let plain_signatures = content(&code_only, "sig/oauth_sdk.rbs");
    assert!(!plain_signatures.contains("ReplayingCredentialProvider"));
}

const REPLAY_BEHAVIOR: &str = r#"# frozen_string_literal: true
require 'json'
$LOAD_PATH.unshift(File.join(__dir__, 'ruby', 'lib'))
require 'oauth_sdk'

# Scripted API and token endpoints. The API answers 401 for the armed stale
# token and 200 afterwards, so the driver counts exactly one refresh and one
# replay per scenario. Token requests ride the same transport as operations.
class ReplayScriptedTransport
  def initialize
    @requests = []
    @lock = Mutex.new
    @mode = 'ok'
    @fail_from = nil
    @stale_next = false
    @stale_token = nil
    @service_tokens = 0
    @feed_tokens = 0
  end

  def set_mode(value) = @mode = value
  def arm_stale = @stale_next = true

  # The token request after the next one is refused, so the attach succeeds
  # and the coordinated refresh fails.
  def arm_refresh_failure = @fail_from = @service_tokens + 2

  def hits(prefix)
    @lock.synchronize { @requests.select { |entry| entry[0].start_with?(prefix) } }
  end

  def reset = @lock.synchronize { @requests.clear }

  def exchange(request:, context:)
    context.check!
    @lock.synchronize do
      authorization = request.headers.find { |name, _| name.downcase == 'authorization' }&.last
      @requests << [request.url, authorization, request.body]
      sleep(0.02)
      json = ->(body, status) {
        OauthSdk::WireResponse.new(status: status,
                                   headers: { 'Content-Type' => 'application/json' },
                                   body: body)
      }
      url = request.url
      wire =
        if url == 'https://auth.oauth.test/token'
          @service_tokens += 1
          if @fail_from && @service_tokens >= @fail_from
            json.call({ 'error' => 'server_error' }.to_json, 500)
          else
            token = "svc-#{@service_tokens}"
            @stale_token = token if @stale_next
            @stale_next = false
            json.call({ 'access_token' => token, 'token_type' => 'Bearer', 'expires_in' => 3600 }.to_json, 200)
          end
        elsif url == 'https://auth.oauth.test/feed-token'
          @feed_tokens += 1
          json.call({ 'access_token' => "feed-#{@feed_tokens}", 'token_type' => 'Bearer', 'expires_in' => 3600 }.to_json, 200)
        elsif url == 'https://api.oauth.test/v1/widgets'
          if @mode == 'always-401'
            json.call({ 'error' => 'unauthorized' }.to_json, 401)
          elsif @stale_token && authorization == "Bearer #{@stale_token}"
            json.call({ 'error' => 'stale' }.to_json, 401)
          else
            json.call('"ok"', 200)
          end
        elsif url == 'https://api.oauth.test/v1/events'
          json.call({ 'error' => 'stream-denied' }.to_json, 401)
        else
          raise "unexpected request #{url}"
        end
      yield wire
    end
  end
end

def count(transport, prefix)
  transport.hits(prefix).length
end

# (a) 401 then success: one refresh, one replay, and the caller sees 200 with
# the fresh token.
transport = ReplayScriptedTransport.new
transport.arm_stale
credentials = OauthSdk::OAuth.replaying_credential('serviceOAuth', client_id: 'c',
                                                   client_secret: 's', transport: transport)
client = OauthSdk::Client.new(credential_provider: credentials,
                              transport: credentials.replay_transport(transport))
ok = client.list_widgets
raise 'replay changed the response' unless ok.data == 'ok'
raise 'replay count changed' unless count(transport, 'https://api.oauth.test/v1/widgets') == 2
raise 'refresh count changed' unless count(transport, 'https://auth.oauth.test/token') == 2
raise 'cross-scheme refresh leaked' unless count(transport, 'https://auth.oauth.test/feed-token').zero?
widget_values = transport.hits('https://api.oauth.test/v1/widgets').map { |entry| entry[1] }
raise 'attach value changed' unless widget_values[0] == 'Bearer svc-1'
raise 'the replay did not carry the fresh token' unless widget_values[1] == 'Bearer svc-2'

# (b) 401 then 401: the second 401 surfaces and exactly one refresh ran; the
# stored set from (a) serves the attach.
transport.set_mode('always-401')
transport.reset
begin
  client.list_widgets
  raise 'the second 401 must surface'
rescue OauthSdk::ResponseError => error
  raise 'wrong surfaced status' unless error.status == 401
end
raise 'loop detected' unless count(transport, 'https://api.oauth.test/v1/widgets') == 2
raise 'refresh budget changed' unless count(transport, 'https://auth.oauth.test/token') == 1

# (c) concurrent 401s: ONE refresh, two replays.
transport.set_mode('ok')
transport.reset
transport.arm_stale
shared = OauthSdk::OAuth.replaying_credential('serviceOAuth', client_id: 'c',
                                              client_secret: 's', transport: transport)
shared_client = OauthSdk::Client.new(credential_provider: shared,
                                     transport: shared.replay_transport(transport))
results = Array.new(2) { Thread.new { shared_client.list_widgets } }.map(&:value)
raise 'a concurrent replay lost its response' unless results.all? { |response| response.data == 'ok' }
raise 'replay count changed' unless count(transport, 'https://api.oauth.test/v1/widgets') == 4
raise 'the refresh was not shared' unless count(transport, 'https://auth.oauth.test/token') == 2

# (d) a streaming operation is never replayed: the typed 401 surfaces and the
# feed token is never refreshed.
transport.reset
feed = OauthSdk::OAuth.replaying_credential('feedOAuth', client_id: 'f',
                                            client_secret: 's', transport: transport)
feed_client = OauthSdk::Client.new(credential_provider: feed,
                                   transport: feed.replay_transport(transport))
begin
  feed_client.stream_events
  raise 'a stream-protected 401 must surface without a replay'
rescue OauthSdk::ResponseError => error
  raise 'wrong stream status' unless error.status == 401
end
raise 'the streaming operation replayed' unless count(transport, 'https://api.oauth.test/v1/events') == 1
raise 'the streaming operation refreshed' unless count(transport, 'https://auth.oauth.test/feed-token') == 1

# (e) replay disabled by default: the plain provider surfaces the 401 without
# any refresh.
transport.reset
transport.arm_stale
plain_client = OauthSdk::Client.new(
  credential_provider: OauthSdk::OAuth.client_credential('serviceOAuth', client_id: 'c',
                                                         client_secret: 's', transport: transport),
  transport: transport)
begin
  plain_client.list_widgets
  raise 'the plain provider must surface the 401'
rescue OauthSdk::ResponseError => error
  raise 'wrong plain status' unless error.status == 401
end
raise 'the plain provider replayed' unless count(transport, 'https://api.oauth.test/v1/widgets') == 1
raise 'the plain provider refreshed' unless count(transport, 'https://auth.oauth.test/token') == 1

# (f) refresh failure: the typed auth failure surfaces instead of a replay.
transport.reset
transport.arm_stale
transport.arm_refresh_failure
failing = OauthSdk::OAuth.replaying_credential('serviceOAuth', client_id: 'c',
                                               client_secret: 's', transport: transport)
failing_client = OauthSdk::Client.new(credential_provider: failing,
                                      transport: failing.replay_transport(transport))
begin
  failing_client.list_widgets
  raise 'a failed refresh must surface instead of a replay'
rescue OauthSdk::OAuth::AuthError => error
  raise 'wrong refresh failure' unless error.auth_kind == 'authorization-error' && error.server_code == 'server_error'
end
raise 'replay after a failed refresh' unless count(transport, 'https://api.oauth.test/v1/widgets') == 1
raise 'the refresh was not attempted exactly once' unless count(transport, 'https://auth.oauth.test/token') == 2

puts 'replay behavior verified'
"#;

fn ruby_home() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}

/// The gem requires Ruby >= 3.3; older interpreters cannot even parse the
/// emitted runtime syntax, so discovery refuses them.
fn ruby() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_RUBY_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let ruby = ruby_home().join("bin/ruby");
    if !ruby.is_file() {
        return None;
    }
    let output = Command::new(&ruby).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let minor = text
        .strip_prefix("ruby ")
        .and_then(|rest| rest.split('.').nth(1))
        .and_then(|minor| minor.parse::<u32>().ok())?;
    (minor >= 3).then_some(ruby)
}

fn checked(command: &mut Command, root: &std::path::Path, label: &str) {
    let output = command.output().unwrap();
    std::fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    std::fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_lifecycle_counts_requests_and_protects_secrets() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_oauth: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    // The interactive package compiles every executable flow plus the
    // configured revocation and introspection endpoints, so one scripted
    // probe covers the whole lifecycle.
    let files = generate(&interactive_document(), &interactive_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "syntax",
            );
        }
    }

    std::fs::write(root.path().join("behavior.rb"), BEHAVIOR).unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "behavior",
    );
    eprintln!("ruby_oauth: native Ruby behavioral gate passed");
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_discovery_lifecycle_resolves_compiled_precedence() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_oauth: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&discovery_document(), &discovery_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "discovery-syntax",
            );
        }
    }

    std::fs::write(
        root.path().join("discovery_behavior.rb"),
        DISCOVERY_BEHAVIOR,
    )
    .unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("discovery_behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "discovery-behavior",
    );
    eprintln!("ruby_oauth: native Ruby discovery behavioral gate passed");
}

#[ignore = "requires the Ruby interpreter on the test host"]
#[test]
fn native_replay_lifecycle_budgets_one_refresh_and_one_replay() {
    let Some(ruby) = ruby() else {
        eprintln!("ruby_oauth replay: no Ruby >= 3.3 toolchain; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = generate(&replay_document(), &replay_options());
    suspect_codegen::write_files(&files, root.path()).unwrap();

    // Every emitted Ruby file must at least be syntactically valid.
    for file in &files {
        if file.path.ends_with(".rb") {
            checked(
                Command::new(&ruby)
                    .arg("-c")
                    .arg(root.path().join(&file.path)),
                root.path(),
                "replay-syntax",
            );
        }
    }

    std::fs::write(root.path().join("replay_behavior.rb"), REPLAY_BEHAVIOR).unwrap();
    checked(
        Command::new(&ruby)
            .arg(root.path().join("replay_behavior.rb"))
            .current_dir(root.path()),
        root.path(),
        "replay-behavior",
    );
    eprintln!("ruby_oauth: native Ruby replay behavioral gate passed");
}
