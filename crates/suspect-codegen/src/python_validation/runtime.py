"""Checked portable schema execution with exact values and shared finite budgets."""
from __future__ import annotations
import functools
import json
import pathlib
from collections.abc import Generator
from typing import Any, NoReturn, TYPE_CHECKING, cast

if TYPE_CHECKING or __package__:
    from . import json_runtime as _json
    from .validation_number import Exact, divisible
else:
    import json_runtime as _json
    from validation_number import Exact, divisible

class ValidationError(Exception):
    def __init__(self, kind: str, source: str, instance_path: str, message: str) -> None:
        super().__init__(f'{kind} at {source} {instance_path}: {message}')
        self.kind, self.source, self.instance_path, self.message = kind, source, instance_path, message

def _loc(source: dict[str, Any]) -> str:
    return cast(str, source['document']) + '#' + cast(str, source['pointer'])

def _child(path: str, key: str) -> str:
    return path + '/' + key.replace('~', '~0').replace('/', '~1')

@functools.lru_cache(maxsize=1)
def _load() -> dict[str, Any]:
    # JSON metadata stays int; arbitrary literal integers/decimals both use
    # exact token wrappers until a structural metadata field is accessed.
    text = pathlib.Path(__file__).with_name('validation_program.json').read_text(encoding='utf-8')
    return cast(dict[str, Any], json.loads(text, parse_int=_json.JsonNumber, parse_float=_json.JsonNumber))

def _int(value: Any) -> int:
    if type(value) is int:
        return value
    if type(value) is _json.JsonNumber:
        return value.to_int(18)
    raise ValueError('invalid integer metadata')

def _kind(value: Any) -> str:
    if value is None: return 'null'
    if type(value) is bool: return 'boolean'
    if type(value) is str: return 'string'
    if type(value) is list: return 'array'
    if type(value) is dict: return 'object'
    if type(value) in (int, _json.JsonNumber): return 'number'
    return 'invalid'

class ValidationSession:
    """One work/equality/numeric budget shared by every root and branch trial."""
    def __init__(self, program: dict[str, Any] | None = None) -> None:
        self.program = _load() if program is None else program
        if self.program['version'] != 'suspect.validation.experimental.v1' or self.program['profile'] != 'oas31-jsonschema202012-static-subset':
            raise ValueError('unsupported program profile')
        self.nodes = self.program['nodes']
        self.roots = {_int(root['target']) for root in self.program['roots']}
        self.limits = {key: _int(value) for key, value in self.program['limits'].items()}
        self.steps = self.limits['maxEvaluationSteps']
        self.equalities = self.limits['maxEqualitySteps']
        self.numeric = self.limits['maxEvaluationSteps']
        self.findings: list[ValidationError] = []
    def failure(self, source: dict[str, Any], path: str, message: str) -> NoReturn:
        raise ValidationError('evaluation_failure', _loc(source), path, message)
    def spend(self, source: dict[str, Any], path: str, amount: int = 1) -> None:
        self.steps -= amount
        if self.steps < 0: self.failure(source, path, 'evaluation work budget exhausted')
    def mismatch(self, source: dict[str, Any], path: str, message: str) -> bool:
        if self.limits['maxErrors'] == 0 or len(self.findings) < self.limits['maxErrors']:
            self.findings.append(ValidationError('invalid', _loc(source), path, message))
        return False
    def number(self, value: Any, source: dict[str, Any], path: str) -> Exact:
        try:
            token = value.token if type(value) is _json.JsonNumber else _json.stringify_json(value, _json.JsonLimits(max_output_bytes=self.limits['maxNumberBytes'])) if type(value) is int else value
            if len(token) > self.limits['maxNumberBytes']: self.failure(source, path, 'numeric operand budget exhausted')
            return Exact.parse(token)
        except _json.JsonError as error:
            self.failure(source, path, error.message)
    def check(self, root_index: int, value: Any) -> None:
        if root_index not in self.roots:
            raise ValidationError('evaluation_failure', '', '', 'root was not selected')
        self.findings = []
        active: set[tuple[int, int]] = set()
        # Explicit driver stack: deeply nested schemas do not consume the
        # interpreter call stack or mutate its global recursion setting.
        stack: list[tuple[Generator[Any, bool, bool], tuple[int, int], int | None, bool]] = []
        def push(index: int, instance: Any, path: str, trial: bool) -> None:
            if not 0 <= index < len(self.nodes): raise ValidationError('evaluation_failure', '', path, 'invalid program target')
            at = self.nodes[index]['source']
            if len(stack) >= self.limits['maxDepth']: self.failure(at, path, 'evaluation depth budget exhausted')
            identity = (index, id(instance))
            if identity in active: self.failure(at, path, 'nonproductive recursive evaluation')
            active.add(identity)
            stack.append((self.node(index, instance, path), identity, len(self.findings) if trial else None, False))
        push(root_index, value, '', False)
        result = False
        while stack:
            generator, identity, restore, started = stack[-1]
            try:
                if started:
                    request = generator.send(result)
                else:
                    stack[-1] = (generator, identity, restore, True)
                    request = next(generator)
                push(*request)
                result = False
            except StopIteration as done:
                result = bool(done.value)
                stack.pop(); active.remove(identity)
                if restore is not None: del self.findings[restore:]
        if not result:
            if self.findings: raise self.findings[0]
            raise ValidationError('invalid', _loc(self.nodes[root_index]['source']), '', 'source schema rejected the value')
    def node(self, index: int, value: Any, path: str) -> Generator[Any, bool, bool]:
        node = self.nodes[index]; self.spend(node['source'], path)
        valid = True
        for check in node['checks']:
            at, op = check['source'], check['op']; self.spend(at, path)
            if op == 'ref':
                result = yield (_int(check['target']), value, path, False)
            elif op in ('allOf', 'anyOf', 'oneOf'):
                count = 0
                for target in check['targets']:
                    self.spend(at, path)
                    matched = yield (_int(target), value, path, op != 'allOf')
                    count += bool(matched)
                result = count == len(check['targets']) if op == 'allOf' else count > 0 if op == 'anyOf' else count == 1
                if not result: self.mismatch(at, path, op + ' branch count rejected the value')
            elif op == 'not':
                matched = yield (_int(check['target']), value, path, True)
                result = not matched
                if not result: self.mismatch(at, path, 'negated schema accepted the value')
            elif op in ('properties', 'additionalProperties'):
                result = True
                if type(value) is dict:
                    if op == 'properties': pairs = [(p['name'], _int(p['target'])) for p in check['properties'] if p['name'] in value]
                    else: pairs = [(key, _int(check['target'])) for key in value if key not in check['declared']]
                    for name, target in pairs:
                        self.spend(at, path)
                        matched = yield (target, value[name], _child(path, name), False)
                        result = bool(matched) and result
            elif op in ('items', 'prefixItems'):
                result = True
                if type(value) is list:
                    targets = [(i, _int(check['target'])) for i in range(_int(check['start']), len(value))] if op == 'items' else list(enumerate(map(_int, check['targets'])))[:len(value)]
                    for i, target in targets:
                        self.spend(at, path)
                        matched = yield (target, value[i], _child(path, str(i)), False)
                        result = bool(matched) and result
            else:
                result = self.scalar(check, value, path)
            valid = bool(result) and valid
        return valid
    def equal(self, left: Any, right: Any, source: dict[str, Any], path: str) -> bool:
        pending = [(left, right, 0)]
        while pending:
            a, b, depth = pending.pop(); self.equalities -= 1
            if self.equalities < 0 or depth > self.limits['maxDepth']: self.failure(source, path, 'equality budget exhausted')
            kind = _kind(a)
            if kind != _kind(b): return False
            if kind == 'number':
                if self.number(a, source, path).compare(self.number(b, source, path)) != 0: return False
            elif kind == 'array':
                if len(a) != len(b): return False
                pending.extend((x, y, depth + 1) for x, y in zip(a, b))
            elif kind == 'object':
                if a.keys() != b.keys(): return False
                pending.extend((a[key], b[key], depth + 1) for key in a)
            elif kind == 'invalid' or a != b: return False
        return True
    def pattern(self, program: dict[str, Any], text: str, source: dict[str, Any], path: str) -> bool:
        states = program['states']
        for offset in range(len(text) + 1):
            current = {_int(program['start'])}
            for position in range(offset, len(text) + 1):
                pending = list(current); seen: set[int] = set(); consuming = set()
                while pending:
                    self.spend(source, path); index = pending.pop()
                    if index in seen: continue
                    seen.add(index); state = states[index]; op = state['op']
                    if op == 'match': return True
                    if op == 'split': pending.extend((_int(state['first']), _int(state['second'])))
                    elif op == 'jump' or op == 'start' and position == 0 or op == 'end' and position == len(text): pending.append(_int(state['target']))
                    elif op == 'char': consuming.add(index)
                if position == len(text): break
                current = set()
                for index in consuming:
                    state = states[index]
                    for low, high in state['ranges']:
                        self.spend(source, path)
                        if _int(low) <= ord(text[position]) <= _int(high): current.add(_int(state['target'])); break
                if not current: break
        return False
    def scalar(self, check: dict[str, Any], value: Any, path: str) -> bool:
        at, op, kind = check['source'], check['op'], _kind(value)
        result = True
        if op == 'always': result = check['value'] is True
        elif op == 'type': result = kind in check['types'] or kind == 'number' and 'integer' in check['types'] and self.number(value, at, path).integral()
        elif op == 'required' and kind == 'object':
            for name in check['names']:
                self.spend(at, path)
                if name not in value: result = self.mismatch(at, path, f'required property {name!r} is absent')
        elif op == 'bound' and kind == 'number':
            order = self.number(value, at, path).compare(self.number(check['value'], at, path))
            result = order < 0 if check['maximum'] and check['exclusive'] else order <= 0 if check['maximum'] else order > 0 if check['exclusive'] else order >= 0
        elif op == 'multipleOf' and kind == 'number':
            def spend(amount: int) -> None:
                self.numeric -= amount
                if self.numeric < 0: self.failure(at, path, 'numeric work budget exhausted')
            result = divisible(self.number(value, at, path), self.number(check['value'], at, path), spend)
        elif op == 'count' and kind == check['target']:
            order = Exact.parse(str(len(value))).compare(self.number(check['value'], at, path))
            result = order <= 0 if check['maximum'] else order >= 0
        elif op == 'const': result = self.equal(value, check['value'], at, path)
        elif op == 'enum':
            matches = [self.equal(value, candidate, at, path) for candidate in check['values']]
            result = any(matches)
        elif op == 'uniqueItems' and kind == 'array':
            for i in range(len(value)):
                for j in range(i):
                    self.spend(at, path)
                    if self.equal(value[i], value[j], at, path): result = False
        elif op == 'pattern' and kind == 'string': result = self.pattern(check['program'], value, at, path)
        elif op not in ('always','type','required','bound','multipleOf','count','const','enum','uniqueItems','pattern'): self.failure(at, path, 'unsupported program instruction ' + op)
        if not result: self.mismatch(at, path, op + ' assertion rejected the value')
        return result

def validate(root_index: int, value: Any) -> None:
    ValidationSession().check(root_index, value)
