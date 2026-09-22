"""Finite form/part codecs; byte values never enter a JSON aggregate codec."""
from __future__ import annotations

import secrets
from collections.abc import Mapping
from typing import Any, cast
from urllib.parse import parse_qsl
from . import json_runtime as J
from .models import UNSET, Unset
from ._types import Part, RawHeaders, Source, SdkError
from . import _media as media, _registry as registry
from ._wire import Budget, TOKEN, header_value, location, parse_header, parse_scalar, percent, scalar, serialize, text_bytes


def count_bound(value: int, lower: dict[str, Any] | None, upper: dict[str, Any] | None, source: Source) -> None:
    if lower is not None and value < lower['value'] or upper is not None and value > upper['value']:
        raise SdkError('request-validation', source, code='http-structural-cardinality')


def structure(names: set[str], rules: dict[str, Any], source: Source) -> None:
    if any(field['value'] not in names for field in rules['required']):
        raise SdkError('request-validation', source, code='http-required-part')
    count_bound(len(names), rules['min_properties'], rules['max_properties'], source)


def encode_headers(group_name: str | None, owner: object, maximum: int, source: Source) -> RawHeaders:
    if group_name is None:
        if owner is not None:
            raise SdkError('request-representation', source, code='http-undeclared-header-group')
        return ()
    group = registry.plan()['groups'][group_name]
    if owner is not None and type(owner) is not registry.native_class(group_name):
        raise SdkError('request-representation', source, code='http-header-group-type')
    result: list[tuple[str, str]] = []
    budget = Budget(maximum, source)
    for header in group['headers']:
        descriptor = header['protocol']
        value = getattr(owner, header['member']) if owner is not None else UNSET
        at = location(descriptor['source'])
        if isinstance(value, Unset):
            if descriptor['required']:
                raise SdkError('request-validation', at, code='http-required-header')
            continue
        wire = registry.codec(descriptor['codec']).encode_value(value)
        encoded = serialize(descriptor['name'], 'header', descriptor['serialization'], wire, maximum, at, optional=not descriptor['required'])
        if encoded is not None:
            header_value(encoded, at)
            budget.charge(len(descriptor['name']) + len(encoded) + 4)
            result.append((descriptor['name'], encoded))
    return tuple(result)


def decode_headers(group_name: str | None, headers: RawHeaders, source: Source) -> object:
    if group_name is None:
        return None
    group = registry.plan()['groups'][group_name]
    fields: dict[str, object] = {}
    for header in group['headers']:
        descriptor = header['protocol']
        values = [value for name, value in headers if name.lower() == descriptor['name'].lower()]
        if not values:
            if descriptor['required']:
                raise SdkError('response-decoding', location(descriptor['source']), code='http-required-header')
            continue
        descriptor = dict(descriptor, text_scalar=header.get('textScalar', 'string'))
        wire = parse_header(values, descriptor, location(descriptor['source']))
        fields[header['member']] = registry.codec(descriptor['codec']).decode_value(wire)
    return registry.native_class(group_name)(**fields)


def group_values(name: str, body: object, source: Source) -> dict[str, object]:
    group = registry.plan()['groups'][name]
    if type(body) is not registry.native_class(name):
        raise SdkError('request-representation', source, code='http-part-body-type')
    result: dict[str, object] = {}
    known = set()
    for field in group['fields']:
        known.add(field['wire'])
        value = getattr(body, field['member'])
        if not isinstance(value, Unset):
            result[field['wire']] = value
    if group['additional'] is not None:
        extra = getattr(body, '_extra_fields')
        if type(extra) is not dict:
            raise SdkError('request-representation', source, code='http-part-extras-type')
        for key, value in extra.items():
            if type(key) is not str or key in known:
                raise SdkError('request-representation', source, code='http-part-extra-name')
            result[key] = value
    return result


def part_entries(values: Mapping[str, object], descriptor: dict[str, Any], maximum: int, source: Source) -> list[tuple[str, object, dict[str, Any]]]:
    definitions = {part['name']: part for part in descriptor.get('fields', descriptor.get('parts', []))}
    additional = descriptor['additional']
    result: list[tuple[str, object, dict[str, Any]]] = []
    present = set()
    for name, value in values.items():
        if type(name) is not str:
            raise SdkError('request-representation', source, code='http-part-name')
        part = definitions.get(name)
        if part is None:
            if additional['kind'] != 'allowed':
                raise SdkError('request-validation', source, code='http-undeclared-part')
            part = additional['part']
        at = location(part['source'])
        items = [value]
        if part['multiplicity'] == 'repeated-array-items':
            if type(value) is not list:
                raise SdkError('request-representation', at, code='http-repeated-part-type')
            items = cast(list[object], value)
            count_bound(len(items), part['min_items'], part['max_items'], at)
            if not items and part['required']:
                raise SdkError('request-representation', at, code='http-empty-required-parts')
        if items:
            present.add(name)
        if len(items) > maximum - len(result):
            raise SdkError('resource-limit', at, code='http-part-count-limit')
        result.extend((name, item, part) for item in items)
    structure(present, descriptor['rules'], source)
    return result


def part_content(value: object, part: dict[str, Any], maximum: int, source: Source, *, wire_value: bool = False) -> bytes:
    representation = part['representation']
    kind = representation['kind']
    if kind == 'binary':
        maximum = min(maximum, representation['bytes']['max_bytes'])
        if type(value) is not bytes:
            raise SdkError('request-representation', source, code='http-byte-part-required')
        data = value
        if len(data) > maximum:
            raise SdkError('resource-limit', source, code='http-part-byte-limit')
        return data
    wire = cast(J.JsonValue, value) if wire_value else registry.codec(representation['codec']).encode_value(value)
    if kind == 'json':
        text = J.stringify_json(wire)
    elif kind == 'text':
        text = scalar(wire, representation['scalar'], source)
    else:
        styled = serialize(part['name'] or '', 'query', representation['serialization'], wire, maximum, source)
        if styled is None:
            raise SdkError('request-representation', source, code='http-empty-part')
        text = styled
    return text_bytes(text, maximum, source)


def select_part(part: dict[str, Any], selected: str | None, source: Source) -> str | None:
    choices = part['content_types']
    if not choices:
        if selected is not None:
            raise SdkError('request-representation', source, code='http-style-part-content-type')
        return None
    if selected is None:
        if len(choices) != 1 or choices[0]['range']['kind'] != 'concrete':
            raise SdkError('request-representation', source, code='http-part-content-type-required')
        selected = choices[0]['declared']
    try:
        actual = media.parse(selected)
    except ValueError as error:
        raise SdkError('request-representation', source, code='http-part-content-type', cause=error) from None
    if not any(media.matches(choice, actual) for choice in choices):
        raise SdkError('request-representation', source, code='http-part-content-type')
    if part['representation']['kind'] == 'text' and actual[2].get('charset', 'utf-8').lower() != 'utf-8':
        raise SdkError('request-representation', source, code='http-unsupported-charset')
    return selected


def encode_form(values: Mapping[str, object], descriptor: dict[str, Any], maximum: int, max_parts: int, source: Source, *, wire_values: bool = False) -> bytes:
    output: list[str] = []
    budget = Budget(maximum, source)
    for name, value, part in part_entries(values, descriptor, max_parts, source):
        at = location(part['source'])
        if isinstance(value, Part):
            if value.filename is not None or value.headers is not None or value.extra_headers:
                raise SdkError('request-representation', at, code='http-form-part-metadata')
            select_part(part, value.content_type, at)
            value = value.value
        # RFC6570 style is already the complete field expansion. Content-based
        # form values get exactly one form-urlencoded encoding pass.
        content = part_content(value, part, maximum - budget.used, at, wire_value=wire_values).decode('utf-8')
        if part['representation']['kind'] == 'style':
            field = content
        else:
            field = percent(name, 'form-url-encoded', maximum, at) + '=' + percent(content, 'form-url-encoded', maximum, at)
        budget.charge(len(field) + (1 if output else 0))
        output.append(field)
    return '&'.join(output).encode('ascii')


def disposition_text(value: str, maximum: int, source: Source) -> bytes:
    if any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise SdkError('request-representation', source, code='http-disposition-control')
    return text_bytes(value.replace('\\', '\\\\').replace('"', '\\"'), maximum, source)


def part_group(group_name: str, wire_name: str) -> dict[str, Any] | None:
    group = registry.plan()['groups'][group_name]
    field = next((field for field in group['fields'] if field['wire'] == wire_name), None)
    wrapper = field['partClass'] if field is not None else group.get('additionalPartClass')
    return registry.plan()['groups'][wrapper] if wrapper is not None else None


def encode_multipart(group_name: str, values: Mapping[str, object], descriptor: dict[str, Any], selected: str,
                     maximum: int, part_maximum: int, max_parts: int, source: Source) -> tuple[bytes, str]:
    entries: list[tuple[bytes, bytes]] = []
    budget = Budget(maximum, source)
    for name, value, part in part_entries(values, descriptor, max_parts, source):
        at = location(part['source'])
        wrapper = value if isinstance(value, Part) else Part(value=value)
        group = part_group(group_name, name)
        if group is not None and type(wrapper) is not registry.native_class(group['name']):
            if group['requiredHeaders']:
                raise SdkError('request-validation', at, code='http-part-headers-required')
        content_type = select_part(part, wrapper.content_type, at)
        headers = list(encode_headers(group['headerGroup'] if group else None, wrapper.headers, part_maximum, at))
        known = {key.lower() for key, _ in headers}
        for key, value in wrapper.extra_headers:
            if TOKEN.fullmatch(key) is None or key.lower() in known or key.lower() in ('content-type', 'content-disposition', 'content-length', 'transfer-encoding', 'content-transfer-encoding'):
                raise SdkError('request-representation', at, code='http-part-header-conflict')
            headers.append((key, header_value(value, at)))
            known.add(key.lower())
        disposition = b'form-data; name="' + disposition_text(name, part_maximum, at) + b'"'
        if wrapper.filename is not None:
            disposition += b'; filename="' + disposition_text(wrapper.filename, part_maximum, at) + b'"'
        if 'content-disposition' in known:
            actual = next(value for key, value in headers if key.lower() == 'content-disposition')
            parsed_name, _ = parse_disposition(actual, at)
            if parsed_name != name:
                raise SdkError('request-representation', at, code='http-part-name-conflict')
            disposition = actual.encode('ascii')
            headers = [(key, value) for key, value in headers if key.lower() != 'content-disposition']
        header_bytes = b'Content-Disposition: ' + disposition + b'\r\n'
        if content_type is not None:
            header_bytes += b'Content-Type: ' + header_value(content_type, at).encode('ascii') + b'\r\n'
        for key, value in headers:
            header_bytes += key.encode('ascii') + b': ' + header_value(value, at).encode('ascii') + b'\r\n'
        content = part_content(wrapper.value, part, min(part_maximum, maximum - budget.used), at)
        budget.charge(len(header_bytes) + len(content) + 4)
        entries.append((header_bytes, content))
    # A caller-selected multipart boundary is not permitted to collide with data.
    try:
        _, _, params = media.parse(selected)
    except ValueError as error:
        raise SdkError('request-representation', source, cause=error) from None
    specified = params.get('boundary')
    boundary = b''
    for _ in range(8):
        token = specified or 'python-sdk-' + secrets.token_hex(16)
        boundary = boundary_bytes(token, source)
        if all(find_boundary(content, b'--' + boundary, 0) < 0 and find_boundary(headers, b'--' + boundary, 0) < 0 for headers, content in entries):
            break
        if specified:
            raise SdkError('request-representation', source, code='http-boundary-collision')
    else:
        raise SdkError('request-representation', source, code='http-boundary-collision')
    budget.charge((len(boundary) + 6) * len(entries) + len(boundary) + 6)
    body = b''.join(b'--' + boundary + b'\r\n' + headers + b'\r\n' + content + b'\r\n' for headers, content in entries) + b'--' + boundary + b'--\r\n'
    return body, selected if specified else selected + '; boundary=' + boundary.decode('ascii')


def boundary_bytes(value: str, source: Source) -> bytes:
    if not value or len(value) > 70 or value.endswith(' ') or any(not (c.isascii() and (c.isalnum() or c in "'()+_,-./:=? ")) for c in value):
        raise SdkError('request-representation', source, code='http-multipart-boundary')
    return value.encode('ascii')


def find_boundary(body: bytes, marker: bytes, start: int) -> int:
    """Match whole MIME delimiter lines, not a byte payload's marker prefix."""
    at = start
    while True:
        at = body.find(marker, at)
        if at < 0:
            return -1
        if at and body[at - 2:at] != b'\r\n':
            at += 1
            continue
        end = at + len(marker)
        if body[end:end + 2] == b'--':
            end += 2
        while end < len(body) and body[end] in b' \t':
            end += 1
        if end == len(body) or body[end:end + 2] == b'\r\n':
            return at
        at += 1


def parse_disposition(value: str, source: Source) -> tuple[str, str | None]:
    try:
        pieces = media.split_quoted(value, ';')
        if pieces[0].strip().lower() != 'form-data':
            raise ValueError('form-data disposition required')
        params = media.parameters(pieces[1:])
        if 'name' not in params or 'filename*' in params:
            raise ValueError('named multipart disposition required')
        return params['name'], params.get('filename')
    except ValueError as error:
        raise SdkError('response-decoding', source, cause=error, code='http-part-disposition') from None


def decode_part(content: bytes, part: dict[str, Any], maximum: int, source: Source) -> object:
    if len(content) > maximum:
        raise SdkError('resource-limit', source, code='http-part-byte-limit')
    representation = part['representation']
    kind = representation['kind']
    if kind == 'binary':
        if len(content) > representation['bytes']['max_bytes']:
            raise SdkError('resource-limit', source, code='http-part-byte-limit')
        return content
    if kind == 'json':
        return registry.codec(representation['codec']).decode(content)
    if kind == 'text':
        value = parse_scalar(content.decode('utf-8'), representation['scalar'], source)
    else:
        # Style parts retain a complete named field expansion.
        value = parse_style(content.decode('utf-8'), part, source)
    return registry.codec(representation['codec']).decode_value(value)


def parse_style(text: str, part: dict[str, Any], source: Source) -> J.JsonValue:
    descriptor = part['representation']['serialization']
    name = part['name'] or ''
    shape, style = descriptor['shape'], descriptor['style']
    if descriptor['percent_encoding'] == 'none':
        pairs: list[tuple[str, str]] = []
        for field in text.split('&'):
            key, separator, value = field.partition('=')
            if not separator or len(pairs) >= 1024:
                raise SdkError('response-decoding', source, code='http-form-style')
            pairs.append((key, value))
    else:
        pairs = parse_qsl(text, keep_blank_values=True, strict_parsing=True, encoding='utf-8', errors='strict', max_num_fields=1024)
    if style == 'deepObject':
        values = [(key[len(name) + 1:-1], value) for key, value in pairs if key.startswith(name + '[') and key.endswith(']')]
        if len(values) != len(pairs):
            raise SdkError('response-decoding', source, code='http-form-style')
    elif shape['kind'] == 'flat-object' and descriptor['explode']:
        values = pairs
    else:
        if any(key != name for key, _ in pairs):
            raise SdkError('response-decoding', source, code='http-form-style')
        if shape['kind'] == 'scalar':
            if len(pairs) != 1:
                raise SdkError('response-decoding', source, code='http-form-repetition')
            return parse_scalar(pairs[0][1], shape['scalar'], source)
        separator = ' ' if style == 'spaceDelimited' else '|' if style == 'pipeDelimited' else ','
        items = [value for _, value in pairs] if descriptor['explode'] else pairs[0][1].split(separator) if len(pairs) == 1 else []
        if not items:
            raise SdkError('response-decoding', source, code='http-form-style')
        if shape['kind'] == 'array':
            return [parse_scalar(value, shape['items'], source) for value in items]
        if len(items) % 2:
            raise SdkError('response-decoding', source, code='http-form-style')
        values = list(zip(items[::2], items[1::2]))
    result: dict[str, J.JsonValue] = {}
    for key, value in values:
        kind = shape['properties'].get(key) or shape['additional'].get('scalar')
        if kind is None or key in result:
            raise SdkError('response-decoding', source, code='http-form-ambiguous-object')
        result[key] = parse_scalar(value, kind, source)
    return result


def construct(group_name: str, values: dict[str, object], source: Source) -> object:
    group = registry.plan()['groups'][group_name]
    members = {field['wire']: field['member'] for field in group['fields']}
    known = {members[key]: value for key, value in values.items() if key in members}
    result = registry.native_class(group_name)(**known)
    for key, value in values.items():
        if key not in members:
            if group['additional'] is None:
                raise SdkError('response-decoding', source, code='http-undeclared-part')
            result.set_extra(key, value)
    return result


def decode_form(group_name: str, body: bytes, descriptor: dict[str, Any], max_parts: int, source: Source) -> object:
    text = body.decode('utf-8')
    pairs = parse_qsl(text, keep_blank_values=True, strict_parsing=True, encoding='utf-8', errors='strict', max_num_fields=max_parts)
    definitions = {field['name']: field for field in descriptor['fields']}
    raw: dict[str, list[tuple[str, str]]] = {}
    for name, value in pairs:
        owner = name if name in definitions else None
        if owner is None:
            matches = []
            for key, part in definitions.items():
                rep = part['representation']
                if rep['kind'] != 'style':
                    continue
                ser = rep['serialization']
                if ser['style'] == 'deepObject' and name.startswith(key + '[') and name.endswith(']') or ser['shape']['kind'] == 'flat-object' and ser['explode'] and name in ser['shape']['properties']:
                    matches.append(key)
            if len(matches) > 1:
                raise SdkError('response-decoding', source, code='http-form-ambiguous-field')
            owner = matches[0] if matches else name
        raw.setdefault(owner, []).append((name, value))
    values: dict[str, object] = {}
    for name, entries in raw.items():
        part = definitions.get(name)
        if part is None:
            if descriptor['additional']['kind'] != 'allowed':
                raise SdkError('response-decoding', source, code='http-undeclared-part')
            part = dict(descriptor['additional']['part'], name=name)
        rep = part['representation']
        if rep['kind'] == 'style':
            groups = [[entry] for entry in entries] if part['multiplicity'] == 'repeated-array-items' else [entries]
            style_values = []
            for fields in groups:
                encoded = '&'.join(percent(key, 'form-url-encoded', len(body) * 3 + 1, source) + '=' + percent(value, 'form-url-encoded', len(body) * 3 + 1, source) for key, value in fields)
                style_values.append(registry.codec(rep['codec']).decode_value(parse_style(encoded, part, source)))
            if part['multiplicity'] == 'repeated-array-items':
                count_bound(len(style_values), part['min_items'], part['max_items'], source)
                values[name] = style_values
            else:
                values[name] = style_values[0]
        else:
            decoded = [decode_part(value.encode('utf-8'), part, len(body), source) for _, value in entries]
            if part['multiplicity'] == 'repeated-array-items':
                count_bound(len(decoded), part['min_items'], part['max_items'], source)
                values[name] = decoded
            elif len(decoded) == 1:
                values[name] = decoded[0]
            else:
                raise SdkError('response-decoding', source, code='http-form-repetition')
    structure(set(values), descriptor['rules'], source)
    return construct(group_name, values, source)


def decode_multipart(group_name: str, body: bytes, descriptor: dict[str, Any], content_type: str,
                     part_limit: int, max_parts: int, source: Source) -> object:
    boundary = boundary_bytes(media.parse(content_type)[2].get('boundary', ''), source)
    marker = b'--' + boundary
    at = find_boundary(body, marker, 0)
    if at < 0:
        raise SdkError('response-decoding', source, code='http-multipart-framing')
    definitions = {part['name']: part for part in descriptor['parts']}
    values: dict[str, object] = {}
    count = 0
    while True:
        at += len(marker)
        if body[at:at + 2] == b'--':
            break
        while at < len(body) and body[at] in b' \t':
            at += 1
        if body[at:at + 2] != b'\r\n':
            raise SdkError('response-decoding', source, code='http-multipart-framing')
        at += 2
        end = body.find(b'\r\n\r\n', at, min(len(body), at + 65536))
        if end < 0:
            raise SdkError('response-decoding', source, code='http-part-headers')
        headers: list[tuple[str, str]] = []
        for line in body[at:end].split(b'\r\n'):
            header_name, separator, header_data = line.partition(b':')
            if not separator or TOKEN.fullmatch(header_name.decode('ascii')) is None:
                raise SdkError('response-decoding', source, code='http-part-headers')
            headers.append((header_name.decode('ascii'), header_data.strip(b' \t').decode('utf-8')))
        dispositions = [value for name, value in headers if name.lower() == 'content-disposition']
        types = [value for name, value in headers if name.lower() == 'content-type']
        if len(dispositions) != 1 or len(types) > 1 or any(name.lower() in ('content-transfer-encoding', 'transfer-encoding') for name, _ in headers):
            raise SdkError('response-decoding', source, code='http-part-headers')
        name, filename = parse_disposition(dispositions[0], source)
        part = definitions.get(name)
        if part is None:
            if descriptor['additional']['kind'] != 'allowed':
                raise SdkError('response-decoding', source, code='http-undeclared-part')
            part = descriptor['additional']['part']
        selected = select_part(part, types[0] if types else None, source)
        start = end + 4
        next_marker = find_boundary(body, marker, start)
        if next_marker < 2:
            raise SdkError('response-decoding', source, code='http-multipart-framing')
        next_at = next_marker - 2
        if next_at - start > part_limit:
            raise SdkError('resource-limit', source, code='http-part-byte-limit')
        count += 1
        if count > max_parts:
            raise SdkError('resource-limit', source, code='http-part-count-limit')
        value = decode_part(body[start:next_at], part, part_limit, source)
        group = part_group(group_name, name)
        header_group = group['headerGroup'] if group else None
        typed = decode_headers(header_group, tuple(headers), source)
        cls = registry.native_class(group['name']) if group else Part
        item = cls(value=value, content_type=selected, filename=filename, headers=typed,
                   extra_headers=tuple((key, value) for key, value in headers if key.lower() not in ('content-type', 'content-disposition') and key.lower() not in {header['protocol']['name'].lower() for header in registry.plan()['groups'].get(header_group, {}).get('headers', [])}))
        if part['multiplicity'] == 'repeated-array-items':
            previous = values.setdefault(name, [])
            cast(list[object], previous).append(item)
        elif name in values:
            raise SdkError('response-decoding', source, code='http-part-repetition')
        else:
            values[name] = item
        at = next_marker
    structure(set(values), descriptor['rules'], source)
    for name, value in values.items():
        part = definitions.get(name) or descriptor['additional']['part']
        if part['multiplicity'] == 'repeated-array-items':
            count_bound(len(cast(list[object], value)), part['min_items'], part['max_items'], source)
    return construct(group_name, values, source)
