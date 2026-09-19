//! Emitted-only first-party OAuth 2.0 / OpenID Connect lifecycle for the Ruby
//! gem.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome (carried on
//! the plan when client defaults are configured) compiles into one generated
//! `lib/<require>/oauth.rb` module, required from the generated entry beside
//! `http.rb`; the static runtime files and the shared planner stay untouched,
//! and plans without configured defaults — or without at least one usable
//! scheme — emit no new bytes at all, so no-policy output stays byte-identical.
//!
//! What counts as usable mirrors every backend: a non-deprecated flow whose
//! declared endpoints satisfy its grant — `client-credentials` needs a token
//! URL, `authorization-code` needs authorization and token URLs, and
//! `device-authorization` needs device-authorization and token URLs — or a
//! compiled discovery URL, whose document defines the scheme's endpoints at
//! runtime. Implicit and password flows are represented in the compiled
//! descriptors but are never executed, so schemes with only those flows and no
//! discovery URL emit nothing.
//!
//! The generated module embeds the compiled descriptors as frozen constants —
//! the runtime never parses OpenAPI and never invents an endpoint — and owns:
//! a frozen `TokenSet`, the `TokenStore` interface with a `Mutex`-guarded
//! instance-owned `MemoryTokenStore`, skew-aware client-credentials acquisition
//! with per-provider single-flight, explicit refresh, and — only when the
//! compiled scheme carries them — authorization-code with PKCE S256, RFC 8628
//! device polling with an injectable sleeper, RFC 7009 revocation and RFC 7662
//! introspection. `ClientCredentialProvider` integrates with the generated
//! client's existing credential attach path (`credential_provider:` /
//! `auth:`) by minting `AuthorizationCredential` values from a cached or
//! freshly acquired set. Client identity resolves from explicit arguments
//! first and otherwise from the compiled environment variable names, read at
//! call time. Token endpoint requests ride the caller's transport (the
//! generated `NetHTTPTransport` by default), so caller transport policy covers
//! the lifecycle too. Error values never carry token or client-secret
//! material.
//!
//! When at least one compiled scheme carries a discovery URL, every emitted
//! section that resolves an endpoint has a discovery-aware variant: the
//! compiled precedence is explicit-endpoint first, the RFC 8414 / OpenID
//! Connect discovery document second. The document is fetched once per scheme
//! through the module's existing transport path (one GET, `accept:
//! application/json`, the compiled token timeout), issuer-validated against
//! the discovery URL's origin, bounded at about a mebibyte, cached per
//! provider instance keyed by scheme under a `Mutex` (single-flight across
//! concurrent callers; a failed fetch stays uncached, so the next call
//! retries), and its typed `discovery-failed` failures never carry response
//! body text. Plans without a discovery URL assemble byte-identically to the
//! pre-discovery emission.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::http_protocol as wire;

/// Whether one compiled flow can be executed by the generated runtime.
fn executable(flow: &wire::OAuthFlowDescriptor) -> bool {
    if flow.deprecated_flow {
        return false;
    }
    match flow.kind {
        wire::OAuthFlowDescriptorKind::ClientCredentials => flow.token_url.is_some(),
        wire::OAuthFlowDescriptorKind::AuthorizationCode => {
            flow.authorization_url.is_some() && flow.token_url.is_some()
        }
        wire::OAuthFlowDescriptorKind::DeviceAuthorization => {
            flow.device_authorization_url.is_some() && flow.token_url.is_some()
        }
        // Implicit and password flows are never executed by generated code.
        wire::OAuthFlowDescriptorKind::Implicit | wire::OAuthFlowDescriptorKind::Password => false,
    }
}

/// The executable flow subset of one scheme, in declaration order.
fn executable_flows(scheme: &wire::OAuthSchemePlan) -> Vec<&wire::OAuthFlowDescriptor> {
    scheme
        .flows
        .iter()
        .filter(|flow| executable(flow))
        .collect()
}

/// Whether the compiled plan admits at least one usable scheme, so the gem
/// carries the generated `oauth.rb`. A compiled discovery URL makes a scheme
/// usable even with no declared flows: OpenID Connect schemes have their
/// endpoints defined by the discovery document at runtime.
pub(super) fn emits(oauth: &wire::OAuthPlan) -> bool {
    oauth.mode != wire::OAuthMode::Off
        && oauth
            .schemes
            .iter()
            .any(|scheme| scheme.discovery.is_some() || !executable_flows(scheme).is_empty())
}

/// Compiled OAuth planning carried by the plan: only a configured policy is
/// lowered, mirroring pagination, so no-policy output stays byte-identical for
/// every document. Planning errors are the shared planner's
/// [`crate::http_protocol::HttpDiagnostic`]s. The module is a namespace under
/// the generated root (`<Namespace>::OAuth`), so no client or model symbol
/// reservation is needed.
pub(super) fn plan(
    contract: &suspect_ir::contract::Contract,
    wire_plan: &wire::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
) -> Result<Option<wire::OAuthPlan>, Vec<crate::http_contract::HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    if defaults.oauth.mode == wire::OAuthMode::Off {
        return Ok(None);
    }
    let plan = wire::plan_oauth(contract, wire_plan, Some(defaults))?;
    Ok(emits(&plan).then_some(plan))
}

/// One usable scheme lowered into its compiled descriptor fields. Empty
/// strings mean "not compiled": endpoints are never invented.
struct Scheme {
    name: String,
    client_auth: &'static str,
    client_id_env: String,
    client_secret_env: String,
    skew: u32,
    discovery: String,
    token_url: String,
    refresh_url: String,
    client_credentials: String,
    authorization_url: String,
    code_token_url: String,
    device_url: String,
    device_token_url: String,
    revocation_url: String,
    introspection_url: String,
}

fn lower(scheme: &wire::OAuthSchemePlan) -> Option<Scheme> {
    let flows = executable_flows(scheme);
    let discovery = scheme.discovery.clone().unwrap_or_default();
    if flows.is_empty() && discovery.is_empty() {
        return None;
    }
    let first = flows.first();
    let mut lowered = Scheme {
        name: scheme.name.clone(),
        client_auth: match first {
            Some(flow) => match flow.client_auth {
                wire::OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
                wire::OAuthClientAuth::None => "none",
            },
            // A discovery-defined scheme has no declared flow client
            // authentication: the public profile unless a client secret
            // variable is compiled.
            None if scheme.client_secret_env.is_some() => "client-secret-basic",
            None => "none",
        },
        client_id_env: scheme.client_id_env.clone().unwrap_or_default(),
        client_secret_env: scheme.client_secret_env.clone().unwrap_or_default(),
        skew: scheme.refresh_skew_seconds,
        discovery,
        token_url: first
            .and_then(|flow| flow.token_url.clone())
            .unwrap_or_default(),
        refresh_url: first
            .and_then(|flow| flow.refresh_url.clone().or_else(|| flow.token_url.clone()))
            .unwrap_or_default(),
        client_credentials: String::new(),
        authorization_url: String::new(),
        code_token_url: String::new(),
        device_url: String::new(),
        device_token_url: String::new(),
        revocation_url: scheme.revocation_endpoint.clone().unwrap_or_default(),
        introspection_url: scheme.introspection_endpoint.clone().unwrap_or_default(),
    };
    for flow in flows {
        match flow.kind {
            wire::OAuthFlowDescriptorKind::ClientCredentials
                if lowered.client_credentials.is_empty() =>
            {
                lowered.client_credentials = flow.token_url.clone().unwrap_or_default();
            }
            wire::OAuthFlowDescriptorKind::AuthorizationCode
                if lowered.authorization_url.is_empty() =>
            {
                lowered.authorization_url = flow.authorization_url.clone().unwrap_or_default();
                lowered.code_token_url = flow.token_url.clone().unwrap_or_default();
            }
            wire::OAuthFlowDescriptorKind::DeviceAuthorization if lowered.device_url.is_empty() => {
                lowered.device_url = flow.device_authorization_url.clone().unwrap_or_default();
                lowered.device_token_url = flow.token_url.clone().unwrap_or_default();
            }
            _ => {}
        }
    }
    Some(lowered)
}

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap().replace('#', "\\#")
}

/// Render the generated `lib/<require>/oauth.rb` for a plan with usable
/// schemes. Called only when at least one usable scheme exists, so no-policy
/// output stays byte-identical. Plans without a discovery URL assemble
/// byte-identically to the pre-discovery emission; plans with one emit the
/// discovery-aware sections and the discovery engine. The replaying
/// credential wrapper joins only when a compiled scheme carries an executable
/// client-credentials flow; every other plan compiles exactly the pre-replay
/// bytes.
pub(super) fn runtime(oauth: &wire::OAuthPlan, operations: &[super::PlannedOperation]) -> String {
    let schemes: Vec<Scheme> = oauth.schemes.iter().filter_map(lower).collect();
    debug_assert!(
        !schemes.is_empty(),
        "emission is gated on at least one usable scheme"
    );
    let discovery = schemes.iter().any(|scheme| !scheme.discovery.is_empty());
    let has_client_credentials = schemes
        .iter()
        .any(|scheme| !scheme.client_credentials.is_empty());
    let has_authorization = schemes
        .iter()
        .any(|scheme| !scheme.authorization_url.is_empty());
    let has_device = schemes.iter().any(|scheme| !scheme.device_url.is_empty());
    let has_revocation = schemes
        .iter()
        .any(|scheme| !scheme.revocation_url.is_empty());
    let has_introspection = schemes
        .iter()
        .any(|scheme| !scheme.introspection_url.is_empty());
    // The replaying credential wrapper joins only when a compiled scheme
    // carries an executable client-credentials flow, the provider it wraps;
    // every other plan compiles exactly the pre-replay bytes.
    let no_replay = if has_client_credentials {
        no_replay_requirements(&schemes, operations)
    } else {
        BTreeMap::new()
    };

    let mut out = String::new();
    out.push_str(HEAD);
    if discovery {
        out.push_str(HEAD_DISCOVERY);
    }
    let names = schemes
        .iter()
        .map(|scheme| scheme.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "  # Compiled source schemes: {names}.");
    out.push_str("  module OAuth\n");
    out.push_str(&descriptors(&schemes, discovery));
    out.push_str(CORE);
    if has_client_credentials || discovery {
        out.push_str(if discovery {
            PROVIDER_DISCOVERY
        } else {
            PROVIDER
        });
    }
    out.push_str(if discovery {
        REFRESH_DISCOVERY
    } else {
        REFRESH
    });
    if has_authorization {
        out.push_str(AUTHORIZATION_CODE);
    }
    if has_device {
        out.push_str(DEVICE);
    }
    if has_revocation {
        out.push_str(if discovery {
            REVOCATION_DISCOVERY
        } else {
            REVOCATION
        });
    }
    if has_introspection {
        out.push_str(if discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION
        });
    }
    if discovery {
        out.push_str(DISCOVERY);
    }
    if has_client_credentials {
        out.push_str(&replay_section(&schemes, &no_replay, discovery));
    }
    out.push_str("  end\nend\n");
    out
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
fn no_replay_requirements(
    schemes: &[Scheme],
    operations: &[super::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire.responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), wire::Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire.security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    wire::CredentialHook::OAuth2 { .. }
                        | wire::CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if schemes
                    .iter()
                    .any(|scheme| scheme.name == requirement.name())
                {
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(requirement.source().source().pointer().to_owned());
                }
            }
        }
    }
    pointers
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow; its plain
/// and discovery variants resolve the refresh endpoint and the
/// lifecycle-endpoint exclusion through the same compiled precedence as the
/// provider they wrap.
fn replay_section(
    schemes: &[Scheme],
    no_replay: &BTreeMap<String, BTreeSet<String>>,
    discovery: bool,
) -> String {
    let mut code = String::from(
        "\n    # Compiled stream-protected requirements: security-requirement source\n    # pointers whose attaches are never replayed, because delivered stream data\n    # prevents a transparent restart.\n    NO_REPLAY_REQUIREMENTS = {\n",
    );
    for scheme in schemes {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| q(pointer))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(code, "      {} => [{}].freeze,", q(&scheme.name), rendered);
    }
    code.push_str("    }.freeze\n");
    code.push_str(REPLAY_CREDENTIAL);
    code.push_str(if discovery {
        REPLAY_DISCOVERY
    } else {
        REPLAY_PLAIN
    });
    code
}

/// The compiled descriptor constant: one entry per usable source scheme.
fn descriptors(schemes: &[Scheme], discovery: bool) -> String {
    let mut code = String::from(if discovery {
        "    # Compiled scheme descriptors. Empty endpoint strings mean \"not\n    # compiled\": endpoints are never invented and supplemental endpoints\n    # exist only when configuration supplied them. A compiled discovery URL\n    # resolves the endpoint URLs the compiled flows omit at call time.\n    SCHEMES = {\n"
    } else {
        "    # Compiled scheme descriptors. Empty endpoint strings mean \"not\n    # compiled\": endpoints are never invented and supplemental endpoints\n    # exist only when configuration supplied them.\n    SCHEMES = {\n"
    });
    for scheme in schemes {
        let discovery_line = if discovery {
            format!("        discovery: {},\n", q(&scheme.discovery))
        } else {
            String::new()
        };
        let _ = write!(
            code,
            "      {key} => {{\n        name: {name},\n        client_auth: {client_auth},\n        client_id_env: {client_id_env},\n        client_secret_env: {client_secret_env},\n        skew: {skew},\n{discovery_line}        token_url: {token_url},\n        refresh_url: {refresh_url},\n        client_credentials: {client_credentials},\n        authorization_url: {authorization_url},\n        code_token_url: {code_token_url},\n        device_url: {device_url},\n        device_token_url: {device_token_url},\n        revocation: {revocation},\n        introspection: {introspection},\n      }},\n",
            key = q(&scheme.name),
            name = q(&scheme.name),
            client_auth = q(scheme.client_auth),
            client_id_env = q(&scheme.client_id_env),
            client_secret_env = q(&scheme.client_secret_env),
            skew = scheme.skew,
            discovery_line = discovery_line,
            token_url = q(&scheme.token_url),
            refresh_url = q(&scheme.refresh_url),
            client_credentials = q(&scheme.client_credentials),
            authorization_url = q(&scheme.authorization_url),
            code_token_url = q(&scheme.code_token_url),
            device_url = q(&scheme.device_url),
            device_token_url = q(&scheme.device_token_url),
            revocation = q(&scheme.revocation_url),
            introspection = q(&scheme.introspection_url),
        );
    }
    code.push_str("    }.freeze\n\n");
    code
}

const HEAD: &str = r##"# frozen_string_literal: true
require 'securerandom'
require 'openssl'

module __NAMESPACE__
  # Generated OAuth 2.0 / OpenID Connect token lifecycle for the source-declared
  # schemes compiled into OAuth::SCHEMES below. Every endpoint, environment
  # variable name and policy value is a generation-time constant: this module
  # never parses OpenAPI, never invents an endpoint and never embeds credential
  # values. Client identifiers and secrets resolve from explicit arguments
  # first and otherwise from the configured environment variable names, read at
  # call time.
  #
  # Implemented here: client-credentials acquisition with skew-aware caching
  # and per-provider single-flight, explicit refresh, and — exactly for the
  # schemes compiled below — authorization-code with PKCE S256, RFC 8628
  # device authorization with interval, authorization_pending, slow_down and
  # expiry polling, RFC 7009 revocation and RFC 7662 introspection. Implicit
  # and password flows are never executed: generated code does not perform
  # interactive resource-owner credential handling.
  #
  # Token sets live only in the token store owned by the provider or caller
  # that created it, under [scheme, token-endpoint issuer, client identity]
  # keys; there is no module-level token cache. Error values never carry token
  # or client-secret material. Token endpoint requests ride the caller's
  # transport (the generated NetHTTPTransport by default), so caller transport
  # policy covers the lifecycle too.
  #
"##;

/// The discovery paragraph appended to the header comment exactly when at
/// least one compiled scheme carries a discovery URL.
const HEAD_DISCOVERY: &str = r##"  # Endpoint URLs the compiled flows omit resolve through RFC 8414 / OpenID
  # Connect discovery when the scheme compiles a discovery URL: one GET
  # through the caller's transport, issuer-validated against the discovery
  # URL's origin, bounded at about a mebibyte, and cached per provider
  # instance keyed by scheme under a Mutex — concurrent callers share the one
  # in-flight fetch, and a failed fetch stays uncached, so the next call
  # retries. Discovery failures are typed AuthError values that never carry
  # response body text.
  #
"##;

const CORE: &str = r##"    # Typed OAuth lifecycle failure. +auth_kind+ and +auth_scheme+ classify
    # it; +server_code+ carries the authorization server's declared error and
    # +status+ its HTTP status when one was reached. The message and inspect
    # output contain only this safe metadata: never token values, client
    # secrets or response bodies.
    class AuthError < SdkError
      attr_reader :auth_kind, :auth_scheme, :server_code
      def initialize(auth_kind:, auth_scheme:, server_code: nil, status: nil, cause: nil)
        message = 'OAuth ' + auth_kind + ' failure'
        message += ' for scheme ' + auth_scheme unless auth_scheme.nil? || auth_scheme.empty?
        message += ': ' + server_code unless server_code.nil? || server_code.empty?
        super(message, kind: :auth, source: SOURCE, operation_id: auth_scheme, status: status)
        @auth_kind = auth_kind.dup.freeze
        @auth_scheme = (auth_scheme || '').dup.freeze
        @server_code = server_code&.dup&.freeze
      end
      def inspect = "#<#{self.class.name} auth_kind=#{@auth_kind.inspect} auth_scheme=#{@auth_scheme.inspect} server_code=#{@server_code.inspect}>"
    end

    # One issued token. +expires_at+ is epoch seconds already reduced by the
    # compiled refresh skew, so +expired?+ is a plain comparison. Inspect and
    # to_s never contain the access or refresh token.
    class TokenSet
      attr_reader :access_token, :token_type, :expires_at, :refresh_token, :scope
      def initialize(access_token:, token_type: 'Bearer', expires_at: nil, refresh_token: nil, scope: nil)
        raise ::ArgumentError, 'TokenSet needs a non-empty access_token' unless access_token.instance_of?(String) && !access_token.empty?
        @access_token = access_token.dup.freeze
        @token_type = (token_type.nil? || token_type.empty? ? 'Bearer' : token_type).dup.freeze
        @expires_at = expires_at
        @refresh_token = refresh_token.nil? || refresh_token.empty? ? nil : refresh_token.dup.freeze
        @scope = scope.instance_of?(String) && !scope.empty? ? scope.dup.freeze : nil
        freeze
      end

      # Whether +now+ (epoch seconds) has reached the compiled expiry. A set
      # with no declared lifetime never expires locally.
      def expired?(now: ::Process.clock_gettime(::Process::CLOCK_REALTIME))
        !@expires_at.nil? && now >= @expires_at
      end

      # Whether the set carries an issued or retained refresh token.
      def refreshable? = !@refresh_token.nil?

      # The complete Authorization credential consumed by the generated
      # client's source-declared oauth2/openid-connect attach path: the
      # server's token type, or the conventional Bearer when the response
      # omitted or malformed it.
      def authorization_credential
        AuthorizationCredential.new(scheme: @token_type, token: @access_token)
      rescue ::ArgumentError
        AuthorizationCredential.new(scheme: 'Bearer', token: @access_token)
      end
      def inspect = "#<#{self.class.name} token_type=#{@token_type.inspect} expires_at=#{@expires_at.inspect}>"
    end

    # Partitioned token storage. Keys are [scheme, token-endpoint issuer,
    # client identity] triples, so one store instance may be shared across
    # schemes, issuers and client identities while the partitions stay
    # independent. A store is owned by whoever creates it; nothing in this
    # module keeps a module-level store.
    module TokenStore
      def load(key) = raise ::NotImplementedError, 'TokenStore implementations must load by key'
      def replace(key, set) = raise ::NotImplementedError, 'TokenStore implementations must replace by key'
      def clear(key) = raise ::NotImplementedError, 'TokenStore implementations must clear by key'
    end

    # In-process store guarded by a lock. Instance-owned, never module state.
    class MemoryTokenStore
      include TokenStore
      def initialize
        @tokens = {}
        @lock = ::Mutex.new
      end
      def load(key) = @lock.synchronize { @tokens[key] }
      def replace(key, set) = @lock.synchronize { @tokens[key] = set }
      def clear(key) = @lock.synchronize { @tokens.delete(key) }
      def inspect = "#<#{self.class.name}>"
    end

    SOURCE = 'suspect-oauth'
    TOKEN_TIMEOUT = 30.0
    MAX_RESPONSE_BYTES = 1 << 20
    DEVICE_GRANT = 'urn:ietf:params:oauth:grant-type:device_code'

    module_function

    # The compiled descriptor for one source scheme name.
    def scheme!(name)
      SCHEMES.fetch(name) { raise AuthError.new(auth_kind: 'unknown-scheme', auth_scheme: name) }
    end

    # Reads one configured variable name at call time; absence stays nil.
    def environment(variable)
      return nil if variable.nil? || variable.empty?
      value = ::ENV[variable]
      value.instance_of?(String) && !value.empty? ? value : nil
    end

    # Resolves the call's client identity: explicit arguments first, then the
    # compiled environment variable names, read at call time. Empty strings
    # mean absent; callers may legitimately configure only one.
    def resolve(descriptor, client_id: nil, client_secret: nil)
      id = client_id.instance_of?(String) && !client_id.empty? ? client_id : (environment(descriptor[:client_id_env]) || '')
      secret = client_secret.instance_of?(String) && !client_secret.empty? ? client_secret : (environment(descriptor[:client_secret_env]) || '')
      [id, secret]
    end

    # Store keys partition by scheme, token-endpoint issuer and client
    # identity, so distinct clients and endpoints never share a set.
    def store_key(scheme, issuer, client_id)
      [scheme.dup.freeze, issuer.dup.freeze, client_id.to_s.dup.freeze].freeze
    end

    def now(clock)
      clock ? clock.call : ::Process.clock_gettime(::Process::CLOCK_REALTIME)
    end

    # RFC 6749 request form: absent optional entries are simply not sent.
    def form(fields)
      fields.reject { |_, value| value.nil? }
            .map { |name, value| percent(name) + '=' + percent(value) }
            .join('&')
    end

    def percent(value) = ::URI.encode_www_form_component(value)

    # RFC 6749 2.3.1 Basic credentials: form-encoded id and secret.
    def basic(client_id, client_secret)
      [percent(client_id) + ':' + percent(client_secret)].pack('m0')
    end

    # Posts one form-encoded request to a compiled OAuth endpoint and returns
    # [status, body]. It applies the scheme's compiled client authentication:
    # HTTP Basic for confidential clients, the client_id form member for
    # public ones. Requests ride the caller's transport.
    def endpoint_request(transport:, scheme:, endpoint:, client_id:, client_secret:, fields:)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if endpoint.nil? || endpoint.empty?
      descriptor = SCHEMES[scheme]
      basic = !descriptor.nil? && descriptor[:client_auth] == 'client-secret-basic'
      unless basic
        fields += [['client_id', client_id]] unless client_id.nil? || client_id.empty?
      end
      headers = { 'Content-Type' => 'application/x-www-form-urlencoded', 'Accept' => 'application/json' }
      headers['Authorization'] = 'Basic ' + basic(client_id, client_secret) if basic
      request = PreparedRequest.new(method: 'POST', url: endpoint, headers: headers,
                                    body: form(fields), source: SOURCE, operation_id: scheme)
      context = ExchangeContext.new(timeout: TOKEN_TIMEOUT, cancellation: nil,
                                    operation_id: scheme, source: SOURCE)
      status = nil
      body = nil
      context.run do
        transport.exchange(request: request, context: context) do |wire|
          begin
            chunks = +''.b
            wire.each_chunk { |chunk| chunks << chunk.to_s.b }
            status = wire.status
            body = chunks
          ensure
            wire.close
          end
        end
      end
      raise AuthError.new(auth_kind: 'resource-limit', auth_scheme: scheme, status: status) if body.bytesize > MAX_RESPONSE_BYTES
      [status, body]
    rescue TimeoutError, CancelledError, ResourceLimitError
      raise
    rescue SdkError
      raise
    rescue ::StandardError => error
      raise AuthError.new(auth_kind: 'transport', auth_scheme: scheme, cause: error), cause: error
    end

    # Decodes one bounded JSON object body.
    def parse_json(bytes, scheme:, status:)
      payload = Json.parse(bytes, max_bytes: MAX_RESPONSE_BYTES, max_depth: 32, max_work: 16 * 1024 * 1024)
      raise AuthError.new(auth_kind: 'invalid-response', auth_scheme: scheme, status: status) unless payload.instance_of?(Hash)
      payload
    rescue JsonError
      raise AuthError.new(auth_kind: 'invalid-response', auth_scheme: scheme, status: status)
    end

    # Executes one RFC 6749 token-endpoint request and decodes the response.
    # +retained+ keeps the previous set's refresh token when the server
    # returns no rotated one.
    def token_request(scheme:, endpoint:, client_id:, client_secret:, fields:, retained: nil, transport:)
      status, body = endpoint_request(transport: transport, scheme: scheme, endpoint: endpoint,
                                      client_id: client_id, client_secret: client_secret, fields: fields)
      payload = parse_json(body, scheme: scheme, status: status)
      code = payload['error']
      if code.instance_of?(String) && !code.empty?
        raise AuthError.new(auth_kind: 'authorization-error', auth_scheme: scheme, server_code: code, status: status)
      end
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      parse_token(payload, descriptor: SCHEMES.fetch(scheme), retained: retained)
    end

    # Decodes one RFC 6749 token response. A server-returned rotated refresh
    # token wins; otherwise +retained+ keeps the previous set's token.
    def parse_token(payload, descriptor:, retained: nil)
      scheme = descriptor.fetch(:name)
      access = payload['access_token']
      unless access.instance_of?(String) && !access.empty?
        raise AuthError.new(auth_kind: 'invalid-response', auth_scheme: scheme)
      end
      token_type = payload['token_type']
      token_type = token_type.instance_of?(String) && !token_type.empty? ? token_type : 'Bearer'
      token_type = 'Bearer' if token_type.casecmp('bearer').zero?
      expires_in = payload['expires_in']
      expires_at = nil
      if expires_in.instance_of?(JsonNumber) || expires_in.instance_of?(Integer)
        seconds = expires_in.instance_of?(Integer) ? expires_in : expires_in.to_i(max_digits: 20)
        if seconds.positive?
          expires_at = ::Process.clock_gettime(::Process::CLOCK_REALTIME) + [seconds - descriptor.fetch(:skew), 0].max
        end
      end
      refresh = payload['refresh_token']
      refresh = retained if !(refresh.instance_of?(String) && !refresh.empty?) && retained
      scope = payload['scope']
      TokenSet.new(access_token: access, token_type: token_type, expires_at: expires_at,
                   refresh_token: refresh, scope: scope.instance_of?(String) ? scope : nil)
    end
"##;

const PROVIDER: &str = r##"
    module_function

    # Returns the scheme's cached token set from +store+, acquiring one from
    # the compiled client-credentials endpoint when the stored set is absent
    # or expired beyond the compiled skew. The read and the replace are
    # atomic; concurrent callers each serialize on the provider or session
    # that owns the store.
    def client_credentials_token(scheme, store:, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if descriptor.fetch(:client_credentials).empty?
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      key = store_key(scheme, descriptor.fetch(:token_url), id)
      stored = store.load(key)
      return stored unless stored.nil? || stored.expired?
      set = token_request(scheme: scheme, endpoint: descriptor.fetch(:client_credentials),
                          client_id: id, client_secret: secret,
                          fields: [['grant_type', 'client_credentials']],
                          retained: stored&.refresh_token, transport: transport || NetHTTPTransport.new)
      store.replace(key, set)
      set
    end

    # One compiled scheme's on-demand credential provider. Attach it under the
    # scheme's source name or pass it as the generated client's
    # +credential_provider:+; the client's existing credential attach path
    # calls it with the located requirement and receives an
    # AuthorizationCredential built from a cached or freshly acquired
    # TokenSet. Acquisition is single-flighted per provider instance, so
    # concurrent attaches share one token request. A held refresh token drives
    # the refresh grant; a rotated refresh token is adopted, otherwise the
    # current one is retained.
    class ClientCredentialProvider
      attr_reader :scheme
      def initialize(scheme:, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
        descriptor = OAuth.scheme!(scheme)
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if descriptor.fetch(:client_credentials).empty?
        @descriptor = descriptor
        @scheme = scheme.dup.freeze
        @client_id = client_id
        @client_secret = client_secret
        @store = store || MemoryTokenStore.new
        @transport = transport || NetHTTPTransport.new
        @clock = clock
        @lock = ::Mutex.new
      end

      # The generated client's credential attach path: returns the
      # Authorization credential for the located requirement.
      def call(_context)
        acquire.authorization_credential
      end

      # Returns the cached token set, acquiring or refreshing when the stored
      # set is absent or expired beyond the compiled skew. Callers that arrive
      # while an acquisition is in flight re-read the store after the holder
      # finishes instead of acquiring again.
      def acquire
        key = OAuth.store_key(@scheme, @descriptor.fetch(:token_url),
                              OAuth.resolve(@descriptor, client_id: @client_id, client_secret: @client_secret).first)
        cached = @store.load(key)
        return cached unless cached.nil? || cached.expired?(now: OAuth.now(@clock))
        @lock.synchronize do
          cached = @store.load(key)
          return cached unless cached.nil? || cached.expired?(now: OAuth.now(@clock))
          held = @store.load(key)
          set = begin
            OAuth.refresh_request(@descriptor, token: held, client_id: @client_id,
                                  client_secret: @client_secret, transport: @transport)
          rescue AuthError
            nil
          end
          set ||= OAuth.client_credentials_grant(@descriptor, held: held, client_id: @client_id,
                                                 client_secret: @client_secret, transport: @transport)
          @store.replace(key, set)
          set
        end
      end
      def inspect = "#<#{self.class.name} scheme=#{@scheme.inspect}>"
    end

    # Builds the provider for one compiled scheme:
    #   client = Client.new(credential_provider: OAuth.client_credential('service'))
    # +client_id+/+client_secret+ win over the configured environment
    # variables, which are read at call time; +store+ defaults to a fresh
    # per-provider MemoryTokenStore; +transport+ injects the token transport;
    # +clock+ replaces the wall clock (tests pass a stub).
    def client_credential(scheme, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
      ClientCredentialProvider.new(scheme: scheme, client_id: client_id, client_secret: client_secret,
                                   store: store, transport: transport, clock: clock)
    end

    # @api private
    def client_credentials_grant(descriptor, held:, client_id: nil, client_secret: nil, transport:)
      scheme = descriptor.fetch(:name)
      if descriptor.fetch(:client_credentials).empty?
        raise AuthError.new(auth_kind: 'renewal-required', auth_scheme: scheme)
      end
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      token_request(scheme: scheme, endpoint: descriptor.fetch(:client_credentials), client_id: id,
                    client_secret: secret, fields: [['grant_type', 'client_credentials']],
                    retained: held&.refresh_token, transport: transport)
    end
"##;

const REFRESH: &str = r##"
    module_function

    # Explicitly refreshes +token+ (RFC 6749 section 6) over the scheme's
    # declared refresh URL, or its token endpoint when no refresh URL is
    # declared. The returned set adopts a rotated refresh token and retains
    # the given one otherwise. The store is neither read nor updated: callers
    # decide which set to keep. When +store+ is supplied the refreshed set
    # atomically replaces the stored entry under the partition key.
    def refresh_token_set(scheme, token, client_id: nil, client_secret: nil, transport: nil, store: nil)
      descriptor = scheme!(scheme)
      raise AuthError.new(auth_kind: 'no-refresh-token', auth_scheme: scheme) unless token.refreshable?
      refreshed = refresh_request(descriptor, token: token, client_id: client_id,
                                  client_secret: client_secret, transport: transport || NetHTTPTransport.new)
      if store
        id, = resolve(descriptor, client_id: client_id, client_secret: client_secret)
        store.replace(store_key(scheme, descriptor.fetch(:token_url), id), refreshed)
      end
      refreshed
    end

    # @api private
    def refresh_request(descriptor, token:, client_id: nil, client_secret: nil, transport:)
      scheme = descriptor.fetch(:name)
      url = descriptor.fetch(:refresh_url)
      raise AuthError.new(auth_kind: 'no-refresh-token', auth_scheme: scheme) unless token&.refreshable?
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if url.empty?
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      token_request(scheme: scheme, endpoint: url, client_id: id, client_secret: secret,
                    fields: [['grant_type', 'refresh_token'], ['refresh_token', token.refresh_token]],
                    retained: token.refresh_token, transport: transport)
    end
"##;

const AUTHORIZATION_CODE: &str = r##"
    # One bound, single-use authorization-code transaction.
    # +authorization_url+ carries the response type, client identity, redirect
    # URI, state and the PKCE S256 challenge derived from the retained
    # verifier. The verifier and the consumed flag never leave this object
    # except through the code exchange.
    class AuthorizationTransaction
      attr_reader :scheme, :authorization_url, :state, :created_at
      def initialize(scheme:, authorization_url:, state:, verifier:, token_url:, redirect_uri:)
        @scheme = scheme.dup.freeze
        @authorization_url = authorization_url.dup.freeze
        @state = state.dup.freeze
        @created_at = ::Process.clock_gettime(::Process::CLOCK_REALTIME)
        @verifier = verifier
        @token_url = token_url
        @redirect_uri = redirect_uri
        @consumed = false
        @lock = ::Mutex.new
      end

      # Whether the transaction was already consumed by a completion attempt.
      def consumed? = @lock.synchronize { @consumed }

      # Consumes the transaction exactly once, whatever the completion
      # outcome: a repeated completion is a typed state failure.
      def take
        @lock.synchronize do
          return nil if @consumed
          @consumed = true
          [@verifier, @token_url, @redirect_uri].freeze
        end
      end
      def inspect = "#<#{self.class.name} scheme=#{@scheme.inspect}>"
    end

    module_function

    # RFC 7636 S256: a random verifier from the system CSPRNG and its hashed
    # challenge.
    def pkce_pair
      verifier = ::SecureRandom.urlsafe_base64(48, padding: false)
      digest = ::OpenSSL::Digest::SHA256.digest(verifier)
      [verifier, [digest].pack('m0').tr('+/', '-_').delete('=')]
    end

    # Starts one authorization-code + PKCE S256 transaction. It performs no
    # network call: send the user to +authorization_url+ and complete the
    # transaction with the callback parameters.
    def begin_authorization(scheme, redirect_uri:, client_id: nil)
      descriptor = scheme!(scheme)
      if descriptor.fetch(:authorization_url).empty? || descriptor.fetch(:code_token_url).empty?
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
      end
      id, = resolve(descriptor, client_id: client_id, client_secret: nil)
      raise AuthError.new(auth_kind: 'missing-client-credentials', auth_scheme: scheme) if id.empty?
      verifier, challenge = pkce_pair
      state = ::SecureRandom.urlsafe_base64(16, padding: false)
      query = form([['response_type', 'code'], ['client_id', id], ['redirect_uri', redirect_uri],
                    ['state', state], ['code_challenge', challenge], ['code_challenge_method', 'S256']])
      separator = descriptor.fetch(:authorization_url).include?('?') ? '&' : '?'
      AuthorizationTransaction.new(scheme: scheme,
                                   authorization_url: descriptor.fetch(:authorization_url) + separator + query,
                                   state: state, verifier: verifier,
                                   token_url: descriptor.fetch(:code_token_url),
                                   redirect_uri: redirect_uri)
    end

    # Exchanges the transaction's authorization code for a TokenSet. Refuses a
    # server-declared error or a missing code, validates the callback's state,
    # marks the transaction consumed before any network work, and exchanges
    # the code with the retained PKCE verifier over the compiled token URL.
    # When +store+ is supplied the resulting set atomically replaces the
    # stored entry under the partition key.
    def complete_authorization(transaction, callback_params, client_id: nil, client_secret: nil, store: nil, transport: nil)
      raise AuthError.new(auth_kind: 'transaction-used', auth_scheme: transaction.scheme) if transaction.consumed?
      code = callback_params['error']
      if code.instance_of?(String) && !code.empty?
        raise AuthError.new(auth_kind: 'authorization-error', auth_scheme: transaction.scheme, server_code: code)
      end
      raise AuthError.new(auth_kind: 'state-mismatch', auth_scheme: transaction.scheme) unless callback_params['state'] == transaction.state
      code = callback_params['code']
      raise AuthError.new(auth_kind: 'invalid-callback', auth_scheme: transaction.scheme) unless code.instance_of?(String) && !code.empty?
      bound = transaction.take
      raise AuthError.new(auth_kind: 'transaction-used', auth_scheme: transaction.scheme) unless bound
      descriptor = scheme!(transaction.scheme)
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      fields = [['grant_type', 'authorization_code'], ['code', code], ['code_verifier', bound[0]]]
      fields << ['redirect_uri', bound[2]] unless bound[2].nil? || bound[2].empty?
      set = token_request(scheme: transaction.scheme, endpoint: bound[1], client_id: id,
                          client_secret: secret, fields: fields, retained: nil,
                          transport: transport || NetHTTPTransport.new)
      if store
        store.replace(store_key(transaction.scheme, descriptor.fetch(:token_url), id), set)
      end
      set
    end
"##;

const DEVICE: &str = r##"
    # One device-authorization grant from the declared endpoint (RFC 8628).
    # The device code never appears in the inspect output.
    class DeviceAuthorization
      attr_reader :scheme, :device_code, :user_code, :verification_uri, :verification_uri_complete, :expires_at, :interval
      def initialize(scheme:, device_code:, user_code:, verification_uri:, verification_uri_complete: nil, expires_at:, interval:)
        @scheme = scheme.dup.freeze
        @device_code = device_code.dup.freeze
        @user_code = user_code.dup.freeze
        @verification_uri = verification_uri.dup.freeze
        @verification_uri_complete = verification_uri_complete&.dup&.freeze
        @expires_at = expires_at
        @interval = interval
        freeze
      end
      def inspect = "#<#{self.class.name} scheme=#{@scheme.inspect} user_code=#{@user_code.inspect}>"
    end

    module_function

    # Requests one device grant from the compiled device-authorization URL
    # (RFC 8628 sections 3.1-3.2). Show the user the returned user code and
    # verification URI, then poll with poll_device_authorization.
    def begin_device_authorization(scheme, client_id: nil, transport: nil)
      descriptor = scheme!(scheme)
      if descriptor.fetch(:device_url).empty? || descriptor.fetch(:device_token_url).empty?
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
      end
      id, secret = resolve(descriptor, client_id: client_id, client_secret: nil)
      raise AuthError.new(auth_kind: 'missing-client-credentials', auth_scheme: scheme) if id.empty?
      status, body = endpoint_request(transport: transport || NetHTTPTransport.new, scheme: scheme,
                                      endpoint: descriptor.fetch(:device_url), client_id: id,
                                      client_secret: secret, fields: [['client_id', id]])
      payload = parse_json(body, scheme: scheme, status: status)
      code = payload['error']
      if code.instance_of?(String) && !code.empty?
        raise AuthError.new(auth_kind: 'authorization-error', auth_scheme: scheme, server_code: code, status: status)
      end
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      device_code = payload['device_code']
      user_code = payload['user_code']
      verification_uri = payload['verification_uri']
      unless [device_code, user_code, verification_uri].all? { |value| value.instance_of?(String) && !value.empty? }
        raise AuthError.new(auth_kind: 'invalid-response', auth_scheme: scheme, status: status)
      end
      complete = payload['verification_uri_complete']
      complete = complete.instance_of?(String) && !complete.empty? ? complete : nil
      interval = payload['interval']
      if interval.instance_of?(JsonNumber) || interval.instance_of?(Integer)
        interval = interval.instance_of?(Integer) ? interval : interval.to_i(max_digits: 20)
      else
        interval = 0
      end
      interval = interval.positive? ? interval : 5
      expires_in = payload['expires_in']
      expires_at = nil
      if expires_in.instance_of?(JsonNumber) || expires_in.instance_of?(Integer)
        seconds = expires_in.instance_of?(Integer) ? expires_in : expires_in.to_i(max_digits: 20)
        expires_at = seconds.positive? ? ::Process.clock_gettime(::Process::CLOCK_REALTIME) + seconds : nil
      end
      DeviceAuthorization.new(scheme: scheme, device_code: device_code, user_code: user_code,
                              verification_uri: verification_uri, verification_uri_complete: complete,
                              expires_at: expires_at, interval: interval)
    end

    # Polls the compiled token URL for the device grant (RFC 8628 3.5).
    # +authorization_pending+ waits the declared interval and retries,
    # +slow_down+ grows the interval by five seconds, any other refusal is a
    # typed failure, and polling stops once the grant's declared expiry
    # passes. The resulting TokenSet atomically replaces the stored entry when
    # +store+ is supplied. +wait+ replaces the sleeper (tests pass a no-op);
    # it defaults to Kernel#sleep.
    def poll_device_authorization(device, client_id: nil, client_secret: nil, store: nil, transport: nil, wait: nil)
      descriptor = scheme!(device.scheme)
      pause = wait || ::Kernel.method(:sleep)
      interval = device.interval
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      loop do
        if device.expires_at && ::Process.clock_gettime(::Process::CLOCK_REALTIME) >= device.expires_at
          raise AuthError.new(auth_kind: 'device-flow-expired', auth_scheme: device.scheme)
        end
        pause.call(interval)
        begin
          set = token_request(scheme: device.scheme, endpoint: descriptor.fetch(:device_token_url),
                              client_id: id, client_secret: secret,
                              fields: [['grant_type', DEVICE_GRANT], ['device_code', device.device_code]],
                              retained: nil, transport: transport || NetHTTPTransport.new)
          if store
            store.replace(store_key(device.scheme, descriptor.fetch(:token_url), id), set)
          end
          return set
        rescue AuthError => error
          case error.server_code
          when 'authorization_pending' then next
          when 'slow_down' then interval += 5
          else raise
          end
        end
      end
    end
"##;

const REVOCATION: &str = r##"
    module_function

    # Posts the token value to the scheme's configured revocation endpoint
    # (RFC 7009). Any 2xx response is success: RFC 7009 declares the token
    # revoked even when the server reports an unsupported-token error.
    def revoke(scheme, token_value, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if descriptor.fetch(:revocation).empty?
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      status, body = endpoint_request(transport: transport || NetHTTPTransport.new, scheme: scheme,
                                      endpoint: descriptor.fetch(:revocation), client_id: id,
                                      client_secret: secret, fields: [['token', token_value]])
      payload = parse_json(body, scheme: scheme, status: status) rescue nil
      code = payload && payload['error']
      if code.instance_of?(String) && !code.empty?
        raise AuthError.new(auth_kind: 'authorization-error', auth_scheme: scheme, server_code: code, status: status)
      end
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      nil
    end
"##;

const INTROSPECTION: &str = r##"
    module_function

    # Queries the scheme's configured introspection endpoint (RFC 7662) with
    # the token value. Returns the frozen response Hash: the server's
    # description of the token, without returning the token itself.
    def introspect(scheme, token_value, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if descriptor.fetch(:introspection).empty?
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      status, body = endpoint_request(transport: transport || NetHTTPTransport.new, scheme: scheme,
                                      endpoint: descriptor.fetch(:introspection), client_id: id,
                                      client_secret: secret, fields: [['token', token_value]])
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      parse_json(body, scheme: scheme, status: status).freeze
    end
"##;

/// The discovery-aware credential provider: endpoint resolution follows the
/// compiled precedence (an explicit compiled endpoint always wins; otherwise
/// the provider's cached discovery document). Byte-exact replacement for
/// PROVIDER exactly when at least one compiled scheme carries a discovery
/// URL.
const PROVIDER_DISCOVERY: &str = r##"
    module_function

    # Returns the scheme's cached token set from +store+, acquiring one from
    # the resolved client-credentials endpoint when the stored set is absent
    # or expired beyond the compiled skew. Endpoint resolution follows the
    # compiled precedence: the compiled client-credentials token URL always
    # wins; otherwise, when the scheme compiles a discovery URL, the discovery
    # document's token endpoint resolves the request. The read and the replace
    # are atomic; concurrent callers each serialize on the provider or session
    # that owns the store.
    def client_credentials_token(scheme, store:, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      if descriptor.fetch(:client_credentials).empty? && descriptor.fetch(:discovery).empty?
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
      end
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      url, discovered = resolve_endpoint(scheme, descriptor, descriptor.fetch(:client_credentials),
                                         'token_endpoint', transport)
      discovery_client_auth(descriptor, client_id: client_id, client_secret: client_secret) if discovered
      key = store_key(scheme, url, id)
      stored = store.load(key)
      return stored unless stored.nil? || stored.expired?
      set = token_request(scheme: scheme, endpoint: url,
                          client_id: id, client_secret: secret,
                          fields: [['grant_type', 'client_credentials']],
                          retained: stored&.refresh_token, transport: transport || NetHTTPTransport.new)
      store.replace(key, set)
      set
    end

    # One compiled scheme's on-demand credential provider. Attach it under the
    # scheme's source name or pass it as the generated client's
    # +credential_provider:+; the client's existing credential attach path
    # calls it with the located requirement and receives an
    # AuthorizationCredential built from a cached or freshly acquired
    # TokenSet. Acquisition is single-flighted per provider instance, so
    # concurrent attaches share one token request. A held refresh token drives
    # the refresh grant; a rotated refresh token is adopted, otherwise the
    # current one is retained.
    #
    # Endpoint resolution follows the compiled precedence: the compiled
    # client-credentials token URL always wins; otherwise, when the scheme
    # compiles a discovery URL, the discovery document's token endpoint
    # resolves the request — fetched once per scheme and cached for this
    # provider's lifetime under a Mutex, single-flighted across concurrent
    # callers, with a failed fetch retried on the next call.
    class ClientCredentialProvider
      attr_reader :scheme
      def initialize(scheme:, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
        descriptor = OAuth.scheme!(scheme)
        if descriptor.fetch(:client_credentials).empty? && descriptor.fetch(:discovery).empty?
          raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
        end
        @descriptor = descriptor
        @scheme = scheme.dup.freeze
        @client_id = client_id
        @client_secret = client_secret
        @store = store || MemoryTokenStore.new
        @transport = transport || NetHTTPTransport.new
        @clock = clock
        @lock = ::Mutex.new
        @discovered = {}
        @discovery_lock = ::Mutex.new
      end

      # The generated client's credential attach path: returns the
      # Authorization credential for the located requirement.
      def call(_context)
        acquire.authorization_credential
      end

      # This provider's cached discovery document for its scheme, fetched once
      # per provider under the discovery lock: concurrent callers share the
      # one in-flight fetch, and a failed fetch stays uncached so the next
      # caller retries.
      def discovered
        @discovery_lock.synchronize do
          cached = @discovered[@scheme]
          return cached if cached
          fetched = OAuth.discovery_payload(@scheme, @descriptor, @transport)
          @discovered[@scheme] = fetched
          fetched
        end
      end

      # The resolved token endpoint: the compiled client-credentials URL wins;
      # otherwise this provider's cached discovery document supplies it.
      def token_endpoint
        compiled = @descriptor.fetch(:client_credentials)
        return compiled unless compiled.empty?
        OAuth.discovered_endpoint(@scheme, discovered, 'token_endpoint') ||
          raise(AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: @scheme))
      end

      # The refresh endpoint when a held refresh token drives the refresh
      # grant: the compiled refresh or token URL wins; otherwise this
      # provider's cached discovery document supplies the token endpoint.
      # nil when the compiled plan declares neither.
      def refresh_endpoint
        compiled = @descriptor.fetch(:refresh_url)
        return compiled unless compiled.empty?
        OAuth.discovered_endpoint(@scheme, discovered, 'token_endpoint')
      end

      # Returns the cached token set, acquiring or refreshing when the stored
      # set is absent or expired beyond the compiled skew. Callers that arrive
      # while an acquisition is in flight re-read the store after the holder
      # finishes instead of acquiring again.
      def acquire
        url = token_endpoint
        id, = OAuth.resolve(@descriptor, client_id: @client_id, client_secret: @client_secret)
        key = OAuth.store_key(@scheme, url, id)
        cached = @store.load(key)
        return cached unless cached.nil? || cached.expired?(now: OAuth.now(@clock))
        @lock.synchronize do
          cached = @store.load(key)
          return cached unless cached.nil? || cached.expired?(now: OAuth.now(@clock))
          held = @store.load(key)
          set = nil
          if held&.refreshable? && (refresh = refresh_endpoint)
            set = begin
              OAuth.token_request(scheme: @scheme, endpoint: refresh, client_id: @client_id,
                                  client_secret: @client_secret,
                                  fields: [['grant_type', 'refresh_token'],
                                           ['refresh_token', held.refresh_token]],
                                  retained: held.refresh_token, transport: @transport)
            rescue AuthError
              nil
            end
          end
          set ||= OAuth.client_credentials_grant_for(@descriptor, url, held: held,
                                                     client_id: @client_id,
                                                     client_secret: @client_secret,
                                                     transport: @transport)
          @store.replace(key, set)
          set
        end
      end
      def inspect = "#<#{self.class.name} scheme=#{@scheme.inspect}>"
    end

    # Builds the provider for one compiled scheme:
    #   client = Client.new(credential_provider: OAuth.client_credential('service'))
    # +client_id+/+client_secret+ win over the configured environment
    # variables, which are read at call time; +store+ defaults to a fresh
    # per-provider MemoryTokenStore; +transport+ injects the token transport;
    # +clock+ replaces the wall clock (tests pass a stub).
    def client_credential(scheme, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
      ClientCredentialProvider.new(scheme: scheme, client_id: client_id, client_secret: client_secret,
                                   store: store, transport: transport, clock: clock)
    end

    # @api private
    # The client-credentials grant over one resolved token endpoint; the
    # discovery-aware provider's acquisition step.
    def client_credentials_grant_for(descriptor, url, held:, client_id: nil, client_secret: nil, transport:)
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      token_request(scheme: descriptor.fetch(:name), endpoint: url, client_id: id,
                    client_secret: secret, fields: [['grant_type', 'client_credentials']],
                    retained: held&.refresh_token, transport: transport)
    end
"##;

/// The discovery-aware explicit refresh: the compiled refresh or token URL
/// always wins; otherwise the discovery document's token endpoint.
const REFRESH_DISCOVERY: &str = r##"
    module_function

    # Explicitly refreshes +token+ (RFC 6749 section 6) over the resolved
    # refresh endpoint: the declared refresh URL, else the declared token URL,
    # always wins; otherwise, when the scheme compiles a discovery URL, the
    # discovery document's token endpoint resolves the exchange (fetched per
    # call; this one-shot helper keeps no cache). The returned set adopts a
    # rotated refresh token and retains the given one otherwise. The store is
    # neither read nor updated: callers decide which set to keep. When +store+
    # is supplied the refreshed set atomically replaces the stored entry under
    # the partition key.
    def refresh_token_set(scheme, token, client_id: nil, client_secret: nil, transport: nil, store: nil)
      descriptor = scheme!(scheme)
      raise AuthError.new(auth_kind: 'no-refresh-token', auth_scheme: scheme) unless token.refreshable?
      url, discovered = resolve_endpoint(scheme, descriptor, descriptor.fetch(:refresh_url),
                                         'token_endpoint', transport || NetHTTPTransport.new)
      discovery_client_auth(descriptor, client_id: client_id, client_secret: client_secret) if discovered
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      refreshed = token_request(scheme: scheme, endpoint: url, client_id: id, client_secret: secret,
                                fields: [['grant_type', 'refresh_token'], ['refresh_token', token.refresh_token]],
                                retained: token.refresh_token, transport: transport || NetHTTPTransport.new)
      if store
        store.replace(store_key(scheme, url, id), refreshed)
      end
      refreshed
    end
"##;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the discovery document's `revocation_endpoint`.
const REVOCATION_DISCOVERY: &str = r##"
    module_function

    # Posts the token value to the resolved revocation endpoint (RFC 7009):
    # the configured revocation endpoint always wins; otherwise, when the
    # scheme compiles a discovery URL, the discovery document's
    # revocation_endpoint resolves the request (fetched per call; this
    # one-shot helper keeps no cache). Any 2xx response is success: RFC 7009
    # declares the token revoked even when the server reports an
    # unsupported-token error.
    def revoke(scheme, token_value, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      url, discovered = resolve_endpoint(scheme, descriptor, descriptor.fetch(:revocation),
                                         'revocation_endpoint', transport || NetHTTPTransport.new)
      discovery_client_auth(descriptor, client_id: client_id, client_secret: client_secret) if discovered
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      status, body = endpoint_request(transport: transport || NetHTTPTransport.new, scheme: scheme,
                                      endpoint: url, client_id: id,
                                      client_secret: secret, fields: [['token', token_value]])
      payload = parse_json(body, scheme: scheme, status: status) rescue nil
      code = payload && payload['error']
      if code.instance_of?(String) && !code.empty?
        raise AuthError.new(auth_kind: 'authorization-error', auth_scheme: scheme, server_code: code, status: status)
      end
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      nil
    end
"##;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECTION_DISCOVERY: &str = r##"
    module_function

    # Queries the resolved introspection endpoint (RFC 7662) with the token
    # value: the configured introspection endpoint always wins; otherwise,
    # when the scheme compiles a discovery URL, the discovery document's
    # introspection_endpoint resolves the request (fetched per call; this
    # one-shot helper keeps no cache). Returns the frozen response Hash: the
    # server's description of the token, without returning the token itself.
    def introspect(scheme, token_value, client_id: nil, client_secret: nil, transport: nil)
      descriptor = scheme!(scheme)
      url, discovered = resolve_endpoint(scheme, descriptor, descriptor.fetch(:introspection),
                                         'introspection_endpoint', transport || NetHTTPTransport.new)
      discovery_client_auth(descriptor, client_id: client_id, client_secret: client_secret) if discovered
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      status, body = endpoint_request(transport: transport || NetHTTPTransport.new, scheme: scheme,
                                      endpoint: url, client_id: id,
                                      client_secret: secret, fields: [['token', token_value]])
      raise AuthError.new(auth_kind: 'server-error', auth_scheme: scheme, status: status) unless status.between?(200, 299)
      parse_json(body, scheme: scheme, status: status).freeze
    end
"##;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-provider cache with Mutex single-flight,
/// and the endpoint-resolution precedence.
const DISCOVERY: &str = r##"
    # The compiled ceiling for one discovery document response (about a
    # mebibyte).
    DISCOVERY_MAX_BYTES = 1 << 20

    module_function

    # The origin of one absolute http(s) URL: scheme, host and the port with
    # the scheme default made explicit. nil when the value is not an absolute
    # http(s) URL.
    def discovery_origin(url)
      parsed = ::URI.parse(url)
      return nil unless parsed.instance_of?(::URI::HTTP) || parsed.instance_of?(::URI::HTTPS)
      parsed.scheme + '://' + parsed.host + ':' + parsed.port.to_s
    rescue ::URI::Error
      nil
    end

    # GETs one compiled discovery document through the caller's transport and
    # returns [status, body]. The compiled ~1 MiB ceiling bounds the buffered
    # document; timeouts stay with the caller's transport, exactly like every
    # other lifecycle request.
    def discovery_request(transport:, scheme:, url:)
      headers = { 'Accept' => 'application/json' }
      request = PreparedRequest.new(method: 'GET', url: url, headers: headers, body: '',
                                    source: SOURCE, operation_id: scheme)
      context = ExchangeContext.new(timeout: TOKEN_TIMEOUT, cancellation: nil,
                                    operation_id: scheme, source: SOURCE)
      status = nil
      body = nil
      context.run do
        transport.exchange(request: request, context: context) do |response|
          begin
            chunks = +''.b
            response.each_chunk { |chunk| chunks << chunk.to_s.b }
            status = response.status
            body = chunks
          ensure
            response.close
          end
        end
      end
      raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme, status: status) if body.bytesize > DISCOVERY_MAX_BYTES
      [status, body]
    rescue TimeoutError, CancelledError, ResourceLimitError
      raise
    rescue SdkError
      raise
    rescue ::StandardError => error
      raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme, cause: error), cause: error
    end

    # Decodes and validates one discovery response body into the document this
    # module resolves endpoints from: a JSON object whose +issuer+ claim, when
    # present, is an absolute http(s) URL sharing the discovery URL's origin
    # (scheme, host and the port with the scheme default made explicit).
    # OpenID Connect openIdConnectUrl documents are validated against their
    # +issuer+ claim exactly this way, as are RFC 8414 authorization-server
    # metadata documents. A missing claim is tolerated; a mismatching or
    # unparseable one is a typed discovery failure. Failure messages carry
    # only safe metadata, never response body text.
    def discovery_document(scheme, url, body, status)
      unless status.instance_of?(Integer) && status.between?(200, 299)
        raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme, status: status)
      end
      payload = Json.parse(body, max_bytes: DISCOVERY_MAX_BYTES, max_depth: 32, max_work: 16 * 1024 * 1024)
      raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme) unless payload.instance_of?(Hash)
      issuer = payload['issuer']
      if issuer.instance_of?(String) && !issuer.empty?
        issuer_origin = discovery_origin(issuer)
        document_origin = discovery_origin(url)
        if issuer_origin.nil? || document_origin.nil? || issuer_origin != document_origin
          raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme)
        end
      end
      payload.freeze
    rescue JsonError
      raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme)
    end

    # One discovery document endpoint: absent stays nil, and an unusable value
    # is a typed discovery failure. Unknown members are ignored.
    def discovered_endpoint(scheme, payload, member)
      value = payload[member]
      return nil if value.nil?
      unless value.instance_of?(String) && !value.empty?
        raise AuthError.new(auth_kind: 'discovery-failed', auth_scheme: scheme)
      end
      value.dup.freeze
    end

    # @api private
    # One un-cached discovery fetch and decode: the compiled discovery URL's
    # document through the caller's transport. One-shot helpers fetch per call
    # and keep no cache; the credential provider caches per instance.
    def discovery_payload(scheme, descriptor, transport)
      url = descriptor.fetch(:discovery)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) if url.empty?
      status, body = discovery_request(transport: transport || NetHTTPTransport.new, scheme: scheme, url: url)
      discovery_document(scheme, url, body, status)
    end

    # Resolves one lifecycle endpoint through the compiled precedence: an
    # explicitly compiled endpoint always wins; otherwise the resolved
    # discovery document's endpoint when the scheme compiles a discovery URL;
    # otherwise the typed unsupported-flow refusal the compiled plan alone
    # would produce. The second member says whether discovery supplied the
    # endpoint, so the caller can apply the discovery client-authentication
    # rule.
    def resolve_endpoint(scheme, descriptor, compiled, member, transport)
      return [compiled, false] unless compiled.empty?
      payload = discovery_payload(scheme, descriptor, transport)
      discovered = discovered_endpoint(scheme, payload, member)
      raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme) unless discovered
      [discovered, true]
    end

    # @api private
    # Validates the client identity for one discovery-resolved endpoint: the
    # public profile sends the client id in the form and needs no secret; the
    # Basic profile requires an explicit or configured client id and secret.
    def discovery_client_auth(descriptor, client_id: nil, client_secret: nil)
      return nil unless descriptor.fetch(:client_auth) == 'client-secret-basic'
      id, secret = resolve(descriptor, client_id: client_id, client_secret: client_secret)
      if id.empty? || secret.empty?
        raise AuthError.new(auth_kind: 'missing-client-credentials', auth_scheme: descriptor.fetch(:name))
      end
      nil
    end
"##;

/// The replaying wrapper's shared half: the coordinated-refresh round, the
/// served-attach record and the replaying transport. The plain and discovery
/// variants append the creation guard, the lifecycle-endpoint exclusion and
/// the refresh-key derivation.
const REPLAY_CREDENTIAL: &str = r##"
    # One coordinated refresh round: exactly one forced acquisition, shared
    # by every concurrent 401 that presented the same stale token.
    class ReplayRound
      def initialize
        @lock = ::Mutex.new
        @signal = ::ConditionVariable.new
        @done = false
        @outcome = nil
      end

      def complete(value)
        @lock.synchronize do
          @outcome = ['fresh', value]
          @done = true
          @signal.broadcast
        end
        nil
      end

      def fail(error)
        @lock.synchronize do
          @outcome = ['failed', error]
          @done = true
          @signal.broadcast
        end
        nil
      end

      def wait
        @lock.synchronize do
          @signal.wait(@lock) until @done
          raise AuthError.new(auth_kind: 'replay-refresh', auth_scheme: '') if @outcome.nil?
          @outcome
        end
      end
      def inspect = "#<#{self.class.name}>"
    end

    # One compiled scheme's client-credentials provider plus the unified 401
    # replay policy.
    #
    # Attach behavior is exactly ClientCredentialProvider's: single-flighted
    # acquisition, skew-aware cache, atomic store replacement. On top of it
    # the provider remembers which Authorization values its attaches served,
    # and replay_transport wraps the client transport so a 401 (and only a
    # 401) on a request carrying this provider's token
    # triggers exactly one coordinated refresh — concurrent 401s share one
    # token request round — and exactly one replay of the request with the
    # fresh token. The second response is surfaced whatever it is. The overall
    # budget is one refresh plus one replay, never nested with other retry
    # policies (requests are not retried today). Attaches for stream-protected
    # requirements are never replayed, because delivered stream data prevents a transparent restart.
    # A refresh failure surfaces as the typed AuthError instead of a replay.
    # The plain provider keeps today's semantics: replay is this wrapper's
    # opt-in only. Wire it in two places: pass the provider as the generated
    # client's +credential_provider:+ and pass replay_transport(inner) as the
    # client's transport:
    #
    #   credentials = OAuth.replaying_credential('serviceOAuth')
    #   client = Client.new(credential_provider: credentials,
    #                       transport: credentials.replay_transport(my_transport))
    class ReplayingCredentialProvider
      attr_reader :scheme
      def initialize(scheme:, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
        descriptor = OAuth.scheme!(scheme)
        _replay_check(descriptor, scheme)
        @descriptor = descriptor
        @scheme = scheme.dup.freeze
        @client_id = client_id
        @client_secret = client_secret
        @store = store || MemoryTokenStore.new
        @plain = ClientCredentialProvider.new(scheme: scheme, client_id: client_id,
                                              client_secret: client_secret, store: @store,
                                              transport: transport, clock: clock)
        @never_replay = NO_REPLAY_REQUIREMENTS.fetch(scheme, [].freeze)
        @served = []
        @record_lock = ::Mutex.new
        @refresh_lock = ::Mutex.new
        @rounds = {}
      end

      # The generated client's credential attach path: serves the plain
      # provider's credential and records the attach so the replaying
      # transport can tell which requests carried this provider's token.
      def call(context)
        credential = @plain.call(context)
        record(credential_value(credential), replay_eligible?(context))
        credential
      end

      # Wraps +inner+ with the one-refresh-one-replay 401 policy; call once
      # per client. Token requests keep traveling through +inner+ directly.
      def replay_transport(inner)
        ReplayTransport.new(self, inner)
      end

      # @api private
      # Whether one 401 request qualifies for the replay: it must carry an
      # Authorization value this provider attached on an eligible attach and
      # must not target a lifecycle endpoint.
      def replayable?(request, presented)
        return false unless presented.instance_of?(String) && !presented.empty?
        return false if lifecycle_target?(request.url)
        !served_entry(presented).nil?
      end

      # @api private
      # One coordinated refresh: a newer stored set wins over a stale
      # re-refresh, concurrent 401s share one round, and a failed round fails
      # every waiter exactly once. The round resolves to the fresh complete
      # Authorization value.
      def refresh(presented)
        key = refresh_key
        waiter = nil
        @refresh_lock.synchronize do
          round = @rounds[key]
          if round.nil?
            round = ReplayRound.new
            @rounds[key] = round
            waiter = [:leader, round]
          else
            waiter = [:joined, round]
          end
        end
        kind, round = waiter
        if kind == :leader
          begin
            stored = @store.load(key)
            fresh = if stored && authorization_value(stored) != presented
              authorization_value(stored)
            else
              @store.clear(key)
              credential_value(@plain.call(nil))
            end
          rescue ::Exception => error
            round.fail(error)
            @refresh_lock.synchronize { @rounds.delete(key) }
            raise
          end
          round.complete(fresh)
          @refresh_lock.synchronize { @rounds.delete(key) }
          return fresh
        end
        outcome = round.wait
        raise outcome[1] unless outcome[0] == 'fresh'
        outcome[1]
      end

      def inspect = "#<#{self.class.name} scheme=#{@scheme.inspect}>"

      private

      # The complete Authorization value of one stored set, exactly the value
      # the client's attach path puts on the wire.
      def authorization_value(set)
        credential = set.authorization_credential
        credential.scheme + ' ' + credential.token
      end

      # The complete Authorization value of one freshly minted credential.
      def credential_value(credential)
        credential.scheme + ' ' + credential.token
      end

      # Attaches for stream-protected requirements are never replayed: their
      # requirement pointers are compiled into NO_REPLAY_REQUIREMENTS.
      def replay_eligible?(context)
        return true unless context.respond_to?(:source) && context.source.instance_of?(String)
        @never_replay.none? { |pointer| context.source.end_with?(pointer) }
      end

      def record(value, eligible)
        @record_lock.synchronize do
          @served.unshift({ value: value, eligible: eligible })
          @served.pop while @served.length > 8
        end
        nil
      end

      def served_entry(presented)
        @record_lock.synchronize do
          @served.each { |entry| return entry if entry[:value] == presented && entry[:eligible] }
        end
        nil
      end
    end

    # The replaying credential's client transport: one coordinated refresh
    # and, when the request carried the provider's token and no stream is
    # protected on it, exactly one replay with the fresh token. Lifecycle
    # endpoint requests are never replayed: they carry no bearer token of
    # this provider, and the exact-target guard is defense in depth.
    class ReplayTransport
      def initialize(provider, inner)
        @provider = provider
        @inner = inner
      end

      def exchange(request:, context:)
        presented = nil
        @inner.exchange(request: request, context: context) do |wire|
          presented = authorization_of(request) if wire.status == 401
          if presented && @provider.replayable?(request, presented)
            # The 401 response is discarded unread: the replay carries the
            # fresh token and the caller sees only its response.
            wire.close
          else
            presented = nil
            yield wire
          end
        end
        return nil if presented.nil?
        fresh = @provider.refresh(presented)
        headers = request.headers.reject { |name, _| name.downcase == 'authorization' }
        headers['Authorization'] = fresh
        replayed = PreparedRequest.new(method: request.method, url: request.url, headers: headers,
                                       body: request.body, source: request.source,
                                       operation_id: request.operation_id)
        @inner.exchange(request: replayed, context: context) { |wire| yield wire }
        nil
      end

      def close
        @inner.close if @inner.respond_to?(:close)
        nil
      end

      def closed?
        @inner.respond_to?(:closed?) ? @inner.closed? : false
      end

      def inspect = "#<#{self.class.name}>"

      private

      # The request's Authorization value, whatever its header case.
      def authorization_of(request)
        request.headers.each do |name, value|
          return value if name.downcase == 'authorization'
        end
        nil
      end
    end

    # Builds the replaying provider for one compiled scheme:
    #   credentials = OAuth.replaying_credential('serviceOAuth')
    #   client = Client.new(credential_provider: credentials,
    #                       transport: credentials.replay_transport(my_transport))
    # Every provider argument behaves exactly as in client_credential; the
    # replay semantics are strictly additive and the plain provider keeps
    # today's attach-only semantics.
    def replaying_credential(scheme, client_id: nil, client_secret: nil, store: nil, transport: nil, clock: nil)
      ReplayingCredentialProvider.new(scheme: scheme, client_id: client_id, client_secret: client_secret,
                                      store: store, transport: transport, clock: clock)
    end
"##;

/// The plain variant's creation guard, lifecycle-endpoint exclusion and
/// refresh-key derivation: the compiled client-credentials token URL serves
/// both the store key and the exclusion.
const REPLAY_PLAIN: &str = r##"
    # The compiled client-credentials token URL serves the creation guard, the
    # lifecycle-endpoint exclusion and the refresh-key derivation.
    class ReplayingCredentialProvider
      private

      # Creation refuses a compiled scheme whose client-credentials flow
      # declares no token URL, exactly like the plain provider's first attach.
      def _replay_check(descriptor, scheme)
        return unless descriptor.fetch(:client_credentials).empty?
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
      end

      # Lifecycle endpoints are never replay targets: the exact-target guard
      # is defense in depth against loops.
      def lifecycle_target?(url)
        return true if url == @descriptor.fetch(:client_credentials)
        refresh_url = @descriptor.fetch(:refresh_url)
        !refresh_url.empty? && url == refresh_url
      end

      # The store key the refresh round partitions: exactly the key the plain
      # attach path uses, so a newer stored set wins over a stale re-refresh.
      def refresh_key
        id, = OAuth.resolve(@descriptor, client_id: @client_id, client_secret: @client_secret)
        OAuth.store_key(@scheme, @descriptor.fetch(:token_url), id)
      end
    end
"##;

/// The discovery variant's creation guard, lifecycle-endpoint exclusion and
/// refresh-key derivation: the compiled token URL wins and the discovery
/// document's token endpoint resolves the store key; the resolved endpoint
/// rides the exact-token match and the compiled discovery URL joins the
/// compiled endpoints.
const REPLAY_DISCOVERY: &str = r##"
    # The refresh endpoint and the lifecycle-endpoint exclusion resolve
    # through the compiled precedence: the compiled token URL always wins;
    # otherwise the discovery document's token_endpoint resolves the request.
    class ReplayingCredentialProvider
      private

      # Creation refuses a compiled scheme that declares neither a
      # client-credentials token URL nor a discovery URL, exactly like the
      # plain provider's first attach.
      def _replay_check(descriptor, scheme)
        return unless descriptor.fetch(:client_credentials).empty? && descriptor.fetch(:discovery).empty?
        raise AuthError.new(auth_kind: 'unsupported-flow', auth_scheme: scheme)
      end

      # Lifecycle endpoints are never replay targets: the exact-target guard
      # is defense in depth against loops; the discovery-resolved token
      # endpoint rides the exact-token match.
      def lifecycle_target?(url)
        return true if url == @descriptor.fetch(:client_credentials)
        refresh_url = @descriptor.fetch(:refresh_url)
        return true if !refresh_url.empty? && url == refresh_url
        discovery = @descriptor.fetch(:discovery)
        !discovery.empty? && url == discovery
      end

      # The store key the refresh round partitions: the compiled
      # client-credentials token URL wins; otherwise this provider's cached
      # discovery document supplies the token endpoint.
      def refresh_key
        id, = OAuth.resolve(@descriptor, client_id: @client_id, client_secret: @client_secret)
        OAuth.store_key(@scheme, @plain.token_endpoint, id)
      end
    end
"##;

/// The generated module's RBS signatures, appended to the package signature
/// file only when the lifecycle module is emitted.
pub(super) fn signatures(oauth: &wire::OAuthPlan) -> String {
    let schemes: Vec<Scheme> = oauth.schemes.iter().filter_map(lower).collect();
    // The provider exists exactly when the emitted module defines one: a
    // compiled client-credentials flow or, for discovery-defined schemes, the
    // discovery-aware provider.
    let has_discovery = schemes.iter().any(|scheme| !scheme.discovery.is_empty());
    let has_client_credentials = schemes
        .iter()
        .any(|scheme| !scheme.client_credentials.is_empty())
        || has_discovery;
    let has_authorization = schemes
        .iter()
        .any(|scheme| !scheme.authorization_url.is_empty());
    let has_device = schemes.iter().any(|scheme| !scheme.device_url.is_empty());
    let has_revocation = schemes
        .iter()
        .any(|scheme| !scheme.revocation_url.is_empty());
    let has_introspection = schemes
        .iter()
        .any(|scheme| !scheme.introspection_url.is_empty());

    let mut out = String::from("\n  # Generated OAuth 2.0 / OpenID Connect lifecycle (lib/");
    out.push_str("oauth.rb section compiled from sdk_defaults).\n");
    out.push_str("  module OAuth\n");
    out.push_str(
        r##"    class AuthError < SdkError
      attr_reader auth_kind: String
      attr_reader auth_scheme: String
      attr_reader server_code: String?
      def initialize: (auth_kind: String, auth_scheme: String, ?server_code: String?, ?status: Integer?, ?cause: untyped) -> void
    end
    class TokenSet
      attr_reader access_token: String
      attr_reader token_type: String
      attr_reader expires_at: Float?
      attr_reader refresh_token: String?
      attr_reader scope: String?
      def initialize: (access_token: String, ?token_type: String, ?expires_at: Float?, ?refresh_token: String?, ?scope: String?) -> void
      def expired?: (?now: Float) -> bool
      def refreshable?: () -> bool
      def authorization_credential: () -> AuthorizationCredential
    end
    module TokenStore
      def load: ([String, String, String] key) -> TokenSet?
      def replace: ([String, String, String] key, TokenSet set) -> TokenSet
      def clear: ([String, String, String] key) -> void
    end
    class MemoryTokenStore
      include TokenStore
      def initialize: () -> void
    end
"##,
    );
    if has_client_credentials {
        out.push_str(
            r##"    class ClientCredentialProvider
      attr_reader scheme: String
      def initialize: (scheme: String, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?, ?clock: untyped) -> void
      def call: (CredentialContext _context) -> AuthorizationCredential
      def acquire: () -> TokenSet
    end
"##,
        );
        out.push_str(
            "    def self.client_credential: (String scheme, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?, ?clock: untyped) -> ClientCredentialProvider\n",
        );
        out.push_str(
            "    def self.client_credentials_token: (String scheme, store: TokenStore, ?client_id: String?, ?client_secret: String?, ?transport: _Transport?) -> TokenSet\n",
        );
    }
    // The replaying credential wrapper joins the signature surface exactly
    // when the emitted module defines one: a compiled scheme with an
    // executable client-credentials flow, the provider it wraps.
    if schemes
        .iter()
        .any(|scheme| !scheme.client_credentials.is_empty())
    {
        out.push_str(
            r##"    class ReplayingCredentialProvider
      attr_reader scheme: String
      def initialize: (scheme: String, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?, ?clock: untyped) -> void
      def call: (CredentialContext _context) -> AuthorizationCredential
      def replay_transport: (_Transport inner) -> _Transport
    end
"##,
        );
        out.push_str(
            "    def self.replaying_credential: (String scheme, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?, ?clock: untyped) -> ReplayingCredentialProvider\n",
        );
    }
    out.push_str(
        "    def self.refresh_token_set: (String scheme, TokenSet token, ?client_id: String?, ?client_secret: String?, ?transport: _Transport?, ?store: TokenStore?) -> TokenSet\n",
    );
    if has_authorization {
        out.push_str(
            r##"    class AuthorizationTransaction
      attr_reader scheme: String
      attr_reader authorization_url: String
      attr_reader state: String
      attr_reader created_at: Float
    end
"##,
        );
        out.push_str(
            "    def self.begin_authorization: (String scheme, redirect_uri: String, ?client_id: String?) -> AuthorizationTransaction\n",
        );
        out.push_str(
            "    def self.complete_authorization: (AuthorizationTransaction transaction, Hash[String, String] callback_params, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?) -> TokenSet\n",
        );
    }
    if has_device {
        out.push_str(
            r##"    class DeviceAuthorization
      attr_reader scheme: String
      attr_reader user_code: String
      attr_reader verification_uri: String
      attr_reader verification_uri_complete: String?
      attr_reader expires_at: Float?
      attr_reader interval: Integer
    end
"##,
        );
        out.push_str(
            "    def self.begin_device_authorization: (String scheme, ?client_id: String?, ?transport: _Transport?) -> DeviceAuthorization\n",
        );
        out.push_str(
            "    def self.poll_device_authorization: (DeviceAuthorization device, ?client_id: String?, ?client_secret: String?, ?store: TokenStore?, ?transport: _Transport?, ?wait: ^(Integer | Float) -> void) -> TokenSet\n",
        );
    }
    if has_revocation {
        out.push_str(
            "    def self.revoke: (String scheme, String token_value, ?client_id: String?, ?client_secret: String?, ?transport: _Transport?) -> void\n",
        );
    }
    if has_introspection {
        out.push_str(
            "    def self.introspect: (String scheme, String token_value, ?client_id: String?, ?client_secret: String?, ?transport: _Transport?) -> Hash[String, untyped]\n",
        );
    }
    out.push_str("  end\n");
    out
}
