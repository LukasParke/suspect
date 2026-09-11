"""Indexed resource scope and dynamic lookup over the frozen v2 value rules."""
from __future__ import annotations

import dataclasses
from collections.abc import Generator
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING or __package__:
    from . import validation_v1 as _v1, validation_v2 as _v2
    from .validation_guard import integer, child, snapshot
    from .validation_resource_guard import V3, check_program, ProgramError as ProgramError
    from .validation_number import Exact
else:
    import validation_v1 as _v1
    import validation_v2 as _v2
    from validation_guard import integer, child, snapshot
    from validation_resource_guard import V3, check_program, ProgramError as ProgramError
    from validation_number import Exact

ValidationError = _v1.ValidationError
ValidationError.__module__ = __name__
_Annotations = _v2._Annotations
_Evaluated = _v2._Evaluated
_Request = _v2._Request


@dataclasses.dataclass
class _Frame:
    iterator: Generator[_Request, _Evaluated, _Evaluated]
    identity: tuple[int, int, tuple[int, ...]]
    previous: tuple[int, ...]
    findings: list[ValidationError] | None
    started: bool = False


class ValidationSession(_v2.ValidationSession):
    """Shared finite budgets with an exact, ordered, actually-entered resource stack."""
    def __init__(self, program: dict[str, Any] | None = None) -> None:
        program = _v1._load() if program is None else program
        limits = check_program(program)
        self.resources: dict[str, Any] | None = None
        self._scope: tuple[int, ...] = ()
        if (program['version'], program['profile']) != V3:
            super().__init__(program)
            return
        self.program = snapshot(program)
        self.resources = self.program['resourceContext']
        self.nodes = self.program['nodes']
        self.roots = {integer(root['target']) for root in self.program['roots']}
        self.limits = limits
        self.steps = limits['maxEvaluationSteps']
        self.equalities = limits['maxEqualitySteps']
        self.numeric = limits['maxEvaluationSteps']
        self.scoped = True
        self._numbers: dict[str, Exact] = {}
        self.findings: list[ValidationError] = []

    def check(self, root_index: int, value: Any) -> None:
        if self.resources is None:
            return super().check(root_index, value)
        if type(root_index) is not int or root_index not in self.roots:
            raise ValidationError('evaluation_failure', '', '', 'root was not selected')
        self.findings = []
        self._scope = ()
        active: set[tuple[int, int, tuple[int, ...]]] = set()
        stack: list[_Frame] = []
        scopes = self.resources['nodeScopes']

        def push(request: _Request) -> None:
            node = self.nodes[request.index]
            self.spend(node['source'], request.path)
            if len(stack) >= self.limits['maxDepth']:
                self.failure(node['source'], request.path, 'evaluation depth budget exhausted')
            previous = self._scope
            resource = integer(scopes[request.index][0])
            if resource not in self._scope:
                self.spend(node['source'], request.path)
                self._scope += (resource,)
            marker = request.value if request.identity is None else request.identity
            identity = request.index, id(marker), self._scope
            if identity in active:
                self._scope = previous
                self.failure(node['source'], request.path, 'nonproductive recursive evaluation in the same resource context')
            active.add(identity)
            findings = self.findings if request.trial else None
            if request.trial:
                self.findings = []
            stack.append(_Frame(self.scoped_node(node, request.value, request.path, marker), identity, previous, findings))

        result = _Evaluated(False)
        try:
            push(_Request(root_index, value, ''))
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
                    self._scope = frame.previous
                    if frame.findings is not None:
                        self.findings = frame.findings
                else:
                    push(request)
        finally:
            for frame in reversed(stack):
                frame.iterator.close()
                if frame.findings is not None:
                    self.findings = frame.findings
            self._scope = ()
        if not result.valid:
            if self.findings:
                raise self.findings[0]
            raise ValidationError('invalid', _v1._loc(self.nodes[root_index]['source']), '', 'source schema rejected the value')

    def _dynamic_target(self, check: dict[str, Any], path: str) -> int:
        target = integer(check['target'])
        name = check['anchor']
        if name is not None:
            assert self.resources is not None
            for index in self._scope:
                self.spend(check['source'], path)
                for candidate, _, binding in self.resources['resources'][index]['dynamicAnchors']:
                    self.spend(check['source'], path)
                    if candidate == name:
                        return integer(binding)
        return target

    # Retain the v2 opcode rules verbatim in this additive dispatcher. The frozen
    # v2 template remains byte-identical; DynamicRef adds only target selection.
    def scoped_node(self, node: dict[str, Any], value: Any, path: str, marker: object) -> Generator[_Request, _Evaluated, _Evaluated]:
        local = _Annotations()
        valid = True
        for check in node['checks']:
            at, op = check['source'], check['op']
            self.spend(at, path)
            produced = _Annotations()
            ok = True
            if op in ('ref', 'dynamicRef'):
                target = self._dynamic_target(check, path) if op == 'dynamicRef' else integer(check['target'])
                result = yield _Request(target, value, path, identity=marker)
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


def validate(root_index: int, value: Any) -> None:
    ValidationSession().check(root_index, value)
