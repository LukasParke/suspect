"""Bounded interpretation of admitted parameter and header descriptors."""
from __future__ import annotations

from collections.abc import Mapping
from typing import Any, cast
import re
from . import json_runtime as J
from ._types import Source, SdkError

TOKEN = re.compile(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", re.ASCII)


def location(value: Mapping[str, Any]) -> Source:
    if 'source' in value:
        return location(value['source'])
    if 'terminal' in value:
        return location(value['terminal'])
    return Source(value['document'], value['pointer'])


class Budget:
    def __init__(self, maximum: int, source: Source) -> None:
        self.maximum, self.source, self.used = maximum, source, 0

    def charge(self, amount: int) -> None:
        if amount < 0 or amount > self.maximum - self.used:
            raise SdkError('resource-limit', self.source)
        self.used += amount


def text_bytes(value: str, maximum: int, source: Source) -> bytes:
    if type(value) is not str or len(value) > maximum:
        raise SdkError('resource-limit' if type(value) is str else 'request-representation', source)
    width = 0
    for char in value:
        code = ord(char)
        if 0xD800 <= code <= 0xDFFF:
            raise SdkError('request-representation', source, code='http-wire-unicode')
        width += 1 if code < 128 else 2 if code < 2048 else 3 if code < 65536 else 4
        if width > maximum:
            raise SdkError('resource-limit', source)
    return value.encode('utf-8')


def scalar(value: J.JsonValue, kind: str | None, source: Source) -> str:
    if type(value) is str and kind in (None, 'string'):
        return value
    if type(value) is bool and kind in (None, 'boolean'):
        return 'true' if value else 'false'
    if type(value) in (int, J.JsonNumber) and kind in (None, 'integer', 'number'):
        token = value.token if isinstance(value, J.JsonNumber) else J.stringify_json(value)
        if kind != 'integer' or J.JsonNumber(token).is_integer():
            return token
    raise SdkError('request-representation', source, code='http-wire-value')


def parse_scalar(value: str, kind: str, source: Source) -> J.JsonValue:
    if kind == 'string':
        return value
    parsed = J.parse_json(value)
    scalar(parsed, kind, source)
    return parsed


def canonical(value: J.JsonValue) -> J.JsonValue:
    # Values have already crossed a finite source-codec JSON boundary. Preserve
    # numeric tokens while applying the protocol's deterministic object order.
    if type(value) is dict:
        return {key: canonical(item) for key, item in sorted(value.items())}
    if type(value) is list:
        return [canonical(item) for item in value]
    return value


def percent(value: str, mode: str, maximum: int, source: Source) -> str:
    data = text_bytes(value, maximum, source)
    if mode == 'none':
        return value
    output: list[str] = []
    width, at = 0, 0
    while at < len(data):
        byte = data[at]
        if mode == 'reserved-expansion' and byte == 37 and at + 2 < len(data) and all(chr(b) in '0123456789abcdefABCDEF' for b in data[at + 1:at + 3]):
            piece = data[at:at + 3].decode('ascii')
            at += 3
        else:
            safe = b'*-._' if mode == 'form-url-encoded' else b'-._~'
            if 48 <= byte <= 57 or 65 <= byte <= 90 or 97 <= byte <= 122 or byte in safe or mode == 'reserved-expansion' and byte in b":/?#[]@!$&'()*+,;=":
                piece = chr(byte)
            elif byte == 32 and mode == 'form-url-encoded':
                piece = '+'
            else:
                piece = f'%{byte:02X}'
            at += 1
        width += len(piece)
        if width > maximum:
            raise SdkError('resource-limit', source)
        output.append(piece)
    return ''.join(output)


def header_value(value: str, source: Source) -> str:
    if type(value) is not str or any(ord(c) < 32 and c != '\t' or ord(c) == 127 for c in value):
        raise SdkError('request-representation', source, code='http-wire-control')
    try:
        value.encode('ascii')
    except UnicodeEncodeError as error:
        raise SdkError('request-representation', source, cause=error, code='http-header-charset') from None
    return value


def cookie_value(value: str, source: Source) -> str:
    header_value(value, source)
    if any(c in ' \t",;\\' for c in value):
        raise SdkError('request-representation', source, code='http-cookie-escaping')
    return value


def serialize(name: str, where: str, descriptor: Mapping[str, Any], value: J.JsonValue,
              maximum: int, source: Source, *, optional: bool = False) -> str | None:
    style = descriptor.get('style')
    shape = descriptor.get('shape', {})
    mode = descriptor['percent_encoding']

    def encode(value: str) -> str:
        if mode == 'none':
            header_value(value, source)
            if where == 'cookie':
                cookie_value(value, source)
        if style == 'spaceDelimited' and ' ' in value or style == 'pipeDelimited' and '|' in value or style == 'deepObject' and any(c in '[]' for c in value):
            raise SdkError('request-representation', source, code='http-delimiter-escaping')
        if mode == 'reserved-expansion':
            hazards = {'path': '#[]/?', 'query': '#[]&=+', 'cookie': ';,'}.get(where, '')
            if any(c in value for c in hazards):
                raise SdkError('request-representation', source, code='http-reserved-value-escaping')
            if shape.get('kind') != 'scalar':
                separators = {'simple': ',', 'form': ',', 'cookie': ',', 'label': '.,', 'matrix': ';,'}.get(style or '', '')
                if any(c in value for c in separators):
                    raise SdkError('request-representation', source, code='http-reserved-value-escaping')
        return percent(value, mode, maximum, source)

    if descriptor['kind'] == 'content':
        media = descriptor['media_type']['range']
        is_json = media.get('subtype') == 'json' or media.get('subtype', '').endswith('+json')
        wire = encode(J.stringify_json(canonical(value)) if is_json else scalar(value, None, source))
        if where == 'querystring':
            return wire
        return percent(name, 'uri-component', maximum, source) + '=' + wire if where in ('query', 'cookie') else wire
    if type(value) in (list, dict) and not value:
        if optional:
            return None
        raise SdkError('request-representation', source, code='http-empty-composite')
    label = percent(name, 'none' if where == 'header' or style == 'cookie' else 'uri-component', maximum, source)
    simple: str | None = None
    items: list[str] = []
    properties: list[tuple[str, str]] = []
    if shape['kind'] == 'scalar':
        simple = encode(scalar(value, shape['scalar'], source))
    elif shape['kind'] == 'array' and type(value) is list:
        items = [encode(scalar(v, shape['items'], source)) for v in value]
    elif shape['kind'] == 'flat-object' and type(value) is dict:
        for key, item in sorted(value.items()):
            kind = shape['properties'].get(key)
            if kind is None:
                additional = shape['additional']
                if additional['kind'] == 'forbidden':
                    raise SdkError('request-representation', source, code='http-wire-value')
                kind = additional.get('scalar')
            properties.append((encode(key), encode(scalar(item, kind, source))))
    else:
        raise SdkError('request-representation', source, code='http-wire-value')
    explode = descriptor['explode']
    def flat(delimiter: str) -> str:
        return delimiter.join(x for pair in properties for x in pair)
    def pairs(delimiter: str) -> str:
        return delimiter.join(key + '=' + item for key, item in properties)
    array = shape['kind'] == 'array'
    if style == 'simple':
        result = simple if simple is not None else ','.join(items) if array else pairs(',') if explode else flat(',')
    elif style == 'label':
        result = '.' + (simple if simple is not None else ('.' if explode else ',').join(items) if array else pairs('.') if explode else flat(','))
    elif style == 'matrix':
        def named(key: str, item: str) -> str:
            return ';' + key + ('=' + item if item else '')
        if simple is not None:
            result = named(label, simple)
        elif array:
            result = ''.join(named(label, item) for item in items) if explode else ';' + label + '=' + ','.join(items)
        else:
            result = ''.join(named(key, item) for key, item in properties) if explode else ';' + label + '=' + flat(',')
    elif style in ('form', 'cookie'):
        delimiter = '; ' if style == 'cookie' else '&'
        result = label + '=' + simple if simple is not None else delimiter.join(label + '=' + item for item in items) if array and explode else label + '=' + ','.join(items) if array else pairs(delimiter) if explode else label + '=' + flat(',')
    elif style in ('spaceDelimited', 'pipeDelimited'):
        delimiter = '%20' if style == 'spaceDelimited' else '%7C'
        result = label + '=' + (delimiter.join(items) if array else flat(delimiter))
    elif style == 'deepObject':
        result = '&'.join(label + '%5B' + key + '%5D=' + item for key, item in properties)
    else:
        raise SdkError('request-representation', source, code='http-wire-descriptor')
    if len(result) > maximum:
        raise SdkError('resource-limit', source)
    return result


def parse_header(values: list[str], descriptor: Mapping[str, Any], source: Source) -> J.JsonValue:
    """The admitted simple/content header grammar; no CSV quoting is inferred."""
    serialization = descriptor['serialization']
    if descriptor['name'].lower() == 'set-cookie' and len(values) != 1:
        raise SdkError('response-decoding', source, code='http-set-cookie-repetition')
    text = ','.join(values)
    if serialization['kind'] == 'content':
        media = serialization['media_type']['range']
        if media.get('subtype') == 'json' or media.get('subtype', '').endswith('+json'):
            return J.parse_json(text)
        return parse_scalar(text, descriptor.get('text_scalar', 'string'), source)
    shape = serialization['shape']
    if shape['kind'] == 'scalar':
        if len(values) != 1:
            raise SdkError('response-decoding', source, code='http-header-repetition')
        return parse_scalar(text, shape['scalar'], source)
    parts = [part.strip(' \t') for part in text.split(',')]
    if shape['kind'] == 'array':
        return [parse_scalar(part, shape['items'], source) for part in parts]
    result: dict[str, J.JsonValue] = {}
    if serialization['explode']:
        pairs = [part.partition('=') for part in parts]
        if any(not sep for _, sep, _ in pairs):
            raise SdkError('response-decoding', source, code='http-header-object')
        entries = [(key, value) for key, _, value in pairs]
    else:
        if len(parts) % 2:
            raise SdkError('response-decoding', source, code='http-header-object')
        entries = list(zip(parts[::2], parts[1::2]))
    for key, value in entries:
        if key in result:
            raise SdkError('response-decoding', source, code='http-header-duplicate-key')
        kind = shape['properties'].get(key)
        if kind is None:
            if shape['additional']['kind'] != 'typed':
                raise SdkError('response-decoding', source, code='http-header-ambiguous-extra')
            kind = shape['additional']['scalar']
        result[key] = parse_scalar(value, kind, source)
    return result
