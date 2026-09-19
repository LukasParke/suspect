"""Structural admission for scoped portable programs; literal data stays literal."""
from __future__ import annotations

import re
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING or __package__:
    from . import json_runtime as J
    from .validation_number import Exact
else:
    import json_runtime as J
    from validation_number import Exact

V1 = ('suspect.validation.experimental.v1', 'oas31-jsonschema202012-static-subset')
V2 = ('suspect.validation.experimental.v2', 'oas31-jsonschema202012-static-applicators')
SCOPED = frozenset(('if', 'dependentRequired', 'dependentSchemas', 'contains', 'patternProperties',
                    'additionalPropertiesWithPatterns', 'propertyNames', 'unevaluatedProperties', 'unevaluatedItems'))
_OPERANDS = {
    'always': ('value',), 'type': ('types',), 'ref': ('target',),
    'properties': ('properties',), 'additionalProperties': ('declared', 'target'),
    'required': ('names',), 'items': ('start', 'target'), 'prefixItems': ('targets',),
    'allOf': ('targets',), 'anyOf': ('targets',), 'oneOf': ('targets',), 'not': ('target',),
    'bound': ('value', 'maximum', 'exclusive'), 'multipleOf': ('value',),
    'count': ('value', 'maximum', 'target'), 'enum': ('values',), 'const': ('value',),
    'uniqueItems': (), 'pattern': ('program',),
    'if': ('condition', 'thenTarget', 'elseTarget'), 'dependentRequired': ('dependencies',),
    'dependentSchemas': ('dependencies',), 'contains': ('target', 'minimum', 'maximum'),
    'patternProperties': ('patterns',), 'additionalPropertiesWithPatterns': ('declared', 'target'),
    'propertyNames': ('target',), 'unevaluatedProperties': ('target',), 'unevaluatedItems': ('target',),
}


class ProgramError(ValueError):
    """A located malformed program, distinct from schema invalidity."""
    def __init__(self, message: str, source: dict[str, Any] | None = None) -> None:
        self.message = message
        self.source = '' if source is None else str(source.get('document', '')) + '#' + str(source.get('pointer', ''))
        super().__init__(message + (' at ' + self.source if self.source else ''))


def integer(value: Any, source: dict[str, Any] | None = None) -> int:
    if type(value) is J.JsonNumber:
        token = value.token
        if not token or not all('0' <= char <= '9' for char in token) or len(token) > 20:
            raise ProgramError('structural integer requires an unsigned integer token', source)
        value = value.to_int(20)
    if type(value) is not int or not 0 <= value <= 18_446_744_073_709_551_615:
        raise ProgramError('structural integer is outside the portable domain', source)
    return value


def text(value: Any, source: dict[str, Any] | None = None) -> str:
    if type(value) is not str or any(0xD800 <= ord(char) <= 0xDFFF for char in value):
        raise ProgramError('Unicode scalar string required', source)
    return value


def array(value: Any, source: dict[str, Any] | None = None) -> list[Any]:
    if type(value) is not list:
        raise ProgramError('array operand required', source)
    return value


def source(value: Any) -> dict[str, Any]:
    if type(value) is not dict or set(value) != {'document', 'pointer'}:
        raise ProgramError('source requires document and pointer')
    document, pointer = text(value['document'], value), text(value['pointer'], value)
    if re.match(r'^[A-Za-z][A-Za-z0-9+.-]*:', document) is None or '#' in document or any(ord(c) <= 32 or ord(c) >= 127 or c in '<>"{}|\\^`' for c in document) or re.search(r'%(?![0-9A-Fa-f]{2})', document):
        raise ProgramError('source requires an absolute fragment-free URI', value)
    if pointer and not pointer.startswith('/') or re.search(r'~(?![01])', pointer):
        raise ProgramError('source requires an escaped JSON Pointer', value)
    return value


def identity(value: dict[str, Any]) -> tuple[str, str]:
    return value['document'], value['pointer']


def child(value: dict[str, Any], name: str) -> dict[str, Any]:
    return {'document': value['document'], 'pointer': value['pointer'] + '/' + name.replace('~', '~0').replace('/', '~1')}


def distinct(values: Any, at: dict[str, Any]) -> set[str]:
    seen: set[str] = set()
    for value in array(values, at):
        value = text(value, at)
        if value in seen:
            raise ProgramError('duplicate decoded name', at)
        seen.add(value)
    return seen


def number(token: Any, maximum: int, at: dict[str, Any], *, count: bool = False, positive: bool = False) -> None:
    token = text(token, at)
    if len(token) > maximum:
        raise ProgramError('numeric operand exceeds maxNumberBytes', at)
    try:
        J.JsonNumber(token)
    except J.JsonError as error:
        raise ProgramError('numeric operand is not one JSON number token', at) from error
    value = Exact.parse(token)
    if count and (value.sign < 0 or not value.integral()) or positive and value.sign <= 0:
        raise ProgramError('invalid exact count or divisor', at)


def pattern(program: Any, at: dict[str, Any]) -> None:
    if type(program) is not dict or set(program) != {'version', 'start', 'states'} or program['version'] != 'suspect.pattern.experimental.v1':
        raise ProgramError('unsupported pattern program', at)
    states = array(program['states'], at)
    if not 1 <= len(states) <= 8192 or integer(program['start'], at) >= len(states):
        raise ProgramError('invalid finite pattern graph', at)
    total = 0
    for state in states:
        if type(state) is not dict:
            raise ProgramError('invalid pattern state', at)
        op = state.get('op')
        if type(op) is not str:
            raise ProgramError('invalid pattern opcode', at)
        operands = {'match': (), 'char': ('ranges', 'target'), 'split': ('first', 'second'),
                    'jump': ('target',), 'start': ('target',), 'end': ('target',)}.get(op)
        if operands is None or set(state) != {'op', *operands}:
            raise ProgramError('unknown or malformed pattern instruction', at)
        for key in ('target', 'first', 'second'):
            if key in state and integer(state[key], at) >= len(states):
                raise ProgramError('pattern target outside graph', at)
        if op == 'char':
            ranges = array(state['ranges'], at)
            total += len(ranges)
            if len(ranges) > 8192 or total > 65_536:
                raise ProgramError('pattern range limit exceeded', at)
            previous = -2
            for pair in ranges:
                if len(array(pair, at)) != 2:
                    raise ProgramError('invalid scalar range', at)
                low, high = integer(pair[0], at), integer(pair[1], at)
                if low > high or high > 0x10FFFF or low <= previous + 1 or low <= 0xDFFF and high >= 0xD800:
                    raise ProgramError('ranges must be normalized Unicode scalar ranges', at)
                previous = high


def literal(value: Any, at: dict[str, Any]) -> None:
    # Literal number byte/equality budgets belong to evaluation, not admission.
    pending: list[tuple[Any, bool]] = [(value, False)]
    active: set[int] = set()
    while pending:
        value, leave = pending.pop()
        if leave:
            active.remove(id(value))
            continue
        kind = type(value)
        if value is None or kind in (bool, int):
            continue
        if kind is str:
            text(value, at)
        elif kind is J.JsonNumber:
            try:
                J.JsonNumber(value.token)
            except J.JsonError as error:
                raise ProgramError('invalid literal number', at) from error
        elif kind in (dict, list):
            if id(value) in active:
                raise ProgramError('cyclic literal metadata', at)
            active.add(id(value))
            pending.append((value, True))
            if kind is dict:
                for key in value:
                    text(key, at)
                pending.extend((item, False) for item in value.values())
            else:
                pending.extend((item, False) for item in value)
        else:
            raise ProgramError('literal is outside the exact JSON domain', at)


def snapshot(program: dict[str, Any]) -> dict[str, Any]:
    """Own checked metadata without recursive copying or mutable caller aliases."""
    result: dict[str, Any] = {}
    copied: dict[int, Any] = {id(program): result}
    pending: list[tuple[Any, Any]] = [(program, result)]
    while pending:
        original, destination = pending.pop()
        entries = original.items() if type(original) is dict else enumerate(original)
        for key, value in entries:
            if type(value) in (dict, list):
                if id(value) not in copied:
                    copied[id(value)] = {} if type(value) is dict else []
                    pending.append((value, copied[id(value)]))
                value = copied[id(value)]
            if type(destination) is dict:
                destination[key] = value
            else:
                destination.append(value)
    return result


def check_program(program: Any) -> dict[str, int]:
    if type(program) is not dict or set(program) != {'version', 'profile', 'roots', 'nodes', 'limits'}:
        raise ProgramError('invalid program envelope')
    pair = program['version'], program['profile']
    if pair not in (V1, V2):
        raise ProgramError('unsupported compiled validation version/profile')
    raw_limits = program['limits']
    keys = {'maxDepth', 'maxErrors', 'maxNumberBytes', 'maxEqualitySteps', 'maxEvaluationSteps'}
    if type(raw_limits) is not dict or set(raw_limits) != keys:
        raise ProgramError('invalid program limits')
    limits = {key: integer(value) for key, value in raw_limits.items()}
    if limits['maxDepth'] > 512 or limits['maxNumberBytes'] > 65_536:
        raise ProgramError('limits exceed the Python native profile')
    nodes = array(program['nodes'])
    identities: set[tuple[str, str]] = set()
    for node in nodes:
        if type(node) is not dict or set(node) != {'source', 'checks'}:
            raise ProgramError('invalid schema node')
        at = source(node['source'])
        if identity(at) in identities:
            raise ProgramError('duplicate schema source identity', at)
        identities.add(identity(at))
        array(node['checks'], at)

    def target(index: Any, at: dict[str, Any], expected: dict[str, Any] | None = None) -> None:
        index = integer(index, at)
        if index >= len(nodes):
            raise ProgramError('schema target outside graph', at)
        if expected is not None and nodes[index]['source'] != expected:
            raise ProgramError('applicator target has the wrong source identity', at)

    for node in nodes:
        locations: set[tuple[str, str]] = set()
        declared: tuple[dict[str, Any], set[str]] | None = None
        properties: set[str] = set()
        prefix, pattern_count = 0, 0
        items: tuple[dict[str, Any], int] | None = None
        aware, tail = False, False
        for check in node['checks']:
            if type(check) is not dict:
                raise ProgramError('invalid schema instruction', node['source'])
            at = source(check.get('source'))
            op = check.get('op')
            if type(op) is not str or op not in _OPERANDS or set(check) != {'source', 'op', *_OPERANDS[op]}:
                raise ProgramError('unknown or malformed schema instruction', at)
            if pair == V1 and op in SCOPED:
                raise ProgramError('v2 instruction in a v1 program', at)
            unevaluated = op in ('unevaluatedProperties', 'unevaluatedItems')
            if pair == V2 and tail and not unevaluated:
                raise ProgramError('unevaluated checks must follow other checks', at)
            tail = tail or unevaluated
            keyword = '$ref' if op == 'ref' else 'additionalProperties' if op == 'additionalPropertiesWithPatterns' else op
            if op == 'bound':
                if type(check['maximum']) is not bool or type(check['exclusive']) is not bool:
                    raise ProgramError('Boolean bound flags required', at)
                keyword = ('exclusiveMaximum' if check['exclusive'] else 'maximum') if check['maximum'] else ('exclusiveMinimum' if check['exclusive'] else 'minimum')
            elif op == 'count':
                if type(check['maximum']) is not bool or check['target'] not in ('string', 'array', 'object'):
                    raise ProgramError('invalid count target/flag', at)
                keyword = ('max' if check['maximum'] else 'min') + {'string': 'Length', 'array': 'Items', 'object': 'Properties'}[check['target']]
            expected = node['source'] if op == 'always' else child(node['source'], keyword)
            normalized = op == 'bound' and check['exclusive'] and at == child(node['source'], 'maximum' if check['maximum'] else 'minimum')
            if at != expected and not normalized or identity(at) in locations:
                raise ProgramError('instruction must uniquely identify its source keyword', at)
            locations.add(identity(at))
            if op == 'always':
                if type(check['value']) is not bool or len(node['checks']) != 1:
                    raise ProgramError('Boolean schema requires one Boolean check', at)
            elif op == 'type':
                names = distinct(check['types'], at)
                if not names or not names <= {'null', 'boolean', 'integer', 'number', 'string', 'array', 'object'}:
                    raise ProgramError('invalid type names', at)
            elif op == 'ref':
                target(check['target'], at)
            elif op in ('properties', 'dependentSchemas'):
                entries = array(check['properties' if op == 'properties' else 'dependencies'], at)
                field_names: list[str] = []
                for entry in entries:
                    if type(entry) is not dict or set(entry) != {'name', 'target'}:
                        raise ProgramError('invalid named schema target', at)
                    name = text(entry['name'], at)
                    field_names.append(name)
                    target(entry['target'], at, child(at, name))
                fields = distinct(field_names, at)
                if op == 'properties':
                    properties = fields
            elif op in ('additionalProperties', 'additionalPropertiesWithPatterns'):
                aware = op == 'additionalPropertiesWithPatterns'
                declared = at, distinct(check['declared'], at)
                target(check['target'], at, at)
            elif op == 'required':
                distinct(check['names'], at)
            elif op == 'items':
                items = at, integer(check['start'], at)
                target(check['target'], at, at)
            elif op in ('not', 'propertyNames', 'unevaluatedProperties', 'unevaluatedItems'):
                target(check['target'], at, at)
            elif op == 'if':
                target(check['condition'], at, at)
                for name in ('then', 'else'):
                    index = check[name + 'Target']
                    if index is not None:
                        target(index, at, child(node['source'], name))
            elif op == 'dependentRequired':
                triggers = []
                for entry in array(check['dependencies'], at):
                    if len(array(entry, at)) != 2:
                        raise ProgramError('invalid dependency tuple', at)
                    trigger = text(entry[0], at)
                    triggers.append(trigger)
                    distinct(entry[1], child(at, trigger))
                distinct(triggers, at)
            elif op == 'contains':
                target(check['target'], at, at)
                for name in ('minimum', 'maximum'):
                    if check[name] is not None:
                        number(check[name], limits['maxNumberBytes'], child(node['source'], 'minContains' if name == 'minimum' else 'maxContains'), count=True)
            elif op == 'patternProperties':
                entries = array(check['patterns'], at)
                pattern_count = len(entries)
                patterns = []
                for entry in entries:
                    if len(array(entry, at)) != 3:
                        raise ProgramError('invalid pattern tuple', at)
                    name = text(entry[0], at)
                    patterns.append(name)
                    target(entry[2], at, child(at, name))
                    pattern(entry[1], child(at, name))
                distinct(patterns, at)
            elif op in ('allOf', 'anyOf', 'oneOf', 'prefixItems'):
                entries = array(check['targets'], at)
                if not entries:
                    raise ProgramError('nonempty applicator targets required', at)
                if op == 'prefixItems':
                    prefix = len(entries)
                for position, index in enumerate(entries):
                    target(index, at, child(at, str(position)))
            elif op in ('bound', 'count', 'multipleOf'):
                number(check['value'], limits['maxNumberBytes'], at, count=op == 'count', positive=op == 'multipleOf')
            elif op == 'enum':
                for value in array(check['values'], at):
                    literal(value, at)
            elif op == 'const':
                literal(check['value'], at)
            elif op == 'pattern':
                pattern(check['program'], at)
        if declared is not None:
            at, names = declared
            if names != properties or aware != (pattern_count != 0):
                raise ProgramError('additionalProperties disagrees with adjacent properties/patterns', at)
        if items is not None and items[1] != prefix:
            raise ProgramError('items start disagrees with adjacent prefixItems', items[0])
    roots: set[tuple[str, str]] = set()
    for root in array(program['roots']):
        if type(root) is not dict or set(root) != {'source', 'target'}:
            raise ProgramError('invalid root')
        at = source(root['source'])
        target(root['target'], at, at)
        if identity(at) in roots:
            raise ProgramError('duplicate root identity', at)
        roots.add(identity(at))
    return limits
