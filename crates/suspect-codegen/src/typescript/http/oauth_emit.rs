//! Generated-only OAuth 2.0 lifecycle emission for the TypeScript HTTP adapter.
//!
//! The compiled [`OAuthPlan`] becomes one generated module holding frozen
//! per-scheme descriptors plus a native token store, credential providers and
//! flow helpers. Like pagination, nothing is emitted without usable compiled
//! schemes and the remaining package stays byte-identical; schemes carrying
//! only deprecated implicit/password flows emit nothing, while schemes
//! carrying a discovery URL (including OpenID Connect schemes, whose flows a
//! discovery document defines at runtime) emit the discovery-driven endpoint
//! resolution alongside the compiled descriptors. The runtime never parses
//! OpenAPI: every compiled endpoint is a frozen constant, and discovery
//! supplies only the endpoints the compiled plan omits.
//!
//! Emission is byte-identical for plans without a discovery URL: the plain
//! variants of each emitted section concatenate into exactly the pre-discovery
//! bytes, and every discovery-aware section (the header paragraph, the
//! `discovery-failed` error kind, the endpoint-resolution providers and the
//! discovery engine) is emitted only when at least one compiled scheme carries
//! a discovery URL.

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

fn executable(scheme: &OAuthSchemePlan, kind: OAuthFlowDescriptorKind) -> bool {
    scheme
        .flows
        .iter()
        .any(|flow| flow.kind == kind && !flow.deprecated_flow)
}

/// Whether the plan compiles at least one scheme with an executable flow or a
/// discovery URL, so `typescript/oauth.ts` participates in the package. A
/// discovery URL makes a scheme usable even with no declared flows: OpenID
/// Connect schemes have their endpoints defined by the discovery document at
/// runtime.
pub(super) fn has_usable(plan: &OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        has_discovery(scheme) || scheme.flows.iter().any(|flow| !flow.deprecated_flow)
    })
}

/// The usable subset: schemes carrying at least one executable flow or a
/// discovery URL. Deprecated implicit/password flows are represented in their
/// scheme's frozen descriptor but never execute, and a scheme with only those
/// flows and no discovery URL emits nothing.
pub(super) fn usable(plan: &OAuthPlan) -> Vec<&OAuthSchemePlan> {
    plan.schemes
        .iter()
        .filter(|scheme| {
            has_discovery(scheme) || scheme.flows.iter().any(|flow| !flow.deprecated_flow)
        })
        .collect()
}

/// Whether one compiled scheme carries a discovery/metadata URL, which the
/// generated runtime resolves at call time.
fn has_discovery(scheme: &OAuthSchemePlan) -> bool {
    scheme.discovery.is_some()
}

fn has_discovery_among(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes.iter().any(|scheme| has_discovery(scheme))
}

fn has_revocation(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| scheme.revocation_endpoint.is_some())
}

fn has_introspection(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| scheme.introspection_endpoint.is_some())
}

fn has_device(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| executable(scheme, OAuthFlowDescriptorKind::DeviceAuthorization))
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so schemes without one compile
/// exactly the pre-replay bytes.
fn has_client_credentials(schemes: &[&OAuthSchemePlan]) -> bool {
    schemes
        .iter()
        .any(|scheme| executable(scheme, OAuthFlowDescriptorKind::ClientCredentials))
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
pub(super) fn no_replay_requirements(
    schemes: &[&OAuthSchemePlan],
    operations: &[super::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.protocol().responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.protocol().security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. }
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

/// The value exports of the generated module, exactly as emitted.
const VALUE_EXPORTS: &[&str] = &[
    "MemoryTokenStore",
    "AuthError",
    "isAuthError",
    "oauthSchemes",
    "tokenStoreKey",
    "createClientCredentialsProvider",
    "createRefreshProvider",
    "refreshToken",
    "beginAuthorization",
    "completeAuthorization",
];

/// The type exports of the generated module, exactly as emitted.
const TYPE_EXPORTS: &[&str] = &[
    "TokenSet",
    "TokenStore",
    "AuthErrorKind",
    "CompiledFlowKind",
    "CompiledFlow",
    "CompiledScheme",
    "AuthorizationTransaction",
    "AuthorizationBegin",
    "ClientCredentialsProviderOptions",
    "RefreshProviderOptions",
    "RefreshOptions",
    "BeginAuthorizationOptions",
    "CompleteAuthorizationOptions",
];

/// Every public TypeScript symbol the generated module exports for this
/// scheme set; reserved against operation and model symbol collisions when
/// OAuth emission participates. The replaying credential wrapper joins the
/// surface only when a compiled scheme carries an executable
/// client-credentials flow, the provider type it wraps.
pub(super) fn exported_symbols(schemes: &[&OAuthSchemePlan]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = VALUE_EXPORTS
        .iter()
        .chain(TYPE_EXPORTS.iter())
        .copied()
        .collect();
    if has_revocation(schemes) {
        names.extend(["revoke", "RevokeOptions"]);
    }
    if has_introspection(schemes) {
        names.extend(["introspect", "IntrospectOptions"]);
    }
    if has_device(schemes) {
        names.extend([
            "beginDeviceAuthorization",
            "BeginDeviceAuthorizationOptions",
        ]);
    }
    if has_client_credentials(schemes) {
        names.extend(["createReplayingCredentialsProvider", "ReplayingCredential"]);
    }
    names
}

/// The operations.ts re-export block, byte-exact with the generated module's
/// conditional export surface.
pub(super) fn reexports(schemes: &[&OAuthSchemePlan]) -> String {
    let mut values = VALUE_EXPORTS.to_vec();
    let mut types = TYPE_EXPORTS.to_vec();
    if has_revocation(schemes) {
        values.push("revoke");
        types.push("RevokeOptions");
    }
    if has_introspection(schemes) {
        values.push("introspect");
        types.push("IntrospectOptions");
    }
    if has_device(schemes) {
        values.push("beginDeviceAuthorization");
        types.push("BeginDeviceAuthorizationOptions");
    }
    if has_client_credentials(schemes) {
        values.push("createReplayingCredentialsProvider");
        types.push("ReplayingCredential");
    }
    format!(
        "/** Compiled OAuth lifecycle: frozen per-scheme descriptors plus the native token store, credential providers and flow helpers. The operation runtime itself neither acquires nor refreshes tokens. */\nexport {{ {} }} from './oauth.js';\nexport type {{ {} }} from './oauth.js';\n",
        values.join(", "),
        types.join(", "),
    )
}

fn q(value: &str) -> String {
    serde_json::to_string(value).expect("string is JSON")
}

fn optional(value: &Option<String>) -> String {
    value
        .as_ref()
        .map_or_else(|| "null".to_owned(), |value| q(value))
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
                .map(|(name, description)| format!("{}:{}", q(name), q(description)))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "/* @__PURE__ */ Object.freeze({{ kind: {}, authorizationUrl: {}, tokenUrl: {}, refreshUrl: {}, deviceAuthorizationUrl: {}, clientAuth: {}, deprecated: {}, scopes: /* @__PURE__ */ Object.freeze({{{}}} as const) }} as const)",
                q(flow_kind(flow.kind)),
                optional(&flow.authorization_url),
                optional(&flow.token_url),
                optional(&flow.refresh_url),
                optional(&flow.device_authorization_url),
                q(client_auth(flow.client_auth)),
                flow.deprecated_flow,
                scopes,
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "  {}: /* @__PURE__ */ Object.freeze({{ name: {}, kind: {}, refreshSkewSeconds: {}, discovery: {}, revocationEndpoint: {}, introspectionEndpoint: {}, clientIdEnv: {}, clientSecretEnv: {}, flows: /* @__PURE__ */ Object.freeze([{}]) }} as const),",
        q(&scheme.name),
        q(&scheme.name),
        q(scheme_kind(scheme.kind)),
        scheme.refresh_skew_seconds,
        optional(&scheme.discovery),
        optional(&scheme.revocation_endpoint),
        optional(&scheme.introspection_endpoint),
        optional(&scheme.client_id_env),
        optional(&scheme.client_secret_env),
        flows,
    )
}

/// The generated `typescript/oauth.ts` module: frozen compiled descriptors
/// plus the native lifecycle library. Plans without a discovery URL assemble
/// byte-identically to the pre-discovery emission; plans with one emit the
/// discovery-aware providers and the discovery engine. The replaying
/// credential wrapper joins only when a compiled scheme carries an executable
/// client-credentials flow; everything else stays byte-identical.
pub(super) fn emit(
    schemes: &[&OAuthSchemePlan],
    no_replay: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    let discovery = has_discovery_among(schemes);
    let mut code = String::new();
    code.push_str(CORE_PREFIX);
    code.push_str(if discovery {
        CORE_HEADER_DISCOVERY
    } else {
        CORE_HEADER_PLAIN
    });
    code.push_str(CORE_MID);
    code.push_str(if discovery {
        CORE_KINDS_DISCOVERY
    } else {
        CORE_KINDS_PLAIN
    });
    code.push_str(CORE_TAIL);
    code.push_str(if discovery {
        SCHEMES_HEADER_DISCOVERY
    } else {
        SCHEMES_HEADER_PLAIN
    });
    code.push_str("const schemes: Readonly<Record<string, CompiledScheme>> = {\n");
    for scheme in schemes {
        code.push_str(&scheme_entry(scheme));
        code.push('\n');
    }
    code.push_str("};\n/** The compiled OAuth scheme descriptors for this package, keyed by exact source scheme name. The initializer only freezes the plain generated scheme data, so a bundler may shed it when the consumer references no OAuth lifecycle helper. */\nexport const oauthSchemes = /* @__PURE__ */ Object.freeze(schemes);\n");
    if discovery {
        code.push_str(API_CC_DISCOVERY);
    } else {
        code.push_str(API_CC);
    }
    if discovery {
        code.push_str(API_REFRESH_DISCOVERY);
    } else {
        code.push_str(API_REFRESH);
    }
    code.push_str(API_CODE);
    if has_revocation(schemes) {
        code.push_str(if discovery { REVOKE_DISCOVERY } else { REVOKE });
    }
    if has_introspection(schemes) {
        code.push_str(if discovery {
            INTROSPECT_DISCOVERY
        } else {
            INTROSPECT
        });
    }
    if has_device(schemes) {
        code.push_str(DEVICE);
    }
    if discovery {
        code.push_str(DISCOVERY);
    }
    if has_client_credentials(schemes) {
        code.push_str(&replay_section(schemes, no_replay));
    }
    code
}

/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow, and its
/// plain and discovery variants resolve the refresh endpoint through the
/// same compiled precedence as the provider they wrap.
fn replay_section(
    schemes: &[&OAuthSchemePlan],
    no_replay: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    let discovery = has_discovery_among(schemes);
    let mut code = String::from(
        "\n/** Compiled stream-protected requirements: security-requirement source pointers whose attaches are never replayed, because delivered stream data prevents a transparent restart. */\nconst noReplayRequirements: Readonly<Record<string, ReadonlySet<string>>> = {\n",
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
        code.push_str(&format!(
            "    {}: /* @__PURE__ */ new Set([{}] as const),\n",
            q(&scheme.name),
            rendered
        ));
    }
    code.push_str("};\n");
    code.push_str(REPLAY_INTERFACES);
    if discovery {
        code.push_str(REPLAY_DISCOVERY);
    } else {
        code.push_str(REPLAY_PLAIN);
    }
    code
}

/// The static core half of the generated module: types, the typed error, the
/// token store, request plumbing and PKCE. Descriptors and the public API are
/// appended around these constants by `emit`. The constants concatenate into
/// exactly the pre-discovery core bytes for plans without a discovery URL.
const CORE_PREFIX: &str = r#"// Generated OAuth 2.0 lifecycle for this package's compiled OAuth schemes.
//
// The frozen `oauthSchemes` descriptors compile the generation-time OAuth
// plan: endpoint URLs, per-flow client authentication, refresh skew,
// environment variable names and scope metadata are exactly what the source
// declared plus the explicitly configured supplements. This module never
"#;

/// The header paragraph after "This module never": byte-exact for plans
/// without a discovery URL, and the discovery paragraph for plans with one.
const CORE_HEADER_PLAIN: &str = r#"// parses OpenAPI and never fetches discovery documents.
//
"#;
const CORE_HEADER_DISCOVERY: &str = r#"// parses OpenAPI; endpoint URLs that the compiled flows omit resolve through
// RFC 8414 / OpenID Connect discovery when the scheme compiles a discovery
// URL.
//
"#;

const CORE_MID: &str = r#"// Token requests are form-encoded (RFC 6749). Client authentication follows
// the compiled per-flow policy: `client-secret-basic` sends HTTP Basic on the
// request; `none` is the public profile and never sends a secret. Access
// tokens, refresh tokens and client secrets never appear in error messages;
// AuthError carries only safe metadata (kind, scheme, status, server error
// code, retry hint).
//
// The operation runtime neither acquires nor refreshes tokens and never
// retries requests. Providers here integrate as ordinary caller credentials:
// pass the returned callback as the scheme's `auth` member. On-demand refresh
// hook: when a protected call still fails with a declared 401, call
// `refreshToken` explicitly and retry.
import type { AuthorizationCredential, CredentialContext, CredentialProvider, Fetch } from './http/types.js';

/** One acquired token set. `expiresAt` is the epoch millisecond at which the access token expires; a response without `expires_in` never expires. Freshness checks additionally apply the compiled refresh skew. */
export interface TokenSet {
    readonly accessToken: string;
    readonly tokenType: string;
    readonly expiresAt: number;
    readonly refreshToken?: string;
    readonly scope?: string;
    /** Optional account partition metadata; this module never invents a value for it. */
    readonly issuerAccountId?: string;
}

/** Caller-implementable token persistence. Keys partition stored token sets by scheme, token-endpoint issuer and client identity (see tokenStoreKey); values are whole token sets replaced atomically. */
export interface TokenStore {
    load(key: string): Promise<TokenSet | null>;
    replace(key: string, tokenSet: TokenSet): Promise<void>;
    clear(key: string): Promise<void>;
}

/** In-process token store owned by the provider or caller that created it; this module never keeps a global store. */
export class MemoryTokenStore implements TokenStore {
    private readonly tokens = new Map<string, TokenSet>();
    async load(key: string): Promise<TokenSet | null> {
        const found = this.tokens.get(key);
        return found === undefined ? null : found;
    }
    async replace(key: string, tokenSet: TokenSet): Promise<void> {
        this.tokens.set(key, Object.freeze({ ...tokenSet }));
    }
    async clear(key: string): Promise<void> {
        this.tokens.delete(key);
    }
}

/** Typed OAuth lifecycle failure kinds. */
"#;

/// The typed failure kinds: byte-exact without discovery, with the
/// `discovery-failed` kind added when any compiled scheme carries a
/// discovery URL.
const CORE_KINDS_PLAIN: &str = r#"export type AuthErrorKind =
    | 'invalid-request'
    | 'invalid-client'
    | 'invalid-grant'
    | 'unauthorized-client'
    | 'unsupported-grant-type'
    | 'invalid-scope'
    | 'server-error'
    | 'invalid-response'
    | 'transport-failure'
    | 'state-mismatch'
    | 'transaction-consumed'
    | 'missing-credential'
    | 'endpoint-unavailable'
    | 'crypto-unavailable'
    | 'authorization-pending'
    | 'slow-down'
    | 'device-code-expired'
    | 'aborted';
"#;
const CORE_KINDS_DISCOVERY: &str = r#"export type AuthErrorKind =
    | 'invalid-request'
    | 'invalid-client'
    | 'invalid-grant'
    | 'unauthorized-client'
    | 'unsupported-grant-type'
    | 'invalid-scope'
    | 'server-error'
    | 'invalid-response'
    | 'transport-failure'
    | 'state-mismatch'
    | 'transaction-consumed'
    | 'missing-credential'
    | 'endpoint-unavailable'
    | 'discovery-failed'
    | 'crypto-unavailable'
    | 'authorization-pending'
    | 'slow-down'
    | 'device-code-expired'
    | 'aborted';
"#;

const CORE_TAIL: &str = r#"
/** Typed OAuth lifecycle failure. Messages never contain token or secret values; fields carry only safe metadata. */
export class AuthError extends Error {
    readonly suspectAuthError = true as const;
    readonly kind: AuthErrorKind;
    readonly scheme: string;
    readonly status: number | undefined;
    readonly serverError: string | undefined;
    readonly retryAfterSeconds: number | undefined;
    constructor(kind: AuthErrorKind, scheme: string, message: string, metadata: { readonly status?: number; readonly serverError?: string; readonly retryAfterSeconds?: number } = {}) {
        super(message);
        this.name = 'AuthError';
        this.kind = kind;
        this.scheme = scheme;
        this.status = metadata.status;
        this.serverError = metadata.serverError;
        this.retryAfterSeconds = metadata.retryAfterSeconds;
    }
}

/** Tests whether a caught value is the generated OAuth lifecycle error. */
export function isAuthError(error: unknown): error is AuthError {
    return error instanceof AuthError
        || (typeof error === 'object' && error !== null && (error as { readonly suspectAuthError?: unknown }).suspectAuthError === true);
}

/** Compiled flow kinds, including the represented-but-never-executed legacy grants. */
export type CompiledFlowKind = 'client-credentials' | 'authorization-code' | 'device-authorization' | 'implicit' | 'password';

/** One compiled flow descriptor: exactly what the source declared. */
export interface CompiledFlow {
    readonly kind: CompiledFlowKind;
    readonly authorizationUrl: string | null;
    readonly tokenUrl: string | null;
    readonly refreshUrl: string | null;
    readonly deviceAuthorizationUrl: string | null;
    readonly clientAuth: 'client-secret-basic' | 'none';
    readonly deprecated: boolean;
    readonly scopes: Readonly<Record<string, string>>;
}

/** One compiled scheme descriptor: exactly what the source declared plus the explicitly configured supplements. */
export interface CompiledScheme {
    readonly name: string;
    readonly kind: 'oauth2' | 'open-id-connect';
    readonly refreshSkewSeconds: number;
    readonly discovery: string | null;
    readonly revocationEndpoint: string | null;
    readonly introspectionEndpoint: string | null;
    readonly clientIdEnv: string | null;
    readonly clientSecretEnv: string | null;
    readonly flows: readonly CompiledFlow[];
}

/** The exact token-store key for one scheme, token-endpoint issuer and client identity. Identical inputs always yield identical keys; stored token sets are partitioned by all three. */
export function tokenStoreKey(scheme: string, issuer: string, clientId?: string): string {
    return `${scheme}|${issuer}|${clientId ?? 'public'}`;
}

const consumedTransactions = new WeakSet<AuthorizationTransaction>();

interface ClientIdentity {
    readonly id: string | undefined;
    readonly secret: string | undefined;
}

interface ClientAuthPlan {
    readonly basic: boolean;
    readonly id: string | undefined;
    readonly secret: string | undefined;
}

interface DeviceGrant {
    readonly deviceCode: string;
    readonly expiresAt: number;
    readonly intervalSeconds: number;
}

function compiled(scheme: string): CompiledScheme {
    if (!Object.hasOwn(schemes, scheme)) throw new AuthError('endpoint-unavailable', scheme, 'no compiled OAuth scheme carries that name; oauth.ts compiles exactly the source-declared schemes with executable flows');
    return schemes[scheme]!;
}

/** Resolves one executable (non-deprecated) compiled flow. */
function executableFlow(scheme: CompiledScheme, kind: Exclude<CompiledFlowKind, 'implicit' | 'password'>): CompiledFlow {
    const found = scheme.flows.find(candidate => candidate.kind === kind && !candidate.deprecated);
    if (found === undefined) throw new AuthError('endpoint-unavailable', scheme.name, `scheme ${JSON.stringify(scheme.name)} has no executable ${kind} flow in its source declaration`);
    return found;
}

/** The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL. */
function refreshFlow(scheme: CompiledScheme): CompiledFlow {
    const executable = scheme.flows.filter(candidate => !candidate.deprecated);
    const preferred = executable.find(candidate => candidate.kind === 'authorization-code') ?? executable.find(candidate => candidate.tokenUrl !== null);
    if (preferred === undefined) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled scheme carries no executable flow with a token URL');
    return preferred;
}

/** The refresh endpoint: the declared refresh URL, else the flow's token URL. */
function refreshEndpoint(grant: CompiledFlow, scheme: CompiledScheme): string {
    const endpoint = grant.refreshUrl ?? grant.tokenUrl;
    if (endpoint === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled flow declares neither a refresh URL nor a token URL');
    return endpoint;
}

/** Reads one compiled environment variable name. Generation supplied the names only; values are read at request time. */
function environmentValue(variable: string | null): string | undefined {
    if (variable === null) return undefined;
    try {
        const host = (globalThis as { process?: { env?: unknown } }).process;
        const value = host?.env;
        if (value !== null && typeof value === 'object') {
            const found = (value as Record<string, unknown>)[variable];
            return typeof found === 'string' && found !== '' ? found : undefined;
        }
    } catch { /* An unavailable host environment means no defaults. */ }
    return undefined;
}

/** Client identity for one request: explicit arguments win, then the compiled environment variable names. */
function resolveIdentity(scheme: CompiledScheme, clientId: string | undefined, clientSecret: string | undefined): ClientIdentity {
    return {
        id: clientId ?? environmentValue(scheme.clientIdEnv),
        secret: clientSecret ?? environmentValue(scheme.clientSecretEnv),
    };
}

/** Client authentication follows the compiled per-flow policy: `client-secret-basic` sends HTTP Basic; `none` is the public profile and never sends a secret. */
function clientAuthFor(grant: CompiledFlow, scheme: CompiledScheme, identity: ClientIdentity): ClientAuthPlan {
    const basic = grant.clientAuth === 'client-secret-basic';
    return { basic, id: identity.id, secret: basic ? identity.secret : undefined };
}

/** Freshness applies the compiled refresh skew: a token is fresh when it outlives now by more than the skew. */
function isFresh(tokenSet: TokenSet, skewMilliseconds: number, now: number): boolean {
    return tokenSet.expiresAt - skewMilliseconds > now;
}

/** Builds the complete Authorization value for one stored token set. Values never appear in errors. */
function authorization(tokenSet: TokenSet, scheme: string): AuthorizationCredential {
    const type = tokenSet.tokenType.trim();
    if (type === '' || !/^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/.test(type)) throw new AuthError('invalid-response', scheme, 'the token type is not a usable authorization scheme');
    if (typeof tokenSet.accessToken !== 'string' || tokenSet.accessToken === '' || /[\u0000-\u001f\u007f]/.test(tokenSet.accessToken)) throw new AuthError('invalid-response', scheme, 'the access token is not a usable credential value');
    return { authorization: `${type} ${tokenSet.accessToken}` };
}

/** Atomic store replacement: a server-rotated refresh token is adopted; when the response carries none, the previous refresh token is retained. */
function adopt(previous: TokenSet | null, next: TokenSet): TokenSet {
    if (next.refreshToken !== undefined || previous === null || previous.refreshToken === undefined) return next;
    return Object.freeze({ ...next, refreshToken: previous.refreshToken });
}

function base64(bytes: Uint8Array): string {
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    let result = '';
    for (let index = 0; index < bytes.length; index += 3) {
        const a = bytes[index]!, b = bytes[index + 1], c = bytes[index + 2];
        result += alphabet[a >> 2]! + alphabet[(a & 3) << 4 | (b ?? 0) >> 4]! +
            (b === undefined ? '=' : alphabet[(b & 15) << 2 | (c ?? 0) >> 6]!) + (c === undefined ? '=' : alphabet[c & 63]!);
    }
    return result;
}

function base64Url(bytes: Uint8Array): string {
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
    let result = '';
    for (let index = 0; index < bytes.length; index += 3) {
        const a = bytes[index]!, b = bytes[index + 1], c = bytes[index + 2];
        result += alphabet[a >> 2]!;
        if (b === undefined) { result += alphabet[(a & 3) << 4]!; break; }
        result += alphabet[(a & 3) << 4 | b >> 4]!;
        if (c === undefined) { result += alphabet[(b & 15) << 2]!; break; }
        result += alphabet[(b & 15) << 2 | c >> 6]! + alphabet[c & 63]!;
    }
    return result;
}

function webCrypto(scheme: string): Crypto {
    const value = (globalThis as { crypto?: Crypto }).crypto;
    if (value === undefined || typeof value.getRandomValues !== 'function' || value.subtle === undefined) {
        throw new AuthError('crypto-unavailable', scheme, 'Web Crypto (crypto.getRandomValues and crypto.subtle) is required for the authorization-code PKCE flow');
    }
    return value;
}

function randomBase64Url(scheme: string, byteLength: number): string {
    const bytes = new Uint8Array(byteLength);
    webCrypto(scheme).getRandomValues(bytes);
    return base64Url(bytes);
}

async function sha256Base64Url(scheme: string, text: string): Promise<string> {
    const digest = await webCrypto(scheme).subtle.digest('SHA-256', new TextEncoder().encode(text));
    return base64Url(new Uint8Array(digest));
}

function formBody(fields: Readonly<Record<string, string | undefined>>): string {
    const params = new URLSearchParams();
    for (const [name, value] of Object.entries(fields)) {
        if (value !== undefined) params.set(name, value);
    }
    return params.toString();
}

function retryAfter(response: Response): number | undefined {
    const value = response.headers.get('retry-after');
    if (value === null || !/^[0-9]+$/.test(value.trim())) return undefined;
    return Number.parseInt(value, 10);
}

const SERVER_ERROR_KINDS: Readonly<Record<string, AuthErrorKind>> = Object.freeze({
    invalid_request: 'invalid-request',
    invalid_client: 'invalid-client',
    invalid_grant: 'invalid-grant',
    unauthorized_client: 'unauthorized-client',
    unsupported_grant_type: 'unsupported-grant-type',
    invalid_scope: 'invalid-scope',
    access_denied: 'server-error',
    unsupported_response_type: 'server-error',
    server_error: 'server-error',
    temporarily_unavailable: 'server-error',
    authorization_pending: 'authorization-pending',
    slow_down: 'slow-down',
    expired_token: 'device-code-expired',
});

/** Maps one rejected endpoint response to the typed error; the message carries only the machine error code, status and scheme. */
function serverFailure(scheme: string, serverError: string | undefined, status: number, retryAfterSeconds: number | undefined): AuthError {
    const kind = serverError === undefined ? 'server-error' : SERVER_ERROR_KINDS[serverError] ?? 'server-error';
    const label = serverError === undefined ? `HTTP ${status}` : `${serverError}, HTTP ${status}`;
    return new AuthError(kind, scheme, `the authorization server rejected the request (${label})`, {
        status,
        ...(serverError === undefined ? {} : { serverError }),
        ...(retryAfterSeconds === undefined ? {} : { retryAfterSeconds }),
    });
}

/** One form-encoded endpoint POST with the compiled client authentication; a non-2xx response becomes the typed error. */
async function postForm(scheme: string, endpoint: string, auth: ClientAuthPlan, fields: Readonly<Record<string, string | undefined>>, transport: Fetch, signal: AbortSignal | undefined): Promise<Response> {
    const headers = new Headers({ 'content-type': 'application/x-www-form-urlencoded', accept: 'application/json' });
    if (auth.basic) {
        const secret = auth.secret;
        if (secret === undefined) throw new AuthError('missing-credential', scheme, 'the compiled client authentication is client-secret-basic and no client secret is available');
        headers.set('authorization', `Basic ${base64(new TextEncoder().encode(`${auth.id ?? ''}:${secret}`))}`);
    }
    let response: Response;
    try {
        response = await transport(endpoint, { method: 'POST', headers, body: formBody(fields), ...(signal === undefined ? {} : { signal }) });
    } catch {
        if (signal?.aborted === true) throw new AuthError('aborted', scheme, 'the endpoint request was aborted');
        throw new AuthError('transport-failure', scheme, 'the endpoint request failed before a response arrived');
    }
    if (response.status < 200 || response.status > 299) {
        const after = retryAfter(response);
        let serverError: string | undefined;
        try {
            const decoded: unknown = await response.json();
            if (decoded !== null && typeof decoded === 'object' && !Array.isArray(decoded)) {
                const value = (decoded as Record<string, unknown>).error;
                if (typeof value === 'string' && value !== '') serverError = value;
            }
        } catch { /* An unreadable error body is safe metadata loss. */ }
        throw serverFailure(scheme, serverError, response.status, after);
    }
    return response;
}

async function jsonObject(scheme: string, response: Response, what: string): Promise<Record<string, unknown>> {
    let decoded: unknown;
    try { decoded = await response.json(); }
    catch { throw new AuthError('invalid-response', scheme, `the ${what} response is not readable JSON`); }
    if (decoded === null || typeof decoded !== 'object' || Array.isArray(decoded)) {
        throw new AuthError('invalid-response', scheme, `the ${what} response is not a JSON object`);
    }
    return decoded as Record<string, unknown>;
}

function tokenSetFrom(scheme: string, body: Record<string, unknown>, clock: () => number): TokenSet {
    const accessToken = body.access_token;
    if (typeof accessToken !== 'string' || accessToken === '' || /[\u0000-\u001f\u007f]/.test(accessToken)) {
        throw new AuthError('invalid-response', scheme, 'the token response carries no usable access token');
    }
    const tokenType = typeof body.token_type === 'string' && body.token_type.trim() !== '' ? body.token_type.trim() : 'Bearer';
    const expiresIn = body.expires_in;
    const expiresAt = typeof expiresIn === 'number' && Number.isFinite(expiresIn) && expiresIn > 0
        ? clock() + expiresIn * 1000
        : Number.MAX_SAFE_INTEGER;
    const refreshToken = typeof body.refresh_token === 'string' && body.refresh_token !== '' ? body.refresh_token : undefined;
    const scope = typeof body.scope === 'string' && body.scope !== '' ? body.scope : undefined;
    return Object.freeze({
        accessToken,
        tokenType,
        expiresAt,
        ...(refreshToken === undefined ? {} : { refreshToken }),
        ...(scope === undefined ? {} : { scope }),
    });
}

async function tokenRequest(scheme: string, endpoint: string, auth: ClientAuthPlan, fields: Readonly<Record<string, string | undefined>>, transport: Fetch, signal: AbortSignal | undefined, clock: () => number): Promise<TokenSet> {
    const response = await postForm(scheme, endpoint, auth, fields, transport, signal);
    const body = await jsonObject(scheme, response, 'token');
    return tokenSetFrom(scheme, body, clock);
}

async function performRefresh(scheme: CompiledScheme, grant: CompiledFlow, endpoint: string, identity: ClientIdentity, refreshToken: string, previous: TokenSet | null, store: TokenStore | undefined, transport: Fetch, signal: AbortSignal | undefined, clock: () => number): Promise<TokenSet> {
    const acquired = await tokenRequest(scheme.name, endpoint, clientAuthFor(grant, scheme, identity), { grant_type: 'refresh_token', refresh_token: refreshToken }, transport, signal, clock);
    const adopted = adopt(previous, acquired);
    if (store !== undefined) await store.replace(tokenStoreKey(scheme.name, endpoint, identity.id), adopted);
    return adopted;
}

function delay(milliseconds: number, signal: AbortSignal | undefined): Promise<void> {
    return new Promise((resolve, reject) => {
        if (signal?.aborted === true) { reject(new Error('aborted')); return; }
        const onAbort = () => { clearTimeout(timer); reject(new Error('aborted')); };
        const timer = setTimeout(() => { if (signal !== undefined) signal.removeEventListener('abort', onAbort); resolve(); }, milliseconds);
        if (signal !== undefined) signal.addEventListener('abort', onAbort, { once: true });
    });
}

async function deviceRequest(scheme: string, endpoint: string, identity: ClientIdentity, transport: Fetch, signal: AbortSignal | undefined, clock: () => number): Promise<DeviceGrant> {
    const response = await postForm(scheme, endpoint, { basic: false, id: identity.id, secret: undefined }, { ...(identity.id === undefined ? {} : { client_id: identity.id }) }, transport, signal);
    const body = await jsonObject(scheme, response, 'device-authorization');
    const deviceCode = body.device_code;
    if (typeof deviceCode !== 'string' || deviceCode === '' || /[\u0000-\u001f\u007f]/.test(deviceCode)) {
        throw new AuthError('invalid-response', scheme, 'the device-authorization response carries no usable device code');
    }
    const expiresInSeconds = typeof body.expires_in === 'number' && Number.isFinite(body.expires_in) && body.expires_in > 0 ? body.expires_in : 600;
    const intervalSeconds = typeof body.interval === 'number' && Number.isFinite(body.interval) && body.interval > 0 ? body.interval : 5;
    return { deviceCode, expiresAt: clock() + expiresInSeconds * 1000, intervalSeconds };
}
"#;

/// The public API half of the generated module: credential providers, explicit
/// refresh and the authorization-code flow. Conditional helpers are appended
/// after these constants by `emit`. The plain variants concatenate into
/// exactly the pre-discovery API bytes for plans without a discovery URL.
const API_CC: &str = r#"
/** Options for the compiled client-credentials credential provider. Client identity defaults to the compiled environment variable names when arguments are omitted. */
export interface ClientCredentialsProviderOptions {
    readonly scheme: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    /** Explicit scope string sent with the token request; nothing is inferred from operations. */
    readonly scope?: string;
}

/**
 * Creates a caller credential for the compiled client-credentials flow: on
 * attach it serves a fresh token from the store, otherwise it acquires one
 * with a single form-encoded token request. Concurrent attaches share one
 * in-flight acquisition (single-flight) and the store is replaced atomically;
 * a response refresh token is adopted, else a previous one is retained. Pass
 * the returned callback as the scheme's `auth` member.
 */
export function createClientCredentialsProvider(options: ClientCredentialsProviderOptions): CredentialProvider<AuthorizationCredential> {
    const store = options.store ?? new MemoryTokenStore();
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const inflight = new Map<string, Promise<TokenSet>>();
    return (context: CredentialContext): Promise<AuthorizationCredential> => {
        const scheme = compiled(options.scheme);
        const grant = executableFlow(scheme, 'client-credentials');
        const tokenUrl = grant.tokenUrl;
        if (tokenUrl === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled client-credentials flow declares no token URL');
        const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
        const key = tokenStoreKey(scheme.name, tokenUrl, identity.id);
        const pending = inflight.get(key);
        if (pending !== undefined) return pending.then(tokenSet => authorization(tokenSet, scheme.name));
        const tracked = (async (): Promise<TokenSet> => {
            const stored = await store.load(key);
            if (stored !== null && isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) return stored;
            const acquired = await tokenRequest(scheme.name, tokenUrl, clientAuthFor(grant, scheme, identity), {
                grant_type: 'client_credentials',
                ...(options.scope === undefined ? {} : { scope: options.scope }),
                ...(grant.clientAuth === 'none' && identity.id !== undefined ? { client_id: identity.id } : {}),
            }, transport, context.signal, clock);
            const adopted = adopt(stored, acquired);
            await store.replace(key, adopted);
            return adopted;
        })();
        const guarded = tracked.finally(() => { inflight.delete(key); });
        inflight.set(key, guarded);
        return guarded.then(tokenSet => authorization(tokenSet, scheme.name));
    };
}
"#;

const API_REFRESH: &str = r#"
/** Options for the store-backed refresh credential provider. */
export interface RefreshProviderOptions {
    readonly scheme: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
}

/**
 * Creates a caller credential that serves stored tokens and refreshes them on
 * demand: on attach a fresh stored token is returned; an expired one is
 * refreshed exactly once with its stored refresh token before serving. There
 * is no token at all until an authorization or device flow has completed.
 * Automatic retry hook: when a protected call still fails with a declared 401,
 * call `refreshToken` explicitly and retry; the SDK itself never retries.
 */
export function createRefreshProvider(options: RefreshProviderOptions): CredentialProvider<AuthorizationCredential> {
    const store = options.store ?? new MemoryTokenStore();
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    return async (context: CredentialContext): Promise<AuthorizationCredential> => {
        const scheme = compiled(options.scheme);
        const grant = refreshFlow(scheme);
        const endpoint = refreshEndpoint(grant, scheme);
        const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
        const stored = await store.load(tokenStoreKey(scheme.name, endpoint, identity.id));
        if (stored === null) throw new AuthError('missing-credential', scheme.name, 'no stored token set exists for this scheme; complete an authorization or device flow first');
        if (isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) return authorization(stored, scheme.name);
        if (stored.refreshToken === undefined) throw new AuthError('missing-credential', scheme.name, 'the stored token set carries no refresh token');
        const refreshed = await performRefresh(scheme, grant, endpoint, identity, stored.refreshToken, stored, store, transport, context.signal, clock);
        return authorization(refreshed, scheme.name);
    };
}

/** Options for an explicit refresh-token exchange. */
export interface RefreshOptions {
    readonly scheme: string;
    readonly refreshToken: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    /** When supplied, the refreshed set (adopted or retained refresh token included) atomically replaces the stored set. */
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Exchanges one refresh token at the declared refresh URL or token URL
 * (grant_type=refresh_token) and returns the token set. A rotated refresh
 * token from the response is adopted; when the response carries none and a
 * previous stored set exists, the previous refresh token is retained.
 */
export async function refreshToken(options: RefreshOptions): Promise<TokenSet> {
    if (typeof options.refreshToken !== 'string' || options.refreshToken === '' || /[\u0000-\u001f\u007f]/.test(options.refreshToken)) {
        throw new AuthError('invalid-request', options.scheme, 'refreshToken must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    const grant = refreshFlow(scheme);
    const endpoint = refreshEndpoint(grant, scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const previous = options.store === undefined ? null : await options.store.load(tokenStoreKey(scheme.name, endpoint, identity.id));
    return performRefresh(scheme, grant, endpoint, identity, options.refreshToken, previous, options.store, transport, options.signal, clock);
}
"#;

const API_CODE: &str = r#"
/** A bound authorization-code transaction: frozen, session-scoped and consumed exactly once by completeAuthorization, whether the exchange succeeds or fails. */
export interface AuthorizationTransaction {
    readonly scheme: string;
    readonly state: string;
    readonly codeVerifier: string;
    readonly redirectUri: string;
    readonly tokenUrl: string;
    readonly createdAt: number;
}

/** The authorization redirect target plus the bound transaction. */
export interface AuthorizationBegin {
    readonly authorizationUrl: string;
    readonly state: string;
    readonly transaction: AuthorizationTransaction;
}

export interface BeginAuthorizationOptions {
    readonly scheme: string;
    readonly redirectUri: string;
    readonly clientId?: string;
    readonly scopes?: readonly string[];
    readonly clock?: () => number;
}

/**
 * Starts an authorization-code flow with PKCE (S256): generates the random
 * state and code verifier with crypto.getRandomValues, computes the S256
 * challenge with SHA-256 and returns the exact authorization URL to redirect
 * to plus the bound transaction. No network request is made.
 */
export async function beginAuthorization(options: BeginAuthorizationOptions): Promise<AuthorizationBegin> {
    const scheme = compiled(options.scheme);
    const grant = executableFlow(scheme, 'authorization-code');
    const authorizationEndpoint = grant.authorizationUrl;
    if (authorizationEndpoint === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled authorization-code flow declares no authorization URL');
    const tokenUrl = grant.tokenUrl;
    if (tokenUrl === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled authorization-code flow declares no token URL');
    if (typeof options.redirectUri !== 'string' || options.redirectUri === '') throw new AuthError('invalid-request', scheme.name, 'redirectUri is required');
    let redirect: URL;
    try { redirect = new URL(options.redirectUri); }
    catch { throw new AuthError('invalid-request', scheme.name, 'redirectUri must be an absolute URI'); }
    if (redirect.hash !== '') throw new AuthError('invalid-request', scheme.name, 'redirectUri must not include a fragment');
    const identity = resolveIdentity(scheme, options.clientId, undefined);
    if (identity.id === undefined) throw new AuthError('missing-credential', scheme.name, 'a client id is required for the authorization-code flow; pass clientId or set the compiled environment variable');
    const clock = options.clock ?? (() => Date.now());
    const state = randomBase64Url(scheme.name, 16);
    const codeVerifier = randomBase64Url(scheme.name, 32);
    const codeChallenge = await sha256Base64Url(scheme.name, codeVerifier);
    let authorizationUrl: URL;
    try { authorizationUrl = new URL(authorizationEndpoint); }
    catch { throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled authorization URL cannot be parsed'); }
    authorizationUrl.searchParams.set('response_type', 'code');
    authorizationUrl.searchParams.set('client_id', identity.id);
    authorizationUrl.searchParams.set('redirect_uri', options.redirectUri);
    authorizationUrl.searchParams.set('state', state);
    authorizationUrl.searchParams.set('code_challenge', codeChallenge);
    authorizationUrl.searchParams.set('code_challenge_method', 'S256');
    if (options.scopes !== undefined && options.scopes.length > 0) authorizationUrl.searchParams.set('scope', options.scopes.join(' '));
    const transaction: AuthorizationTransaction = Object.freeze({
        scheme: scheme.name,
        state,
        codeVerifier,
        redirectUri: options.redirectUri,
        tokenUrl,
        createdAt: clock(),
    });
    return Object.freeze({ authorizationUrl: authorizationUrl.toString(), state, transaction });
}

export interface CompleteAuthorizationOptions {
    readonly transaction: AuthorizationTransaction;
    readonly code: string;
    /** The state value received with the redirect; it must equal the transaction's state exactly. */
    readonly state: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Completes one authorization-code transaction: validates the redirect state,
 * exchanges the code at the transaction's token URL with the stored code
 * verifier, and replaces the stored token set atomically. The transaction is
 * consumed exactly once by any attempt; a failed exchange requires beginning
 * a new authorization.
 */
export async function completeAuthorization(options: CompleteAuthorizationOptions): Promise<TokenSet> {
    const transaction = options.transaction;
    if (consumedTransactions.has(transaction)) throw new AuthError('transaction-consumed', transaction.scheme, 'this authorization transaction was already consumed; begin a new authorization');
    consumedTransactions.add(transaction);
    const scheme = compiled(transaction.scheme);
    if (options.state !== transaction.state) throw new AuthError('state-mismatch', scheme.name, 'the redirect state does not match the authorization transaction');
    if (typeof options.code !== 'string' || options.code === '' || /[\u0000-\u001f\u007f]/.test(options.code)) throw new AuthError('invalid-request', scheme.name, 'code must be a nonempty string without control characters');
    const grant = executableFlow(scheme, 'authorization-code');
    const tokenUrl = transaction.tokenUrl;
    if (tokenUrl === '') throw new AuthError('endpoint-unavailable', scheme.name, 'the authorization transaction carries no token URL');
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const acquired = await tokenRequest(scheme.name, tokenUrl, clientAuthFor(grant, scheme, identity), {
        grant_type: 'authorization_code',
        code: options.code,
        redirect_uri: transaction.redirectUri,
        code_verifier: transaction.codeVerifier,
        ...(grant.clientAuth === 'none' && identity.id !== undefined ? { client_id: identity.id } : {}),
    }, transport, options.signal, clock);
    if (options.store !== undefined) await options.store.replace(tokenStoreKey(scheme.name, tokenUrl, identity.id), acquired);
    return acquired;
}
"#;

/// RFC 7009 revocation, emitted only when a compiled scheme carries the
/// configured endpoint.
const REVOKE: &str = r#"
/** Options for revoking one token (RFC 7009). */
export interface RevokeOptions {
    readonly scheme: string;
    readonly token: string;
    readonly tokenTypeHint?: 'access_token' | 'refresh_token';
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Revokes one token at the compiled revocation endpoint (RFC 7009,
 * form-encoded). Authentication follows the compiled client policy.
 */
export async function revoke(options: RevokeOptions): Promise<void> {
    if (typeof options.token !== 'string' || options.token === '' || /[\u0000-\u001f\u007f]/.test(options.token)) {
        throw new AuthError('invalid-request', options.scheme, 'token must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    if (scheme.revocationEndpoint === null) throw new AuthError('endpoint-unavailable', scheme.name, 'no revocation endpoint was configured for this scheme');
    const grant = refreshFlow(scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    await postForm(scheme.name, scheme.revocationEndpoint, clientAuthFor(grant, scheme, identity), {
        token: options.token,
        ...(options.tokenTypeHint === undefined ? {} : { token_type_hint: options.tokenTypeHint }),
        ...(grant.clientAuth === 'none' && identity.id !== undefined ? { client_id: identity.id } : {}),
    }, transport, options.signal);
}
"#;

/// RFC 7662 introspection, emitted only when a compiled scheme carries the
/// configured endpoint.
const INTROSPECT: &str = r#"
/** Options for introspecting one token (RFC 7662). */
export interface IntrospectOptions {
    readonly scheme: string;
    readonly token: string;
    readonly tokenTypeHint?: 'access_token' | 'refresh_token';
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Introspects one token at the compiled introspection endpoint (RFC 7662,
 * form-encoded) and returns the server's JSON response as a frozen object.
 * Authentication follows the compiled client policy.
 */
export async function introspect(options: IntrospectOptions): Promise<Readonly<Record<string, unknown>>> {
    if (typeof options.token !== 'string' || options.token === '' || /[\u0000-\u001f\u007f]/.test(options.token)) {
        throw new AuthError('invalid-request', options.scheme, 'token must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    if (scheme.introspectionEndpoint === null) throw new AuthError('endpoint-unavailable', scheme.name, 'no introspection endpoint was configured for this scheme');
    const grant = refreshFlow(scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const response = await postForm(scheme.name, scheme.introspectionEndpoint, clientAuthFor(grant, scheme, identity), {
        token: options.token,
        ...(options.tokenTypeHint === undefined ? {} : { token_type_hint: options.tokenTypeHint }),
        ...(grant.clientAuth === 'none' && identity.id !== undefined ? { client_id: identity.id } : {}),
    }, transport, options.signal);
    const body = await jsonObject(scheme.name, response, 'introspection');
    return Object.freeze({ ...body });
}
"#;

/// RFC 8628 device authorization, emitted only when a compiled scheme carries
/// an executable device-authorization flow.
const DEVICE: &str = r#"
export interface BeginDeviceAuthorizationOptions {
    readonly scheme: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Runs the device authorization grant to completion (RFC 8628): requests the
 * device code at the compiled device-authorization URL, then polls the token
 * endpoint, honoring `authorization_pending`, `slow_down` (backing off five
 * seconds per occurrence), the server interval and the code's expiry, and
 * resolves with the typed token set. Resolves only after the user completes
 * authorization at the verification URI the server returned.
 */
export async function beginDeviceAuthorization(options: BeginDeviceAuthorizationOptions): Promise<TokenSet> {
    const scheme = compiled(options.scheme);
    const deviceFlow = executableFlow(scheme, 'device-authorization');
    const deviceEndpoint = deviceFlow.deviceAuthorizationUrl;
    if (deviceEndpoint === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled device-authorization flow declares no device-authorization URL');
    const tokenUrl = deviceFlow.tokenUrl;
    if (tokenUrl === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled device-authorization flow declares no token URL');
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const device = await deviceRequest(scheme.name, deviceEndpoint, identity, transport, options.signal, clock);
    let intervalMilliseconds = Math.max(1, device.intervalSeconds) * 1000;
    for (;;) {
        if (options.signal?.aborted === true) throw new AuthError('aborted', scheme.name, 'device authorization was aborted before completion');
        if (clock() >= device.expiresAt) throw new AuthError('device-code-expired', scheme.name, 'the device code expired before authorization completed');
        try { await delay(intervalMilliseconds, options.signal); }
        catch { throw new AuthError('aborted', scheme.name, 'device authorization was aborted before completion'); }
        if (clock() >= device.expiresAt) throw new AuthError('device-code-expired', scheme.name, 'the device code expired before authorization completed');
        try {
            const acquired = await tokenRequest(scheme.name, tokenUrl, clientAuthFor(deviceFlow, scheme, identity), {
                grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
                device_code: device.deviceCode,
                ...(deviceFlow.clientAuth === 'none' && identity.id !== undefined ? { client_id: identity.id } : {}),
            }, transport, options.signal, clock);
            if (options.store !== undefined) await options.store.replace(tokenStoreKey(scheme.name, tokenUrl, identity.id), acquired);
            return acquired;
        } catch (error) {
            if (isAuthError(error)) {
                if (error.serverError === 'authorization_pending') continue;
                if (error.serverError === 'slow_down') { intervalMilliseconds += 5000; continue; }
                if (error.serverError === 'expired_token') throw new AuthError('device-code-expired', scheme.name, 'the device code expired before authorization completed');
            }
            throw error;
        }
    }
}
"#;

/// The compiled-descriptors header paragraph: byte-exact without discovery,
/// discovery-aware otherwise.
const SCHEMES_HEADER_PLAIN: &str = "/** Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; OpenID Connect schemes compile no executable flows in v1 and emit nothing.\n */\n";
const SCHEMES_HEADER_DISCOVERY: &str = "/** Frozen per-scheme OAuth descriptors compiled from the source declarations plus the explicitly configured supplements. Deprecated flows are represented and never execute; a compiled discovery URL resolves the endpoint URLs the compiled flows omit at call time.\n */\n";

/// The discovery-aware client-credentials provider: endpoint resolution
/// follows the compiled precedence (explicit compiled endpoints always win;
/// otherwise the provider's cached discovery document).
const API_CC_DISCOVERY: &str = r#"
/** Options for the compiled client-credentials credential provider. Client identity defaults to the compiled environment variable names when arguments are omitted. */
export interface ClientCredentialsProviderOptions {
    readonly scheme: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    /** Explicit scope string sent with the token request; nothing is inferred from operations. */
    readonly scope?: string;
}

/**
 * Creates a caller credential for the compiled client-credentials flow: on
 * attach it serves a fresh token from the store, otherwise it acquires one
 * with a single form-encoded token request. Concurrent attaches share one
 * in-flight acquisition (single-flight) and the store is replaced atomically;
 * a response refresh token is adopted, else a previous one is retained. Pass
 * the returned callback as the scheme's `auth` member.
 *
 * Endpoint resolution follows the compiled precedence: the compiled
 * client-credentials flow's token URL always wins; otherwise, when the scheme
 * compiles a discovery URL, the discovery document's `token_endpoint`
 * resolves the request (fetched once per scheme and cached for this
 * provider's lifetime, single-flighted across concurrent callers, with a
 * failed fetch retried on the next call); otherwise the typed
 * endpoint-unavailable refusal stands.
 */
export function createClientCredentialsProvider(options: ClientCredentialsProviderOptions): CredentialProvider<AuthorizationCredential> {
    const store = options.store ?? new MemoryTokenStore();
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const inflight = new Map<string, Promise<TokenSet>>();
    const discoveryCache: DiscoveryCache = new Map();
    return async (context: CredentialContext): Promise<AuthorizationCredential> => {
        const scheme = compiled(options.scheme);
        const grant = compiledFlowOrNull(scheme, 'client-credentials');
        const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
        const auth = grant === null ? discoveryClientAuth(scheme, identity) : clientAuthFor(grant, scheme, identity);
        const tokenUrl = await resolveEndpoint(scheme, grant === null ? null : grant.tokenUrl, 'tokenEndpoint', transport, discoveryCache);
        const key = tokenStoreKey(scheme.name, tokenUrl, identity.id);
        const pending = inflight.get(key);
        if (pending !== undefined) return pending.then(tokenSet => authorization(tokenSet, scheme.name));
        const tracked = (async (): Promise<TokenSet> => {
            const stored = await store.load(key);
            if (stored !== null && isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) return stored;
            const acquired = await tokenRequest(scheme.name, tokenUrl, auth, {
                grant_type: 'client_credentials',
                ...(options.scope === undefined ? {} : { scope: options.scope }),
                ...(!auth.basic && identity.id !== undefined ? { client_id: identity.id } : {}),
            }, transport, context.signal, clock);
            const adopted = adopt(stored, acquired);
            await store.replace(key, adopted);
            return adopted;
        })();
        const guarded = tracked.finally(() => { inflight.delete(key); });
        inflight.set(key, guarded);
        return guarded.then(tokenSet => authorization(tokenSet, scheme.name));
    };
}
"#;

/// The discovery-aware refresh providers: the compiled refresh URL, else the
/// compiled token URL, always wins; otherwise the cached discovery document's
/// token endpoint resolves the refresh.
const API_REFRESH_DISCOVERY: &str = r#"
/** Options for the store-backed refresh credential provider. */
export interface RefreshProviderOptions {
    readonly scheme: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
}

/**
 * Creates a caller credential that serves stored tokens and refreshes them on
 * demand: on attach a fresh stored token is returned; an expired one is
 * refreshed exactly once with its stored refresh token before serving. There
 * is no token at all until an authorization or device flow has completed.
 * Automatic retry hook: when a protected call still fails with a declared 401,
 * call `refreshToken` explicitly and retry; the SDK itself never retries.
 *
 * Endpoint resolution follows the compiled precedence: the declared refresh
 * URL, else the compiled flow's token URL, always wins; otherwise, when the
 * scheme compiles a discovery URL, the discovery document's `token_endpoint`
 * resolves the refresh (fetched once per scheme and cached for this
 * provider's lifetime, single-flighted across concurrent callers, with a
 * failed fetch retried on the next call).
 */
export function createRefreshProvider(options: RefreshProviderOptions): CredentialProvider<AuthorizationCredential> {
    const store = options.store ?? new MemoryTokenStore();
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const discoveryCache: DiscoveryCache = new Map();
    return async (context: CredentialContext): Promise<AuthorizationCredential> => {
        const scheme = compiled(options.scheme);
        const grant = refreshFlowOrNull(scheme);
        const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
        const auth = grant === null ? discoveryClientAuth(scheme, identity) : clientAuthFor(grant, scheme, identity);
        const endpoint = grant === null
            ? await resolveEndpoint(scheme, null, 'tokenEndpoint', transport, discoveryCache)
            : refreshEndpoint(grant, scheme);
        const stored = await store.load(tokenStoreKey(scheme.name, endpoint, identity.id));
        if (stored === null) throw new AuthError('missing-credential', scheme.name, 'no stored token set exists for this scheme; complete an authorization or device flow first');
        if (isFresh(stored, scheme.refreshSkewSeconds * 1000, clock())) return authorization(stored, scheme.name);
        if (stored.refreshToken === undefined) throw new AuthError('missing-credential', scheme.name, 'the stored token set carries no refresh token');
        const acquired = await tokenRequest(scheme.name, endpoint, auth, {
            grant_type: 'refresh_token',
            refresh_token: stored.refreshToken,
        }, transport, context.signal, clock);
        const refreshed = adopt(stored, acquired);
        await store.replace(tokenStoreKey(scheme.name, endpoint, identity.id), refreshed);
        return authorization(refreshed, scheme.name);
    };
}

/** Options for an explicit refresh-token exchange. */
export interface RefreshOptions {
    readonly scheme: string;
    readonly refreshToken: string;
    readonly clientId?: string;
    readonly clientSecret?: string;
    /** When supplied, the refreshed set (adopted or retained refresh token included) atomically replaces the stored set. */
    readonly store?: TokenStore;
    readonly clock?: () => number;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Exchanges one refresh token (grant_type=refresh_token) and returns the
 * token set. A rotated refresh token from the response is adopted; when the
 * response carries none and a previous stored set exists, the previous
 * refresh token is retained.
 *
 * Endpoint resolution follows the compiled precedence: the declared refresh
 * URL, else the compiled flow's token URL, always win; otherwise, when the
 * scheme compiles a discovery URL, the discovery document's `token_endpoint`
 * resolves the exchange. This one-shot helper fetches discovery per call and
 * keeps no cache; `createRefreshProvider` caches it per provider instead.
 */
export async function refreshToken(options: RefreshOptions): Promise<TokenSet> {
    if (typeof options.refreshToken !== 'string' || options.refreshToken === '' || /[\u0000-\u001f\u007f]/.test(options.refreshToken)) {
        throw new AuthError('invalid-request', options.scheme, 'refreshToken must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    const grant = refreshFlowOrNull(scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const auth = grant === null ? discoveryClientAuth(scheme, identity) : clientAuthFor(grant, scheme, identity);
    const clock = options.clock ?? (() => Date.now());
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    const endpoint = grant === null
        ? await resolveEndpoint(scheme, null, 'tokenEndpoint', transport, new Map())
        : refreshEndpoint(grant, scheme);
    const previous = options.store === undefined ? null : await options.store.load(tokenStoreKey(scheme.name, endpoint, identity.id));
    const acquired = await tokenRequest(scheme.name, endpoint, auth, {
        grant_type: 'refresh_token',
        refresh_token: options.refreshToken,
    }, transport, options.signal, clock);
    const adopted = adopt(previous, acquired);
    if (options.store !== undefined) await options.store.replace(tokenStoreKey(scheme.name, endpoint, identity.id), adopted);
    return adopted;
}
"#;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the discovery document's `revocation_endpoint`.
const REVOKE_DISCOVERY: &str = r#"
/** Options for revoking one token (RFC 7009). */
export interface RevokeOptions {
    readonly scheme: string;
    readonly token: string;
    readonly tokenTypeHint?: 'access_token' | 'refresh_token';
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Revokes one token at the compiled revocation endpoint (RFC 7009,
 * form-encoded). Authentication follows the compiled client policy.
 *
 * Endpoint resolution follows the compiled precedence: the configured
 * revocation endpoint always wins; otherwise, when the scheme compiles a
 * discovery URL, the discovery document's `revocation_endpoint` resolves the
 * request. This one-shot helper fetches discovery per call and keeps no
 * cache.
 */
export async function revoke(options: RevokeOptions): Promise<void> {
    if (typeof options.token !== 'string' || options.token === '' || /[\u0000-\u001f\u007f]/.test(options.token)) {
        throw new AuthError('invalid-request', options.scheme, 'token must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    let endpoint = scheme.revocationEndpoint;
    if (endpoint === null) {
        const document = await discover(scheme, transport, new Map());
        if (document.revocationEndpoint === undefined) throw new AuthError('endpoint-unavailable', scheme.name, 'no revocation endpoint was configured for this scheme and the discovery document declares none');
        endpoint = document.revocationEndpoint;
    }
    const grant = refreshFlowOrNull(scheme);
    const auth = grant === null ? discoveryClientAuth(scheme, identity) : clientAuthFor(grant, scheme, identity);
    await postForm(scheme.name, endpoint, auth, {
        token: options.token,
        ...(options.tokenTypeHint === undefined ? {} : { token_type_hint: options.tokenTypeHint }),
        ...(!auth.basic && identity.id !== undefined ? { client_id: identity.id } : {}),
    }, transport, options.signal);
}
"#;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's `introspection_endpoint`.
const INTROSPECT_DISCOVERY: &str = r#"
/** Options for introspecting one token (RFC 7662). */
export interface IntrospectOptions {
    readonly scheme: string;
    readonly token: string;
    readonly tokenTypeHint?: 'access_token' | 'refresh_token';
    readonly clientId?: string;
    readonly clientSecret?: string;
    readonly fetch?: Fetch;
    readonly signal?: AbortSignal;
}

/**
 * Introspects one token at the compiled introspection endpoint (RFC 7662,
 * form-encoded) and returns the server's JSON response as a frozen object.
 * Authentication follows the compiled client policy.
 *
 * Endpoint resolution follows the compiled precedence: the configured
 * introspection endpoint always wins; otherwise, when the scheme compiles a
 * discovery URL, the discovery document's `introspection_endpoint` resolves
 * the request. This one-shot helper fetches discovery per call and keeps no
 * cache.
 */
export async function introspect(options: IntrospectOptions): Promise<Readonly<Record<string, unknown>>> {
    if (typeof options.token !== 'string' || options.token === '' || /[\u0000-\u001f\u007f]/.test(options.token)) {
        throw new AuthError('invalid-request', options.scheme, 'token must be a nonempty string without control characters');
    }
    const scheme = compiled(options.scheme);
    const identity = resolveIdentity(scheme, options.clientId, options.clientSecret);
    const transport = options.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new TypeError('no Fetch transport is available');
    let endpoint = scheme.introspectionEndpoint;
    if (endpoint === null) {
        const document = await discover(scheme, transport, new Map());
        if (document.introspectionEndpoint === undefined) throw new AuthError('endpoint-unavailable', scheme.name, 'no introspection endpoint was configured for this scheme and the discovery document declares none');
        endpoint = document.introspectionEndpoint;
    }
    const grant = refreshFlowOrNull(scheme);
    const auth = grant === null ? discoveryClientAuth(scheme, identity) : clientAuthFor(grant, scheme, identity);
    const response = await postForm(scheme.name, endpoint, auth, {
        token: options.token,
        ...(options.tokenTypeHint === undefined ? {} : { token_type_hint: options.tokenTypeHint }),
        ...(!auth.basic && identity.id !== undefined ? { client_id: identity.id } : {}),
    }, transport, options.signal);
    const body = await jsonObject(scheme.name, response, 'introspection');
    return Object.freeze({ ...body });
}
"#;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the per-provider cache with single-flight, and the
/// endpoint-resolution precedence.
const DISCOVERY: &str = r#"
/** One RFC 8414 / OpenID Connect discovery document reduced to the endpoints this module resolves. Unknown members are ignored; a known member must be a nonempty control-character-free string when present. */
interface DiscoveredEndpoints {
    tokenEndpoint?: string;
    revocationEndpoint?: string;
    introspectionEndpoint?: string;
}

/** The compiled ceiling for one discovery document response (about a mebibyte). */
const DISCOVERY_MAX_BYTES = 1048576;

/** Reads one discovery document member: absent stays undefined; a non-string or unusable value is a typed discovery failure. Unknown members are ignored. */
function discoveredEndpoint(scheme: string, document: Record<string, unknown>, member: string): string | undefined {
    const value = document[member];
    if (value === undefined) return undefined;
    if (typeof value !== 'string' || value === '' || /[\u0000-\u001f\u007f]/.test(value)) {
        throw new AuthError('discovery-failed', scheme, `the discovery document carries an unusable ${member} value`);
    }
    return value;
}

/** The origin of one absolute http(s) URL: scheme, host and the port with the scheme default made explicit. Returns null when the value is not an absolute http(s) URL. */
function urlOrigin(value: string): string | null {
    let parsed: URL;
    try { parsed = new URL(value); }
    catch { return null; }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    const port = parsed.port === '' ? (parsed.protocol === 'http:' ? '80' : '443') : parsed.port;
    return `${parsed.protocol}//${parsed.hostname}:${port}`;
}

/**
 * Decodes and validates one discovery response body into the endpoints this
 * module resolves. The exact issuer rule: when the document carries an
 * `issuer` claim, it must be an absolute http(s) URL whose origin (scheme,
 * host and the port with the scheme default made explicit) equals the
 * discovery URL's origin; OpenID Connect openIdConnectUrl documents are
 * validated against their `issuer` claim exactly this way, as are RFC 8414
 * OAuth2 authorization-server metadata documents. A missing claim is
 * tolerated; a mismatching or unparseable one is a typed discovery failure.
 * Failure messages carry only safe metadata, never response body text.
 */
function discoveryDocument(scheme: CompiledScheme, url: string, body: string): DiscoveredEndpoints {
    let decoded: unknown;
    try { decoded = JSON.parse(body); }
    catch { throw new AuthError('discovery-failed', scheme.name, 'the discovery document is not readable JSON'); }
    if (decoded === null || typeof decoded !== 'object' || Array.isArray(decoded)) {
        throw new AuthError('discovery-failed', scheme.name, 'the discovery document is not a JSON object');
    }
    const document = decoded as Record<string, unknown>;
    const issuer = document.issuer;
    if (typeof issuer === 'string' && issuer !== '') {
        const issuerOrigin = urlOrigin(issuer);
        const discoveryOrigin = urlOrigin(url);
        if (issuerOrigin === null || discoveryOrigin === null || issuerOrigin !== discoveryOrigin) {
            throw new AuthError('discovery-failed', scheme.name, 'the discovery document issuer does not share the discovery URL origin');
        }
    }
    const endpoints: DiscoveredEndpoints = {};
    const tokenEndpoint = discoveredEndpoint(scheme.name, document, 'token_endpoint');
    if (tokenEndpoint !== undefined) endpoints.tokenEndpoint = tokenEndpoint;
    const revocationEndpoint = discoveredEndpoint(scheme.name, document, 'revocation_endpoint');
    if (revocationEndpoint !== undefined) endpoints.revocationEndpoint = revocationEndpoint;
    const introspectionEndpoint = discoveredEndpoint(scheme.name, document, 'introspection_endpoint');
    if (introspectionEndpoint !== undefined) endpoints.introspectionEndpoint = introspectionEndpoint;
    return endpoints;
}

/** One provider-owned discovery cache. Successful documents are cached per scheme for the provider's lifetime, so repeated attaches never re-fetch; a failed fetch is never cached, so the next call retries; concurrent callers share the one in-flight fetch through the cached promise (single-flight). */
type DiscoveryCache = Map<string, Promise<DiscoveredEndpoints>>;

/**
 * Fetches the scheme's discovery document (GET, `accept: application/json`),
 * returning the cached document when one exists for the provider. The
 * response is bounded at DISCOVERY_MAX_BYTES; timeouts stay with the caller's
 * transport, exactly like every other request in this module.
 */
async function discover(scheme: CompiledScheme, transport: Fetch, cache: DiscoveryCache): Promise<DiscoveredEndpoints> {
    const url = scheme.discovery;
    if (url === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled plan carries no discovery URL, so endpoint resolution cannot fall back to discovery');
    const pending = cache.get(scheme.name);
    if (pending !== undefined) return pending;
    const tracked = (async (): Promise<DiscoveredEndpoints> => {
        let response: Response;
        try {
            response = await transport(url, { method: 'GET', headers: new Headers({ accept: 'application/json' }) });
        } catch {
            throw new AuthError('discovery-failed', scheme.name, 'the discovery document request failed before a response arrived');
        }
        if (response.status < 200 || response.status > 299) {
            throw new AuthError('discovery-failed', scheme.name, `the discovery document request answered HTTP ${response.status}`, { status: response.status });
        }
        const declared = response.headers.get('content-length');
        if (declared !== null && /^[0-9]+$/.test(declared.trim()) && Number.parseInt(declared, 10) > DISCOVERY_MAX_BYTES) {
            throw new AuthError('discovery-failed', scheme.name, 'the discovery document exceeds the compiled response ceiling');
        }
        let body: string;
        try { body = await response.text(); }
        catch { throw new AuthError('discovery-failed', scheme.name, 'the discovery document response body could not be read'); }
        if (new TextEncoder().encode(body).byteLength > DISCOVERY_MAX_BYTES) {
            throw new AuthError('discovery-failed', scheme.name, 'the discovery document exceeds the compiled response ceiling');
        }
        return discoveryDocument(scheme, url, body);
    })();
    const guarded = tracked.catch((error: unknown) => {
        cache.delete(scheme.name);
        throw error;
    });
    cache.set(scheme.name, guarded);
    return guarded;
}

/**
 * Resolves one lifecycle endpoint through the compiled precedence: an
 * explicit compiled endpoint always wins; otherwise the cached discovery
 * document's endpoint when the scheme compiles a discovery URL; otherwise the
 * typed endpoint-unavailable refusal the compiled plan alone would produce.
 */
async function resolveEndpoint(scheme: CompiledScheme, compiledEndpoint: string | null, member: 'tokenEndpoint' | 'revocationEndpoint' | 'introspectionEndpoint', transport: Fetch, cache: DiscoveryCache): Promise<string> {
    if (compiledEndpoint !== null) return compiledEndpoint;
    const document = await discover(scheme, transport, cache);
    const found = document[member];
    if (found === undefined) throw new AuthError('endpoint-unavailable', scheme.name, `neither the compiled plan nor the discovery document carries a ${member} for this scheme`);
    return found;
}

/** Client authentication for endpoints the discovery document supplies (no compiled flow declares one): client-secret-basic when the compiled configuration carries a client secret variable — an unavailable value becomes the typed missing-credential refusal — else the public profile, which sends the client id in the form. */
function discoveryClientAuth(scheme: CompiledScheme, identity: ClientIdentity): ClientAuthPlan {
    const basic = scheme.clientSecretEnv !== null;
    return { basic, id: identity.id, secret: basic ? identity.secret : undefined };
}

/** Resolves one executable (non-deprecated) compiled flow, or null when the scheme compiles none: a discovery-defined scheme's flows live in the discovery document. */
function compiledFlowOrNull(scheme: CompiledScheme, kind: Exclude<CompiledFlowKind, 'implicit' | 'password'>): CompiledFlow | null {
    return scheme.flows.find(candidate => candidate.kind === kind && !candidate.deprecated) ?? null;
}

/** The flow whose token/refresh endpoints serve refreshes: the authorization-code flow when compiled, else the first executable flow with a token URL, else null for a scheme the discovery document defines. */
function refreshFlowOrNull(scheme: CompiledScheme): CompiledFlow | null {
    const executable = scheme.flows.filter(candidate => !candidate.deprecated);
    const preferred = executable.find(candidate => candidate.kind === 'authorization-code') ?? executable.find(candidate => candidate.tokenUrl !== null);
    return preferred ?? null;
}
"#;

/// The replaying credential wrapper's shared half: the stream-protection map
/// key plus the served-attach record and the public credential type. The
/// plain and discovery variants append complete factories, exactly like the
/// plain and discovery client-credentials providers above.
const REPLAY_INTERFACES: &str = r#"
/** One attach this provider served, remembered so the wrapped transport can tell which requests carried this provider's token. The record keeps only safe metadata plus the attach context; token values already traveled on the wire. */
interface ServedAttach {
    /** The complete Authorization value this provider attached. */
    value: string;
    /** Whether a qualifying 401 on this attach may be replayed: attaches for stream-protected requirements never are, because delivered stream data prevents a transparent restart. */
    eligible: boolean;
    /** The attach context; its abort signal also drives the coordinated refresh. */
    context: CredentialContext;
}

/**
 * A replaying client-credentials credential: the plain provider's attach
 * behavior plus the unified request policy. Use it in two places — pass the
 * credential itself as the scheme's `auth` member, and pass
 * `credentials.fetch(transport)` as the client's transport:
 *
 * ```ts
 * const credentials = createReplayingCredentialsProvider({ scheme: 'name' });
 * const result = await listWidgets({ auth: { name: credentials }, fetch: credentials.fetch(myFetch) });
 * ```
 *
 * A 401 (and only a 401) on a request whose Authorization value this
 * provider attached triggers exactly one coordinated refresh — concurrent
 * 401s share one token request through the same single-flight store round —
 * and exactly one replay of the request with the fresh token. The second
 * response is surfaced whatever it is: a second 401 reaches the caller as
 * the declared error. The overall budget is one refresh plus one replay,
 * never nested with other retry policies (requests are not retried today).
 * Attaches for stream-protected requirements are never replayed, because
 * delivered stream data prevents a transparent restart. A refresh failure
 * surfaces as the typed `AuthError` instead of a replay. The plain provider
 * keeps today's semantics: replay is this wrapper's opt-in only.
 */
export interface ReplayingCredential {
    (context: CredentialContext): Promise<AuthorizationCredential>;
    /** Wraps `inner` with the one-refresh-one-replay 401 policy; call once per client. Token requests keep traveling through `inner` directly. */
    fetch(inner: Fetch): Fetch;
}
"#;

/// The plain-variant replaying factory: the compiled client-credentials
/// token endpoint serves both the store key and the lifecycle exclusion.
const REPLAY_PLAIN: &str = r#"
/**
 * Creates the replaying variant of the compiled client-credentials provider.
 * Every provider option behaves exactly as in `createClientCredentialsProvider`;
 * the replay semantics are strictly additive and the plain provider keeps
 * today's attach-only semantics. Creation refuses a compiled scheme whose
 * client-credentials flow declares no token URL, exactly like the plain
 * provider's first attach.
 */
export function createReplayingCredentialsProvider(options: ClientCredentialsProviderOptions): ReplayingCredential {
    const store = options.store ?? new MemoryTokenStore();
    const plain = createClientCredentialsProvider({ ...options, store });
    const scheme = compiled(options.scheme);
    const grant = executableFlow(scheme, 'client-credentials');
    const tokenUrl = grant.tokenUrl;
    if (tokenUrl === null) throw new AuthError('endpoint-unavailable', scheme.name, 'the compiled client-credentials flow declares no token URL');
    const neverReplay: ReadonlySet<string> = noReplayRequirements[scheme.name] ?? new Set();
    const served: ServedAttach[] = [];
    const rounds = new Map<string, Promise<string>>();
    /** One coordinated refresh: concurrent 401s share one store round, a newer stored set wins over a stale re-refresh, and a failed round fails every waiter exactly once. The round resolves to the fresh complete Authorization value. */
    const refresh = async (presented: string, context: CredentialContext): Promise<string> => {
        const key = tokenStoreKey(scheme.name, tokenUrl, resolveIdentity(scheme, options.clientId, options.clientSecret).id);
        const pending = rounds.get(key);
        if (pending !== undefined) return pending;
        const tracked = (async (): Promise<string> => {
            const stored = await store.load(key);
            if (stored !== null && authorization(stored, scheme.name).authorization !== presented) {
                return authorization(stored, scheme.name).authorization;
            }
            await store.clear(key);
            return (await plain(context)).authorization;
        })();
        const guarded = tracked.finally(() => { rounds.delete(key); });
        rounds.set(key, guarded);
        return guarded;
    };
    const serve = async (context: CredentialContext): Promise<AuthorizationCredential> => {
        const credential = await plain(context);
        served.unshift({ value: credential.authorization, eligible: !neverReplay.has(context.requirement.source.source.pointer), context });
        if (served.length > 8) served.length = 8;
        return credential;
    };
    const replaying = ((context: CredentialContext): Promise<AuthorizationCredential> => serve(context)) as ReplayingCredential;
    replaying.fetch = (inner: Fetch): Fetch => async (input, init) => {
        if (typeof input !== 'string') return inner(input, init);
        const response = await inner(input, init);
        if (response.status !== 401) return response;
        const presented = new Headers(init?.headers).get('authorization');
        if (presented === null) return response;
        const entry = served.find(candidate => candidate.value === presented && candidate.eligible);
        if (entry === undefined) return response;
        // Lifecycle endpoint requests carry no bearer token of this provider,
        // so this exact-target guard is defense in depth against loops.
        if (input === tokenUrl) return response;
        const fresh = await refresh(presented, entry.context);
        const headers = new Headers(init?.headers);
        headers.set('authorization', fresh);
        return inner(input, { ...init, headers });
    };
    return replaying;
}
"#;

/// The discovery-variant replaying factory: the refresh endpoint resolves
/// through the compiled precedence (compiled token URL, else the discovery
/// document's), cached per provider exactly like the plain provider's
/// discovery resolution.
const REPLAY_DISCOVERY: &str = r#"
/**
 * Creates the replaying variant of the compiled client-credentials provider.
 * Every provider option behaves exactly as in `createClientCredentialsProvider`;
 * the replay semantics are strictly additive and the plain provider keeps
 * today's attach-only semantics. The refresh endpoint resolves through the
 * compiled precedence — the compiled token URL when the client-credentials
 * flow compiles one, otherwise the discovery document's `token_endpoint`,
 * fetched once and cached for this provider's lifetime.
 */
export function createReplayingCredentialsProvider(options: ClientCredentialsProviderOptions): ReplayingCredential {
    const store = options.store ?? new MemoryTokenStore();
    const plain = createClientCredentialsProvider({ ...options, store });
    const scheme = compiled(options.scheme);
    const grant = compiledFlowOrNull(scheme, 'client-credentials');
    const neverReplay: ReadonlySet<string> = noReplayRequirements[scheme.name] ?? new Set();
    const served: ServedAttach[] = [];
    const rounds = new Map<string, Promise<string>>();
    const discovery: DiscoveryCache = new Map();
    /** The refresh endpoint through the compiled precedence; cached per provider and single-flighted like every other discovery resolution. */
    const endpoint = async (): Promise<string> =>
        resolveEndpoint(scheme, grant === null ? null : grant.tokenUrl, 'tokenEndpoint', options.fetch ?? globalThis.fetch, discovery);
    /** One coordinated refresh: concurrent 401s share one store round, a newer stored set wins over a stale re-refresh, and a failed round fails every waiter exactly once. The round resolves to the fresh complete Authorization value. */
    const refresh = async (presented: string, context: CredentialContext): Promise<string> => {
        const key = tokenStoreKey(scheme.name, await endpoint(), resolveIdentity(scheme, options.clientId, options.clientSecret).id);
        const pending = rounds.get(key);
        if (pending !== undefined) return pending;
        const tracked = (async (): Promise<string> => {
            const stored = await store.load(key);
            if (stored !== null && authorization(stored, scheme.name).authorization !== presented) {
                return authorization(stored, scheme.name).authorization;
            }
            await store.clear(key);
            return (await plain(context)).authorization;
        })();
        const guarded = tracked.finally(() => { rounds.delete(key); });
        rounds.set(key, guarded);
        return guarded;
    };
    const serve = async (context: CredentialContext): Promise<AuthorizationCredential> => {
        const credential = await plain(context);
        served.unshift({ value: credential.authorization, eligible: !neverReplay.has(context.requirement.source.source.pointer), context });
        if (served.length > 8) served.length = 8;
        return credential;
    };
    const replaying = ((context: CredentialContext): Promise<AuthorizationCredential> => serve(context)) as ReplayingCredential;
    replaying.fetch = (inner: Fetch): Fetch => async (input, init) => {
        if (typeof input !== 'string') return inner(input, init);
        const response = await inner(input, init);
        if (response.status !== 401) return response;
        const presented = new Headers(init?.headers).get('authorization');
        if (presented === null) return response;
        const entry = served.find(candidate => candidate.value === presented && candidate.eligible);
        if (entry === undefined) return response;
        // Lifecycle endpoint requests carry no bearer token of this provider,
        // so this exact-target guard is defense in depth against loops; the
        // discovery-resolved token endpoint rides the exact-token match.
        if (scheme.discovery !== null && input === scheme.discovery) return response;
        const fresh = await refresh(presented, entry.context);
        const headers = new Headers(init?.headers);
        headers.set('authorization', fresh);
        return inner(input, { ...init, headers });
    };
    return replaying;
}
"#;
