//! The pinned generated runtime source: custom sync/async transports over
//! httpx 0.28.1, `UNSET` sentinel, bounded URL building and redacted errors.

/// Emitted verbatim as `<import_name>/_runtime.py`.
pub(super) const RUNTIME_PY: &str = r#"""Generated runtime: transport adapter, UNSET sentinel, wire helpers.

Transport is a custom Protocol so tests can inject an independent loopback
implementation without patching httpx. The default adapter pins httpx
0.28.1 behaviour: trust_env=False, follow_redirects=False, no automatic
decompression of raw captures.
"""
from __future__ import annotations

import typing as t
from urllib.parse import quote

import httpx

__all__ = [
    "UNSET", "Unset", "ApiFailureBase", "AsyncTransportProtocol",
    "DefaultAsyncTransport", "DefaultSyncTransport", "DecodeError",
    "SyncTransportProtocol", "TransportError", "build_url",
    "content_type_is_json",
]


class Unset:
    """Distinct optional-argument sentinel; never None and never a value."""

    __slots__ = ()

    def __repr__(self) -> str:  # pragma: no cover - debug only
        return "UNSET"

    def __bool__(self) -> bool:
        raise TypeError("UNSET has no truthiness; test identity with `is`")


UNSET = Unset()


class TransportError(Exception):
    """Transport, limit or undeclared-status failure; never carries a body."""

    def __init__(self, message: str, status: int | None) -> None:
        super().__init__(message)
        self.status = status


class DecodeError(Exception):
    """Exact codec failure; locates the response that failed to decode."""


class ApiFailureBase:
    """Declared 4xx/5xx response base; data and raw bytes are per-subclass."""

    __slots__ = ("status", "headers", "data", "raw")

    def __init__(self, status: int, headers: t.Sequence[t.Tuple[str, str]], data: t.Any, raw: bytes) -> None:
        self.status = status
        self.headers = tuple(headers)
        self.data = data
        self.raw = raw


class SyncTransportProtocol(t.Protocol):
    def handle(self, request: httpx.Request) -> httpx.Response: ...


class AsyncTransportProtocol(t.Protocol):
    async def handle_async(self, request: httpx.Request) -> httpx.Response: ...


class DefaultSyncTransport:
    """httpx adapter pinned to trust_env=False, follow_redirects=False."""

    __slots__ = ("_client",)

    def __init__(self) -> None:
        self._client = httpx.Client(trust_env=False, follow_redirects=False)

    def handle(self, request: httpx.Request) -> httpx.Response:
        return self._client.send(request, stream=True)


class DefaultAsyncTransport:
    """httpx async adapter; awaits only on the caller's task."""

    __slots__ = ("_client",)

    def __init__(self) -> None:
        self._client = httpx.AsyncClient(trust_env=False, follow_redirects=False)

    async def handle_async(self, request: httpx.Request) -> httpx.Response:
        return await self._client.send(request, stream=True)


def content_type_is_json(headers: t.Sequence[t.Tuple[str, str]]) -> bool:
    """Exact media-type match: `application/json` (optionally + parameters)."""
    for name, value in headers:
        if name.lower() == "content-type":
            media = value.split(";", 1)[0].strip().lower()
            return media == "application/json"
    return False


def build_url(
    base_url: str,
    path: str,
    path_args: t.Mapping[str, str],
    query: t.Mapping[str, t.Any],
) -> str:
    """Assemble base + templated path + RFC 3986 form query without urljoin.

    Path arguments are percent-encoded byte-wise so placeholders never
    change route structure; scalars and arrays follow the admitted form
    style. `base_url` is used verbatim up to the path.
    """
    resolved = path
    for name, value in path_args.items():
        encoded = quote(value.encode("utf-8"), safe=b"-._~".decode())
        resolved = resolved.replace("{" + name + "}", encoded)
    if "{" in resolved or "}" in resolved:
        raise TransportError("unfilled path placeholder", None)
    url = base_url.rstrip("/") + resolved
    parts = []
    for key, value in query.items():
        if isinstance(value, list):
            parts.extend(f"{quote_key(key)}={quote_val(v)}" for v in value)
        else:
            parts.append(f"{quote_key(key)}={quote_val(value)}")
    if parts:
        url += "?" + "&".join(parts)
    if len(url.encode("utf-8")) > 8 * 1024 * 1024:
        raise TransportError("assembled URL exceeded MAX_REQUEST_BYTES", None)
    return url


def quote_key(key: str) -> str:
    return quote(key.encode("utf-8"), safe=b"-._~".decode())


def quote_val(value: t.Any) -> str:
    if value is True:
        text = "true"
    elif value is False:
        text = "false"
    elif value is None:
        text = "null"
    else:
        text = str(value)
    return quote(text.encode("utf-8"), safe=b"-._~".decode())
"#;
