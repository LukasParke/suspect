"""Scoped evaluated locations over checked v2 instructions, with a frozen v1 path."""
from __future__ import annotations

import dataclasses
from collections.abc import Generator
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING or __package__:
    from . import json_runtime as _json, validation_v1 as _v1
    from .validation_guard import ProgramError as ProgramError, V2, check_program, child, integer, snapshot
    from .validation_number import Exact
else:
    import json_runtime as _json
    import validation_v1 as _v1
    from validation_guard import ProgramError as ProgramError, V2, check_program, child, integer, snapshot
    from validation_number import Exact

ValidationError = _v1.ValidationError
ValidationError.__module__ = __name__


@dataclasses.dataclass
class _Annotations:
    properties: set[str] = dataclasses.field(default_factory=set)
    items: set[int] = dataclasses.field(default_factory=set)


@dataclasses.dataclass
class _Evaluated:
    valid: bool
    annotations: _Annotations = dataclasses.field(default_factory=_Annotations)


@dataclasses.dataclass
class _Request:
    index: int
    value: Any
    path: str
    trial: bool = False
    identity: object | None = None


@dataclasses.dataclass
class _Frame:
    iterator: Generator[_Request, _Evaluated, _Evaluated]
    identity: tuple[int, int]
    findings: list[ValidationError] | None
    started: bool = False


class ValidationSession(_v1.ValidationSession):
    """One shared budget across roots/trials; each evaluation has a fresh scope."""
    def __init__(self, program: dict[str, Any] | None = None) -> None:
        program = _v1._load() if program is None else program
        limits = check_program(program)
        self.scoped = (program['version'], program['profile']) == V2
        if not self.scoped:
            super().__init__(program)
        else:
            program = snapshot(program)
            self.program = program
            self.nodes = program['nodes']
            self.roots = {integer(root['target']) for root in program['roots']}
            self.limits = limits
            self.steps = limits['maxEvaluationSteps']
            self.equalities = limits['maxEqualitySteps']
            self.numeric = limits['maxEvaluationSteps']
            self.findings: list[ValidationError] = []
        self._numbers: dict[str, Exact] = {}

    def merge(self, into: _Annotations, other: _Annotations, source: dict[str, Any], path: str) -> None:
        for name in sorted(other.properties):
            self.spend(source, path)
            into.properties.add(name)
        for index in sorted(other.items):
            self.spend(source, path)
            into.items.add(index)

    def number(self, value: Any, source: dict[str, Any], path: str) -> Exact:
        if not self.scoped:
            return super().number(value, source, path)
        try:
            token = value.token if type(value) is _json.JsonNumber else _json.stringify_json(value, _json.JsonLimits(max_output_bytes=self.limits['maxNumberBytes'])) if type(value) is int else value
            if len(token) > self.limits['maxNumberBytes']:
                self.failure(source, path, 'numeric operand budget exhausted')
            if token not in self._numbers:
                self._numbers[token] = Exact.parse(token)
            return self._numbers[token]
        except _json.JsonError as error:
            self.failure(source, path, error.message)

    def check(self, root_index: int, value: Any) -> None:
        if not self.scoped:
            return super().check(root_index, value)
        if type(root_index) is not int or root_index not in self.roots:
            raise ValidationError('evaluation_failure', '', '', 'root was not selected')
        self.findings = []
        active: set[tuple[int, int]] = set()
        stack: list[_Frame] = []

        def push(request: _Request) -> None:
            node = self.nodes[request.index]
            self.spend(node['source'], request.path)
            if len(stack) >= self.limits['maxDepth']:
                self.failure(node['source'], request.path, 'evaluation depth budget exhausted')
            marker = request.value if request.identity is None else request.identity
            identity = request.index, id(marker)
            if identity in active:
                self.failure(node['source'], request.path, 'nonproductive recursive evaluation')
            active.add(identity)
            findings = self.findings if request.trial else None
            if request.trial:
                self.findings = []
            stack.append(_Frame(self.scoped_node(node, request.value, request.path, marker), identity, findings))

        result = _Evaluated(False)
        push(_Request(root_index, value, ''))
        try:
            while stack:
                frame = stack[-1]
                try:
                    if frame.started:
                        request = frame.iterator.send(result)
                    else:
                        frame.started = True
                        request = next(frame.iterator)
                except StopIteration as complete:
                    result = complete.value
                    active.remove(frame.identity)
                    stack.pop()
                    if frame.findings is not None:
                        self.findings = frame.findings
                else:
                    push(request)
        finally:
            # Only internal generators are closed; no detached tasks or global
            # recursion-state mutation are needed for deep schemas/instances.
            for frame in reversed(stack):
                frame.iterator.close()
        if not result.valid:
            if self.findings:
                raise self.findings[0]
            raise ValidationError('invalid', _v1._loc(self.nodes[root_index]['source']), '', 'source schema rejected the value')

    def scoped_node(self, node: dict[str, Any], value: Any, path: str, marker: object) -> Generator[_Request, _Evaluated, _Evaluated]:
        local = _Annotations()
        valid = True
        for check in node['checks']:
            at, op = check['source'], check['op']
            self.spend(at, path)
            produced = _Annotations()
            ok = True
            if op == 'ref':
                result = yield _Request(integer(check['target']), value, path, identity=marker)
                ok, produced = result.valid, result.annotations
            elif op in ('allOf', 'anyOf', 'oneOf'):
                passing: list[_Annotations] = []
                all_valid = True
                for target in check['targets']:
                    self.spend(at, path)
                    result = yield _Request(integer(target), value, path, op != 'allOf', marker)
                    all_valid = result.valid and all_valid
                    if result.valid:
                        passing.append(result.annotations)
                ok = all_valid if op == 'allOf' else bool(passing) if op == 'anyOf' else len(passing) == 1
                if ok:
                    for annotations in passing:
                        self.merge(produced, annotations, at, path)
                elif op != 'allOf':
                    self.mismatch(at, path, op + ' branch count rejected the value')
            elif op == 'not':
                result = yield _Request(integer(check['target']), value, path, True, marker)
                ok = not result.valid
                if not ok:
                    self.mismatch(at, path, 'negated schema accepted the value')
            elif op == 'if':
                condition = yield _Request(integer(check['condition']), value, path, True, marker)
                target = check['thenTarget'] if condition.valid else check['elseTarget']
                if condition.valid:
                    self.merge(local, condition.annotations, at, path)
                if target is not None:
                    result = yield _Request(integer(target), value, path, identity=marker)
                    ok, produced = result.valid, result.annotations
            elif op == 'dependentRequired':
                if type(value) is dict:
                    for trigger, names in check['dependencies']:
                        self.spend(at, path)
                        if trigger in value:
                            for name in names:
                                self.spend(at, path)
                                if name not in value:
                                    ok = self.mismatch(child(at, trigger), path, 'dependent required property is absent')
            elif op == 'dependentSchemas':
                passing = []
                if type(value) is dict:
                    for dependency in check['dependencies']:
                        self.spend(at, path)
                        if dependency['name'] in value:
                            result = yield _Request(integer(dependency['target']), value, path, identity=marker)
                            ok = result.valid and ok
                            if result.valid:
                                passing.append(result.annotations)
                if ok:
                    for annotations in passing:
                        self.merge(produced, annotations, at, path)
            elif op == 'contains':
                if type(value) is list:
                    for index, item in enumerate(value):
                        self.spend(at, path)
                        result = yield _Request(integer(check['target']), item, _v1._child(path, str(index)), True)
                        if result.valid:
                            produced.items.add(index)
                    count = len(produced.items)
                    minimum = Exact.parse(check['minimum']) if check['minimum'] is not None else Exact.parse('1')
                    maximum = Exact.parse(check['maximum']) if check['maximum'] is not None else None
                    if count or minimum.sign == 0:
                        self.merge(local, produced, at, path)
                    produced = _Annotations()
                    actual = Exact.parse(str(count))
                    lower = actual.compare(minimum) >= 0
                    upper = maximum is None or actual.compare(maximum) <= 0
                    if not lower:
                        self.mismatch(child(node['source'], 'minContains') if check['minimum'] is not None else at, path, 'fewer contains matches than required')
                    if not upper:
                        self.mismatch(child(node['source'], 'maxContains'), path, 'more contains matches than allowed')
                    ok = lower and upper
            elif op == 'properties':
                if type(value) is dict:
                    for property in check['properties']:
                        self.spend(at, path)
                        name = property['name']
                        if name in value:
                            result = yield _Request(integer(property['target']), value[name], _v1._child(path, name))
                            ok = result.valid and ok
                            produced.properties.add(name)
            elif op == 'patternProperties':
                if type(value) is dict:
                    for name in sorted(value):
                        self.spend(at, path)
                        for _, pattern, target in check['patterns']:
                            self.spend(at, path)
                            if self.pattern(pattern, name, at, path):
                                result = yield _Request(integer(target), value[name], _v1._child(path, name))
                                ok = result.valid and ok
                                produced.properties.add(name)
            elif op in ('additionalProperties', 'additionalPropertiesWithPatterns'):
                if type(value) is dict:
                    patterns: list[Any] = next((item['patterns'] for item in node['checks'] if item['op'] == 'patternProperties'), []) if op == 'additionalPropertiesWithPatterns' else []
                    declared = set(check['declared'])
                    for name in sorted(value):
                        self.spend(at, path)
                        if name in declared:
                            continue
                        excluded = False
                        for _, pattern, _ in patterns:
                            self.spend(at, path)
                            if self.pattern(pattern, name, at, path):
                                excluded = True
                                break
                        if not excluded:
                            result = yield _Request(integer(check['target']), value[name], _v1._child(path, name))
                            ok = result.valid and ok
                            produced.properties.add(name)
            elif op == 'propertyNames':
                if type(value) is dict:
                    for name in sorted(value):
                        self.spend(at, path)
                        # A temporary name has its own identity, retained through
                        # same-instance references/branches in the child scope.
                        result = yield _Request(integer(check['target']), name, _v1._child(path, name), identity=object())
                        ok = result.valid and ok
            elif op in ('items', 'prefixItems'):
                if type(value) is list:
                    indices = range(integer(check['start']), len(value)) if op == 'items' else range(min(len(check['targets']), len(value)))
                    for index in indices:
                        self.spend(at, path)
                        target = check['target'] if op == 'items' else check['targets'][index]
                        result = yield _Request(integer(target), value[index], _v1._child(path, str(index)))
                        ok = result.valid and ok
                        produced.items.add(index)
            elif op == 'unevaluatedProperties':
                if type(value) is dict:
                    for name in sorted(value):
                        self.spend(at, path)
                        if name not in local.properties:
                            result = yield _Request(integer(check['target']), value[name], _v1._child(path, name))
                            ok = result.valid and ok
                            produced.properties.add(name)
            elif op == 'unevaluatedItems':
                if type(value) is list:
                    for index, item in enumerate(value):
                        self.spend(at, path)
                        if index not in local.items:
                            result = yield _Request(integer(check['target']), item, _v1._child(path, str(index)))
                            ok = result.valid and ok
                            produced.items.add(index)
            elif op == 'required':
                if type(value) is dict:
                    for name in check['names']:
                        self.spend(at, path)
                        if name not in value:
                            ok = self.mismatch(at, path, 'required property is absent')
            else:
                ok = self.scalar(check, value, path)
            valid = ok and valid
            if ok:
                self.merge(local, produced, at, path)
        return _Evaluated(valid, local if valid else _Annotations())

    def equal(self, left: Any, right: Any, source: dict[str, Any], path: str) -> bool:
        if not self.scoped:
            return super().equal(left, right, source, path)
        pending = [(left, right, 0)]
        while pending:
            a, b, depth = pending.pop()
            self.equalities -= 1
            if self.equalities < 0 or depth > self.limits['maxDepth']:
                self.failure(source, path, 'equality budget exhausted')
            kind = _v1._kind(a)
            if kind != _v1._kind(b):
                return False
            if kind == 'number':
                if self.number(a, source, path).compare(self.number(b, source, path)) != 0:
                    return False
            elif kind == 'array':
                if len(a) != len(b):
                    return False
                pending.extend((a[index], b[index], depth + 1) for index in reversed(range(len(a))))
            elif kind == 'object':
                if a.keys() != b.keys():
                    return False
                pending.extend((a[name], b[name], depth + 1) for name in sorted(a, reverse=True))
            elif kind == 'invalid' or a != b:
                return False
        return True

    def pattern(self, program: dict[str, Any], text: str, source: dict[str, Any], path: str) -> bool:
        if not self.scoped:
            return super().pattern(program, text, source, path)
        states = program['states']
        seeds: list[int] = []
        for position in range(len(text) + 1):
            self.spend(source, path)
            seen: set[int] = set()
            stack: list[int] = []
            active: list[int] = []
            def enqueue(index: int) -> None:
                self.spend(source, path)
                if index not in seen:
                    seen.add(index)
                    stack.append(index)
            enqueue(integer(program['start']))
            for index in seeds:
                enqueue(index)
            while stack:
                self.spend(source, path)
                index = stack.pop()
                state = states[index]
                op = state['op']
                if op == 'match':
                    return True
                if op == 'split':
                    enqueue(integer(state['second']))
                    enqueue(integer(state['first']))
                elif op == 'jump' or op == 'start' and position == 0 or op == 'end' and position == len(text):
                    enqueue(integer(state['target']))
                elif op == 'char':
                    # Preserve compiled DFS order; range visits share steps.
                    active.append(index)
            if position == len(text):
                return False
            seeds = []
            code = ord(text[position])
            for index in active:
                state = states[index]
                for low, high in state['ranges']:
                    self.spend(source, path)
                    if code < integer(low):
                        break
                    if code <= integer(high):
                        seeds.append(integer(state['target']))
                        break
        return False

    def scalar(self, check: dict[str, Any], value: Any, path: str) -> bool:
        if not self.scoped:
            return super().scalar(check, value, path)
        at, op = check['source'], check['op']
        if op == 'enum':
            for candidate in check['values']:
                self.spend(at, path)
                if self.equal(value, candidate, at, path):
                    return True
            return self.mismatch(at, path, 'enum assertion rejected the value')
        if op == 'uniqueItems' and type(value) is list:
            for index, item in enumerate(value):
                self.spend(at, path)
                for previous in value[:index]:
                    self.spend(at, path)
                    if self.equal(item, previous, at, path):
                        return self.mismatch(at, path, 'uniqueItems assertion rejected the value')
            return True
        return super().scalar(check, value, path)


def validate(root_index: int, value: Any) -> None:
    ValidationSession().check(root_index, value)
