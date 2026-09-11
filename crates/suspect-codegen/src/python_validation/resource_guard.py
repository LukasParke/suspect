"""Checked physical resource scopes and dynamic bindings for portable v3."""
from __future__ import annotations

import ipaddress
import re
from typing import Any, TYPE_CHECKING
from urllib.parse import quote, unquote_to_bytes

if TYPE_CHECKING or __package__:
    from . import validation_guard as G
else:
    import validation_guard as G

ProgramError = G.ProgramError
V3 = ('suspect.validation.experimental.v3', 'oas31-jsonschema202012-resources-dynamic')
_URI = re.compile(r'^([A-Za-z][A-Za-z0-9+.-]*):(?://([^/?#]*))?([^?#]*)(\?[^#]*)?(?:#(.*))?$')
_PCHAR = r"[A-Za-z0-9._~!$&'()*+,;=:@%/-]*"


def uri(value: Any, at: dict[str, Any], *, fragment_free: bool = False) -> tuple[str, str]:
    value = G.text(value, at)
    match = _URI.fullmatch(value)
    if match is None or re.search(r'%(?![0-9A-Fa-f]{2})', value) or fragment_free and '#' in value:
        raise ProgramError('resource identifier requires an absolute RFC 3986 URI', at)
    scheme, authority, path, query, fragment = match.groups()
    if re.fullmatch(_PCHAR, path) is None or query is not None and re.fullmatch(_PCHAR + r'(?:\?' + _PCHAR + r')*', query[1:]) is None or fragment is not None and re.fullmatch(_PCHAR + r'(?:\?' + _PCHAR + r')*', fragment) is None:
        raise ProgramError('invalid resource URI component', at)
    document = scheme.lower() + ':'
    if authority is not None:
        user, marker, host_port = authority.rpartition('@')
        if not marker:
            host_port = authority
        elif re.fullmatch(r"[A-Za-z0-9._~!$&'()*+,;=:%-]*", user) is None:
            raise ProgramError('invalid resource URI userinfo', at)
        if host_port.startswith('['):
            close = host_port.find(']')
            if close == -1:
                raise ProgramError('invalid IP-literal authority', at)
            host, suffix = host_port[:close + 1], host_port[close + 1:]
            address = host[1:-1]
            try:
                if re.fullmatch(r"[vV][0-9A-Fa-f]+\.[A-Za-z0-9._~!$&'()*+,;=:-]+", address) is None:
                    ipaddress.IPv6Address(address)
            except ValueError as error:
                raise ProgramError('invalid IP-literal authority', at) from error
        else:
            host, colon, port = host_port.partition(':')
            suffix = colon + port
            if re.fullmatch(r"[A-Za-z0-9._~!$&'()*+,;=%-]*", host) is None:
                raise ProgramError('invalid resource URI host', at)
        if suffix and re.fullmatch(r':[0-9]*', suffix) is None:
            raise ProgramError('invalid resource URI port', at)
        document += '//' + (user + '@' if marker else '') + host.lower() + suffix
    document += path + (query or '')
    try:
        decoded = unquote_to_bytes(fragment or '').decode('utf-8', 'strict')
    except UnicodeError as error:
        raise ProgramError('resource fragment is not encoded Unicode', at) from error
    return document, quote(decoded, safe="-._~!$&'()*+,;=:@/?")


def uri_key(value: Any, at: dict[str, Any]) -> str:
    document, fragment = uri(value, at)
    return document + ('#' + fragment if fragment else '')


def source(value: Any) -> dict[str, Any]:
    at = G.source(value)
    uri(at['document'], at, fragment_free=True)
    return at


def contains(parent: dict[str, Any], child: dict[str, Any]) -> bool:
    return bool(parent['document'] == child['document'] and (parent['pointer'] == child['pointer'] or child['pointer'].startswith(parent['pointer'] + '/')))


def anchor(value: Any, at: dict[str, Any]) -> str:
    name = G.text(value, at)
    if re.fullmatch(r'[A-Za-z_][A-Za-z0-9_.-]*', name) is None:
        raise ProgramError('invalid dynamic anchor name', at)
    return name


def check_program(program: Any) -> dict[str, int]:
    if type(program) is not dict or (program.get('version'), program.get('profile')) != V3:
        return G.check_program(program)
    if set(program) != {'version', 'profile', 'roots', 'nodes', 'limits', 'resourceContext'}:
        raise ProgramError('invalid v3 program envelope')
    nodes = G.array(program['nodes'])
    # Reuse the frozen structural checks for common instructions. DynamicRef is
    # checked separately against the indexed registry, never lowered to Ref.
    common = dict(program)
    common.pop('resourceContext')
    common.update(version=G.V2[0], profile=G.V2[1], nodes=[])
    dynamic: list[dict[str, Any]] = []
    for node in nodes:
        if type(node) is not dict or set(node) != {'source', 'checks'}:
            raise ProgramError('invalid schema node')
        at = source(node['source'])
        checks = G.array(node['checks'], at)
        ordinary = []
        found, tail = False, False
        for check in checks:
            if type(check) is not dict:
                raise ProgramError('invalid schema instruction', at)
            op = check.get('op')
            if tail and op not in ('unevaluatedItems', 'unevaluatedProperties'):
                raise ProgramError('unevaluated checks must follow other checks', at)
            tail = tail or op in ('unevaluatedItems', 'unevaluatedProperties')
            if op == 'dynamicRef':
                location = source(check.get('source'))
                if found or location != G.child(at, '$dynamicRef') or set(check) != {'op','source','target','initialResource','anchor'}:
                    raise ProgramError('invalid dynamicRef source or operands', location)
                found = True
                dynamic.append(check)
            else:
                if op == 'always' and len(checks) != 1:
                    raise ProgramError('Boolean schema requires one check', at)
                ordinary.append(check)
        common['nodes'].append({'source': at, 'checks': ordinary})
    limits = G.check_program(common)
    context = program['resourceContext']
    if type(context) is not dict or set(context) != {'resources', 'nodeScopes'}:
        raise ProgramError('v3 requires indexed resource context')
    resources = G.array(context['resources'])
    scopes = G.array(context['nodeScopes'])
    if len(scopes) != len(nodes):
        raise ProgramError('resource scopes must align with nodes')
    identities: set[tuple[str, str]] = set()
    aliases: dict[str, int] = {}
    for index, resource in enumerate(resources):
        if type(resource) is not dict or set(resource) != {'source','kind','canonicalUri','baseUri','aliases','declarationSource','dynamicAnchors'}:
            raise ProgramError('invalid resource descriptor')
        at = source(resource['source'])
        if G.identity(at) in identities:
            raise ProgramError('duplicate physical resource identity', at)
        identities.add(G.identity(at))
        kind = resource['kind']
        if kind not in ('schema','document','openApiDocument'):
            raise ProgramError('unknown compiled resource kind', at)
        canonical = uri_key(resource['canonicalUri'], at)
        base, _ = uri(resource['baseUri'], at, fragment_free=True)
        canonical_document, _ = uri(resource['canonicalUri'], at)
        if canonical_document != resource['baseUri'] or kind != 'openApiDocument' and canonical != base:
            raise ProgramError('resource canonical identifier/base disagree', at)
        parts = _URI.fullmatch(base)
        assert parts is not None
        if any(segment in ('.', '..') for segment in parts.group(3).split('/')):
            raise ProgramError('resource base must retain canonical RFC path resolution', at)
        declaration = resource['declarationSource']
        if declaration is not None:
            source(declaration)
            if kind == 'document' or declaration != G.child(at, '$id' if kind == 'schema' else '$self'):
                raise ProgramError('identifier declaration is outside its resource boundary', at)
        elif at['pointer'] or canonical != uri_key(at['document'], at):
            raise ProgramError('undeclared resource must retain its retrieval identity', at)
        names = G.distinct(resource['aliases'], at)
        canonical_names: set[str] = set()
        for name in names:
            key = uri_key(name, at)
            if key in aliases and aliases[key] != index:
                raise ProgramError('URI alias identifies multiple physical resources', at)
            aliases[key] = index
            canonical_names.add(key)
        if canonical not in canonical_names or base not in canonical_names:
            raise ProgramError('aliases must include canonical identifier and base', at)
    used: set[int] = set()
    for node, scope in zip(nodes, scopes):
        at = node['source']
        if len(G.array(scope, at)) != 3:
            raise ProgramError('invalid node scope tuple', at)
        index, schema_root, address = G.integer(scope[0], at), source(scope[1]), G.text(scope[2], at)
        if index >= len(resources):
            raise ProgramError('node resource outside registry', at)
        resource = resources[index]
        if not contains(resource['source'], schema_root) or not contains(schema_root, at) or resource['kind'] == 'schema' and schema_root != resource['source']:
            raise ProgramError('node schema/resource roots do not contain its physical source', at)
        suffix = at['pointer'][len(resource['source']['pointer']):]
        expected = resource['baseUri'] + ('#' + quote(suffix, safe="-._~!$&'()*+,;=:@/?") if suffix else '')
        if address != expected:
            raise ProgramError('node address disagrees with resource-relative source', at)
        used.add(index)
    if len(used) != len(resources):
        raise ProgramError('resource registry contains an unreferenced resource')
    for index, resource in enumerate(resources):
        anchor_names: list[str] = []
        for entry in G.array(resource['dynamicAnchors'], resource['source']):
            if len(G.array(entry, resource['source'])) != 3:
                raise ProgramError('invalid dynamic anchor tuple', resource['source'])
            name, at = anchor(entry[0], resource['source']), source(entry[1])
            target = G.integer(entry[2], at)
            if target >= len(nodes) or at != G.child(nodes[target]['source'], '$dynamicAnchor') or G.integer(scopes[target][0]) != index:
                raise ProgramError('dynamic anchor source/target/resource disagree', at)
            anchor_names.append(name)
        G.distinct(anchor_names, resource['source'])
    for check in dynamic:
        at = check['source']
        target, initial = G.integer(check['target'], at), G.integer(check['initialResource'], at)
        if target >= len(nodes) or initial >= len(resources) or G.integer(scopes[target][0]) != initial:
            raise ProgramError('dynamic initial resource disagrees with its target', at)
        if check['anchor'] is not None:
            name = anchor(check['anchor'], at)
            if not any(entry[0] == name and G.integer(entry[2]) == target for entry in resources[initial]['dynamicAnchors']):
                raise ProgramError('dynamic name is not bound at the initial target', at)
    return limits
