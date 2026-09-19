"""HTTP media/status precedence over trusted, source-bound descriptors."""
from __future__ import annotations
from collections.abc import Mapping, Sequence
from typing import Any
from ._types import Source, SdkError
from ._wire import TOKEN


def split_quoted(value: str, delimiter: str) -> list[str]:
    parts: list[str] = []
    start, quoted, escaped = 0, False, False
    for index, char in enumerate(value):
        if escaped:
            escaped = False
        elif quoted and char == '\\':
            escaped = True
        elif char == '"':
            quoted = not quoted
        elif not quoted and char == delimiter:
            parts.append(value[start:index])
            start = index + 1
    if quoted or escaped:
        raise ValueError('unterminated quoted field')
    parts.append(value[start:])
    return parts


def parameters(parts: Sequence[str]) -> dict[str, str]:
    result: dict[str, str] = {}
    for part in parts:
        key, separator, raw = part.strip(' \t').partition('=')
        if not separator or TOKEN.fullmatch(key) is None or key.lower() in result:
            raise ValueError('invalid or duplicate field parameter')
        if raw.startswith('"'):
            if len(raw) < 2 or not raw.endswith('"'):
                raise ValueError('unterminated parameter')
            chars: list[str] = []
            escaped = False
            for char in raw[1:-1]:
                if escaped:
                    chars.append(char)
                    escaped = False
                elif char == '\\':
                    escaped = True
                elif char == '"':
                    raise ValueError('unexpected quote')
                else:
                    chars.append(char)
            if escaped:
                raise ValueError('unterminated quoted pair')
            value = ''.join(chars)
        else:
            if TOKEN.fullmatch(raw) is None:
                raise ValueError('invalid parameter value')
            value = raw
        result[key.lower()] = value
    return result


def parse(value: str) -> tuple[str, str, dict[str, str]]:
    if type(value) is not str or any(ord(c) < 32 and c != '\t' or ord(c) == 127 for c in value):
        raise ValueError('invalid media controls')
    parts = split_quoted(value, ';')
    major, separator, minor = parts[0].strip(' \t').partition('/')
    if not separator or TOKEN.fullmatch(major) is None or TOKEN.fullmatch(minor) is None or '*' in major + minor:
        raise ValueError('concrete media type required')
    return major.lower(), minor.lower(), parameters(parts[1:])


def matches(declared: Mapping[str, Any], actual: tuple[str, str, dict[str, str]]) -> bool:
    major, minor, params = actual
    kind = declared['range']
    if kind['kind'] == 'type' and kind['type_name'] != major:
        return False
    if kind['kind'] == 'concrete' and (kind['type_name'] != major or kind['subtype'] != minor):
        return False
    for name, value in declared['parameters'].items():
        other = params.get(name)
        if other is None or (other.lower() != value.lower() if name == 'charset' else other != value):
            return False
    return True


def select(media: Sequence[dict[str, Any]], content_type: str, source: Source) -> tuple[int, dict[str, Any]]:
    try:
        actual = parse(content_type)
    except ValueError as error:
        raise SdkError('request-representation', source, cause=error, code='http-media-type-invalid') from None
    eligible = [(index, entry) for index, entry in enumerate(media) if matches(entry['media_type'], actual)]
    if not eligible:
        raise SdkError('request-representation', source, code='http-media-type-unmatched')
    index, selected = max(eligible, key=lambda pair: ({'concrete': 2, 'type': 1, 'any': 0}[pair[1]['media_type']['range']['kind']], len(pair[1]['media_type']['parameters'])))
    if selected['representation']['kind'] in ('text', 'stream', 'form') and actual[2].get('charset', 'utf-8').lower() != 'utf-8':
        raise SdkError('request-representation', source, code='http-unsupported-charset')
    return index, selected


def response(operation: Mapping[str, Any], status: int, content_types: Sequence[str], source: Source) -> tuple[int, int | None, dict[str, Any] | None]:
    if type(status) is not int or not 100 <= status < 600:
        raise SdkError('unexpected-response', source, code='http-invalid-status')
    choices: list[tuple[int, int, dict[str, Any]]] = []
    for index, candidate in enumerate(operation['responses']):
        selector = candidate['status']
        kind = selector['kind']
        if kind == 'exact' and selector['value'] == status:
            choices.append((3, index, candidate))
        elif kind == 'range' and selector['value'] == status // 100:
            choices.append((2, index, candidate))
        elif kind == 'default':
            choices.append((1, index, candidate))
    if not choices:
        raise SdkError('unexpected-response', source, code='http-undeclared-status')
    _, index, selected = max(choices, key=lambda choice: choice[0])
    if forbidden(operation['method'], status) or not selected['media']:
        return index, None, None
    if len(content_types) != 1:
        raise SdkError('unexpected-response', source, code='http-content-type-required')
    try:
        media_index, media = select(selected['media'], content_types[0], source)
    except SdkError as error:
        raise SdkError('unexpected-response', source, code=error.code, cause=error) from None
    return index, media_index, media


def forbidden(method: str, status: int) -> bool:
    return method == 'HEAD' or 100 <= status < 200 or status in (204, 205, 304)
