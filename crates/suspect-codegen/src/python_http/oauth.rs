//! Emitted-only OAuth 2.0 / OpenID Connect token lifecycle for the Python
//! backend. The shared, generation-time `http_protocol::plan_oauth` outcome
//! (carried on the plan when client defaults are configured) compiles into one
//! generated `_oauth.py` module; static runtime files and the shared planner
//! stay untouched, and plans without at least one executable scheme emit no
//! new bytes at all.
//!
//! What counts as executable: a non-deprecated flow whose declared endpoints
//! satisfy its grant — `client-credentials` needs a token URL,
//! `authorization-code` needs authorization and token URLs, and
//! `device-authorization` needs device-authorization and token URLs. Implicit
//! and password flows are represented in the compiled descriptors but never
//! executed, so schemes with only those flows emit nothing, exactly like the
//! planner's documented refusal to execute them. A scheme carrying a
//! discovery URL is usable even with no declared flows — OpenID Connect
//! schemes have their endpoints defined by the discovery document at runtime.
//!
//! Emission is byte-identical for plans without a discovery URL: every
//! emitted section has a plain variant (exactly the pre-discovery bytes) and,
//! when at least one compiled scheme carries a discovery URL, a
//! discovery-aware variant resolving the endpoints the compiled plan omits
//! through RFC 8414 / OpenID Connect discovery.
//!
//! The emitted module embeds the compiled descriptors as frozen module
//! constants — the runtime never parses OpenAPI — and integrates with the
//! existing credential attach path by returning `Authorization` values from a
//! callable `Credential`. Client identifiers and secrets resolve from explicit
//! arguments first and otherwise from the configured variable names read at
//! call time; token values and secrets never enter error messages or reprs.

use std::collections::{BTreeMap, BTreeSet};

use crate::http_protocol as p;

/// A single-quoted Python literal for generated values.
fn sq(text: &str) -> String {
    let mut out = String::from("'");
    for character in text.chars() {
        match character {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(character),
        }
    }
    out.push('\'');
    out
}

/// Whether one compiled flow can be executed by the emitted runtime.
fn executable_flow(flow: &p::OAuthFlowDescriptor) -> bool {
    if flow.deprecated_flow {
        return false;
    }
    match flow.kind {
        p::OAuthFlowDescriptorKind::ClientCredentials => flow.token_url.is_some(),
        p::OAuthFlowDescriptorKind::AuthorizationCode => {
            flow.authorization_url.is_some() && flow.token_url.is_some()
        }
        p::OAuthFlowDescriptorKind::DeviceAuthorization => {
            flow.device_authorization_url.is_some() && flow.token_url.is_some()
        }
        p::OAuthFlowDescriptorKind::Implicit | p::OAuthFlowDescriptorKind::Password => false,
    }
}

/// Whether one compiled scheme contributes anything executable, or carries a
/// discovery URL whose document defines its endpoints at runtime.
fn usable_scheme(scheme: &p::OAuthSchemePlan) -> bool {
    scheme.discovery.is_some() || scheme.flows.iter().any(executable_flow)
}

/// Whether the plan justifies emitting `_oauth.py` at all.
pub(super) fn emittable(plan: &p::OAuthPlan) -> bool {
    plan.schemes.iter().any(usable_scheme)
}

/// Whether at least one compiled scheme carries an executable
/// client-credentials flow, so the replaying credential wrapper participates.
/// The wrapper serves exactly that provider, so schemes without one compile
/// exactly the pre-replay bytes.
fn has_client_credentials(plan: &p::OAuthPlan) -> bool {
    plan.schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            flow.kind == p::OAuthFlowDescriptorKind::ClientCredentials
                && !flow.deprecated_flow
                && executable_flow(flow)
        })
    })
}

/// The security-requirement source pointers whose attaches must never be
/// replayed: requirements that name a compiled scheme on an operation whose
/// responses carry a stream representation. Delivered stream data prevents a
/// transparent restart, so those operations are excluded from the replay
/// wrapper's one-replay budget.
pub(super) fn no_replay_requirements(
    plan: &p::OAuthPlan,
    operations: &[super::planning::PlannedOperation],
) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = plan.schemes.iter().map(|s| s.name.as_str()).collect();
    let mut pointers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for operation in operations {
        let streams = operation.wire.responses().iter().any(|response| {
            response
                .media()
                .iter()
                .any(|media| matches!(media.representation(), p::Representation::Stream { .. }))
        });
        if !streams {
            continue;
        }
        for alternative in operation.wire.security().alternatives() {
            for requirement in alternative.requirements() {
                if !matches!(
                    requirement.credential(),
                    p::CredentialHook::OAuth2 { .. } | p::CredentialHook::OpenIdConnect { .. }
                ) {
                    continue;
                }
                if names.contains(requirement.name()) {
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

fn flow_kind_name(kind: p::OAuthFlowDescriptorKind) -> &'static str {
    match kind {
        p::OAuthFlowDescriptorKind::Implicit => "implicit",
        p::OAuthFlowDescriptorKind::Password => "password",
        p::OAuthFlowDescriptorKind::ClientCredentials => "client-credentials",
        p::OAuthFlowDescriptorKind::AuthorizationCode => "authorization-code",
        p::OAuthFlowDescriptorKind::DeviceAuthorization => "device-authorization",
    }
}

/// One compiled scheme as a frozen Python descriptor literal.
fn scheme_entry(scheme: &p::OAuthSchemePlan) -> String {
    let mut entry = String::new();
    entry.push_str(&format!("    {}: {{\n", sq(&scheme.name)));
    entry.push_str(&format!("        'name': {},\n", sq(&scheme.name)));
    entry.push_str(&format!(
        "        'kind': {},\n",
        sq(match scheme.kind {
            p::OAuthSchemeKind::OAuth2 => "oauth2",
            p::OAuthSchemeKind::OpenIdConnect => "open-id-connect",
        })
    ));
    entry.push_str(&format!(
        "        'skew': {},\n",
        scheme.refresh_skew_seconds
    ));
    entry.push_str(&format!(
        "        'storage': {},\n",
        sq(match scheme.storage {
            p::OAuthStorage::Memory => "memory",
        })
    ));
    entry.push_str(&format!(
        "        'refresh': {},\n",
        sq(match scheme.refresh {
            p::OAuthRefresh::OnDemand => "on-demand",
        })
    ));
    if let Some(variable) = &scheme.client_id_env {
        entry.push_str(&format!("        'client_id_env': {},\n", sq(variable)));
    }
    if let Some(variable) = &scheme.client_secret_env {
        entry.push_str(&format!("        'client_secret_env': {},\n", sq(variable)));
    }
    if let Some(endpoint) = &scheme.revocation_endpoint {
        entry.push_str(&format!("        'revocation': {},\n", sq(endpoint)));
    }
    if let Some(endpoint) = &scheme.introspection_endpoint {
        entry.push_str(&format!("        'introspection': {},\n", sq(endpoint)));
    }
    if let Some(discovery) = &scheme.discovery {
        entry.push_str(&format!("        'discovery': {},\n", sq(discovery)));
    }
    entry.push_str("        'flows': {\n");
    // Declaration order stays in the plan; the emitted map keys flows by grant
    // kind, so sort the entries for deterministic bytes.
    let mut flows: BTreeMap<&str, &p::OAuthFlowDescriptor> = BTreeMap::new();
    for flow in &scheme.flows {
        flows.insert(flow_kind_name(flow.kind), flow);
    }
    for (kind, flow) in flows {
        entry.push_str(&format!("            {}: {{\n", sq(kind)));
        for (field, url) in [
            ("authorization_url", &flow.authorization_url),
            ("token_url", &flow.token_url),
            ("refresh_url", &flow.refresh_url),
            ("device_authorization_url", &flow.device_authorization_url),
        ] {
            if let Some(url) = url {
                entry.push_str(&format!("                {}: {},\n", sq(field), sq(url)));
            }
        }
        entry.push_str(&format!(
            "                'client_auth': {},\n",
            sq(match flow.client_auth {
                p::OAuthClientAuth::ClientSecretBasic => "client-secret-basic",
                p::OAuthClientAuth::None => "none",
            })
        ));
        entry.push_str(&format!(
            "                'deprecated': {},\n",
            if flow.deprecated_flow {
                "True"
            } else {
                "False"
            }
        ));
        entry.push_str("                'scopes': {\n");
        for (name, description) in &flow.scopes {
            entry.push_str(&format!(
                "                    {}: {},\n",
                sq(name),
                sq(description)
            ));
        }
        entry.push_str("                },\n");
        entry.push_str("            },\n");
    }
    entry.push_str("        },\n");
    entry.push_str("    },\n");
    entry
}

/// The generated `_oauth.py` module for a plan with usable schemes. Plans
/// without a discovery URL assemble byte-identically to the pre-discovery
/// emission; plans with one emit the discovery-aware provider and helpers.
/// The replaying credential wrapper joins only when a compiled scheme
/// carries an executable client-credentials flow; everything else stays
/// byte-identical.
pub(super) fn runtime(
    plan: &p::OAuthPlan,
    no_replay: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    let schemes: Vec<&p::OAuthSchemePlan> = plan
        .schemes
        .iter()
        .filter(|scheme| usable_scheme(scheme))
        .collect();
    debug_assert!(
        !schemes.is_empty(),
        "emission is gated on at least one usable scheme"
    );
    let authorization = schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            !flow.deprecated_flow
                && executable_flow(flow)
                && flow.kind == p::OAuthFlowDescriptorKind::AuthorizationCode
        })
    });
    let device = schemes.iter().any(|scheme| {
        scheme.flows.iter().any(|flow| {
            !flow.deprecated_flow
                && executable_flow(flow)
                && flow.kind == p::OAuthFlowDescriptorKind::DeviceAuthorization
        })
    });
    let revocation = schemes
        .iter()
        .any(|scheme| scheme.revocation_endpoint.is_some());
    let introspection = schemes
        .iter()
        .any(|scheme| scheme.introspection_endpoint.is_some());
    let discovery = schemes.iter().any(|scheme| scheme.discovery.is_some());

    let mut source = String::new();
    source.push_str(HEAD);
    source.push_str("# Compiled scheme descriptors: the runtime never parses OpenAPI, and every\n");
    source.push_str("# endpoint below is a declared absolute http(s) URL carried by the plan.\n");
    source.push_str("_SCHEMES: dict[str, dict[str, Any]] = {\n");
    for scheme in &schemes {
        source.push_str(&scheme_entry(scheme));
    }
    source.push_str("}\n");
    source.push_str(CORE);
    if discovery {
        source.push_str(PROVIDER_DISCOVERY);
    } else {
        source.push_str(PROVIDER);
    }
    if discovery {
        source.push_str(REFRESH_DISCOVERY);
    } else {
        source.push_str(REFRESH);
    }
    if authorization {
        source.push_str(AUTHORIZATION_CODE);
    }
    if device {
        source.push_str(DEVICE);
    }
    if revocation {
        source.push_str(if discovery {
            REVOCATION_DISCOVERY
        } else {
            REVOCATION
        });
    }
    if introspection {
        source.push_str(if discovery {
            INTROSPECTION_DISCOVERY
        } else {
            INTROSPECTION
        });
    }
    if discovery {
        source.push_str(DISCOVERY);
    }
    if has_client_credentials(plan) {
        source.push_str(&replay_section(plan, no_replay, discovery));
    }

    let mut exports: Vec<&str> = vec![
        "AuthError",
        "TokenSet",
        "TokenStore",
        "MemoryTokenStore",
        "client_credential",
        "refresh_token_set",
        "refresh_token_set_async",
    ];
    if authorization {
        exports.extend([
            "AuthorizationTransaction",
            "begin_authorization",
            "complete_authorization",
            "complete_authorization_async",
        ]);
    }
    if device {
        exports.extend([
            "DeviceAuthorization",
            "begin_device_authorization",
            "poll_device_authorization",
            "poll_device_authorization_async",
        ]);
    }
    if revocation {
        exports.extend(["revoke", "revoke_async"]);
    }
    if introspection {
        exports.extend(["introspect", "introspect_async"]);
    }
    if has_client_credentials(plan) {
        exports.push("replaying_credential");
    }
    let rendered: Vec<String> = exports.iter().map(|name| sq(name)).collect();
    source.push_str(&format!("\n__all__ = [{}]\n", rendered.join(", ")));
    source
}

const HEAD: &str = r##""""Generated OAuth 2.0 / OpenID Connect token lifecycle.

Every endpoint, client-authentication style, scope set and policy constant in
``_SCHEMES`` is a generation-time compilation of the used security schemes in
the source document. This module never parses OpenAPI and never invents an
endpoint. The deprecated implicit and password flows stay described in the
compiled descriptors but are never executed. Client identifiers and secrets
resolve from explicit arguments first and otherwise from the configured
environment variable names read at call time; token values and secrets never
appear in error messages, reprs or logs.
"""
from __future__ import annotations

import asyncio
import base64
import dataclasses
import hashlib
import httpx
import os
import secrets
import threading
import time
import urllib.parse
from collections.abc import Callable, Mapping, Sequence
from typing import Any, Protocol

from ._types import Authorization, Credential
from ._wire import TOKEN

_TOKEN_TIMEOUT = 30.0
_DEVICE_GRANT = 'urn:ietf:params:oauth:grant-type:device_code'
_FLOW_KINDS = ('authorization-code', 'client-credentials', 'device-authorization')
"##;

const CORE: &str = r##"

class AuthError(Exception):
    """Typed OAuth lifecycle failure.

    ``kind`` and ``scheme`` classify the failure; ``code`` carries the
    authorization server's error code and ``status`` its HTTP status when one
    exists. The message and repr contain only this safe metadata: never token
    values, client secrets or response bodies.
    """

    def __init__(self, kind: str, scheme: str, *, code: str | None = None,
                 status: int | None = None, cause: BaseException | None = None) -> None:
        message = 'OAuth ' + kind + ' failure for scheme ' + repr(scheme)
        if code is not None:
            message = message + ' (server code: ' + code + ')'
        super().__init__(message)
        self.kind, self.scheme, self.code, self.status, self.cause = kind, scheme, code, status, cause

    def __repr__(self) -> str:
        return ('AuthError(kind=' + repr(self.kind) + ', scheme=' + repr(self.scheme)
                + ', status=' + repr(self.status) + ')')


@dataclasses.dataclass(frozen=True, kw_only=True)
class TokenSet:
    """One issued token. ``expires_at`` is epoch seconds already reduced by
    the compiled refresh skew, so ``expired(now)`` is a plain comparison.
    Repr and str never contain the access or refresh token."""

    access_token: str = dataclasses.field(repr=False)
    token_type: str = 'Bearer'
    expires_at: int | None = None
    refresh_token: str | None = dataclasses.field(default=None, repr=False)
    scope: str | None = None
    issuer_account: str | None = None

    def __post_init__(self) -> None:
        if type(self.access_token) is not str or not self.access_token:
            raise ValueError('TokenSet needs a non-empty access_token')

    def expired(self, now: int) -> bool:
        """Whether ``now`` (epoch seconds) has reached the compiled expiry."""
        return self.expires_at is not None and now >= self.expires_at


class TokenStore(Protocol):
    """Partitioned token storage.

    Keys are ``(scheme, issuer, client identity)`` tuples, so one store
    instance may be shared across schemes, issuers and client identities
    while the partitions stay independent. A store is owned by whoever
    creates it; nothing in this module keeps a global store.
    """

    def load(self, key: tuple[str, str, str]) -> TokenSet | None:
        """Return the stored token for ``key``, or None."""

    def replace(self, key: tuple[str, str, str], token: TokenSet) -> None:
        """Atomically replace the stored token for ``key``."""

    def clear(self, key: tuple[str, str, str]) -> None:
        """Drop the stored token for ``key``."""


class MemoryTokenStore(TokenStore):
    """In-process store guarded by a lock. Instance-owned, never module state."""

    def __init__(self) -> None:
        self._tokens: dict[tuple[str, str, str], TokenSet] = {}
        self._lock = threading.Lock()

    def load(self, key: tuple[str, str, str]) -> TokenSet | None:
        with self._lock:
            return self._tokens.get(key)

    def replace(self, key: tuple[str, str, str], token: TokenSet) -> None:
        with self._lock:
            self._tokens[key] = token

    def clear(self, key: tuple[str, str, str]) -> None:
        with self._lock:
            self._tokens.pop(key, None)


def _environment(variable: str) -> str | None:
    """Read one configured variable name at call time; absence stays None."""
    try:
        value = os.environ.get(variable)
    except Exception:
        return None
    return value if type(value) is str and value else None


def _origin(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    return parsed.scheme + '://' + (parsed.netloc or '')


def _server_code(response: httpx.Response) -> str | None:
    """The authorization server's ``error`` code; descriptions never surface."""
    try:
        payload = response.json()
    except Exception:
        return None
    if isinstance(payload, dict) and type(payload.get('error')) is str and payload['error']:
        return payload['error']
    return None


def _basic(client_id: str, client_secret: str) -> str:
    """RFC 6749 2.3.1 Basic credentials: form-encoded id and secret."""
    pair = urllib.parse.quote(client_id, safe='') + ':' + urllib.parse.quote(client_secret, safe='')
    return base64.b64encode(pair.encode('utf-8')).decode('ascii')


def _form(fields: Mapping[str, str | None]) -> dict[str, str]:
    """RFC 6749 request form: absent optional entries are simply not sent."""
    return {name: value for name, value in fields.items() if value is not None}


def _scheme(name: str) -> dict[str, Any]:
    try:
        return _SCHEMES[name]
    except KeyError:
        raise AuthError('unknown-scheme', name) from None


def _flow(descriptor: Mapping[str, Any], kind: str) -> dict[str, Any]:
    flow = descriptor['flows'].get(kind)
    if flow is None or flow['deprecated']:
        raise AuthError('unsupported', descriptor['name'])
    return flow


def _client_auth_flow(descriptor: Mapping[str, Any]) -> dict[str, Any]:
    """The flow whose client authentication guards auxiliary endpoints."""
    fallback: dict[str, Any] | None = None
    for kind in _FLOW_KINDS:
        flow = descriptor['flows'].get(kind)
        if flow is None or flow['deprecated']:
            continue
        if flow['client_auth'] == 'client-secret-basic':
            return flow
        if fallback is None:
            fallback = flow
    if fallback is not None:
        return fallback
    raise AuthError('unsupported', descriptor['name'])


def _issuer(descriptor: Mapping[str, Any]) -> str:
    """The issuing authority origin that store keys partition on."""
    for kind in _FLOW_KINDS:
        flow = descriptor['flows'].get(kind)
        if flow is None or flow['deprecated']:
            continue
        url = flow.get('token_url')
        if type(url) is str and url:
            return _origin(url)
    discovery = descriptor.get('discovery')
    if type(discovery) is str and discovery:
        return _origin(discovery)
    return ''


def _refresh_endpoint(descriptor: Mapping[str, Any]) -> tuple[str, dict[str, Any]]:
    """The declared refresh URL if one exists, else a declared token URL."""
    for field in ('refresh_url', 'token_url'):
        for kind in _FLOW_KINDS:
            flow = descriptor['flows'].get(kind)
            if flow is None or flow['deprecated']:
                continue
            url = flow.get(field)
            if type(url) is str and url:
                return url, flow
    raise AuthError('unsupported', descriptor['name'])


def _store_key(scheme: str, issuer: str, client_id: str | None) -> tuple[str, str, str]:
    return (scheme, issuer, client_id or '')


def _requested(descriptor: Mapping[str, Any], kind: str, scopes: Sequence[str] | None) -> tuple[str, ...]:
    """Declared-only scope vocabulary: the compiled scopes are the contract."""
    if scopes is None:
        return ()
    requested = tuple(scopes)
    declared = descriptor['flows'].get(kind, {}).get('scopes', {})
    if any(name not in declared for name in requested):
        raise AuthError('unknown-scope', descriptor['name'])
    return requested


def _parse_token(response: httpx.Response, descriptor: Mapping[str, Any],
                 clock: Callable[[], float], *, retained_refresh: str | None = None) -> TokenSet:
    """Decode one RFC 6749 token response. Expiry already includes the skew."""
    scheme = descriptor['name']
    status = response.status_code
    if status != 200:
        raise AuthError('token-request', scheme, code=_server_code(response), status=status)
    try:
        payload = response.json()
    except Exception:
        raise AuthError('invalid-response', scheme, status=status) from None
    if not isinstance(payload, dict):
        raise AuthError('invalid-response', scheme, status=status)
    access = payload.get('access_token')
    if type(access) is not str or not access:
        raise AuthError('invalid-response', scheme, status=status)
    token_type = payload.get('token_type')
    token_type = token_type if type(token_type) is str and token_type else 'Bearer'
    token_type = 'Bearer' if token_type.lower() == 'bearer' else token_type
    if TOKEN.fullmatch(token_type) is None:
        raise AuthError('invalid-response', scheme, status=status)
    expires_in = payload.get('expires_in')
    expires_at = None
    if type(expires_in) is int or type(expires_in) is float:
        expires_at = int(clock()) + max(int(expires_in) - descriptor['skew'], 0)
    refresh = payload.get('refresh_token')
    refresh = refresh if type(refresh) is str and refresh else retained_refresh
    scope = payload.get('scope')
    scope = scope if type(scope) is str and scope else None
    issuer = _issuer(descriptor)
    return TokenSet(access_token=access, token_type=token_type, expires_at=expires_at,
                    refresh_token=refresh, scope=scope, issuer_account=issuer or None)


class _Session:
    """Resolved client identity, clock and HTTP plumbing for one scheme.

    Only the compiled v1 policy (memory storage, on-demand refresh) is
    executable here; anything else is refused rather than approximated.
    """

    def __init__(self, scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                 clock: Callable[[], float] | None = None,
                 transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> None:
        descriptor = _scheme(scheme)
        if descriptor['storage'] != 'memory' or descriptor['refresh'] != 'on-demand':
            raise AuthError('unsupported', scheme)
        self.descriptor = descriptor
        self.scheme = scheme
        self._explicit_id, self._explicit_secret = client_id, client_secret
        self.clock = clock if clock is not None else time.time
        self._transport = transport
        self._sync: httpx.Client | None = None
        self._async: httpx.AsyncClient | None = None

    def resolve(self) -> tuple[str | None, str | None]:
        """Explicit arguments win; configured variable names are read now."""
        descriptor = self.descriptor
        client_id = self._explicit_id
        if client_id is None:
            variable = descriptor.get('client_id_env')
            client_id = _environment(variable) if type(variable) is str else None
        client_secret = self._explicit_secret
        if client_secret is None:
            variable = descriptor.get('client_secret_env')
            client_secret = _environment(variable) if type(variable) is str else None
        return client_id, client_secret

    def key(self) -> tuple[str, str, str]:
        client_id, _ = self.resolve()
        return _store_key(self.scheme, _issuer(self.descriptor), client_id)

    def basic_auth(self, flow: Mapping[str, Any]) -> str | None:
        """RFC 6749 2.3.1 Basic credentials for confidential clients."""
        client_id, client_secret = self.resolve()
        if flow['client_auth'] == 'client-secret-basic':
            if type(client_id) is not str or not client_id or type(client_secret) is not str or not client_secret:
                raise AuthError('missing-client-credentials', self.scheme)
            return _basic(client_id, client_secret)
        return None

    def request_sync(self, url: str, fields: Mapping[str, str], basic: str | None) -> httpx.Response:
        if self._sync is None:
            self._sync = httpx.Client(trust_env=False, timeout=_TOKEN_TIMEOUT, transport=self._transport)
        try:
            return self._sync.post(url, data=dict(fields), headers=None if basic is None else {'Authorization': 'Basic ' + basic})
        except (httpx.HTTPError, TypeError) as error:
            raise AuthError('network', self.scheme, cause=error) from None

    async def request_async(self, url: str, fields: Mapping[str, str], basic: str | None) -> httpx.Response:
        if self._async is None:
            self._async = httpx.AsyncClient(trust_env=False, timeout=_TOKEN_TIMEOUT, transport=self._transport)
        try:
            return await self._async.post(url, data=dict(fields), headers=None if basic is None else {'Authorization': 'Basic ' + basic})
        except (httpx.HTTPError, TypeError) as error:
            raise AuthError('network', self.scheme, cause=error) from None

    def close(self) -> None:
        if self._sync is not None:
            self._sync.close()
            self._sync = None

    async def aclose(self) -> None:
        if self._async is not None:
            await self._async.aclose()
            self._async = None


def _running_loop() -> bool:
    """Whether the caller runs inside a live event loop (async resolve path)."""
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return False
    return True
"##;

const PROVIDER: &str = r##"

class _ClientCredential:
    """One compiled scheme's on-demand credential provider.

    The generated client's existing credential attach path calls it with the
    located requirement and receives an ``Authorization`` value built from a
    cached or freshly acquired ``TokenSet``. Acquisition is single-flighted
    per provider instance — ``threading.Lock`` on synchronous call sites,
    ``asyncio.Lock`` inside a running event loop — so concurrent attaches
    share one token request. A held refresh token drives the on-demand
    refresh grant; a rotated refresh token is adopted, otherwise the current
    one is retained. Inside a running loop the call returns the awaitable the
    async resolve path expects; loopless synchronous call sites receive the
    Authorization directly.
    """

    def __init__(self, scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                 store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                 transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                 scopes: Sequence[str] | None = None) -> None:
        self._session = _Session(scheme, client_id=client_id, client_secret=client_secret,
                                 clock=clock, transport=transport)
        self._store: TokenStore = store if store is not None else MemoryTokenStore()
        self._scopes = None if scopes is None else tuple(scopes)
        self._sync_lock = threading.Lock()
        self._async_lock: asyncio.Lock | None = None

    def __call__(self, request: object) -> Authorization:
        if _running_loop():
            return self._acquire_async()  # type: ignore[return-value]
        return self._acquire_sync()

    def _authorization(self, token: TokenSet) -> Authorization:
        return Authorization(value=token.token_type + ' ' + token.access_token)

    def _valid(self, key: tuple[str, str, str]) -> TokenSet | None:
        token = self._store.load(key)
        if token is not None and token.expired(int(self._session.clock())):
            return None
        return token

    def _acquire_sync(self) -> Authorization:
        key = self._session.key()
        cached = self._valid(key)
        if cached is not None:
            return self._authorization(cached)
        with self._sync_lock:
            cached = self._valid(key)
            if cached is not None:
                return self._authorization(cached)
            held = self._store.load(key)
            token = self._grant_sync(held)
            self._store.replace(key, token)
            return self._authorization(token)

    def _grant_sync(self, held: TokenSet | None) -> TokenSet:
        url, fields, basic = self._grant(held)
        response = self._session.request_sync(url, fields, basic)
        return _parse_token(response, self._session.descriptor, self._session.clock,
                            retained_refresh=None if held is None else held.refresh_token)

    async def _acquire_async(self) -> Authorization:
        key = self._session.key()
        cached = self._valid(key)
        if cached is not None:
            return self._authorization(cached)
        if self._async_lock is None:
            self._async_lock = asyncio.Lock()
        async with self._async_lock:
            cached = self._valid(key)
            if cached is not None:
                return self._authorization(cached)
            held = self._store.load(key)
            url, fields, basic = self._grant(held)
            response = await self._session.request_async(url, fields, basic)
            token = _parse_token(response, self._session.descriptor, self._session.clock,
                                 retained_refresh=None if held is None else held.refresh_token)
            self._store.replace(key, token)
            return self._authorization(token)

    def _grant(self, held: TokenSet | None) -> tuple[str, dict[str, str], str | None]:
        """The refresh grant for a held refresh token, else client-credentials."""
        session = self._session
        if held is not None and type(held.refresh_token) is str and held.refresh_token:
            try:
                url, flow = _refresh_endpoint(session.descriptor)
            except AuthError:
                pass
            else:
                basic = session.basic_auth(flow)
                client_id, _ = session.resolve()
                fields = _form({'grant_type': 'refresh_token', 'refresh_token': held.refresh_token,
                                'client_id': client_id if basic is None else None})
                return url, fields, basic
        try:
            flow = _flow(session.descriptor, 'client-credentials')
        except AuthError as error:
            raise AuthError('renewal-required', session.scheme, cause=error) from None
        requested = _requested(session.descriptor, 'client-credentials', self._scopes)
        basic = session.basic_auth(flow)
        client_id, _ = session.resolve()
        fields = _form({'grant_type': 'client_credentials',
                        'scope': ' '.join(requested) if requested else None,
                        'client_id': client_id if basic is None else None})
        return flow['token_url'], fields, basic

    def close(self) -> None:
        self._session.close()

    async def aclose(self) -> None:
        await self._session.aclose()


def client_credential(scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                      store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                      transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                      scopes: Sequence[str] | None = None) -> Credential:
    """A generated ``Credential`` for one compiled OAuth scheme.

    Attach it under the scheme's source name::

        client = Client(auth={'serviceOAuth': client_credential('serviceOAuth')})

    The provider answers the client's credential attach path with an
    ``Authorization`` value from the store; when the stored ``TokenSet`` is
    absent or expired beyond the compiled skew it acquires a fresh one over
    the compiled token endpoint with the compiled client authentication
    (``client_secret_basic`` per RFC 6749 2.3.1, or a public body client id),
    then replaces the stored entry atomically. ``client_id`` and
    ``client_secret`` win over the configured environment variables, which are
    read at call time. ``store`` defaults to a fresh per-provider
    ``MemoryTokenStore``; ``clock`` defaults to ``time.time``; ``transport``
    injects the httpx transport used for the token requests; ``scopes`` must
    stay within the compiled declared scopes.
    """
    return _ClientCredential(scheme, client_id=client_id, client_secret=client_secret,
                             store=store, clock=clock, transport=transport, scopes=scopes)
"##;

const REFRESH: &str = r##"

def refresh_token_set(scheme: str, token: TokenSet, *, client_id: str | None = None,
                      client_secret: str | None = None, store: TokenStore | None = None,
                      clock: Callable[[], float] | None = None,
                      transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Explicitly refresh ``token`` over the declared refresh or token URL.

    Adopts a rotated refresh token from the response, otherwise retains the
    current one. When ``store`` is supplied the refreshed set atomically
    replaces the stored entry under the partition key. Requires a refresh
    token on ``token`` and a scheme whose compiled flows carry a refresh or
    token URL.
    """
    descriptor = _scheme(scheme)
    if type(token.refresh_token) is not str or not token.refresh_token:
        raise AuthError('no-refresh-token', scheme)
    url, flow = _refresh_endpoint(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'grant_type': 'refresh_token', 'refresh_token': token.refresh_token,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(url, fields, basic)
        refreshed = _parse_token(response, descriptor, session.clock, retained_refresh=token.refresh_token)
        if store is not None:
            store.replace(session.key(), refreshed)
        return refreshed
    finally:
        session.close()


async def refresh_token_set_async(scheme: str, token: TokenSet, *, client_id: str | None = None,
                                  client_secret: str | None = None, store: TokenStore | None = None,
                                  clock: Callable[[], float] | None = None,
                                  transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Async variant of :func:`refresh_token_set` on the caller task."""
    descriptor = _scheme(scheme)
    if type(token.refresh_token) is not str or not token.refresh_token:
        raise AuthError('no-refresh-token', scheme)
    url, flow = _refresh_endpoint(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'grant_type': 'refresh_token', 'refresh_token': token.refresh_token,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(url, fields, basic)
        refreshed = _parse_token(response, descriptor, session.clock, retained_refresh=token.refresh_token)
        if store is not None:
            store.replace(session.key(), refreshed)
        return refreshed
    finally:
        await session.aclose()
"##;

const AUTHORIZATION_CODE: &str = r##"

@dataclasses.dataclass(kw_only=True)
class AuthorizationTransaction:
    """One bound, single-use authorization-code transaction.

    ``authorization_url`` carries the response type, client id, redirect URI,
    state and the PKCE S256 challenge derived from the retained verifier.
    :func:`complete_authorization` consumes the transaction exactly once; a
    repeated completion raises ``AuthError('transaction-used', ...)``.
    """

    scheme: str
    authorization_url: str
    state: str
    code_verifier: str = dataclasses.field(repr=False)
    code_challenge: str
    code_challenge_method: str
    redirect_uri: str
    scopes: tuple[str, ...]
    created_at: int
    _consumed: bool = dataclasses.field(default=False, repr=False, compare=False)


def _pkce() -> tuple[str, str]:
    """RFC 7636 S256: a random verifier and its hashed challenge."""
    verifier = secrets.token_urlsafe(64)
    digest = hashlib.sha256(verifier.encode('ascii')).digest()
    challenge = base64.urlsafe_b64encode(digest).decode('ascii').rstrip('=')
    return verifier, challenge


def _flatten(params: Mapping[str, str | Sequence[str]]) -> dict[str, str]:
    flat: dict[str, str] = {}
    for name, value in params.items():
        flat[name] = value[0] if isinstance(value, (list, tuple)) and value else value
    return flat


def begin_authorization(scheme: str, *, redirect_uri: str, scopes: Sequence[str] | None = None,
                        state: str | None = None, client_id: str | None = None,
                        clock: Callable[[], float] | None = None) -> AuthorizationTransaction:
    """Start one authorization-code + PKCE S256 transaction.

    The transaction binds the scheme, redirect URI, state and verifier.
    ``state`` defaults to a fresh random token; ``client_id`` resolves like
    every other client input, from the explicit argument or the configured
    variable read now. Requested scopes must stay within the compiled
    declared scopes. Send the caller to ``authorization_url``; pass the
    resulting query parameters to :func:`complete_authorization`.
    """
    descriptor = _scheme(scheme)
    flow = _flow(descriptor, 'authorization-code')
    authorization_url = flow.get('authorization_url')
    if type(authorization_url) is not str or not authorization_url:
        raise AuthError('unsupported', scheme)
    session = _Session(scheme, client_id=client_id)
    try:
        resolved, _ = session.resolve()
    finally:
        session.close()
    if type(resolved) is not str or not resolved:
        raise AuthError('missing-client-credentials', scheme)
    requested = _requested(descriptor, 'authorization-code', scopes)
    verifier, challenge = _pkce()
    chosen = state if type(state) is str and state else secrets.token_urlsafe(16)
    query = _form({'response_type': 'code', 'client_id': resolved, 'redirect_uri': redirect_uri,
                   'state': chosen, 'code_challenge': challenge, 'code_challenge_method': 'S256',
                   'scope': ' '.join(requested) if requested else None})
    url = authorization_url + ('&' if '?' in authorization_url else '?') + urllib.parse.urlencode(query)
    return AuthorizationTransaction(scheme=scheme, authorization_url=url, state=chosen,
                                    code_verifier=verifier, code_challenge=challenge,
                                    code_challenge_method='S256', redirect_uri=redirect_uri,
                                    scopes=requested, created_at=int((clock if clock is not None else time.time)()))


def complete_authorization(transaction: AuthorizationTransaction,
                           callback_params: Mapping[str, str | Sequence[str]], *, client_id: str | None = None,
                           client_secret: str | None = None, store: TokenStore | None = None,
                           clock: Callable[[], float] | None = None,
                           transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Exchange the transaction's authorization code for a ``TokenSet``.

    Validates the callback's ``state`` against the transaction, refuses a
    server-declared error or a missing code, marks the transaction consumed
    before any network work, and exchanges the code with the PKCE verifier
    over the compiled token URL and client authentication. The resulting set
    replaces the stored entry atomically when ``store`` is supplied.
    """
    if transaction._consumed:
        raise AuthError('transaction-used', transaction.scheme)
    params = _flatten(callback_params)
    error = params.get('error')
    if type(error) is str and error:
        raise AuthError('authorization-denied', transaction.scheme, code=error)
    if params.get('state') != transaction.state:
        raise AuthError('state-mismatch', transaction.scheme)
    code = params.get('code')
    if type(code) is not str or not code:
        raise AuthError('invalid-callback', transaction.scheme)
    descriptor = _scheme(transaction.scheme)
    flow = _flow(descriptor, 'authorization-code')
    session = _Session(transaction.scheme, client_id=client_id, client_secret=client_secret,
                       clock=clock, transport=transport)
    transaction._consumed = True
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'grant_type': 'authorization_code', 'code': code,
                        'redirect_uri': transaction.redirect_uri, 'code_verifier': transaction.code_verifier,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(flow['token_url'], fields, basic)
        token = _parse_token(response, descriptor, session.clock)
        if store is not None:
            store.replace(session.key(), token)
        return token
    finally:
        session.close()


async def complete_authorization_async(transaction: AuthorizationTransaction,
                                       callback_params: Mapping[str, str | Sequence[str]], *,
                                       client_id: str | None = None, client_secret: str | None = None,
                                       store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                                       transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Async variant of :func:`complete_authorization` on the caller task."""
    if transaction._consumed:
        raise AuthError('transaction-used', transaction.scheme)
    params = _flatten(callback_params)
    error = params.get('error')
    if type(error) is str and error:
        raise AuthError('authorization-denied', transaction.scheme, code=error)
    if params.get('state') != transaction.state:
        raise AuthError('state-mismatch', transaction.scheme)
    code = params.get('code')
    if type(code) is not str or not code:
        raise AuthError('invalid-callback', transaction.scheme)
    descriptor = _scheme(transaction.scheme)
    flow = _flow(descriptor, 'authorization-code')
    session = _Session(transaction.scheme, client_id=client_id, client_secret=client_secret,
                       clock=clock, transport=transport)
    transaction._consumed = True
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'grant_type': 'authorization_code', 'code': code,
                        'redirect_uri': transaction.redirect_uri, 'code_verifier': transaction.code_verifier,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(flow['token_url'], fields, basic)
        token = _parse_token(response, descriptor, session.clock)
        if store is not None:
            store.replace(session.key(), token)
        return token
    finally:
        await session.aclose()
"##;

const DEVICE: &str = r##"

@dataclasses.dataclass(frozen=True, kw_only=True)
class DeviceAuthorization:
    """One device-authorization grant from the declared endpoint (RFC 8628).
    The device code never appears in the repr."""

    scheme: str
    device_code: str = dataclasses.field(repr=False)
    user_code: str
    verification_uri: str
    verification_uri_complete: str | None = None
    expires_at: int | None = None
    interval: float = 5.0


def begin_device_authorization(scheme: str, *, client_id: str | None = None,
                               scopes: Sequence[str] | None = None,
                               clock: Callable[[], float] | None = None,
                               transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> DeviceAuthorization:
    """Request one device grant from the compiled device-authorization URL.

    ``client_id`` resolves from the explicit argument or the configured
    variable read at call time; requested scopes must stay within the
    compiled declared scopes. Show the user the returned user code and
    verification URI, then poll with :func:`poll_device_authorization`.
    """
    descriptor = _scheme(scheme)
    flow = _flow(descriptor, 'device-authorization')
    session = _Session(scheme, client_id=client_id, clock=clock, transport=transport)
    try:
        resolved, _ = session.resolve()
        if type(resolved) is not str or not resolved:
            raise AuthError('missing-client-credentials', scheme)
        requested = _requested(descriptor, 'device-authorization', scopes)
        basic = session.basic_auth(flow)
        fields = _form({'client_id': resolved, 'scope': ' '.join(requested) if requested else None})
        response = session.request_sync(flow['device_authorization_url'], fields, basic)
        if response.status_code != 200:
            raise AuthError('device-request', scheme, code=_server_code(response), status=response.status_code)
        try:
            payload = response.json()
        except Exception:
            raise AuthError('invalid-response', scheme, status=response.status_code) from None
        if not isinstance(payload, dict):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        device_code = payload.get('device_code')
        user_code = payload.get('user_code')
        verification_uri = payload.get('verification_uri')
        expires_in = payload.get('expires_in')
        if (type(device_code) is not str or not device_code or type(user_code) is not str or not user_code
                or type(verification_uri) is not str or not verification_uri
                or (type(expires_in) is not int and type(expires_in) is not float)):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        complete = payload.get('verification_uri_complete')
        complete = complete if type(complete) is str and complete else None
        interval = payload.get('interval')
        interval = float(interval) if type(interval) is int or type(interval) is float else 5.0
        return DeviceAuthorization(scheme=scheme, device_code=device_code, user_code=user_code,
                                   verification_uri=verification_uri, verification_uri_complete=complete,
                                   expires_at=int(session.clock()) + int(expires_in), interval=interval)
    finally:
        session.close()


def poll_device_authorization(device: DeviceAuthorization, *, client_id: str | None = None,
                              client_secret: str | None = None, store: TokenStore | None = None,
                              clock: Callable[[], float] | None = None,
                              transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                              wait: Callable[[float], None] | None = None) -> TokenSet:
    """Poll the compiled token URL for the device grant (RFC 8628 3.5).

    ``authorization_pending`` waits the declared interval and retries,
    ``slow_down`` grows the interval by five seconds, any other refusal is a
    typed failure, and polling stops once the grant's declared expiry passes.
    The resulting ``TokenSet`` replaces the stored entry atomically when
    ``store`` is supplied. ``wait`` replaces the sleeper (tests pass a no-op).
    """
    descriptor = _scheme(device.scheme)
    flow = _flow(descriptor, 'device-authorization')
    session = _Session(device.scheme, client_id=client_id, client_secret=client_secret,
                       clock=clock, transport=transport)
    pause = wait if wait is not None else time.sleep
    interval = device.interval
    try:
        resolved, _ = session.resolve()
        basic = session.basic_auth(flow)
        fields = _form({'grant_type': _DEVICE_GRANT, 'device_code': device.device_code,
                        'client_id': resolved if basic is None else None})
        while True:
            if device.expires_at is not None and int(session.clock()) >= device.expires_at:
                raise AuthError('device-flow-expired', device.scheme)
            response = session.request_sync(flow['token_url'], fields, basic)
            if response.status_code == 200:
                token = _parse_token(response, descriptor, session.clock)
                if store is not None:
                    store.replace(session.key(), token)
                return token
            code = _server_code(response)
            if code == 'authorization_pending':
                pause(interval)
                continue
            if code == 'slow_down':
                interval = interval + 5.0
                pause(interval)
                continue
            raise AuthError('token-request', device.scheme, code=code, status=response.status_code)
    finally:
        session.close()


async def poll_device_authorization_async(device: DeviceAuthorization, *, client_id: str | None = None,
                                          client_secret: str | None = None, store: TokenStore | None = None,
                                          clock: Callable[[], float] | None = None,
                                          transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                                          wait: Callable[[float], None] | None = None) -> TokenSet:
    """Async variant of :func:`poll_device_authorization` on the caller task."""
    descriptor = _scheme(device.scheme)
    flow = _flow(descriptor, 'device-authorization')
    session = _Session(device.scheme, client_id=client_id, client_secret=client_secret,
                       clock=clock, transport=transport)
    interval = device.interval
    try:
        resolved, _ = session.resolve()
        basic = session.basic_auth(flow)
        fields = _form({'grant_type': _DEVICE_GRANT, 'device_code': device.device_code,
                        'client_id': resolved if basic is None else None})
        while True:
            if device.expires_at is not None and int(session.clock()) >= device.expires_at:
                raise AuthError('device-flow-expired', device.scheme)
            response = await session.request_async(flow['token_url'], fields, basic)
            if response.status_code == 200:
                token = _parse_token(response, descriptor, session.clock)
                if store is not None:
                    store.replace(session.key(), token)
                return token
            code = _server_code(response)
            if code == 'authorization_pending':
                if wait is None:
                    await asyncio.sleep(interval)
                else:
                    wait(interval)
                continue
            if code == 'slow_down':
                interval = interval + 5.0
                if wait is None:
                    await asyncio.sleep(interval)
                else:
                    wait(interval)
                continue
            raise AuthError('token-request', device.scheme, code=code, status=response.status_code)
    finally:
        await session.aclose()
"##;

const REVOCATION: &str = r##"

def revoke(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
           client_id: str | None = None, client_secret: str | None = None,
           store: TokenStore | None = None, clock: Callable[[], float] | None = None,
           transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> None:
    """RFC 7009 revocation over the compiled revocation endpoint.

    ``token`` accepts the raw token string or the ``TokenSet`` holding it.
    The scheme's client authentication applies. A successful revocation
    clears the store partition when ``store`` is supplied.
    """
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('revocation')
    if type(endpoint) is not str or not endpoint:
        raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    flow = _client_auth_flow(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('revocation', scheme, code=_server_code(response), status=response.status_code)
        if store is not None:
            store.clear(session.key())
    finally:
        session.close()


async def revoke_async(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
                       client_id: str | None = None, client_secret: str | None = None,
                       store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                       transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> None:
    """Async variant of :func:`revoke` on the caller task."""
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('revocation')
    if type(endpoint) is not str or not endpoint:
        raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    flow = _client_auth_flow(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('revocation', scheme, code=_server_code(response), status=response.status_code)
        if store is not None:
            store.clear(session.key())
    finally:
        await session.aclose()
"##;

const INTROSPECTION: &str = r##"

def introspect(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
               client_id: str | None = None, client_secret: str | None = None,
               clock: Callable[[], float] | None = None,
               transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> dict[str, Any]:
    """RFC 7662 introspection over the compiled introspection endpoint.

    ``token`` accepts the raw token string or the ``TokenSet`` holding it.
    Returns the server's introspection claim dictionary; any non-200 answer
    is a typed failure whose message stays free of token values.
    """
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('introspection')
    if type(endpoint) is not str or not endpoint:
        raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    flow = _client_auth_flow(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('introspection', scheme, code=_server_code(response), status=response.status_code)
        try:
            payload = response.json()
        except Exception:
            raise AuthError('invalid-response', scheme, status=response.status_code) from None
        if not isinstance(payload, dict):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        return payload
    finally:
        session.close()


async def introspect_async(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
                           client_id: str | None = None, client_secret: str | None = None,
                           clock: Callable[[], float] | None = None,
                           transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> dict[str, Any]:
    """Async variant of :func:`introspect` on the caller task."""
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('introspection')
    if type(endpoint) is not str or not endpoint:
        raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    flow = _client_auth_flow(descriptor)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        basic = session.basic_auth(flow)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('introspection', scheme, code=_server_code(response), status=response.status_code)
        try:
            payload = response.json()
        except Exception:
            raise AuthError('invalid-response', scheme, status=response.status_code) from None
        if not isinstance(payload, dict):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        return payload
    finally:
        await session.aclose()
"##;

/// The discovery-aware credential provider: endpoint resolution follows the
/// compiled precedence (explicit compiled endpoints always win; otherwise the
/// provider's cached discovery document).
const PROVIDER_DISCOVERY: &str = r##"

class _ClientCredential:
    """One compiled scheme's on-demand credential provider.

    The generated client's existing credential attach path calls it with the
    located requirement and receives an ``Authorization`` value built from a
    cached or freshly acquired ``TokenSet``. Acquisition is single-flighted
    per provider instance — ``threading.Lock`` on synchronous call sites,
    ``asyncio.Lock`` inside a running event loop — so concurrent attaches
    share one token request. A held refresh token drives the on-demand
    refresh grant; a rotated refresh token is adopted, otherwise the current
    one is retained. Inside a running loop the call returns the awaitable the
    async resolve path expects; loopless synchronous call sites receive the
    Authorization directly.

    Endpoint resolution follows the compiled precedence: an explicitly
    compiled endpoint always wins; a scheme whose compiled plan declares no
    usable endpoint resolves the RFC 8414 / OpenID Connect discovery document
    cached on this provider instead — fetched once per scheme, single-flighted
    with the acquisition, and retried on the next call after a failure.
    """

    def __init__(self, scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                 store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                 transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                 scopes: Sequence[str] | None = None) -> None:
        self._session = _Session(scheme, client_id=client_id, client_secret=client_secret,
                                 clock=clock, transport=transport)
        self._store: TokenStore = store if store is not None else MemoryTokenStore()
        self._scopes = None if scopes is None else tuple(scopes)
        self._sync_lock = threading.Lock()
        self._async_lock: asyncio.Lock | None = None
        self._discovered_document: dict[str, Any] | None = None

    def __call__(self, request: object) -> Authorization:
        if _running_loop():
            return self._acquire_async()  # type: ignore[return-value]
        return self._acquire_sync()

    def _authorization(self, token: TokenSet) -> Authorization:
        return Authorization(value=token.token_type + ' ' + token.access_token)

    def _valid(self, key: tuple[str, str, str]) -> TokenSet | None:
        token = self._store.load(key)
        if token is not None and token.expired(int(self._session.clock())):
            return None
        return token

    def _discovered(self) -> dict[str, Any]:
        """This provider's cached discovery document, fetched once per scheme."""
        if self._discovered_document is None:
            self._discovered_document = _discover_sync(self._session.scheme, self._session._transport)
        return self._discovered_document

    async def _discovered_async(self) -> dict[str, Any]:
        if self._discovered_document is None:
            self._discovered_document = await _discover_async(self._session.scheme, self._session._transport)
        return self._discovered_document

    def _discovery_member(self, field: str) -> str | None:
        """One discovery document endpoint, through the provider cache."""
        return _discovered_endpoint(self._session.scheme, self._discovered(), field)

    async def _discovery_member_async(self, field: str) -> str | None:
        return _discovered_endpoint(self._session.scheme, await self._discovered_async(), field)

    def _acquire_sync(self) -> Authorization:
        key = self._session.key()
        cached = self._valid(key)
        if cached is not None:
            return self._authorization(cached)
        with self._sync_lock:
            cached = self._valid(key)
            if cached is not None:
                return self._authorization(cached)
            held = self._store.load(key)
            token = self._grant_sync(held)
            self._store.replace(key, token)
            return self._authorization(token)

    def _grant_sync(self, held: TokenSet | None) -> TokenSet:
        url, fields, basic = self._grant(held)
        response = self._session.request_sync(url, fields, basic)
        return _parse_token(response, self._session.descriptor, self._session.clock,
                            retained_refresh=None if held is None else held.refresh_token)

    async def _acquire_async(self) -> Authorization:
        key = self._session.key()
        cached = self._valid(key)
        if cached is not None:
            return self._authorization(cached)
        if self._async_lock is None:
            self._async_lock = asyncio.Lock()
        async with self._async_lock:
            cached = self._valid(key)
            if cached is not None:
                return self._authorization(cached)
            held = self._store.load(key)
            url, fields, basic = await self._grant_async(held)
            response = await self._session.request_async(url, fields, basic)
            token = _parse_token(response, self._session.descriptor, self._session.clock,
                                 retained_refresh=None if held is None else held.refresh_token)
            self._store.replace(key, token)
            return self._authorization(token)

    def _grant(self, held: TokenSet | None) -> tuple[str, dict[str, str], str | None]:
        """The refresh grant for a held refresh token, else client-credentials.

        Endpoint resolution follows the compiled precedence: an explicitly
        compiled endpoint always wins; a scheme whose compiled plan declares
        no usable endpoint resolves the discovery document cached on this
        provider instead.
        """
        session = self._session
        if held is not None and type(held.refresh_token) is str and held.refresh_token:
            try:
                url, flow = _refresh_endpoint(session.descriptor)
            except AuthError:
                pass
            else:
                basic = session.basic_auth(flow)
                client_id, _ = session.resolve()
                fields = _form({'grant_type': 'refresh_token', 'refresh_token': held.refresh_token,
                                'client_id': client_id if basic is None else None})
                return url, fields, basic
            url = self._discovery_member('token_endpoint')
            if url is not None:
                basic = _discovery_auth(session)
                client_id, _ = session.resolve()
                fields = _form({'grant_type': 'refresh_token', 'refresh_token': held.refresh_token,
                                'client_id': client_id if basic is None else None})
                return url, fields, basic
        try:
            flow = _flow(session.descriptor, 'client-credentials')
        except AuthError as error:
            url = self._discovery_member('token_endpoint')
            if url is None:
                raise AuthError('renewal-required', session.scheme, cause=error) from None
            basic = _discovery_auth(session)
            client_id, _ = session.resolve()
            fields = _form({'grant_type': 'client_credentials',
                            'scope': ' '.join(self._scopes) if self._scopes else None,
                            'client_id': client_id if basic is None else None})
            return url, fields, basic
        requested = _requested(session.descriptor, 'client-credentials', self._scopes)
        basic = session.basic_auth(flow)
        client_id, _ = session.resolve()
        fields = _form({'grant_type': 'client_credentials',
                        'scope': ' '.join(requested) if requested else None,
                        'client_id': client_id if basic is None else None})
        return flow['token_url'], fields, basic

    async def _grant_async(self, held: TokenSet | None) -> tuple[str, dict[str, str], str | None]:
        """Async variant of :meth:`_grant`; discovery fetches on the caller task."""
        session = self._session
        if held is not None and type(held.refresh_token) is str and held.refresh_token:
            try:
                url, flow = _refresh_endpoint(session.descriptor)
            except AuthError:
                pass
            else:
                basic = session.basic_auth(flow)
                client_id, _ = session.resolve()
                fields = _form({'grant_type': 'refresh_token', 'refresh_token': held.refresh_token,
                                'client_id': client_id if basic is None else None})
                return url, fields, basic
            url = await self._discovery_member_async('token_endpoint')
            if url is not None:
                basic = _discovery_auth(session)
                client_id, _ = session.resolve()
                fields = _form({'grant_type': 'refresh_token', 'refresh_token': held.refresh_token,
                                'client_id': client_id if basic is None else None})
                return url, fields, basic
        try:
            flow = _flow(session.descriptor, 'client-credentials')
        except AuthError as error:
            url = await self._discovery_member_async('token_endpoint')
            if url is None:
                raise AuthError('renewal-required', session.scheme, cause=error) from None
            basic = _discovery_auth(session)
            client_id, _ = session.resolve()
            fields = _form({'grant_type': 'client_credentials',
                            'scope': ' '.join(self._scopes) if self._scopes else None,
                            'client_id': client_id if basic is None else None})
            return url, fields, basic
        requested = _requested(session.descriptor, 'client-credentials', self._scopes)
        basic = session.basic_auth(flow)
        client_id, _ = session.resolve()
        fields = _form({'grant_type': 'client_credentials',
                        'scope': ' '.join(requested) if requested else None,
                        'client_id': client_id if basic is None else None})
        return flow['token_url'], fields, basic

    def close(self) -> None:
        self._session.close()

    async def aclose(self) -> None:
        await self._session.aclose()


def client_credential(scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                      store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                      transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                      scopes: Sequence[str] | None = None) -> Credential:
    """A generated ``Credential`` for one compiled OAuth scheme.

    Attach it under the scheme's source name::

        client = Client(auth={'serviceOAuth': client_credential('serviceOAuth')})

    The provider answers the client's credential attach path with an
    ``Authorization`` value from the store; when the stored ``TokenSet`` is
    absent or expired beyond the compiled skew it acquires a fresh one over
    the resolved token endpoint with the resolved client authentication
    (``client_secret_basic`` per RFC 6749 2.3.1, or a public body client id),
    then replaces the stored entry atomically. Endpoint resolution follows
    the compiled precedence: an explicitly compiled token URL always wins;
    otherwise the discovery document cached on this provider supplies it.
    ``client_id`` and ``client_secret`` win over the configured environment
    variables, which are read at call time. ``store`` defaults to a fresh
    per-provider ``MemoryTokenStore``; ``clock`` defaults to ``time.time``;
    ``transport`` injects the httpx transport used for the token requests;
    ``scopes`` must stay within the compiled declared scopes when the plan
    declares any.
    """
    return _ClientCredential(scheme, client_id=client_id, client_secret=client_secret,
                             store=store, clock=clock, transport=transport, scopes=scopes)
"##;

/// The discovery-aware explicit refresh: the compiled refresh or token URL
/// always wins; otherwise the discovery document's token endpoint.
const REFRESH_DISCOVERY: &str = r##"

def refresh_token_set(scheme: str, token: TokenSet, *, client_id: str | None = None,
                      client_secret: str | None = None, store: TokenStore | None = None,
                      clock: Callable[[], float] | None = None,
                      transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Explicitly refresh ``token`` over the resolved refresh endpoint.

    Adopts a rotated refresh token from the response, otherwise retains the
    current one. When ``store`` is supplied the refreshed set atomically
    replaces the stored entry under the partition key. Requires a refresh
    token on ``token``. Endpoint resolution follows the compiled precedence:
    the declared refresh URL, else the declared token URL, always wins; a
    scheme whose compiled plan declares neither resolves the discovery
    document's token endpoint (fetched per call; this one-shot helper keeps
    no cache).
    """
    descriptor = _scheme(scheme)
    if type(token.refresh_token) is not str or not token.refresh_token:
        raise AuthError('no-refresh-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        resolved, _ = session.resolve()
        try:
            url, flow = _refresh_endpoint(descriptor)
        except AuthError:
            payload = _discover_sync(scheme, transport)
            url = _discovered_endpoint(scheme, payload, 'token_endpoint')
            if url is None:
                raise
            basic = _discovery_auth(session)
        else:
            basic = session.basic_auth(flow)
        fields = _form({'grant_type': 'refresh_token', 'refresh_token': token.refresh_token,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(url, fields, basic)
        refreshed = _parse_token(response, descriptor, session.clock, retained_refresh=token.refresh_token)
        if store is not None:
            store.replace(session.key(), refreshed)
        return refreshed
    finally:
        session.close()


async def refresh_token_set_async(scheme: str, token: TokenSet, *, client_id: str | None = None,
                                  client_secret: str | None = None, store: TokenStore | None = None,
                                  clock: Callable[[], float] | None = None,
                                  transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> TokenSet:
    """Async variant of :func:`refresh_token_set` on the caller task."""
    descriptor = _scheme(scheme)
    if type(token.refresh_token) is not str or not token.refresh_token:
        raise AuthError('no-refresh-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        resolved, _ = session.resolve()
        try:
            url, flow = _refresh_endpoint(descriptor)
        except AuthError:
            payload = await _discover_async(scheme, transport)
            url = _discovered_endpoint(scheme, payload, 'token_endpoint')
            if url is None:
                raise
            basic = _discovery_auth(session)
        else:
            basic = session.basic_auth(flow)
        fields = _form({'grant_type': 'refresh_token', 'refresh_token': token.refresh_token,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(url, fields, basic)
        refreshed = _parse_token(response, descriptor, session.clock, retained_refresh=token.refresh_token)
        if store is not None:
            store.replace(session.key(), refreshed)
        return refreshed
    finally:
        await session.aclose()
"##;

/// RFC 7009 revocation with discovery fallback: the compiled endpoint always
/// wins; otherwise the discovery document's ``revocation_endpoint``.
const REVOCATION_DISCOVERY: &str = r##"

def revoke(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
           client_id: str | None = None, client_secret: str | None = None,
           store: TokenStore | None = None, clock: Callable[[], float] | None = None,
           transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> None:
    """RFC 7009 revocation over the resolved revocation endpoint.

    ``token`` accepts the raw token string or the ``TokenSet`` holding it.
    Endpoint resolution follows the compiled precedence: the configured
    revocation endpoint always wins; a scheme without one resolves the
    discovery document's ``revocation_endpoint`` (fetched per call; this
    one-shot helper keeps no cache). The scheme's client authentication
    applies. A successful revocation clears the store partition when
    ``store`` is supplied.
    """
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('revocation')
    if type(endpoint) is not str or not endpoint:
        payload = _discover_sync(scheme, transport)
        endpoint = _discovered_endpoint(scheme, payload, 'revocation_endpoint')
        if type(endpoint) is not str or not endpoint:
            raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        try:
            flow = _client_auth_flow(descriptor)
        except AuthError:
            flow = None
        basic = session.basic_auth(flow) if flow is not None else _discovery_auth(session)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('revocation', scheme, code=_server_code(response), status=response.status_code)
        if store is not None:
            store.clear(session.key())
    finally:
        session.close()


async def revoke_async(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
                       client_id: str | None = None, client_secret: str | None = None,
                       store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                       transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> None:
    """Async variant of :func:`revoke` on the caller task."""
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('revocation')
    if type(endpoint) is not str or not endpoint:
        payload = await _discover_async(scheme, transport)
        endpoint = _discovered_endpoint(scheme, payload, 'revocation_endpoint')
        if type(endpoint) is not str or not endpoint:
            raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        try:
            flow = _client_auth_flow(descriptor)
        except AuthError:
            flow = None
        basic = session.basic_auth(flow) if flow is not None else _discovery_auth(session)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('revocation', scheme, code=_server_code(response), status=response.status_code)
        if store is not None:
            store.clear(session.key())
    finally:
        await session.aclose()
"##;

/// RFC 7662 introspection with discovery fallback: the compiled endpoint
/// always wins; otherwise the discovery document's ``introspection_endpoint``.
const INTROSPECTION_DISCOVERY: &str = r##"

def introspect(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
               client_id: str | None = None, client_secret: str | None = None,
               clock: Callable[[], float] | None = None,
               transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> dict[str, Any]:
    """RFC 7662 introspection over the resolved introspection endpoint.

    ``token`` accepts the raw token string or the ``TokenSet`` holding it.
    Endpoint resolution follows the compiled precedence: the configured
    introspection endpoint always wins; a scheme without one resolves the
    discovery document's ``introspection_endpoint`` (fetched per call; this
    one-shot helper keeps no cache). Returns the server's introspection claim
    dictionary; any non-200 answer is a typed failure whose message stays
    free of token values.
    """
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('introspection')
    if type(endpoint) is not str or not endpoint:
        payload = _discover_sync(scheme, transport)
        endpoint = _discovered_endpoint(scheme, payload, 'introspection_endpoint')
        if type(endpoint) is not str or not endpoint:
            raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        try:
            flow = _client_auth_flow(descriptor)
        except AuthError:
            flow = None
        basic = session.basic_auth(flow) if flow is not None else _discovery_auth(session)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = session.request_sync(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('introspection', scheme, code=_server_code(response), status=response.status_code)
        try:
            payload = response.json()
        except Exception:
            raise AuthError('invalid-response', scheme, status=response.status_code) from None
        if not isinstance(payload, dict):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        return payload
    finally:
        session.close()


async def introspect_async(scheme: str, token: str | TokenSet, *, token_type_hint: str | None = None,
                           client_id: str | None = None, client_secret: str | None = None,
                           clock: Callable[[], float] | None = None,
                           transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None) -> dict[str, Any]:
    """Async variant of :func:`introspect` on the caller task."""
    descriptor = _scheme(scheme)
    endpoint = descriptor.get('introspection')
    if type(endpoint) is not str or not endpoint:
        payload = await _discover_async(scheme, transport)
        endpoint = _discovered_endpoint(scheme, payload, 'introspection_endpoint')
        if type(endpoint) is not str or not endpoint:
            raise AuthError('unsupported', scheme)
    value = token.access_token if isinstance(token, TokenSet) else token
    if type(value) is not str or not value:
        raise AuthError('invalid-token', scheme)
    session = _Session(scheme, client_id=client_id, client_secret=client_secret, clock=clock, transport=transport)
    try:
        try:
            flow = _client_auth_flow(descriptor)
        except AuthError:
            flow = None
        basic = session.basic_auth(flow) if flow is not None else _discovery_auth(session)
        resolved, _ = session.resolve()
        fields = _form({'token': value, 'token_type_hint': token_type_hint,
                        'client_id': resolved if basic is None else None})
        response = await session.request_async(endpoint, fields, basic)
        if response.status_code != 200:
            raise AuthError('introspection', scheme, code=_server_code(response), status=response.status_code)
        try:
            payload = response.json()
        except Exception:
            raise AuthError('invalid-response', scheme, status=response.status_code) from None
        if not isinstance(payload, dict):
            raise AuthError('invalid-response', scheme, status=response.status_code)
        return payload
    finally:
        await session.aclose()
"##;

/// The RFC 8414 / OpenID Connect discovery engine, emitted only when at least
/// one compiled scheme carries a discovery URL: the typed decode with the
/// documented issuer rule, the discovery helpers and the client-authentication
/// rule for discovery-resolved endpoints.
const DISCOVERY: &str = r##"

_DISCOVERY_MAX_BYTES = 1 << 20


def _discovery_origin(url: str) -> str | None:
    """The URL's origin with the scheme's default port made explicit."""
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme not in ('http', 'https') or not parsed.netloc:
        return None
    host = parsed.hostname
    if host is None:
        return None
    try:
        port = parsed.port
    except ValueError:
        return None
    if port is None:
        port = 80 if parsed.scheme == 'http' else 443
    return parsed.scheme + '://' + host + ':' + str(port)


def _discovery_document(scheme: str, url: str, response: httpx.Response) -> dict[str, Any]:
    """Typed RFC 8414 / OpenID Connect discovery decode.

    The response must be a JSON object whose ``issuer`` claim, when present,
    is an absolute http(s) URL sharing the discovery URL's origin (scheme and
    host with the default port made explicit): OpenID Connect
    openIdConnectUrl documents are validated against their ``issuer`` claim
    exactly this way, as are RFC 8414 authorization-server metadata
    documents. ``token_endpoint``, ``revocation_endpoint`` and
    ``introspection_endpoint`` are extracted when present and every other
    member is ignored. Failures raise ``AuthError('discovery-failed', ...)``
    carrying no response body text.
    """
    if response.status_code != 200:
        raise AuthError('discovery-failed', scheme, status=response.status_code)
    if len(response.content) > _DISCOVERY_MAX_BYTES:
        raise AuthError('discovery-failed', scheme)
    try:
        payload = response.json()
    except Exception:
        raise AuthError('discovery-failed', scheme) from None
    if not isinstance(payload, dict):
        raise AuthError('discovery-failed', scheme)
    issuer = payload.get('issuer')
    if type(issuer) is str and issuer:
        issuer_origin = _discovery_origin(issuer)
        if issuer_origin is None or issuer_origin != _discovery_origin(url):
            raise AuthError('discovery-failed', scheme)
    return payload


def _discovered_endpoint(scheme: str, payload: Mapping[str, Any], field: str) -> str | None:
    """One discovery document endpoint: absent stays None, and an unusable
    value is a typed discovery failure. Unknown members are ignored."""
    value = payload.get(field)
    if value is None:
        return None
    if type(value) is not str or not value:
        raise AuthError('discovery-failed', scheme)
    return value


def _discover_sync(scheme: str,
                   transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None) -> dict[str, Any]:
    """GET the scheme's compiled discovery document.

    ``accept: application/json`` on the request, the compiled token timeout on
    the httpx client, a ~1 MiB response ceiling, and the caller's transport.
    A scheme whose compiled plan carries no discovery URL is a typed refusal.
    """
    descriptor = _scheme(scheme)
    url = descriptor.get('discovery')
    if type(url) is not str or not url:
        raise AuthError('unsupported', scheme)
    try:
        with httpx.Client(trust_env=False, timeout=_TOKEN_TIMEOUT, transport=transport) as client:
            response = client.get(url, headers={'Accept': 'application/json'})
    except (httpx.HTTPError, TypeError) as error:
        raise AuthError('discovery-failed', scheme, cause=error) from None
    return _discovery_document(scheme, url, response)


async def _discover_async(scheme: str,
                          transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None) -> dict[str, Any]:
    """Async variant of :func:`_discover_sync` on the caller task."""
    descriptor = _scheme(scheme)
    url = descriptor.get('discovery')
    if type(url) is not str or not url:
        raise AuthError('unsupported', scheme)
    try:
        async with httpx.AsyncClient(trust_env=False, timeout=_TOKEN_TIMEOUT, transport=transport) as client:
            response = await client.get(url, headers={'Accept': 'application/json'})
    except (httpx.HTTPError, TypeError) as error:
        raise AuthError('discovery-failed', scheme, cause=error) from None
    return _discovery_document(scheme, url, response)


def _discovery_auth(session: _Session) -> str | None:
    """Client authentication for endpoints the discovery document supplies.

    ``client_secret_basic`` when the compiled configuration supplies a client
    secret variable — missing values raise the typed missing-credentials
    failure — else the public profile, which sends the client id in the form.
    """
    descriptor = session.descriptor
    if type(descriptor.get('client_secret_env')) is not str:
        return None
    client_id, client_secret = session.resolve()
    if type(client_id) is not str or not client_id or type(client_secret) is not str or not client_secret:
        raise AuthError('missing-client-credentials', session.scheme)
    return _basic(client_id, client_secret)
"##;
/// The replaying credential wrapper: one coordinated refresh plus one
/// eligible request replay per qualifying 401, opt-in per provider, and
/// never for stream-protected operations. The section emits only when a
/// compiled scheme carries an executable client-credentials flow; its plain
/// and discovery variants resolve the lifecycle-endpoint exclusion through
/// the same compiled precedence as the provider they wrap.
fn replay_section(
    plan: &p::OAuthPlan,
    no_replay: &BTreeMap<String, BTreeSet<String>>,
    discovery: bool,
) -> String {
    let mut code = String::from(
        "\n\n# Compiled stream-protected requirements: security-requirement source\n# pointers whose attaches are never replayed, because delivered stream data\n# prevents a transparent restart.\n_NO_REPLAY_REQUIREMENTS: dict[str, frozenset[str]] = {\n",
    );
    for scheme in &plan.schemes {
        let Some(pointers) = no_replay.get(&scheme.name) else {
            continue;
        };
        if pointers.is_empty() {
            continue;
        }
        let rendered = pointers
            .iter()
            .map(|pointer| sq(pointer))
            .collect::<Vec<_>>()
            .join(", ");
        code.push_str(&format!(
            "    {}: frozenset({{{}}}),\n",
            sq(&scheme.name),
            rendered
        ));
    }
    code.push_str("}\n");
    code.push_str(REPLAY_CREDENTIAL);
    code.push_str(if discovery {
        REPLAY_LIFECYCLE_DISCOVERY
    } else {
        REPLAY_LIFECYCLE_PLAIN
    });
    code.push_str(REPLAY_TRANSPORT);
    code.push_str(REPLAY_FACTORY);
    code
}

/// The replaying wrapper's shared half: the record of served attaches, the
/// coordinated refresh rounds and the replaying transport. The plain and
/// discovery variants append the lifecycle-endpoint exclusion.
const REPLAY_CREDENTIAL: &str = r##"

class _ReplayRound:
    """One coordinated refresh round: exactly one forced acquisition, shared
    by every concurrent 401 that presented the same stale token."""

    def __init__(self) -> None:
        self._done = threading.Event()
        self.outcome: tuple[str, Any] | None = None

    def complete(self, token: TokenSet) -> None:
        self.outcome = ('fresh', token)
        self._done.set()

    def fail(self, error: BaseException) -> None:
        self.outcome = ('failed', error)
        self._done.set()

    def wait(self) -> tuple[str, Any]:
        self._done.wait()
        if self.outcome is None:
            raise AuthError('replay-refresh', '')
        return self.outcome


class _ReplayingCredential:
    """One compiled scheme's client-credentials provider plus the unified 401
    replay policy.

    Attach behavior is exactly :class:`_ClientCredential`'s: single-flighted
    acquisition, skew-aware cache, atomic store replacement. On top of it the
    provider remembers which Authorization values its attaches served, and
    :meth:`replay_transport` wraps the client transport so a 401 (and only a
    401) on a request carrying this provider's token triggers exactly one
    coordinated refresh — concurrent 401s share one token request round — and
    exactly one replay of the request with the fresh token. The second
    response is surfaced whatever it is. The overall budget is one refresh
    plus one replay, never nested with other retry policies (requests are not
    retried today). Attaches for stream-protected requirements are never
    replayed, because delivered stream data prevents a transparent restart;
    a refresh failure surfaces as the typed :class:`AuthError` instead of a
    replay.
    """

    def __init__(self, scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                 store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                 transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                 scopes: Sequence[str] | None = None) -> None:
        self._store: TokenStore = store if store is not None else MemoryTokenStore()
        self._inner = _ClientCredential(scheme, client_id=client_id, client_secret=client_secret,
                                        store=self._store, clock=clock, transport=transport, scopes=scopes)
        self._descriptor = _scheme(scheme)
        self._never_replay = frozenset(_NO_REPLAY_REQUIREMENTS.get(scheme, ()))
        self._served: list[dict[str, Any]] = []
        self._record_lock = threading.Lock()
        self._refresh_lock = threading.Lock()
        self._async_lock: asyncio.Lock | None = None
        self._rounds: dict[tuple[str, str, str], _ReplayRound] = {}
        self._async_rounds: dict[tuple[str, str, str], asyncio.Future] = {}

    def __call__(self, request: object) -> Authorization:
        if _running_loop():
            return self._serve_async(request)  # type: ignore[return-value]
        return self._serve_sync(request)

    def _eligible(self, request: object) -> bool:
        if request is None:
            return True
        pointer = getattr(getattr(request, 'source', None), 'pointer', None)
        return type(pointer) is not str or pointer not in self._never_replay

    def _serve_sync(self, request: object) -> Authorization:
        authorization = self._inner(request)
        self._record(authorization.value, self._eligible(request), request)
        return authorization

    async def _serve_async(self, request: object) -> Authorization:
        authorization = await self._inner(request)
        self._record(authorization.value, self._eligible(request), request)
        return authorization

    def _record(self, value: str, eligible: bool, request: object) -> None:
        with self._record_lock:
            self._served.insert(0, {'value': value, 'eligible': eligible, 'request': request})
            del self._served[8:]

    def _served_entry(self, presented: str | None) -> dict[str, Any] | None:
        if type(presented) is not str or not presented:
            return None
        with self._record_lock:
            for entry in self._served:
                if entry['value'] == presented and entry['eligible']:
                    return entry
        return None

    def _refresh_sync(self, presented: str) -> TokenSet:
        """One coordinated refresh: a newer stored set wins over a stale
        re-refresh; concurrent 401s share one round; a failed round fails
        every waiter exactly once."""
        key = self._inner._session.key()
        while True:
            with self._refresh_lock:
                stored = self._store.load(key)
                if stored is not None and self._inner._authorization(stored).value != presented:
                    return self._inner._authorization(stored)
                round = self._rounds.get(key)
                if round is None:
                    round = _ReplayRound()
                    self._rounds[key] = round
                    leader = True
                else:
                    leader = False
            if leader:
                try:
                    self._store.clear(key)
                    fresh = self._inner(None)
                except BaseException as error:
                    round.fail(error)
                    with self._refresh_lock:
                        self._rounds.pop(key, None)
                    raise
                round.complete(fresh)
                with self._refresh_lock:
                    self._rounds.pop(key, None)
                return fresh
            outcome = round.wait()
            if outcome[0] == 'fresh':
                return outcome[1]
            raise outcome[1]

    async def _refresh_async(self, presented: str) -> TokenSet:
        if self._async_lock is None:
            self._async_lock = asyncio.Lock()
        while True:
            key = self._inner._session.key()
            async with self._async_lock:
                stored = self._store.load(key)
                if stored is not None and self._inner._authorization(stored).value != presented:
                    return self._inner._authorization(stored)
                future = self._async_rounds.get(key)
                if future is None:
                    future = asyncio.get_running_loop().create_future()
                    self._async_rounds[key] = future
                    leader = True
                else:
                    leader = False
            if leader:
                try:
                    self._store.clear(key)
                    fresh = await self._inner(None)
                except BaseException as error:
                    future.set_exception(error)
                    future.add_done_callback(lambda done: done.exception())
                    self._async_rounds.pop(key, None)
                    raise
                future.set_result(fresh)
                self._async_rounds.pop(key, None)
                return fresh
            return await future
"##;

/// The replaying transport: the 401 interception half of the wrapper, shared
/// by the plain and discovery variants.
const REPLAY_TRANSPORT: &str = r##"

class _ReplayTransport(httpx.BaseTransport):
    """The replaying credential's client transport: one coordinated refresh
    and, when the request carried the provider's token and no stream is
    protected on it, exactly one replay with the fresh token. Lifecycle
    endpoint requests are never replayed: they carry no bearer token of this
    provider, and the exact-target guard below is defense in depth."""

    def __init__(self, provider: '_ReplayingCredential', inner: httpx.BaseTransport | httpx.AsyncBaseTransport) -> None:
        self._provider = provider
        self._inner = inner

    def handle_request(self, request: httpx.Request) -> httpx.Response:
        response = self._inner.handle_request(request)
        if response.status_code != 401:
            return response
        entry = self._provider._served_entry(request.headers.get('authorization'))
        if entry is None or self._provider._lifecycle(request):
            return response
        fresh = self._provider._refresh_sync(entry['value'])
        try:
            request.read()
        except Exception:
            return response
        replayed = httpx.Request(request.method, request.url, headers=request.headers, content=request.content)
        replayed.headers['authorization'] = fresh.value
        return self._inner.handle_request(replayed)

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        response = await self._inner.handle_async_request(request)
        if response.status_code != 401:
            return response
        entry = self._provider._served_entry(request.headers.get('authorization'))
        if entry is None or self._provider._lifecycle(request):
            return response
        fresh = await self._provider._refresh_async(entry['value'])
        try:
            await request.aread()
        except Exception:
            return response
        replayed = httpx.Request(request.method, request.url, headers=request.headers, content=request.content)
        replayed.headers['authorization'] = fresh.value
        return await self._inner.handle_async_request(replayed)

    def close(self) -> None:
        closer = getattr(self._inner, 'close', None)
        if callable(closer):
            closer()

    async def aclose(self) -> None:
        closer = getattr(self._inner, 'aclose', None)
        if callable(closer):
            await closer()
"##;

/// The plain variant's lifecycle-endpoint exclusion: the compiled
/// client-credentials token URL.
const REPLAY_LIFECYCLE_PLAIN: &str = r##"
    def _lifecycle(self, request: httpx.Request) -> bool:
        flow = self._descriptor['flows'].get('client-credentials')
        return flow is not None and str(request.url) == flow.get('token_url')
"##;

/// The discovery variant's lifecycle-endpoint exclusion: the compiled
/// discovery URL; the resolved token endpoint rides the exact-token match.
const REPLAY_LIFECYCLE_DISCOVERY: &str = r##"
    def _lifecycle(self, request: httpx.Request) -> bool:
        discovery = self._descriptor.get('discovery')
        return type(discovery) is str and str(request.url) == discovery
"##;

/// The opt-in factory, mirroring :func:`client_credential`'s signature.
const REPLAY_FACTORY: &str = r##"

def replaying_credential(scheme: str, *, client_id: str | None = None, client_secret: str | None = None,
                         store: TokenStore | None = None, clock: Callable[[], float] | None = None,
                         transport: httpx.BaseTransport | httpx.AsyncBaseTransport | None = None,
                         scopes: Sequence[str] | None = None) -> Credential:
    """A generated ``Credential`` with the replaying 401 policy.

    Attach it under the scheme's source name and pass the transport wrapper
    from ``credentials.replay_transport`` to the client::

        credentials = replaying_credential('serviceOAuth')
        client = Client(auth={'serviceOAuth': credentials},
                        transport=credentials.replay_transport(my_transport))

    Every provider argument behaves exactly as in :func:`client_credential`;
    the replay semantics are strictly additive and the plain provider keeps
    today's attach-only semantics. See :class:`_ReplayingCredential` for the
    one-refresh-one-replay contract.
    """
    return _ReplayingCredential(scheme, client_id=client_id, client_secret=client_secret,
                                store=store, clock=clock, transport=transport, scopes=scopes)


def _replay_transport(self: '_ReplayingCredential',
                      inner: httpx.BaseTransport | httpx.AsyncBaseTransport) -> '_ReplayTransport':
    """Wrap ``inner`` with the one-refresh-one-replay 401 policy; call once
    per client. Token requests keep traveling through ``inner`` directly."""
    return _ReplayTransport(self, inner)


_ReplayingCredential.replay_transport = _replay_transport
"##;
