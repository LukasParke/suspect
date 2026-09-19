"""RFC 3986 server resolution over physical document URLs and literal paths."""
from __future__ import annotations

import re
from urllib.parse import SplitResult, urlsplit, urlunsplit


def _parts(value: str, *, document: bool = False) -> SplitResult:
    if type(value) is not str or not value or any(char.isspace() or ord(char) < 32 or ord(char) == 127 for char in value):
        raise ValueError('invalid HTTP URL reference')
    if any(char in value for char in ('\\{}' if document else '\\{}?#')) or re.search(r'%(?![0-9a-fA-F]{2})', value):
        raise ValueError('invalid HTTP URL reference')
    result = urlsplit(value)
    if result.username is not None or result.password is not None:
        raise ValueError('server userinfo is not a credential hook')
    if result.netloc:
        if result.hostname is None:
            raise ValueError('server requires a hostname')
        _ = result.port
    elif value.startswith('//') or result.scheme:
        raise ValueError('HTTP URL requires an authority')
    return result


def _remove_dots(path: str) -> str:
    # Resolution creates an absolute path when there is an HTTP authority.
    # Empty segments and percent-encoded dots/slashes remain literal data.
    result: list[str] = []
    segments = path.split('/')
    for index, segment in enumerate(segments):
        if segment == '..':
            if len(result) > 1:
                result.pop()
        elif segment != '.':
            result.append(segment)
            continue
        if index == len(segments) - 1:
            result.append('')
    return '/'.join(result)


def resolve_server(reference: str, document_url: str | None) -> str:
    """Resolve one expanded server, preserving encoded and empty path segments."""
    relative = _parts(reference)
    if relative.scheme:
        scheme, authority, path = relative.scheme, relative.netloc, relative.path
    else:
        if document_url is None:
            raise ValueError('relative server needs an HTTP document URL')
        base = _parts(document_url, document=True)
        if base.scheme not in ('http', 'https') or not base.netloc:
            raise ValueError('relative server needs an HTTP document URL')
        scheme = base.scheme
        authority = relative.netloc or base.netloc
        if relative.netloc or relative.path.startswith('/'):
            path = relative.path
        elif not relative.path:
            path = base.path
        else:
            path = (base.path.rsplit('/', 1)[0] if '/' in base.path else '') + '/' + relative.path
    if scheme not in ('http', 'https') or not authority:
        raise ValueError('server requires an absolute HTTP URL')
    return urlunsplit((scheme, authority, _remove_dots(path), '', ''))
