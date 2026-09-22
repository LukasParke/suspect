from __future__ import annotations

import copy
import json
import sys
from pathlib import Path
from typing import Any, Callable, TYPE_CHECKING
from unittest.mock import patch
if TYPE_CHECKING or __package__:
    from .json_runtime import JsonNumber, parse_json
    from .validation import ValidationSession, ValidationError, ProgramError
else:
    from json_runtime import JsonNumber, parse_json
    from validation import ValidationSession, ValidationError, ProgramError

data = json.loads(Path('vectors.json').read_text(), parse_int=JsonNumber, parse_float=JsonNumber)
records = data['cases']
before = sys.getrecursionlimit()
for record in records:
    session = ValidationSession(record['program'])
    with patch('urllib.request.urlopen', side_effect=AssertionError('runtime acquisition')), patch('socket.create_connection', side_effect=AssertionError('runtime connection')):
        try:
            session.check(record['root'].to_int(), parse_json(record['instance']))
        except ValidationError as error:
            actual = 'EvaluationFailure' if error.kind == 'evaluation_failure' else 'Invalid'
            assert actual == record['expected'], (record['id'], actual, error)
            if record['source'] is not None:
                assert error.source == 'https://physical.example.test/api.json#' + record['source'], (record['id'], error)
                assert error.instance_path == record['path'], (record['id'], error)
            else:
                assert error.source.startswith(('https://physical.example.test/', 'http://localhost:1234/')), error
        else:
            assert record['expected'] == 'Valid', record['id']
        assert session._scope == (), ('resource scope leaked', record['id'])
assert sys.getrecursionlimit() == before

def example(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(record['program'] for record in records if record['id'] == name))

def dynamic(program: dict[str, Any]) -> dict[str, Any]:
    return next(check for node in program['nodes'] for check in node['checks'] if check['op'] == 'dynamicRef')

def resource(program: dict[str, Any]) -> dict[str, Any]:
    return next(resource for resource in program['resourceContext']['resources'] if resource['dynamicAnchors'])

base = example('strict-tree-unentered-candidate')
mutations: list[tuple[str, Callable[[dict[str, Any]], None]]] = [
    ('missing-context', lambda p: p.pop('resourceContext')),
    ('null-context', lambda p: p.update(resourceContext=None)),
    ('wrong-profile', lambda p: p.update(profile='wrong')),
    ('legacy-envelope', lambda p: p.update(version='suspect.validation.experimental.v2',profile='oas31-jsonschema202012-static-applicators')),
    ('node-scope-count', lambda p: p['resourceContext']['nodeScopes'].pop()),
    ('node-resource-target', lambda p: p['resourceContext']['nodeScopes'][0].__setitem__(0,99999)),
    ('node-address', lambda p: p['resourceContext']['nodeScopes'][0].__setitem__(2,'urn:invented')),
    ('node-physical-root', lambda p: p['resourceContext']['nodeScopes'][0][1].update(document='https://other.test/source')),
    ('duplicate-resource', lambda p: p['resourceContext']['resources'].append(copy.deepcopy(resource(p)))),
    ('resource-kind', lambda p: resource(p).update(kind='invented')),
    ('resource-base-fragment', lambda p: resource(p).update(baseUri='urn:base#bad')),
    ('canonical-base', lambda p: resource(p).update(canonicalUri='urn:other')),
    ('resource-alias-missing', lambda p: resource(p).update(aliases=[])),
    ('resource-alias-invalid', lambda p: resource(p)['aliases'].append('urn:bad#%FF')),
    ('resource-alias-duplicate', lambda p: resource(p)['aliases'].append(resource(p)['aliases'][0])),
    ('declaration-location', lambda p: resource(p)['declarationSource'].update(pointer='/elsewhere/$id')),
    ('anchor-location', lambda p: resource(p)['dynamicAnchors'][0][1].update(pointer='/invented/$dynamicAnchor')),
    ('anchor-target', lambda p: resource(p)['dynamicAnchors'][0].__setitem__(2,99999)),
    ('anchor-name', lambda p: resource(p)['dynamicAnchors'][0].__setitem__(0,'not/a/name')),
    ('anchor-duplicate', lambda p: resource(p)['dynamicAnchors'].append(copy.deepcopy(resource(p)['dynamicAnchors'][0]))),
    ('initial-resource', lambda p: dynamic(p).update(initialResource=99999)),
    ('dynamic-target', lambda p: dynamic(p).update(target=True)),
    ('dynamic-name', lambda p: dynamic(p).update(anchor='missing')),
    ('dynamic-source', lambda p: dynamic(p)['source'].update(pointer='/invented/$dynamicRef')),
    ('dynamic-operand', lambda p: dynamic(p).update(extra='ignored')),
]
for name, mutate in mutations:
    program = copy.deepcopy(base)
    mutate(program)
    try:
        ValidationSession(program)
    except ProgramError:
        pass
    else:
        raise AssertionError(('malformed v3 program admitted', name))

# A guard must reject dynamic operands in both older profiles without metadata.
for version, profile in [('v1','oas31-jsonschema202012-static-subset'),('v2','oas31-jsonschema202012-static-applicators')]:
    program = copy.deepcopy(base)
    program.pop('resourceContext')
    program.update(version='suspect.validation.experimental.' + version,profile=profile)
    try:
        ValidationSession(program)
    except ProgramError:
        pass
    else:
        raise AssertionError(('dynamic program in legacy profile', version))

# Own metadata and restore scopes across successful roots and failed trials.
program = example('nested-entry-enters-indexed-parent')
session = ValidationSession(program)
for item in program['resourceContext']['resources']:
    item['dynamicAnchors'] = []
selected = program['roots'][0]['target'].to_int()
session.check(selected, parse_json('7'))
try:
    session.check(selected, parse_json('"wrong"'))
except ValidationError as error:
    assert error.kind == 'invalid' and error.source.endswith('/Root/$defs/binding/type'), error
else:
    raise AssertionError('caller mutation altered resource bindings')
assert session._scope == ()
print(json.dumps({'python':sys.version,'official':44,'vectors':len(records),'guards':len(mutations)+2,'owned_metadata':True,'scope_restore':True,'normal_stack_depth':True,'runtime_acquisition':False}))
