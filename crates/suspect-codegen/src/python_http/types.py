"""Public HTTP values and classified, value-redacted failures."""
from __future__ import annotations

import dataclasses
from collections.abc import Awaitable, Callable, Mapping
from typing import Generic, Literal, TypeAlias, TypeVar
from .json_runtime import JsonValue
from .models import UNSET, Unset

T = TypeVar('T')
RawHeaders: TypeAlias = tuple[tuple[str, str], ...]


@dataclasses.dataclass(frozen=True)
class Source:
    document: str
    pointer: str


class SdkError(Exception):
    def __init__(self, kind: str, source: Source, *, status: int | None = None,
                 headers: RawHeaders = (), capture: bytes = b'', truncated: bool = False,
                 cause: BaseException | None = None, code: str | None = None) -> None:
        super().__init__('SDK ' + kind + ' failure')
        self.kind, self.source, self.status = kind, source, status
        self.headers, self.capture, self.truncated, self.cause = headers, capture, truncated, cause
        self.code = code

    def __repr__(self) -> str:
        return f'SdkError(kind={self.kind!r}, status={self.status!r})'


@dataclasses.dataclass(frozen=True, kw_only=True)
class Link:
    """Source link metadata. No expression evaluation or implicit invocation."""
    name: str
    source: Source
    target_source: Source
    operation_id: str | None
    operation_ref: str | None
    parameters: Mapping[str, JsonValue]
    request_body: JsonValue | Unset = UNSET
    description: str | None = None
    server: Mapping[str, JsonValue] | None = None


class ApiError(Exception, Generic[T]):
    def __init__(self, *, status: int, data: T, headers: RawHeaders, source: Source,
                 content_type: str | None = None, typed_headers: object = None,
                 links: tuple[Link, ...] = ()) -> None:
        super().__init__(f'Declared HTTP {status} API failure')
        self.status, self.data, self.headers, self.source = status, data, headers, source
        self.content_type, self.typed_headers, self.links = content_type, typed_headers, links

    def __repr__(self) -> str:
        return f'{type(self).__name__}(status={self.status})'


@dataclasses.dataclass(frozen=True, kw_only=True)
class BasicAuth:
    """Explicit Basic credentials. Charset is caller policy, defaulting to UTF-8."""
    username: str = dataclasses.field(repr=False)
    password: str = dataclasses.field(repr=False)
    encoding: Literal['utf-8', 'latin-1'] = 'utf-8'


@dataclasses.dataclass(frozen=True)
class Authorization:
    """A complete Authorization field supplied for OAuth/OIDC, including its scheme."""
    value: str = dataclasses.field(repr=False)


@dataclasses.dataclass(frozen=True, kw_only=True)
class OAuthFlow:
    kind: str
    source: Source
    authorization_url: str | None
    token_url: str | None
    refresh_url: str | None
    device_authorization_url: str | None
    scopes: Mapping[str, str]


@dataclasses.dataclass(frozen=True, kw_only=True)
class CredentialRequest:
    """Located attachment metadata and effective HTTP server for literal endpoint URLs."""
    name: str
    operation: Source
    source: Source
    scheme_source: Source
    kind: str
    permissions_kind: Literal['scopes', 'roles']
    permissions: tuple[str, ...]
    flows: tuple[OAuthFlow, ...] = ()
    metadata_url: str | None = None
    discovery_url: str | None = None
    effective_server_url: str | None = None


AuthValue: TypeAlias = str | BasicAuth | Authorization
CredentialProvider: TypeAlias = Callable[[CredentialRequest], AuthValue | None | Awaitable[AuthValue | None]]
Credential: TypeAlias = AuthValue | CredentialProvider


@dataclasses.dataclass(frozen=True, kw_only=True)
class Part(Generic[T]):
    """One finite part value. Generated subclasses supply typed declared headers."""
    value: T
    content_type: str | None = None
    filename: str | None = None
    headers: object = None
    extra_headers: RawHeaders = ()
