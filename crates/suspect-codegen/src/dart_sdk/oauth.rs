//! Emitted-only OAuth 2.0 / OpenID Connect token lifecycle for the native Dart
//! HTTP adapter.
//!
//! The shared, generation-time `http_protocol::plan_oauth` outcome lowers into
//! a generated `lib/src/oauth.dart` library part holding the `TokenSet`, the
//! `TokenStore` contract with its instance-owned `MemoryTokenStore`, the typed
//! `AuthException`, frozen per-scheme descriptors and the flow helpers, plus a
//! small conditional transport pair (`oauth_transport_io.dart` for VM builds,
//! `oauth_transport_stub.dart` for portable builds). Static runtime files gain
//! nothing: with no usable compiled scheme the package stays byte-identical.
//! Schemes carrying only deprecated implicit/password flows emit nothing,
//! while a scheme carrying a discovery URL — including OpenID Connect schemes,
//! whose flows a discovery document defines at runtime — is usable and emits
//! the discovery-driven endpoint resolution alongside the compiled
//! descriptors.
//!
//! The runtime never parses OpenAPI: every endpoint is a frozen compiled
//! constant and the environment carries compiled variable names only, with
//! values read at call time. Client-credentials acquisition is skew-aware and
//! single-flight per scheme/token-endpoint/client identity with a re-check of
//! the store after the in-flight future resolves, and the store is replaced
//! atomically, adopting a server-rotated refresh token. Explicit refresh
//! targets the declared refresh URL, else the flow's token URL. Revocation and
//! introspection are emitted only when the compiled scheme carries those
//! configured endpoints. PKCE S256 uses a dependency-free pure-Dart SHA-256,
//! so the pubspec needs no crypto package. Typed `AuthException` values never
//! carry token or secret values, and the discovery engine's failures never
//! carry response body text.
//!
//! Emission is byte-identical for plans without a discovery URL: every emitted
//! section has a plain variant (exactly the pre-discovery bytes) and, when at
//! least one compiled scheme carries a discovery URL, a discovery-aware
//! variant resolving the endpoints the compiled plan omits through RFC 8414 /
//! OpenID Connect discovery, with the compiled precedence compiled >
//! discovered > flow defaults.

use super::Plan;
use crate::http_protocol::{
    CredentialHook, OAuthClientAuth, OAuthFlowDescriptorKind, OAuthPlan, OAuthSchemeKind,
    OAuthSchemePlan, Representation,
};
use std::collections::{BTreeMap, BTreeSet};

/// The compiled flow kind names emitted into the frozen descriptors.
fn flow_kind(kind: OAuthFlowDescriptorKind) -> &'static str {
    match kind {
        OAuthFlowDescriptorKind::Implicit => "implicit",
        OAuthFlowDescriptorKind::Password => "password",
        OAuthFlowDescriptorKind::ClientCredentials => "client-credentials",
        OAuthFlowDescriptorKind::AuthorizationCode => "authorization-code",
        OAuthFlowDescriptorKind::DeviceAuthorization => "device-authorization",
    }
}

/// The compiled client-authentication names emitted into the frozen descriptors.
fn client_auth(kind: OAuthClientAuth) -> &'static str {
    match kind {
        OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
        OAuthClientAuth::None => "none",
    }
}

/// The compiled scheme kinds emitted into the frozen descriptors.
fn scheme_kind(kind: OAuthSchemeKind) -> &'static str {
    match kind {
        OAuthSchemeKind::OAuth2 => "oauth2",
        OAuthSchemeKind::OpenIdConnect => "open-id-connect",
    }
}

/// Every public name the generated part declares; reserved against model and
/// operation names by refusing to emit over a collision. The document-GET
/// transport name is reserved only when discovery participates, so
/// discovery-less plans keep their exact refusal surface.
const EXPORTED_NAMES: &[&str] = &[
    "TokenSet",
    "TokenStore",
    "MemoryTokenStore",
    "AuthException",
    "CompiledFlow",
    "CompiledScheme",
    "AuthorizationTransaction",
    "AuthorizationBegin",
    "OAuthTransport",
    "oauthSchemes",
    "tokenStoreKey",
    "clientCredentialsToken",
    "createClientCredentialsProvider",
    "createRefreshProvider",
    "refreshToken",
    "beginAuthorization",
    "completeAuthorization",
];

/// The usable subset: schemes carrying at least one executable (non-deprecated)
/// flow or a discovery URL. Deprecated implicit/password flows are represented
/// in their scheme's frozen descriptor but never execute; a scheme with only
/// those flows and no discovery URL contributes nothing, while a
/// discovery-defined scheme — an OpenID Connect scheme, for instance — has its
/// endpoints defined by the discovery document at runtime.
pub(super) fn usable(plan: &OAuthPlan) -> Vec<&OAuthSchemePlan> {
    plan.schemes
        .iter()
        .filter(|scheme| {
            scheme.discovery.is_some()
                || scheme.flows.iter().any(|flow| !flow.deprecated_flow)
        })
        .collect()
}

fn has_discovery_among(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| scheme.discovery.is_some())
}

fn has_revocation(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| scheme.revocation_endpoint.is_some())
}

fn has_introspection(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| scheme.introspection_endpoint.is_some())
}

fn has_device(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| scheme.flows.iter().any(|flow| flow.kind == OAuthFlowDescriptorKind::DeviceAuthorization && !flow.deprecated_flow))
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so schemes without one compile
/// exactly the pre-replay bytes.
fn has_client_credentials(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        scheme
            .flows
            .iter()
            .any(|flow| flow.kind == OAuthFlowDescriptorKind::ClientCredentials && !flow.deprecated_flow)
    })
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget. The attach request carries the operation's
/// source (document and pointer) rather than the requirement's, so the
/// compiled values carry the requirement's full document-and-pointer source
/// and the generated runtime resolves the attach by its operation prefix.
fn no_replay_requirements(
    operations: &[super::PlannedOperation],
    plan: &OAuthPlan,
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = plan.schemes.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire.responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire.security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if names.contains(requirement.name()) {
                    let source = requirement.source().source();
                    pointers
                        .entry(requirement.name().to_owned())
                        .or_default()
                        .insert(format!("{}#{}", source.document().as_str(), source.pointer()));
                }
            }
        }
    }
    pointers
}

/// The generated part content, without the `part of` header, plus the flags
/// the conditional transport pair needs. `None` when no usable scheme
/// compiles or when a fixed exported name collides with an allocated model or
/// operation name — the package then stays byte-identical to the unconfigured
/// generation. Plans without a discovery URL assemble byte-identically to the
/// pre-discovery emission; plans with one emit the discovery-aware providers
/// and the discovery engine.
pub(super) struct Emission {
    /// The generated part content, without the `part of` header.
    pub(super) part: String,
    /// Whether at least one compiled scheme carries a discovery URL; the
    /// conditional transport pair then also emits the document GET.
    pub(super) discovery: bool,
}

pub(super) fn emission(plan: &Plan) -> Option<Emission> {
    let oauth_plan = plan.oauth()?;
    let schemes = usable(oauth_plan);
    if schemes.is_empty() {
        return None;
    }
    let discovery = has_discovery_among(&schemes);
    let replay = has_client_credentials(oauth_plan);
    let used = plan.models().used_names();
    let collision = EXPORTED_NAMES
        .iter()
        .chain(if discovery {
            &["OAuthGetTransport"][..]
        } else {
            &[][..]
        })
        .chain(if has_device(&schemes) {
            &["beginDeviceAuthorization"][..]
        } else {
            &[][..]
        })
        .chain(if has_revocation(&schemes) {
            &["revokeToken"][..]
        } else {
            &[][..]
        })
        .chain(if has_introspection(&schemes) {
            &["introspectToken"][..]
        } else {
            &[][..]
        })
        .chain(if replay {
            &["createReplayingCredentialsProvider", "ReplayingCredentials"][..]
        } else {
            &[][..]
        })
        .find(|name| used.contains(**name));
    if collision.is_some() {
        return None;
    }
    let mut part = String::from(CORE_PREFIX);
    part.push_str(if discovery {
        CORE_HEADER_DISCOVERY
    } else {
        CORE_HEADER_PLAIN
    });
    part.push_str(CORE_MID);
    part.push_str(if discovery {
        "/// Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; a compiled discovery URL resolves the endpoint URLs the compiled flows omit at call time. The runtime never parses OpenAPI.\n"
    } else {
        "/// Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; OpenID Connect schemes compile no executable flows in v1 and emit nothing. The runtime never parses OpenAPI and never fetches discovery documents.\n"
    });
    part.push_str("final Map<String, CompiledScheme> oauthSchemes = Map.unmodifiable(<String, CompiledScheme>{\n");
    for scheme in &schemes {
        part.push_str(&scheme_entry(scheme));
    }
    part.push_str("});\n");
    part.push_str(if discovery {
        API_CC_DISCOVERY
    } else {
        API_CC
    });
    part.push_str(if discovery {
        API_REFRESH_DISCOVERY
    } else {
        API_REFRESH
    });
    part.push_str(API_CODE);
    if has_revocation(&schemes) {
        part.push_str(if discovery {
            REVOKE_DISCOVERY
        } else {
            REVOKE
        });
    }
    if has_introspection(&schemes) {
        part.push_str(if discovery {
            INTROSPECT_DISCOVERY
        } else {
            INTROSPECT
        });
    }
    if has_device(&schemes) {
        part.push_str(DEVICE);
    }
    if discovery {
        part.push_str(DISCOVERY);
    }
    if replay {
        part.push_str(&replay_section(plan, oauth_plan, discovery));
    }
    Some(Emission { part, discovery })
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries a non-deprecated client-credentials flow, and its
/// plain and discovery variants resolve the refresh endpoint and the
/// lifecycle-endpoint exclusion through the same compiled precedence as the
/// provider they wrap.
fn replay_section(plan: &Plan, oauth: &OAuthPlan, discovery: bool) -> String {
    let no_replay = no_replay_requirements(plan.operations(), oauth);
    let mut code = String::from(
        "\n/// Compiled stream-protected requirements: security-requirement source\n/// pointers whose attaches are never replayed, because delivered stream data\n/// prevents a transparent restart.\nfinal Map<String, Set<String>> _noReplayRequirements = <String, Set<String>>{\n",
    );
    for scheme in &oauth.schemes {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| super::emit::quote(pointer))
            .collect::<Vec<_>>()
            .join(", ");
        code.push_str(&format!(
            "  {}: <String>{{{}}},\n",
            super::emit::quote(&scheme.name),
            rendered
        ));
    }
    code.push_str("};\n");
    code.push_str(REPLAY_CREDENTIAL);
    code.push_str(REPLAY_TRANSPORT);
    code.push_str(if discovery {
        REPLAY_FACTORY_DISCOVERY
    } else {
        REPLAY_FACTORY_PLAIN
    });
    code
}

fn optional(value: &Option<String>) -> String {
    value
        .as_ref()
        .map_or_else(|| "null".to_owned(), |value| super::emit::quote(value))
}

/// One scheme's frozen descriptor entry, carrying exactly the compiled plan.
fn scheme_entry(scheme: &OAuthSchemePlan) -> String {
    let flows = scheme
        .flows
        .iter()
        .map(|flow| {
            let scopes = flow
                .scopes
                .iter()
                .map(|(name, description)| {
                    format!("{}: {}, ", super::emit::quote(name), super::emit::quote(description))
                })
                .collect::<String>();
            format!(
                "    CompiledFlow({}, {}, {}, {}, {}, {}, {}, const <String, String>{{{}}}),\n",
                super::emit::quote(flow_kind(flow.kind)),
                optional(&flow.authorization_url),
                optional(&flow.token_url),
                optional(&flow.refresh_url),
                optional(&flow.device_authorization_url),
                super::emit::quote(client_auth(flow.client_auth)),
                flow.deprecated_flow,
                scopes,
            )
        })
        .collect::<String>();
    format!(
        "  {}: CompiledScheme({}, {}, {}, {}, {}, {}, {}, {}, <CompiledFlow>[\n{}  ]),\n",
        super::emit::quote(&scheme.name),
        super::emit::quote(&scheme.name),
        super::emit::quote(scheme_kind(scheme.kind)),
        scheme.refresh_skew_seconds,
        optional(&scheme.discovery),
        optional(&scheme.revocation_endpoint),
        optional(&scheme.introspection_endpoint),
        optional(&scheme.client_id_env),
        optional(&scheme.client_secret_env),
        flows,
    )
}

/// The static core half of the generated part: the types, the typed error, the
/// token store and the request plumbing. The header paragraph after "This part
/// never parses" is selected by `emission`; without discovery the three core
/// constants concatenate into exactly the pre-discovery bytes.
const CORE_PREFIX: &str = r#"// Generated OAuth 2.0 lifecycle for this package's compiled OAuth schemes.
//
// The frozen `oauthSchemes` descriptors compile the generation-time OAuth
// plan: endpoint URLs, per-flow client authentication, refresh skew,
// environment variable names and scope metadata are exactly what the source
// declared plus the explicitly configured supplements. This part never parses
"#;

/// The header paragraph after "This part never parses": byte-exact for plans
/// without a discovery URL, and the discovery paragraph for plans with one.
const CORE_HEADER_PLAIN: &str = r#"// OpenAPI and never fetches discovery documents.
"#;
const CORE_HEADER_DISCOVERY: &str = r#"// OpenAPI; endpoint URLs that the compiled flows omit resolve through
// RFC 8414 / OpenID Connect discovery when the scheme compiles a discovery
// URL.
"#;

const CORE_MID: &str = r#"//
// Token requests are form-encoded (RFC 6749). Client authentication follows
// the compiled per-flow policy: `client-secret-basic` sends HTTP Basic on the
// request; `none` is the public profile and never sends a secret. Access
// tokens, refresh tokens and client secrets never appear in error messages;
// AuthException carries only safe metadata (kind, scheme, status, server
// error code, retry hint).
//
// The operation runtime neither acquires nor refreshes tokens and never
// retries requests. Providers here integrate as ordinary caller credentials:
// pass the returned callback as the scheme's credential member. On-demand
// refresh hook: when a protected call still fails with a declared 401, call
// `refreshToken` explicitly and retry.

/// One acquired token set. `expiresAt` is the epoch millisecond at which the
/// access token expires; a response without `expires_in` never expires.
/// Freshness checks additionally apply the compiled refresh skew.
final class TokenSet {
  const TokenSet({required this.accessToken, this.tokenType = 'Bearer', this.expiresAt = _neverExpires, this.refreshToken, this.scope});
  final String accessToken;
  final String tokenType;
  /// Epoch milliseconds of expiry; [_neverExpires] when the response declared none.
  final int expiresAt;
  final String? refreshToken;
  final String? scope;
  @override
  String toString() => 'TokenSet(redacted)';
}

/// An expiry far beyond any real clock, kept finite so web builds stay exact.
const int _neverExpires = 0x1FFFFFFFFFFFFF;

/// Caller-implementable token persistence. Keys partition stored token sets by
/// scheme, token-endpoint issuer and client identity (see tokenStoreKey);
/// values are whole token sets replaced atomically.
abstract interface class TokenStore {
  Future<TokenSet?> load(String key);
  Future<void> replace(String key, TokenSet tokenSet);
  Future<void> clear(String key);
}

/// In-process token store owned by the provider or caller that created it; the
/// library never keeps a global store. Dart's single-event-loop execution
/// makes the plain map operations atomic between awaits.
final class MemoryTokenStore implements TokenStore {
  final Map<String, TokenSet> _tokens = <String, TokenSet>{};
  @override
  Future<TokenSet?> load(String key) async => _tokens[key];
  @override
  Future<void> replace(String key, TokenSet tokenSet) async {
    _tokens[key] = tokenSet;
  }

  @override
  Future<void> clear(String key) async {
    _tokens.remove(key);
  }
}

/// Typed OAuth lifecycle failure. Messages never contain token or secret
/// values; fields carry only safe metadata (kind, scheme, status, server error
/// code, retry hint).
final class AuthException implements Exception {
  const AuthException(this.kind, this.scheme, this.message, {this.status, this.serverError, this.retryAfterSeconds});
  final String kind;
  final String scheme;
  final String message;
  final int? status;
  final String? serverError;
  final int? retryAfterSeconds;
  @override
  String toString() => 'AuthException($kind, $scheme, $message)';
}

/// One compiled flow descriptor: exactly what the source declared.
final class CompiledFlow {
  const CompiledFlow(this.kind, this.authorizationUrl, this.tokenUrl, this.refreshUrl, this.deviceAuthorizationUrl, this.clientAuth, this.deprecated, this.scopes);
  final String kind;
  final String? authorizationUrl;
  final String? tokenUrl;
  final String? refreshUrl;
  final String? deviceAuthorizationUrl;
  final String clientAuth;
  final bool deprecated;
  final Map<String, String> scopes;
}

/// One compiled scheme descriptor: exactly what the source declared plus the
/// explicitly configured supplements.
final class CompiledScheme {
  const CompiledScheme(this.name, this.kind, this.refreshSkewSeconds, this.discovery, this.revocationEndpoint, this.introspectionEndpoint, this.clientIdEnv, this.clientSecretEnv, this.flows);
  final String name;
  final String kind;
  final int refreshSkewSeconds;
  final String? discovery;
  final String? revocationEndpoint;
  final String? introspectionEndpoint;
  final String? clientIdEnv;
  final String? clientSecretEnv;
  final List<CompiledFlow> flows;
}

/// One OAuth endpoint exchange response: status, response headers and body.
/// The default implementations live in `oauth_transport_io.dart` (VM builds)
/// and `oauth_transport_stub.dart` (portable builds); import one of those
/// libraries to construct a response in a caller-injected transport.
typedef OAuthTransport = Future<_oauth_transport.OAuthEndpointResponse> Function(Uri url, Map<String, String> headers, String body);

/// The exact token-store key for one scheme, token-endpoint issuer and client
/// identity. Identical inputs always yield identical keys; stored token sets
/// are partitioned by all three.
String tokenStoreKey(String scheme, String issuer, String? clientId) => '$scheme|$issuer|${clientId ?? 'public'}';

OAuthTransport get _defaultTransport => _oauth_transport.oauthEndpointPost;
int _systemClock() => DateTime.now().toUtc().millisecondsSinceEpoch;
final Map<String, Future<TokenSet>> _inflight = <String, Future<TokenSet>>{};
final Set<AuthorizationTransaction> _consumedTransactions = <AuthorizationTransaction>{};

CompiledScheme _compiled(String scheme) {
  final found = oauthSchemes[scheme];
  if (found == null) {
    throw AuthException('endpoint-unavailable', scheme, 'no compiled OAuth scheme carries that name; oauth compiles exactly the source-declared schemes with executable flows');
  }
  return found;
}

CompiledFlow _executableFlow(CompiledScheme scheme, String kind) {
  for (final candidate in scheme.flows) {
    if (candidate.kind == kind && !candidate.deprecated) {
      return candidate;
    }
  }
  throw AuthException('endpoint-unavailable', scheme.name, 'scheme ${scheme.name} has no executable $kind flow in its source declaration');
}

/// The refresh endpoint: the declared refresh URL, else the flow's token URL.
String _refreshEndpoint(CompiledFlow grant, CompiledScheme scheme) => grant.refreshUrl ?? grant.tokenUrl ?? (throw AuthException('endpoint-unavailable', scheme.name, 'the compiled flow declares neither a refresh URL nor a token URL'));

/// Reads one compiled environment variable name at call time; generation
/// supplied the name only.
String? _environmentValue(String? variable) => variable == null ? null : _environment.readVariable(variable);

/// Client identity for one request: explicit arguments win, then the compiled
/// environment variable names.
(String?, String?) _resolveIdentity(CompiledScheme scheme, String? clientId, String? clientSecret) => (clientId ?? _environmentValue(scheme.clientIdEnv), clientSecret ?? _environmentValue(scheme.clientSecretEnv));

/// Client authentication follows the compiled per-flow policy:
/// `client-secret-basic` sends HTTP Basic; `none` is the public profile and
/// never sends a secret.
(bool, String?, String?) _clientAuth(CompiledFlow grant, (String?, String?) identity) => (grant.clientAuth == 'client-secret-basic', identity.$1, grant.clientAuth == 'client-secret-basic' ? identity.$2 : null);

bool _isTokenChar(int c) => c >= 97 && c <= 122 || c >= 65 && c <= 90 || c >= 48 && c <= 57 || const [33, 35, 36, 37, 38, 39, 42, 43, 45, 46, 94, 95, 96, 124, 126].contains(c);
bool _usableSecret(String value) => value.isNotEmpty && value.length <= 8192 && value.codeUnits.every((c) => c >= 32 && c != 127);

/// Freshness applies the compiled refresh skew: a token is fresh when it
/// outlives now by more than the skew.
bool _isFresh(TokenSet tokenSet, int skewMilliseconds, int now) => tokenSet.expiresAt - skewMilliseconds > now;

/// Builds the complete Authorization value for one stored token set. Values
/// never appear in errors.
AuthorizationCredential _authorization(TokenSet tokenSet, String scheme) {
  final type = tokenSet.tokenType.trim();
  if (type.isEmpty || !type.codeUnits.every(_isTokenChar)) {
    throw AuthException('invalid-response', scheme, 'the token type is not a usable authorization scheme');
  }
  if (!_usableSecret(tokenSet.accessToken)) {
    throw AuthException('invalid-response', scheme, 'the access token is not a usable credential value');
  }
  return AuthorizationCredential('$type ${tokenSet.accessToken}');
}

/// Atomic store replacement: a server-rotated refresh token is adopted; when
/// the response carries none, the previous refresh token is retained.
TokenSet _adopt(TokenSet? previous, TokenSet next) {
  if (next.refreshToken != null || previous == null || previous.refreshToken == null) {
    return next;
  }
  return TokenSet(accessToken: next.accessToken, tokenType: next.tokenType, expiresAt: next.expiresAt, refreshToken: previous.refreshToken, scope: next.scope);
}

int? _retryAfter(_oauth_transport.OAuthEndpointResponse response) {
  final values = response.headers['retry-after'];
  if (values == null || values.isEmpty) {
    return null;
  }
  final parsed = int.tryParse(values.first.trim());
  return parsed != null && parsed >= 0 ? parsed : null;
}

String _serverErrorKind(String? serverError) => switch (serverError) {
  'invalid_request' => 'invalid-request',
  'invalid_client' => 'invalid-client',
  'invalid_grant' => 'invalid-grant',
  'unauthorized_client' => 'unauthorized-client',
  'unsupported_grant_type' => 'unsupported-grant-type',
  'invalid_scope' => 'invalid-scope',
  'authorization_pending' => 'authorization-pending',
  'slow_down' => 'slow-down',
  'expired_token' => 'device-code-expired',
  _ => 'server-error',
};

/// Maps one rejected endpoint response to the typed error; the message carries
/// only the machine error code, status and scheme.
AuthException _serverFailure(String scheme, _oauth_transport.OAuthEndpointResponse response) {
  String? serverError;
  try {
    final decoded = jsonDecode(response.body);
    if (decoded is Map<String, dynamic>) {
      final value = decoded['error'];
      if (value is String && value.isNotEmpty) {
        serverError = value;
      }
    }
  } on Object {
    // An unreadable error body is safe metadata loss.
  }
  final label = serverError == null ? 'HTTP ${response.status}' : '$serverError, HTTP ${response.status}';
  return AuthException(_serverErrorKind(serverError), scheme, 'the authorization server rejected the request ($label)', status: response.status, serverError: serverError, retryAfterSeconds: _retryAfter(response));
}

/// One form-encoded endpoint POST with the compiled client authentication; a
/// non-2xx response becomes the typed error.
Future<String> _postForm(String scheme, String endpoint, (bool, String?, String?) auth, Map<String, String> fields, OAuthTransport transport) async {
  final headers = <String, String>{'accept': 'application/json'};
  if (auth.$1) {
    final secret = auth.$3;
    if (secret == null) {
      throw AuthException('missing-credential', scheme, 'the compiled client authentication is client-secret-basic and no client secret is available');
    }
    headers['authorization'] = 'Basic ${base64Encode(utf8.encode('${auth.$2 ?? ''}:$secret'))}';
  }
  final body = fields.entries.map((entry) => '${Uri.encodeQueryComponent(entry.key)}=${Uri.encodeQueryComponent(entry.value)}').join('&');
  _oauth_transport.OAuthEndpointResponse response;
  try {
    response = await transport(Uri.parse(endpoint), headers, body);
  } on AuthException {
    rethrow;
  } on Object {
    throw AuthException('transport-failure', scheme, 'the endpoint request failed before a response arrived');
  }
  if (response.status < 200 || response.status > 299) {
    throw _serverFailure(scheme, response);
  }
  return response.body;
}

Map<String, dynamic> _jsonObject(String scheme, String body, String what) {
  Object? decoded;
  try {
    decoded = jsonDecode(body);
  } on Object {
    throw AuthException('invalid-response', scheme, 'the $what response is not readable JSON');
  }
  if (decoded is! Map<String, dynamic>) {
    throw AuthException('invalid-response', scheme, 'the $what response is not a JSON object');
  }
  return decoded;
}

TokenSet _tokenSetFrom(String scheme, Map<String, dynamic> body, int now) {
  final accessToken = body['access_token'];
  if (accessToken is! String || !_usableSecret(accessToken)) {
    throw AuthException('invalid-response', scheme, 'the token response carries no usable access token');
  }
  final declaredType = body['token_type'];
  final tokenType = declaredType is String && declaredType.trim().isNotEmpty ? declaredType.trim() : 'Bearer';
  final declaredExpires = body['expires_in'];
  final expiresAt = declaredExpires is num && declaredExpires > 0 && declaredExpires.isFinite ? now + (declaredExpires * 1000).round() : _neverExpires;
  final declaredRefresh = body['refresh_token'];
  final refreshToken = declaredRefresh is String && declaredRefresh.isNotEmpty ? declaredRefresh : null;
  final declaredScope = body['scope'];
  final scope = declaredScope is String && declaredScope.isNotEmpty ? declaredScope : null;
  return TokenSet(accessToken: accessToken, tokenType: tokenType, expiresAt: expiresAt, refreshToken: refreshToken, scope: scope);
}

Future<TokenSet> _tokenRequest(String scheme, String endpoint, (bool, String?, String?) auth, Map<String, String> fields, OAuthTransport transport, int Function() clock) async {
  final body = _jsonObject(scheme, await _postForm(scheme, endpoint, auth, fields, transport), 'token');
  return _tokenSetFrom(scheme, body, clock());
}

/// Dependency-free SHA-256 (FIPS 180-4) for the PKCE S256 challenge, so the
/// generated package needs no crypto dependency.
const List<int> _sha256K = [0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2];

int _rotr32(int value, int bits) => ((value >>> bits) | (value << (32 - bits))) & 0xFFFFFFFF;

List<int> _sha256(List<int> message) {
  final bitLength = message.length * 8;
  final padded = List<int>.of(message)..add(0x80);
  while (padded.length % 64 != 56) {
    padded.add(0);
  }
  for (var i = 7; i >= 0; i--) {
    padded.add(i >= 4 ? 0 : (bitLength >>> (i * 8)) & 0xFF);
  }
  var h0 = 0x6a09e667, h1 = 0xbb67ae85, h2 = 0x3c6ef372, h3 = 0xa54ff53a;
  var h4 = 0x510e527f, h5 = 0x9b05688c, h6 = 0x1f83d9ab, h7 = 0x5be0cd19;
  for (var block = 0; block < padded.length; block += 64) {
    final w = List<int>.filled(64, 0);
    for (var i = 0; i < 16; i++) {
      w[i] = ((padded[block + i * 4] << 24) | (padded[block + i * 4 + 1] << 16) | (padded[block + i * 4 + 2] << 8) | padded[block + i * 4 + 3]) & 0xFFFFFFFF;
    }
    for (var i = 16; i < 64; i++) {
      final s0 = _rotr32(w[i - 15], 7) ^ _rotr32(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      final s1 = _rotr32(w[i - 2], 17) ^ _rotr32(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) & 0xFFFFFFFF;
    }
    var a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7;
    for (var i = 0; i < 64; i++) {
      final s1 = _rotr32(e, 6) ^ _rotr32(e, 11) ^ _rotr32(e, 25);
      final ch = (e & f) ^ (~e & g);
      final temp1 = (h + s1 + ch + _sha256K[i] + w[i]) & 0xFFFFFFFF;
      final s0 = _rotr32(a, 2) ^ _rotr32(a, 13) ^ _rotr32(a, 22);
      final maj = (a & b) ^ (a & c) ^ (b & c);
      final temp2 = (s0 + maj) & 0xFFFFFFFF;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) & 0xFFFFFFFF;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) & 0xFFFFFFFF;
    }
    h0 = (h0 + a) & 0xFFFFFFFF;
    h1 = (h1 + b) & 0xFFFFFFFF;
    h2 = (h2 + c) & 0xFFFFFFFF;
    h3 = (h3 + d) & 0xFFFFFFFF;
    h4 = (h4 + e) & 0xFFFFFFFF;
    h5 = (h5 + f) & 0xFFFFFFFF;
    h6 = (h6 + g) & 0xFFFFFFFF;
    h7 = (h7 + h) & 0xFFFFFFFF;
  }
  final words = [h0, h1, h2, h3, h4, h5, h6, h7];
  final digest = List<int>.filled(32, 0);
  for (var i = 0; i < 8; i++) {
    digest[i * 4] = (words[i] >>> 24) & 0xFF;
    digest[i * 4 + 1] = (words[i] >>> 16) & 0xFF;
    digest[i * 4 + 2] = (words[i] >>> 8) & 0xFF;
    digest[i * 4 + 3] = words[i] & 0xFF;
  }
  return digest;
}

String _s256Challenge(String verifier) => base64Url.encode(_sha256(utf8.encode(verifier))).replaceAll('=', '');

final Random _random = Random.secure();
String _randomBase64Url(int byteLength) {
  final bytes = List<int>.generate(byteLength, (_) => _random.nextInt(256));
  return base64Url.encode(bytes).replaceAll('=', '');
}
"#;

/// The public client-credentials half of the generated part: the credential
/// provider, the one-shot acquisition and its single-flight plumbing.
/// Discovery-aware plans emit API_CC_DISCOVERY instead.
const API_CC: &str = r#"
/// Creates a caller credential for the compiled client-credentials flow: on
/// attach it serves a fresh token from the provider-owned store, otherwise it
/// acquires one with a single form-encoded token request. Concurrent attaches
/// share one in-flight acquisition per scheme, token endpoint and client
/// identity, the store is re-checked after the in-flight future resolves and
/// replaced atomically, and a response refresh token is adopted, else a
/// previous one retained. Pass the returned callback as the scheme's
/// credential member.
CredentialProvider createClientCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport}) {
  final tokenStore = store ?? MemoryTokenStore();
  return (request) async {
    final tokenSet = await clientCredentialsToken(scheme, clientId: clientId, clientSecret: clientSecret, scope: scope, store: tokenStore, clock: clock, transport: transport);
    return _authorization(tokenSet, scheme);
  };
}

/// Acquires a token with the compiled client-credentials flow: a fresh stored
/// token is served, otherwise one form-encoded token request runs. Concurrent
/// callers share one in-flight acquisition per scheme, token endpoint and
/// client identity; the store is replaced atomically and a response refresh
/// token is adopted, else a previous one retained. Without an explicit [store]
/// the call acquires without persistence, because the library never keeps a
/// global store; the provider factories own one store per provider.
Future<TokenSet> clientCredentialsToken(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport}) async {
  final compiled = _compiled(scheme);
  final grant = _executableFlow(compiled, 'client-credentials');
  final tokenUrl = grant.tokenUrl ?? (throw AuthException('endpoint-unavailable', compiled.name, 'the compiled client-credentials flow declares no token URL'));
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final key = tokenStoreKey(compiled.name, tokenUrl, identity.$1);
  final pending = _inflight[key];
  if (pending != null) {
    return pending;
  }
  final tracked = _acquireClientCredentials(compiled, grant, tokenUrl, identity, scope, store, clock ?? _systemClock, transport ?? _defaultTransport);
  _inflight[key] = tracked;
  try {
    return await tracked;
  } finally {
    if (identical(_inflight[key], tracked)) {
      _inflight.remove(key);
    }
  }
}

Future<TokenSet> _acquireClientCredentials(CompiledScheme scheme, CompiledFlow grant, String tokenUrl, (String?, String?) identity, String? scope, TokenStore? store, int Function() clock, OAuthTransport transport) async {
  final key = tokenStoreKey(scheme.name, tokenUrl, identity.$1);
  final stored = store == null ? null : await store.load(key);
  if (stored != null && _isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) {
    return stored;
  }
  final fields = <String, String>{'grant_type': 'client_credentials'};
  if (scope != null) {
    fields['scope'] = scope;
  }
  if (grant.clientAuth == 'none' && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  final acquired = await _tokenRequest(scheme.name, tokenUrl, _clientAuth(grant, identity), fields, transport, clock);
  final adopted = _adopt(stored, acquired);
  await store?.replace(key, adopted);
  return adopted;
}
"#;

/// The public refresh half of the generated part: the store-backed credential
/// provider, the one-shot stored-token refresh and the explicit refresh
/// exchange. Discovery-aware plans emit API_REFRESH_DISCOVERY instead.
const API_REFRESH: &str = r#"
/// Creates a caller credential that serves stored tokens and refreshes them on
/// demand: on attach a fresh stored token is returned; an expired one is
/// refreshed exactly once with its stored refresh token before serving. There
/// is no token at all until an authorization or device flow has completed.
/// Automatic retry hook: when a protected call still fails with a declared
/// 401, call `refreshToken` explicitly and retry; the SDK itself never retries.
CredentialProvider createRefreshProvider(String scheme, {String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport}) {
  final tokenStore = store ?? MemoryTokenStore();
  return (request) async {
    final tokenSet = await _storedToken(scheme, clientId: clientId, clientSecret: clientSecret, store: tokenStore, clock: clock, transport: transport);
    return _authorization(tokenSet, scheme);
  };
}

/// The flow whose token/refresh endpoints serve refreshes: the
/// authorization-code flow when compiled, else the first executable flow with
/// a token URL. Compiled-only: a discovery-defined scheme's refresh endpoint
/// resolves through the discovery document instead, so the discovery-aware
/// refresh half never emits this helper.
CompiledFlow _refreshFlow(CompiledScheme scheme) {
  CompiledFlow? fallback;
  for (final candidate in scheme.flows) {
    if (candidate.deprecated) {
      continue;
    }
    if (candidate.kind == 'authorization-code') {
      return candidate;
    }
    if (fallback == null && candidate.tokenUrl != null) {
      fallback = candidate;
    }
  }
  return fallback ?? (throw AuthException('endpoint-unavailable', scheme.name, 'the compiled scheme carries no executable flow with a token URL'));
}

Future<TokenSet> _performRefresh(CompiledScheme scheme, CompiledFlow grant, String endpoint, (String?, String?) identity, String refreshToken, TokenSet? previous, TokenStore? store, OAuthTransport transport, int Function() clock) async {
  final fields = <String, String>{'grant_type': 'refresh_token', 'refresh_token': refreshToken};
  if (grant.clientAuth == 'none' && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  final acquired = await _tokenRequest(scheme.name, endpoint, _clientAuth(grant, identity), fields, transport, clock);
  final adopted = _adopt(previous, acquired);
  await store?.replace(tokenStoreKey(scheme.name, endpoint, identity.$1), adopted);
  return adopted;
}

Future<TokenSet> _storedToken(String scheme, {String? clientId, String? clientSecret, required TokenStore store, int Function()? clock, OAuthTransport? transport}) async {
  final compiled = _compiled(scheme);
  final grant = _refreshFlow(compiled);
  final endpoint = _refreshEndpoint(grant, compiled);
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final resolvedClock = clock ?? _systemClock;
  final stored = await store.load(tokenStoreKey(compiled.name, endpoint, identity.$1));
  if (stored == null) {
    throw AuthException('missing-credential', compiled.name, 'no stored token set exists for this scheme; complete an authorization or device flow first');
  }
  if (_isFresh(stored, compiled.refreshSkewSeconds * 1000, resolvedClock())) {
    return stored;
  }
  if (stored.refreshToken == null) {
    throw AuthException('missing-credential', compiled.name, 'the stored token set carries no refresh token');
  }
  return _performRefresh(compiled, grant, endpoint, identity, stored.refreshToken!, stored, store, transport ?? _defaultTransport, resolvedClock);
}

/// Exchanges one refresh token at the declared refresh URL or token URL
/// (grant_type=refresh_token) and returns the token set. A rotated refresh
/// token from the response is adopted; when the response carries none and a
/// previous stored set exists, the previous refresh token is retained.
Future<TokenSet> refreshToken(String scheme, {required String refreshToken, String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport}) async {
  if (!_usableSecret(refreshToken)) {
    throw AuthException('invalid-request', scheme, 'refreshToken must be a nonempty string without control characters');
  }
  final compiled = _compiled(scheme);
  final grant = _refreshFlow(compiled);
  final endpoint = _refreshEndpoint(grant, compiled);
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final resolvedClock = clock ?? _systemClock;
  final previous = store == null ? null : await store.load(tokenStoreKey(compiled.name, endpoint, identity.$1));
  return _performRefresh(compiled, grant, endpoint, identity, refreshToken, previous, store, transport ?? _defaultTransport, resolvedClock);
}
"#;

/// The shared authorization-code half of the generated part. Its members are
/// compiled-only: a discovery-defined scheme's authorization endpoints live in
/// the discovery document and a discovery-only scheme refuses with the typed
/// endpoint-unavailable error, exactly like the other compiled-only grants.
const API_CODE: &str = r#"
/// A bound authorization-code transaction: session-scoped and consumed exactly
/// once by completeAuthorization, whether the exchange succeeds or fails.
final class AuthorizationTransaction {
  const AuthorizationTransaction({required this.scheme, required this.state, required this.codeVerifier, required this.redirectUri, required this.tokenUrl, required this.createdAt});
  final String scheme;
  final String state;
  final String codeVerifier;
  final String redirectUri;
  final String tokenUrl;
  final int createdAt;
}

/// The authorization redirect target plus the bound transaction.
final class AuthorizationBegin {
  const AuthorizationBegin(this.authorizationUrl, this.state, this.transaction);
  final String authorizationUrl;
  final String state;
  final AuthorizationTransaction transaction;
}

/// Starts an authorization-code flow with PKCE (S256): generates the random
/// state and code verifier with `Random.secure`, computes the S256 challenge
/// with the dependency-free SHA-256 and returns the exact authorization URL to
/// redirect to plus the bound transaction. No network request is made.
AuthorizationBegin beginAuthorization(String scheme, {required String redirectUri, String? clientId, List<String>? scopes, int Function()? clock}) {
  final compiled = _compiled(scheme);
  final grant = _executableFlow(compiled, 'authorization-code');
  final authorizationEndpoint = grant.authorizationUrl ?? (throw AuthException('endpoint-unavailable', compiled.name, 'the compiled authorization-code flow declares no authorization URL'));
  final tokenUrl = grant.tokenUrl ?? (throw AuthException('endpoint-unavailable', compiled.name, 'the compiled authorization-code flow declares no token URL'));
  final parsed = Uri.tryParse(redirectUri);
  if (redirectUri.isEmpty || parsed == null || !parsed.hasScheme || parsed.hasFragment) {
    throw AuthException('invalid-request', compiled.name, 'redirectUri must be an absolute URI without a fragment');
  }
  final identity = _resolveIdentity(compiled, clientId, null);
  final resolvedClientId = identity.$1 ?? (throw AuthException('missing-credential', compiled.name, 'a client id is required for the authorization-code flow; pass clientId or set the compiled environment variable'));
  final state = _randomBase64Url(16);
  final codeVerifier = _randomBase64Url(32);
  final codeChallenge = _s256Challenge(codeVerifier);
  final query = <String, String>{
    'response_type': 'code',
    'client_id': resolvedClientId,
    'redirect_uri': redirectUri,
    'state': state,
    'code_challenge': codeChallenge,
    'code_challenge_method': 'S256',
  };
  final requested = scopes == null ? '' : scopes.join(' ');
  if (requested.isNotEmpty) {
    query['scope'] = requested;
  }
  final additions = query.entries.map((entry) => '${Uri.encodeQueryComponent(entry.key)}=${Uri.encodeQueryComponent(entry.value)}').join('&');
  final authorizationUrl = authorizationEndpoint.contains('?') ? '$authorizationEndpoint&$additions' : '$authorizationEndpoint?$additions';
  return AuthorizationBegin(authorizationUrl, state, AuthorizationTransaction(scheme: compiled.name, state: state, codeVerifier: codeVerifier, redirectUri: redirectUri, tokenUrl: tokenUrl, createdAt: (clock ?? _systemClock)()));
}

/// Completes one authorization-code transaction: validates the redirect state,
/// exchanges the code at the transaction's token URL with the stored code
/// verifier, and replaces the stored token set atomically. The transaction is
/// consumed exactly once by any attempt; a failed exchange requires beginning
/// a new authorization.
Future<TokenSet> completeAuthorization(AuthorizationTransaction transaction, {required String code, required String state, String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport}) async {
  if (_consumedTransactions.contains(transaction)) {
    throw AuthException('transaction-consumed', transaction.scheme, 'this authorization transaction was already consumed; begin a new authorization');
  }
  _consumedTransactions.add(transaction);
  if (state != transaction.state) {
    throw AuthException('state-mismatch', transaction.scheme, 'the redirect state does not match the authorization transaction');
  }
  if (!_usableSecret(code)) {
    throw AuthException('invalid-request', transaction.scheme, 'code must be a nonempty string without control characters');
  }
  final compiled = _compiled(transaction.scheme);
  final grant = _executableFlow(compiled, 'authorization-code');
  if (transaction.tokenUrl.isEmpty) {
    throw AuthException('endpoint-unavailable', compiled.name, 'the authorization transaction carries no token URL');
  }
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final fields = <String, String>{'grant_type': 'authorization_code', 'code': code, 'redirect_uri': transaction.redirectUri, 'code_verifier': transaction.codeVerifier};
  if (grant.clientAuth == 'none' && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  final acquired = await _tokenRequest(compiled.name, transaction.tokenUrl, _clientAuth(grant, identity), fields, transport ?? _defaultTransport, clock ?? _systemClock);
  await store?.replace(tokenStoreKey(compiled.name, transaction.tokenUrl, identity.$1), acquired);
  return acquired;
}
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries the
/// configured endpoint.
const REVOKE: &str = r#"
/// Revokes one token at the compiled revocation endpoint (RFC 7009,
/// form-encoded). Authentication follows the compiled client policy.
Future<void> revokeToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport}) async {
  if (!_usableSecret(token)) {
    throw AuthException('invalid-request', scheme, 'token must be a nonempty string without control characters');
  }
  final compiled = _compiled(scheme);
  final endpoint = compiled.revocationEndpoint ?? (throw AuthException('endpoint-unavailable', compiled.name, 'no revocation endpoint was configured for this scheme'));
  final grant = _refreshFlow(compiled);
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final fields = <String, String>{'token': token};
  if (tokenTypeHint != null) {
    fields['token_type_hint'] = tokenTypeHint;
  }
  if (grant.clientAuth == 'none' && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  await _postForm(compiled.name, endpoint, _clientAuth(grant, identity), fields, transport ?? _defaultTransport);
}
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries the
/// configured endpoint.
const INTROSPECT: &str = r#"
/// Introspects one token at the compiled introspection endpoint (RFC 7662,
/// form-encoded) and returns the server's JSON response. Authentication
/// follows the compiled client policy.
Future<Map<String, dynamic>> introspectToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport}) async {
  if (!_usableSecret(token)) {
    throw AuthException('invalid-request', scheme, 'token must be a nonempty string without control characters');
  }
  final compiled = _compiled(scheme);
  final endpoint = compiled.introspectionEndpoint ?? (throw AuthException('endpoint-unavailable', compiled.name, 'no introspection endpoint was configured for this scheme'));
  final grant = _refreshFlow(compiled);
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final fields = <String, String>{'token': token};
  if (tokenTypeHint != null) {
    fields['token_type_hint'] = tokenTypeHint;
  }
  if (grant.clientAuth == 'none' && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  return _jsonObject(compiled.name, await _postForm(compiled.name, endpoint, _clientAuth(grant, identity), fields, transport ?? _defaultTransport), 'introspection');
}
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
/// Runs the device authorization grant to completion (RFC 8628): requests the
/// device code at the compiled device-authorization URL, then polls the token
/// endpoint, honoring `authorization_pending`, `slow_down` (backing off five
/// seconds per occurrence), the server interval and the code's expiry, and
/// returns the typed token set. Returns only after the user completes
/// authorization at the verification URI the server returned.
Future<TokenSet> beginDeviceAuthorization(String scheme, {String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, Future<void> Function(int milliseconds)? delay}) async {
  final compiled = _compiled(scheme);
  final deviceFlow = _executableFlow(compiled, 'device-authorization');
  final deviceEndpoint = deviceFlow.deviceAuthorizationUrl ?? (throw AuthException('endpoint-unavailable', compiled.name, 'the compiled device-authorization flow declares no device-authorization URL'));
  final tokenUrl = deviceFlow.tokenUrl ?? (throw AuthException('endpoint-unavailable', compiled.name, 'the compiled device-authorization flow declares no token URL'));
  final identity = _resolveIdentity(compiled, clientId, clientSecret);
  final resolvedClock = clock ?? _systemClock;
  final resolvedTransport = transport ?? _defaultTransport;
  final requestFields = <String, String>{};
  if (identity.$1 != null) {
    requestFields['client_id'] = identity.$1!;
  }
  final body = _jsonObject(compiled.name, await _postForm(compiled.name, deviceEndpoint, (false, identity.$1, null), requestFields, resolvedTransport), 'device-authorization');
  final declaredDeviceCode = body['device_code'];
  if (declaredDeviceCode is! String || !_usableSecret(declaredDeviceCode)) {
    throw AuthException('invalid-response', compiled.name, 'the device-authorization response carries no usable device code');
  }
  final declaredExpires = body['expires_in'];
  final expiresInSeconds = declaredExpires is num && declaredExpires > 0 && declaredExpires.isFinite ? declaredExpires.round() : 600;
  final declaredInterval = body['interval'];
  final intervalSeconds = declaredInterval is num && declaredInterval > 0 && declaredInterval.isFinite ? declaredInterval.round() : 5;
  final expiresAt = resolvedClock() + expiresInSeconds * 1000;
  final wait = delay ?? (milliseconds) => Future<void>.delayed(Duration(milliseconds: milliseconds));
  var intervalMilliseconds = intervalSeconds < 1 ? 1000 : intervalSeconds * 1000;
  for (;;) {
    if (resolvedClock() >= expiresAt) {
      throw AuthException('device-code-expired', compiled.name, 'the device code expired before authorization completed');
    }
    await wait(intervalMilliseconds);
    if (resolvedClock() >= expiresAt) {
      throw AuthException('device-code-expired', compiled.name, 'the device code expired before authorization completed');
    }
    final fields = <String, String>{'grant_type': 'urn:ietf:params:oauth:grant-type:device_code', 'device_code': declaredDeviceCode};
    if (deviceFlow.clientAuth == 'none' && identity.$1 != null) {
      fields['client_id'] = identity.$1!;
    }
    TokenSet acquired;
    try {
      acquired = await _tokenRequest(compiled.name, tokenUrl, _clientAuth(deviceFlow, identity), fields, resolvedTransport, resolvedClock);
    } on AuthException catch (error) {
      if (error.serverError == 'authorization_pending') {
        continue;
      }
      if (error.serverError == 'slow_down') {
        intervalMilliseconds += 5000;
        continue;
      }
      if (error.serverError == 'expired_token') {
        throw AuthException('device-code-expired', compiled.name, 'the device code expired before authorization completed');
      }
      rethrow;
    }
    await store?.replace(tokenStoreKey(compiled.name, tokenUrl, identity.$1), acquired);
    return acquired;
  }
}
"#;

/// The discovery-aware client-credentials half: endpoint resolution follows
/// the compiled precedence (an explicit compiled endpoint always wins;
/// otherwise the provider's cached discovery document supplies the token
/// endpoint; otherwise the typed endpoint-unavailable refusal stands).
const API_CC_DISCOVERY: &str = r#"
/// Creates a caller credential for the compiled client-credentials flow: on
/// attach it serves a fresh token from the provider-owned store, otherwise it
/// acquires one with a single form-encoded token request. Concurrent attaches
/// share one in-flight acquisition per scheme, token endpoint and client
/// identity, the store is re-checked after the in-flight future resolves and
/// replaced atomically, and a response refresh token is adopted, else a
/// previous one retained. Pass the returned callback as the scheme's
/// credential member.
///
/// Endpoint resolution follows the compiled precedence: the compiled
/// client-credentials flow's token URL always wins; otherwise, when the
/// scheme compiles a discovery URL, the discovery document's
/// `token_endpoint` resolves the request (fetched once per scheme and cached
/// for this provider's lifetime, single-flighted across concurrent callers,
/// with a failed fetch retried on the next call); otherwise the typed
/// endpoint-unavailable refusal stands.
CredentialProvider createClientCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) {
  final tokenStore = store ?? MemoryTokenStore();
  final resolvedTransport = transport ?? _defaultTransport;
  final resolvedGetTransport = discoveryTransport ?? _defaultGetTransport;
  final discoveryCache = <String, Future<_DiscoveredEndpoints>>{};
  return (request) async {
    final compiledScheme = _compiled(scheme);
    final grant = _executableFlowOrNull(compiledScheme, 'client-credentials');
    final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
    final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
    final tokenUrl = await _resolveEndpoint(compiledScheme, grant?.tokenUrl, _DiscoveryMember.tokenEndpoint, resolvedGetTransport, discoveryCache);
    final key = tokenStoreKey(compiledScheme.name, tokenUrl, identity.$1);
    final pending = _inflight[key];
    if (pending != null) {
      return _authorization(await pending, scheme);
    }
    final tracked = _acquireWithAuth(compiledScheme, tokenUrl, auth, identity, scope, store, clock ?? _systemClock, resolvedTransport);
    _inflight[key] = tracked;
    try {
      return _authorization(await tracked, scheme);
    } finally {
      if (identical(_inflight[key], tracked)) {
        _inflight.remove(key);
      }
    }
  };
}

/// Acquires a token with the compiled client-credentials flow: a fresh stored
/// token is served, otherwise one form-encoded token request runs. Concurrent
/// callers share one in-flight acquisition per scheme, token endpoint and
/// client identity; the store is replaced atomically and a response refresh
/// token is adopted, else a previous one retained. Without an explicit [store]
/// the call acquires without persistence, because the library never keeps a
/// global store; the provider factories own one store per provider.
///
/// Endpoint resolution follows the compiled precedence: the compiled
/// client-credentials flow's token URL always wins; otherwise, when the
/// scheme compiles a discovery URL, the discovery document's
/// `token_endpoint` resolves the request. This one-shot helper fetches
/// discovery per call and keeps no discovery cache.
Future<TokenSet> clientCredentialsToken(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {
  final compiledScheme = _compiled(scheme);
  final grant = _executableFlowOrNull(compiledScheme, 'client-credentials');
  final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
  final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
  final tokenUrl = await _resolveEndpoint(compiledScheme, grant?.tokenUrl, _DiscoveryMember.tokenEndpoint, discoveryTransport ?? _defaultGetTransport, <String, Future<_DiscoveredEndpoints>>{});
  final key = tokenStoreKey(compiledScheme.name, tokenUrl, identity.$1);
  final pending = _inflight[key];
  if (pending != null) {
    return pending;
  }
  final tracked = _acquireWithAuth(compiledScheme, tokenUrl, auth, identity, scope, store, clock ?? _systemClock, transport ?? _defaultTransport);
  _inflight[key] = tracked;
  try {
    return await tracked;
  } finally {
    if (identical(_inflight[key], tracked)) {
      _inflight.remove(key);
    }
  }
}
"#;

/// The discovery-aware refresh half: the declared refresh URL, else the
/// compiled flow's token URL, always wins; otherwise the cached discovery
/// document's token endpoint resolves the refresh.
const API_REFRESH_DISCOVERY: &str = r#"
/// Creates a caller credential that serves stored tokens and refreshes them on
/// demand: on attach a fresh stored token is returned; an expired one is
/// refreshed exactly once with its stored refresh token before serving. There
/// is no token at all until an authorization or device flow has completed.
/// Automatic retry hook: when a protected call still fails with a declared
/// 401, call `refreshToken` explicitly and retry; the SDK itself never retries.
///
/// Endpoint resolution follows the compiled precedence: the declared refresh
/// URL, else the compiled flow's token URL, always wins; otherwise, when the
/// scheme compiles a discovery URL, the discovery document's
/// `token_endpoint` resolves the refresh (fetched once per scheme and cached
/// for this provider's lifetime, single-flighted across concurrent callers,
/// with a failed fetch retried on the next call).
CredentialProvider createRefreshProvider(String scheme, {String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) {
  final tokenStore = store ?? MemoryTokenStore();
  final resolvedGetTransport = discoveryTransport ?? _defaultGetTransport;
  final discoveryCache = <String, Future<_DiscoveredEndpoints>>{};
  return (request) async {
    final compiledScheme = _compiled(scheme);
    final grant = _refreshFlowOrNull(compiledScheme);
    final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
    final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
    final endpoint = grant == null
        ? await _resolveEndpoint(compiledScheme, null, _DiscoveryMember.tokenEndpoint, resolvedGetTransport, discoveryCache)
        : _refreshEndpoint(grant, compiledScheme);
    final resolvedClock = clock ?? _systemClock;
    final stored = await tokenStore.load(tokenStoreKey(compiledScheme.name, endpoint, identity.$1));
    if (stored == null) {
      throw AuthException('missing-credential', compiledScheme.name, 'no stored token set exists for this scheme; complete an authorization or device flow first');
    }
    if (_isFresh(stored, compiledScheme.refreshSkewSeconds * 1000, resolvedClock())) {
      return _authorization(stored, scheme);
    }
    if (stored.refreshToken == null) {
      throw AuthException('missing-credential', compiledScheme.name, 'the stored token set carries no refresh token');
    }
    final refreshed = await _refreshWithAuth(compiledScheme, endpoint, auth, identity, stored.refreshToken!, stored, tokenStore, transport ?? _defaultTransport, resolvedClock);
    return _authorization(refreshed, scheme);
  };
}

/// Exchanges one refresh token at the declared refresh URL or token URL
/// (grant_type=refresh_token) and returns the token set. A rotated refresh
/// token from the response is adopted; when the response carries none and a
/// previous stored set exists, the previous refresh token is retained.
///
/// Endpoint resolution follows the compiled precedence: the declared refresh
/// URL, else the compiled flow's token URL, always win; otherwise, when the
/// scheme compiles a discovery URL, the discovery document's
/// `token_endpoint` resolves the exchange. This one-shot helper fetches
/// discovery per call and keeps no cache.
Future<TokenSet> refreshToken(String scheme, {required String refreshToken, String? clientId, String? clientSecret, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {
  if (!_usableSecret(refreshToken)) {
    throw AuthException('invalid-request', scheme, 'refreshToken must be a nonempty string without control characters');
  }
  final compiledScheme = _compiled(scheme);
  final grant = _refreshFlowOrNull(compiledScheme);
  final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
  final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
  final resolvedClock = clock ?? _systemClock;
  final endpoint = await _resolveEndpoint(compiledScheme, grant?.refreshUrl ?? grant?.tokenUrl, _DiscoveryMember.tokenEndpoint, discoveryTransport ?? _defaultGetTransport, <String, Future<_DiscoveredEndpoints>>{});
  final previous = store == null ? null : await store.load(tokenStoreKey(compiledScheme.name, endpoint, identity.$1));
  return _refreshWithAuth(compiledScheme, endpoint, auth, identity, refreshToken, previous, store, transport ?? _defaultTransport, resolvedClock);
}
"#;

/// RFC 7009 revocation with discovery fallback: the configured endpoint always
/// wins; otherwise the discovery document's `revocation_endpoint`.
const REVOKE_DISCOVERY: &str = r#"
/// Revokes one token at the compiled revocation endpoint (RFC 7009,
/// form-encoded). Authentication follows the compiled client policy.
///
/// Endpoint resolution follows the compiled precedence: the configured
/// revocation endpoint always wins; otherwise, when the scheme compiles a
/// discovery URL, the discovery document's `revocation_endpoint` resolves
/// the request. This one-shot helper fetches discovery per call and keeps no
/// cache.
Future<void> revokeToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {
  if (!_usableSecret(token)) {
    throw AuthException('invalid-request', scheme, 'token must be a nonempty string without control characters');
  }
  final compiledScheme = _compiled(scheme);
  final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
  var endpoint = compiledScheme.revocationEndpoint;
  if (endpoint == null) {
    final document = await _discover(compiledScheme, discoveryTransport ?? _defaultGetTransport, <String, Future<_DiscoveredEndpoints>>{});
    final discovered = document.revocationEndpoint;
    if (discovered == null) {
      throw AuthException('endpoint-unavailable', compiledScheme.name, 'no revocation endpoint was configured for this scheme and the discovery document declares none');
    }
    endpoint = discovered;
  }
  final grant = _refreshFlowOrNull(compiledScheme);
  final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
  final fields = <String, String>{'token': token};
  if (tokenTypeHint != null) {
    fields['token_type_hint'] = tokenTypeHint;
  }
  if (!auth.$1 && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  await _postForm(compiledScheme.name, endpoint, auth, fields, transport ?? _defaultTransport);
}
"#;

/// RFC 7662 introspection with discovery fallback: the configured endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECT_DISCOVERY: &str = r#"
/// Introspects one token at the compiled introspection endpoint (RFC 7662,
/// form-encoded) and returns the server's JSON response. Authentication
/// follows the compiled client policy.
///
/// Endpoint resolution follows the compiled precedence: the configured
/// introspection endpoint always wins; otherwise, when the scheme compiles a
/// discovery URL, the discovery document's `introspection_endpoint` resolves
/// the request. This one-shot helper fetches discovery per call and keeps no
/// cache.
Future<Map<String, dynamic>> introspectToken(String scheme, {required String token, String? tokenTypeHint, String? clientId, String? clientSecret, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) async {
  if (!_usableSecret(token)) {
    throw AuthException('invalid-request', scheme, 'token must be a nonempty string without control characters');
  }
  final compiledScheme = _compiled(scheme);
  final identity = _resolveIdentity(compiledScheme, clientId, clientSecret);
  var endpoint = compiledScheme.introspectionEndpoint;
  if (endpoint == null) {
    final document = await _discover(compiledScheme, discoveryTransport ?? _defaultGetTransport, <String, Future<_DiscoveredEndpoints>>{});
    final discovered = document.introspectionEndpoint;
    if (discovered == null) {
      throw AuthException('endpoint-unavailable', compiledScheme.name, 'no introspection endpoint was configured for this scheme and the discovery document declares none');
    }
    endpoint = discovered;
  }
  final grant = _refreshFlowOrNull(compiledScheme);
  final auth = grant == null ? _discoveryAuth(compiledScheme, identity) : _clientAuth(grant, identity);
  final fields = <String, String>{'token': token};
  if (tokenTypeHint != null) {
    fields['token_type_hint'] = tokenTypeHint;
  }
  return _jsonObject(compiledScheme.name, await _postForm(compiledScheme.name, endpoint, auth, fields, transport ?? _defaultTransport), 'introspection');
}
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-instance cache keyed by scheme with
/// single-flight through the in-flight-future map, and the endpoint-resolution
/// precedence helpers. Failure messages carry only safe metadata, never
/// response body text.
const DISCOVERY: &str = r#"
/// One RFC 8414 / OpenID Connect discovery document reduced to the endpoints
/// this part resolves. Unknown members are ignored; a known member must be a
/// nonempty control-character-free string when present.
final class _DiscoveredEndpoints {
  const _DiscoveredEndpoints({this.tokenEndpoint, this.revocationEndpoint, this.introspectionEndpoint});
  final String? tokenEndpoint;
  final String? revocationEndpoint;
  final String? introspectionEndpoint;
}

/// The compiled ceiling for one discovery document response (about a mebibyte).
const int _discoveryMaxBytes = 1048576;

/// The discovery document members this part resolves, with the exact document
/// member each one reads.
enum _DiscoveryMember {
  tokenEndpoint('token_endpoint'),
  revocationEndpoint('revocation_endpoint'),
  introspectionEndpoint('introspection_endpoint');
  const _DiscoveryMember(this.member);
  final String member;
}

/// One OAuth document GET with the same response carrier as the POST
/// transport; the default implementations live in `oauth_transport_io.dart`
/// (VM builds) and `oauth_transport_stub.dart` (portable builds).
typedef OAuthGetTransport = Future<_oauth_transport.OAuthEndpointResponse> Function(Uri url, Map<String, String> headers);

OAuthGetTransport get _defaultGetTransport => _oauth_transport.oauthEndpointGet;

/// Reads one discovery document member: absent stays null; a non-string or unusable value is a typed discovery failure. Unknown members are ignored.
String? _discoveredEndpoint(String scheme, Map<String, dynamic> document, String member) {
  if (!document.containsKey(member)) {
    return null;
  }
  final value = document[member];
  if (value is! String || value.isEmpty || value.codeUnits.any((c) => c < 32 || c == 127)) {
    throw AuthException('discovery-failed', scheme, 'the discovery document carries an unusable $member value');
  }
  return value;
}

/// The origin of one absolute http(s) URL: scheme, host and the port with the scheme default made explicit. Returns null when the value is not an absolute http(s) URL.
String? _urlOrigin(String value) {
  final parsed = Uri.tryParse(value);
  if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty || (parsed.scheme != 'http' && parsed.scheme != 'https')) {
    return null;
  }
  return '${parsed.scheme}://${parsed.host}:${parsed.port}';
}

/// Decodes and validates one discovery response body into the endpoints this part resolves. The exact issuer rule: when the document carries an `issuer` claim, it must be an absolute http(s) URL whose origin (scheme, host and the port with the scheme default made explicit) equals the discovery URL's origin; OpenID Connect openIdConnectUrl documents are validated against their `issuer` claim exactly this way, as are RFC 8414 OAuth2 authorization-server metadata documents. A missing claim is tolerated; a mismatching or unparseable one is a typed discovery failure. Failure messages carry only safe metadata, never response body text.
_DiscoveredEndpoints _discoveryDocument(CompiledScheme scheme, String url, String body) {
  Object? decoded;
  try {
    decoded = jsonDecode(body);
  } on Object {
    throw AuthException('discovery-failed', scheme.name, 'the discovery document is not readable JSON');
  }
  if (decoded is! Map<String, dynamic>) {
    throw AuthException('discovery-failed', scheme.name, 'the discovery document is not a JSON object');
  }
  final issuer = decoded['issuer'];
  if (issuer is String && issuer.isNotEmpty) {
    final issuerOrigin = _urlOrigin(issuer);
    final discoveryOrigin = _urlOrigin(url);
    if (issuerOrigin == null || discoveryOrigin == null || issuerOrigin != discoveryOrigin) {
      throw AuthException('discovery-failed', scheme.name, 'the discovery document issuer does not share the discovery URL origin');
    }
  }
  return _DiscoveredEndpoints(
    tokenEndpoint: _discoveredEndpoint(scheme.name, decoded, _DiscoveryMember.tokenEndpoint.member),
    revocationEndpoint: _discoveredEndpoint(scheme.name, decoded, _DiscoveryMember.revocationEndpoint.member),
    introspectionEndpoint: _discoveredEndpoint(scheme.name, decoded, _DiscoveryMember.introspectionEndpoint.member),
  );
}

/// Fetches the scheme's discovery document (GET, `accept: application/json`),
/// returning the cached document when one exists for this provider.
/// Successful documents are cached per scheme for the instance's lifetime, so
/// repeated attaches never re-fetch; a failed fetch is never cached, so the
/// next call retries; concurrent callers share the one in-flight fetch
/// through the cached future (single-flight). The response is bounded at
/// [_discoveryMaxBytes]; timeouts stay with the caller's transport, exactly
/// like every other request in this part.
Future<_DiscoveredEndpoints> _discover(CompiledScheme scheme, OAuthGetTransport transport, Map<String, Future<_DiscoveredEndpoints>> cache) async {
  final url = scheme.discovery;
  if (url == null) {
    throw AuthException('endpoint-unavailable', scheme.name, 'the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery');
  }
  final pending = cache[scheme.name];
  if (pending != null) {
    return pending;
  }
  final tracked = _fetchDiscovery(scheme, url, transport);
  cache[scheme.name] = tracked;
  try {
    return await tracked;
  } on Object {
    // A failed fetch is never cached: the entry is removed so the next call retries.
    if (identical(cache[scheme.name], tracked)) {
      cache.remove(scheme.name);
    }
    rethrow;
  }
}

Future<_DiscoveredEndpoints> _fetchDiscovery(CompiledScheme scheme, String url, OAuthGetTransport transport) async {
  _oauth_transport.OAuthEndpointResponse response;
  try {
    response = await transport(Uri.parse(url), <String, String>{'accept': 'application/json'});
  } on Object {
    throw AuthException('discovery-failed', scheme.name, 'the discovery document request failed before a response arrived');
  }
  if (response.status < 200 || response.status > 299) {
    throw AuthException('discovery-failed', scheme.name, 'the discovery document request answered HTTP ${response.status}', status: response.status);
  }
  final declared = response.headers['content-length'];
  if (declared != null && declared.isNotEmpty) {
    final length = int.tryParse(declared.first.trim());
    if (length != null && length > _discoveryMaxBytes) {
      throw AuthException('discovery-failed', scheme.name, 'the discovery document exceeds the compiled response ceiling');
    }
  }
  final body = response.body;
  if (utf8.encode(body).length > _discoveryMaxBytes) {
    throw AuthException('discovery-failed', scheme.name, 'the discovery document exceeds the compiled response ceiling');
  }
  return _discoveryDocument(scheme, url, body);
}

/// Resolves one lifecycle endpoint through the compiled precedence: an explicit compiled endpoint always wins; otherwise the cached discovery document's endpoint when the scheme compiles a discovery URL; otherwise the typed endpoint-unavailable refusal the compiled plan alone would produce.
Future<String> _resolveEndpoint(CompiledScheme scheme, String? compiledEndpoint, _DiscoveryMember member, OAuthGetTransport transport, Map<String, Future<_DiscoveredEndpoints>> cache) async {
  if (compiledEndpoint != null) {
    return compiledEndpoint;
  }
  final document = await _discover(scheme, transport, cache);
  final found = switch (member) {
    _DiscoveryMember.tokenEndpoint => document.tokenEndpoint,
    _DiscoveryMember.revocationEndpoint => document.revocationEndpoint,
    _DiscoveryMember.introspectionEndpoint => document.introspectionEndpoint,
  };
  if (found == null) {
    throw AuthException('endpoint-unavailable', scheme.name, 'neither the compiled plan nor the discovery document carries a ${member.name} for this scheme');
  }
  return found;
}

/// Client authentication for endpoints the discovery document supplies (no compiled flow declares one): client-secret-basic when the compiled configuration carries a client secret variable — an unavailable value becomes the typed missing-credential refusal — else the public profile, which sends the client id in the form.
(bool, String?, String?) _discoveryAuth(CompiledScheme scheme, (String?, String?) identity) => (scheme.clientSecretEnv != null, identity.$1, scheme.clientSecretEnv != null ? identity.$2 : null);

/// Resolves one executable (non-deprecated) compiled flow, or null when the scheme compiles none: a discovery-defined scheme's flows live in the discovery document.
CompiledFlow? _executableFlowOrNull(CompiledScheme scheme, String kind) {
  for (final candidate in scheme.flows) {
    if (candidate.kind == kind && !candidate.deprecated) {
      return candidate;
    }
  }
  return null;
}

/// The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL, else null for a scheme the discovery document defines.
CompiledFlow? _refreshFlowOrNull(CompiledScheme scheme) {
  CompiledFlow? fallback;
  for (final candidate in scheme.flows) {
    if (candidate.deprecated) {
      continue;
    }
    if (candidate.kind == 'authorization-code') {
      return candidate;
    }
    if (fallback == null && candidate.tokenUrl != null) {
      fallback = candidate;
    }
  }
  return fallback;
}

/// Client-credentials acquisition with a precomputed client-auth policy,
/// shared by the compiled and discovery-resolved paths.
Future<TokenSet> _acquireWithAuth(CompiledScheme scheme, String tokenUrl, (bool, String?, String?) auth, (String?, String?) identity, String? scope, TokenStore? store, int Function() clock, OAuthTransport transport) async {
  final key = tokenStoreKey(scheme.name, tokenUrl, identity.$1);
  final stored = store == null ? null : await store.load(key);
  if (stored != null && _isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) {
    return stored;
  }
  final fields = <String, String>{'grant_type': 'client_credentials'};
  if (scope != null) {
    fields['scope'] = scope;
  }
  if (!auth.$1 && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  final acquired = await _tokenRequest(scheme.name, tokenUrl, auth, fields, transport, clock);
  final adopted = _adopt(stored, acquired);
  await store?.replace(key, adopted);
  return adopted;
}

/// Refresh with a precomputed client-auth policy, shared by the compiled and
/// discovery-resolved paths.
Future<TokenSet> _refreshWithAuth(CompiledScheme scheme, String endpoint, (bool, String?, String?) auth, (String?, String?) identity, String refreshToken, TokenSet? previous, TokenStore? store, OAuthTransport transport, int Function() clock) async {
  final fields = <String, String>{'grant_type': 'refresh_token', 'refresh_token': refreshToken};
  if (!auth.$1 && identity.$1 != null) {
    fields['client_id'] = identity.$1!;
  }
  final acquired = await _tokenRequest(scheme.name, endpoint, auth, fields, transport, clock);
  final adopted = _adopt(previous, acquired);
  await store?.replace(tokenStoreKey(scheme.name, endpoint, identity.$1), adopted);
  return adopted;
}
"#;

/// The replaying credential wrapper's shared half: the record of served
/// attaches, the coordinated refresh rounds and the replaying transport. The
/// plain and discovery variants differ only in their factory.
const REPLAY_CREDENTIAL: &str = r#"
/// One attach this provider served, remembered so the wrapped transport can
/// tell which requests carried this provider's token. The record keeps only
/// safe metadata; token values already traveled on the wire.
final class _ServedAttach {
  const _ServedAttach(this.value, this.eligible);
  final String value;
  final bool eligible;
}

/// The replaying credential: the plain provider's attach behavior plus the
/// unified 401 replay policy. Use it in two places — pass the credential
/// itself as the scheme's credential member, and pass
/// `credentials.replayTransport(transport)` as the client's transport:
///
/// ```dart
/// final credentials = createReplayingCredentialsProvider('name');
/// final client = Client(
///   transport: credentials.replayTransport(myTransport),
///   credentials: Credentials(name: credentials),
/// );
/// ```
///
/// A 401 (and only a 401) on a request whose Authorization value this
/// provider attached triggers exactly one coordinated refresh — concurrent
/// 401s share one token request through the same single-flight store round —
/// and exactly one replay of the request with the fresh token. The second
/// response is surfaced whatever it is: a second 401 reaches the caller as
/// the declared error. The overall budget is one refresh plus one replay,
/// never nested with other retry policies (requests are not retried today).
/// Attaches for stream-protected requirements are never replayed, because
/// delivered stream data prevents a transparent restart. A refresh failure
/// surfaces as the typed AuthException instead of a replay. The plain
/// provider keeps today's semantics: replay is this wrapper's opt-in only.
final class ReplayingCredentials {
  ReplayingCredentials({required CompiledScheme compiledScheme, required TokenStore tokenStore, required CredentialProvider plain, required Future<String> Function() endpoint, String? lifecycleUrl, String? clientId, String? clientSecret})
      : _compiledScheme = compiledScheme,
        _tokenStore = tokenStore,
        _plain = plain,
        _endpoint = endpoint,
        _lifecycleUrl = lifecycleUrl,
        _clientId = clientId,
        _clientSecret = clientSecret;
  final CompiledScheme _compiledScheme;
  final TokenStore _tokenStore;
  final CredentialProvider _plain;
  final Future<String> Function() _endpoint;
  final String? _lifecycleUrl;
  final String? _clientId;
  final String? _clientSecret;
  final List<_ServedAttach> _served = <_ServedAttach>[];
  final Map<String, Future<AuthorizationCredential>> _rounds = <String, Future<AuthorizationCredential>>{};

  /// The replaying credential itself: serves the scheme's tokens exactly like
  /// the plain provider and remembers which Authorization values its attaches
  /// produced.
  CredentialRequest? _attachRequest;

  Future<AuthorizationCredential> call(CredentialRequest request) async {
    _attachRequest = request;
    final supplied = (await _plain(request))!;
    _record(supplied.value, _eligible(request));
    return supplied;
  }

  /// Wraps [inner] with the one-refresh-one-replay 401 policy; call once per
  /// client. Token requests keep traveling through [inner] directly.
  HttpTransport replayTransport(HttpTransport inner) => _ReplayTransport(this, inner);

  bool _eligible(CredentialRequest request) {
    final pointers = _noReplayRequirements[request.requirement.name];
    if (pointers == null) {
      return true;
    }
    final prefix = '${request.operation.document}#${request.operation.pointer}/security/';
    for (final pointer in pointers) {
      if (pointer.startsWith(prefix)) {
        return false;
      }
    }
    return true;
  }

  void _record(String value, bool eligible) {
    _served.insert(0, _ServedAttach(value, eligible));
    if (_served.length > 8) {
      _served.removeRange(8, _served.length);
    }
  }

  _ServedAttach? _servedEntry(String? presented) {
    if (presented == null || presented.isEmpty) {
      return null;
    }
    for (final entry in _served) {
      if (entry.value == presented && entry.eligible) {
        return entry;
      }
    }
    return null;
  }

  /// Lifecycle endpoint requests carry no bearer token of this provider, so
  /// this exact-target guard is defense in depth against loops.
  bool _lifecycle(TransportRequest request) => _lifecycleUrl != null && request.url.toString() == _lifecycleUrl;

  /// One coordinated refresh: concurrent 401s share one store round, a newer
  /// stored set wins over a stale re-refresh, and a failed round fails every
  /// waiter exactly once. The round resolves to the fresh complete
  /// Authorization value.
  Future<AuthorizationCredential> _refresh(String presented, CredentialRequest request) async {
    final key = tokenStoreKey(_compiledScheme.name, await _endpoint(), _resolveIdentity(_compiledScheme, _clientId, _clientSecret).$1);
    final pending = _rounds[key];
    if (pending != null) {
      return pending;
    }
    final tracked = _refreshRound(presented, request, key);
    _rounds[key] = tracked;
    try {
      return await tracked;
    } finally {
      if (identical(_rounds[key], tracked)) {
        _rounds.remove(key);
      }
    }
  }

  Future<AuthorizationCredential> _refreshRound(String presented, CredentialRequest request, String key) async {
    final stored = await _tokenStore.load(key);
    if (stored != null) {
      final current = _authorization(stored, _compiledScheme.name);
      if (current.value != presented) {
        return current;
      }
    }
    await _tokenStore.clear(key);
    return (await _plain(request))!;
  }
}
"#;

/// The replaying transport: the 401 interception half of the wrapper, shared
/// by the plain and discovery variants.
const REPLAY_TRANSPORT: &str = r#"
/// The replaying transport: the 401 interception half of the wrapper.
/// Lifecycle endpoint requests are never replayed: they carry no bearer token
/// of this provider, and the exact-target guard is defense in depth.
final class _ReplayTransport implements HttpTransport {
  _ReplayTransport(this._provider, this._inner);
  final ReplayingCredentials _provider;
  final HttpTransport _inner;

  @override
  Future<TransportResponse> send(TransportRequest request) async {
    final response = await _inner.send(request);
    if (response.status != 401) {
      return response;
    }
    final presented = request.headers['authorization'];
    if (_provider._servedEntry(presented) == null || _provider._lifecycle(request)) {
      return response;
    }
    final attachRequest = _provider._attachRequest;
    if (attachRequest == null) {
      return response;
    }
    final fresh = await _provider._refresh(presented!, attachRequest);
    final replayed = TransportRequest._(
      request.method,
      request.url,
      <String, String>{...request.headers, 'authorization': fresh.value},
      request.body,
      request.cancellation,
      request.timeout,
      request.maxResponseHeaderBytes,
      request.maxResponseBytes,
    );
    await response.close();
    return _inner.send(replayed);
  }

  @override
  Future<void> close() => _inner.close();
}
"#;

/// The plain variant's opt-in factory: the compiled client-credentials token
/// endpoint serves both the store key and the lifecycle-endpoint exclusion.
const REPLAY_FACTORY_PLAIN: &str = r#"
/// Creates the replaying variant of the compiled client-credentials provider.
/// Every provider option behaves exactly as in `createClientCredentialsProvider`;
/// the replay semantics are strictly additive and the plain provider keeps
/// today's attach-only semantics. Creation refuses a compiled scheme whose
/// client-credentials flow declares no token URL, exactly like the plain
/// provider's first attach.
ReplayingCredentials createReplayingCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport}) {
  final tokenStore = store ?? MemoryTokenStore();
  final compiledScheme = _compiled(scheme);
  final grant = _executableFlow(compiledScheme, 'client-credentials');
  final tokenUrl = grant.tokenUrl ?? (throw AuthException('endpoint-unavailable', compiledScheme.name, 'the compiled client-credentials flow declares no token URL'));
  return ReplayingCredentials(
    compiledScheme: compiledScheme,
    tokenStore: tokenStore,
    plain: createClientCredentialsProvider(scheme, clientId: clientId, clientSecret: clientSecret, scope: scope, store: tokenStore, clock: clock, transport: transport),
    endpoint: () async => tokenUrl,
    lifecycleUrl: tokenUrl,
    clientId: clientId,
    clientSecret: clientSecret,
  );
}
"#;

/// The discovery variant's opt-in factory: the refresh endpoint resolves
/// through the compiled precedence (compiled token URL, else the discovery
/// document's), cached per provider exactly like the plain provider's
/// discovery resolution.
const REPLAY_FACTORY_DISCOVERY: &str = r#"
/// Creates the replaying variant of the compiled client-credentials provider.
/// Every provider option behaves exactly as in `createClientCredentialsProvider`;
/// the replay semantics are strictly additive and the plain provider keeps
/// today's attach-only semantics. The refresh endpoint resolves through the
/// compiled precedence — the compiled token URL when the client-credentials
/// flow compiles one, otherwise the discovery document's `token_endpoint`,
/// fetched once and cached for this provider's lifetime.
ReplayingCredentials createReplayingCredentialsProvider(String scheme, {String? clientId, String? clientSecret, String? scope, TokenStore? store, int Function()? clock, OAuthTransport? transport, OAuthGetTransport? discoveryTransport}) {
  final tokenStore = store ?? MemoryTokenStore();
  final compiledScheme = _compiled(scheme);
  final grant = _executableFlowOrNull(compiledScheme, 'client-credentials');
  final discoveryCache = <String, Future<_DiscoveredEndpoints>>{};
  final resolvedGetTransport = discoveryTransport ?? _defaultGetTransport;
  return ReplayingCredentials(
    compiledScheme: compiledScheme,
    tokenStore: tokenStore,
    plain: createClientCredentialsProvider(scheme, clientId: clientId, clientSecret: clientSecret, scope: scope, store: tokenStore, clock: clock, transport: transport, discoveryTransport: discoveryTransport),
    endpoint: () => _resolveEndpoint(compiledScheme, grant?.tokenUrl, _DiscoveryMember.tokenEndpoint, resolvedGetTransport, discoveryCache),
    lifecycleUrl: compiledScheme.discovery,
    clientId: clientId,
    clientSecret: clientSecret,
  );
}
"#;
