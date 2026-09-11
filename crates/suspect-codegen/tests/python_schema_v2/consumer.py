from __future__ import annotations

import asyncio
import importlib.util
import json
import sys
import types
import typing
from collections.abc import Callable
from pathlib import Path
from typing import Any
from unittest.mock import patch

import httpx
import scoped_python_sdk as sdk
from scoped_python_sdk import (
    AsyncClient, Client, CodecError, JsonNumber, JsonValue, SdkError, UNSET,
    codecs, models, operations,
)
from scoped_python_sdk.json_runtime import parse_json, stringify_json

ROOT = Path(__file__).resolve().parent
assert 'site-packages' in str(Path(sdk.__file__).resolve()), sdk.__file__
assert Path(sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
fixture = json.loads((ROOT / 'fixture.json').read_text(), parse_int=JsonNumber, parse_float=JsonNumber)
requests: list[tuple[str, bytes]] = []


def expect_error(action: Callable[[], object], pointer: str, path: str, kind: str = 'invalid') -> None:
    try:
        action()
    except CodecError as error:
        assert error.kind == kind, error
        assert error.source.endswith('#/components/schemas/' + pointer), error
        assert error.path == path, error
    else:
        raise AssertionError(('accepted invalid native value', pointer, path))


def echo(request: httpx.Request) -> httpx.Response:
    assert request.method == 'POST'
    requests.append((request.url.path, request.content))
    return httpx.Response(200, headers={'Content-Type': 'application/json'}, content=request.content)


def pattern() -> models.PatternBag:
    result = models.PatternBag(fixed='label')
    result.set_extra('xt', JsonNumber('1.2300e2'))
    result.set_extra('xzero', JsonNumber('-0'))
    return result


def residual() -> models.PatternResidual:
    result = models.PatternResidual(known='fixed')
    result.set_extra('x', JsonNumber('1e0'))
    result.set_extra('other', 'residual')
    return result


def conditional_extra() -> models.ConditionalExtra:
    result = models.ConditionalExtra()
    result.set_extra('kind', 'x')
    result.set_extra('value', 'kept')
    return result


def intersection() -> models.Intersection:
    result = models.Intersection()
    result.set_extra('a', 1)
    result.set_extra('b', 'kept')
    return result


def union() -> models.Choice:
    result = models.ChoiceAnyOf0(a=1)
    result.set_extra('b', 'kept')
    return result


conditional = models.Conditional(kind='s', text='native', flag=None, amount=JsonNumber('9007199254740993.000000000000000001'))
dependency = models.Dependency(card=None, billing='address', enabled=False, peer=1)
items: models.TupleItems = ['head', JsonNumber('1.0'), 2]
reference: models.RefSibling = {'name': 'native', 'stamp': None}
maybe: models.MaybeObject = {'value': JsonNumber('1.2300')}
literal: models.CompoundConst = {'type': 'literal-data', 'required': ['not-keywords'], 'value': JsonNumber('1.23')}
null_fields = models.NullFields(required=None, flag=None, items=[None], same=None, choice=None)

with httpx.MockTransport(echo) as transport:
    with Client(transport=transport) as client:
        first: operations.WriteConditionalSuccess = client.write_conditional(body=conditional)
        assert first.data.flag is None
        assert first.data.count is UNSET
        assert isinstance(first.data.amount, JsonNumber)
        assert first.data.amount.token == '9007199254740993.000000000000000001'
        assert first.__class__.__module__ == 'scoped_python_sdk.operations'
        assert client.write_dependency(body=dependency).data.card is None
        returned = client.write_patterns(body=pattern()).data
        assert set(returned.extra_fields) == {'xt', 'xzero'}
        extra = returned.extra_fields['xt']
        assert isinstance(extra, JsonNumber) and extra.token == '1.2300e2'
        zero = returned.extra_fields['xzero']
        assert isinstance(zero, JsonNumber) and zero.token == '-0'
        assert client.write_pattern_residual(body=residual()).data.extra_fields['other'] == 'residual'
        tuple_result = client.write_tuple(body=items).data
        assert tuple_result[0] == 'head' and isinstance(tuple_result[1], JsonNumber)
        assert tuple_result[1].token == '1.0'
        assert client.write_condition_scope(body=conditional_extra()).data.extra_fields['kind'] == 'x'
        choice = client.write_union(body=union()).data
        assert isinstance(choice, models.ChoiceAnyOf0) and choice.extra_fields['b'] == 'kept'
        assert client.write_intersection(body=intersection()).data.extra_fields['b'] == 'kept'
        assert isinstance(client.write_ref_sibling(body=reference).data, dict)
        assert isinstance(client.write_nullable(body=maybe).data, dict)
        assert isinstance(client.write_literal(body=literal).data, dict)
        assert client.write_null_fields(body=null_fields).data.same is None
        assert len(requests) == 12
        assert b'1.2300e2' in requests[2][1] and b'"xzero":-0' in requests[2][1]
        assert b'"flag":null' in requests[0][1] and b'"count"' not in requests[0][1]

        # Source assertions are checked before sending every mutable native input.
        before = len(requests)
        conditional.text = UNSET
        expect_error(lambda: client.write_conditional(body=conditional), 'Conditional/then/required', '')
        conditional.text = 'native'
        dependency.billing = UNSET
        expect_error(lambda: client.write_dependency(body=dependency), 'Dependency/dependentRequired/card', '')
        dependency.billing = 'address'
        dependency.peer = 0
        expect_error(lambda: client.write_dependency(body=dependency), 'Dependency/dependentSchemas/enabled/properties/peer/minimum', '/peer')
        dependency.peer = 1
        bag = pattern()
        bag.set_extra('xt', 0)
        expect_error(lambda: client.write_patterns(body=bag), 'PatternBag/patternProperties/t$/minimum', '/xt')
        bag = pattern()
        bag.set_extra('rogue', 1)
        expect_error(lambda: client.write_patterns(body=bag), 'PatternBag/additionalProperties', '/rogue')
        bag = pattern()
        bag.set_extra('x_toolong', 1)
        expect_error(lambda: client.write_patterns(body=bag), 'PatternBag/propertyNames/maxLength', '/x_toolong')
        extra_bag = residual()
        extra_bag.set_extra('other', 2)
        expect_error(lambda: client.write_pattern_residual(body=extra_bag), 'PatternResidual/additionalProperties/type', '/other')
        expect_error(lambda: client.write_tuple(body=['head', 1, 'tail']), 'TupleItems/unevaluatedItems', '/2')
        expect_error(lambda: client.write_tuple(body=['head']), 'TupleItems/minContains', '')
        expect_error(lambda: client.write_tuple(body=['head', 1, 2, 3]), 'TupleItems/maxContains', '')
        expect_error(lambda: client.write_ref_sibling(body={'stamp': None}), 'RefSibling/dependentRequired/stamp', '')
        expect_error(lambda: client.write_nullable(body={'value': 'wrong'}), 'MaybeObject/properties/value/type', '/value')
        expect_error(lambda: client.write_literal(body={'type': 'literal-data'}), 'CompoundConst/const', '')
        assert len(requests) == before
        assert client.write_nullable(body=None).data is None
        assert client.write_dependency(body=models.Dependency()).data.card is UNSET
        assert client.write_conditional(body=models.Conditional(kind='n', count=2)).data.count == 2


async def async_operations() -> None:
    with httpx.MockTransport(echo) as transport:
        async with AsyncClient(transport=transport) as client:
            response: operations.WriteConditionalSuccess = await client.write_conditional(body=conditional)
            assert response.data.text == 'native'
            await client.write_dependency(body=dependency)
            await client.write_patterns(body=pattern())
            await client.write_pattern_residual(body=residual())
            await client.write_tuple(body=items)
            await client.write_condition_scope(body=conditional_extra())
            await client.write_union(body=union())
            await client.write_intersection(body=intersection())
            await client.write_ref_sibling(body=reference)
            await client.write_nullable(body=maybe)
            await client.write_literal(body=literal)
            await client.write_null_fields(body=null_fields)


asyncio.run(async_operations())

# Every declared negative is rejected on decode and as an actual HTTP response.
for case in fixture['cases']:
    codec = getattr(codecs, case['name'] + 'Codec')
    try:
        codec.decode_value(case['bad'])
    except CodecError as error:
        assert error.kind == 'invalid' and error.source.startswith('file:'), error
    else:
        raise AssertionError(('bad example passed decode', case['name']))
    body = codec.decode_value(case['good'])
    def invalid_response(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, headers={'Content-Type': 'application/json'}, content=stringify_json(case['bad']))
    method = next(operation['method'] for operation in json.loads((Path(sdk.__file__).parent / 'http-manifest.json').read_text())['operations'] if operation['operationId'] == case['operation'])
    with httpx.MockTransport(invalid_response) as transport:
        with Client(transport=transport) as client:
            try:
                getattr(client, method)(body=body)
            except SdkError as error:
                assert error.kind == 'response-decoding', error
                assert isinstance(error.cause, CodecError) and error.cause.kind == 'invalid'
            else:
                raise AssertionError(('bad response passed', case['name']))

# JSON carriers own their decoded values, including pattern extras and arrays.
original = parse_json('{"fixed":"native","x":1.2500}')
decoded = codecs.PatternBagCodec.decode_value(original)
assert isinstance(original, dict)
original['x'] = 'mutated'
kept = decoded.extra_fields['x']
assert isinstance(kept, JsonNumber) and kept.token == '1.2500'
mapping: Any = decoded.extra_fields
try:
    mapping['x'] = 'mutated'
except TypeError:
    pass
else:
    raise AssertionError('extra_fields must expose a read-only mapping')
output = codecs.TupleItemsCodec.encode_value(items)
items.append(3)
assert isinstance(output, list) and len(output) == 3
items.pop()
cyclic: list[JsonValue] = ['head', 1]
cyclic.append(cyclic)
expect_error(lambda: codecs.TupleItemsCodec.encode(cyclic), 'TupleItems', '/2', 'conversion')

# The handed-off null-only annotation/alias rules apply inside a scoped package.
null: models.Null = None
assert models.Null is type(None)
assert sdk.ValidationError is sdk.validation.ValidationError
assert sdk.ValidationError.__module__ == 'scoped_python_sdk.validation'
assert type(None) in typing.get_args(typing.get_type_hints(models.NullFields)['flag'])
absent = models.NullFields(required=null)
present = models.NullFields(required=null, flag=None)
absent_wire = codecs.NullFieldsCodec.encode_value(absent)
present_wire = codecs.NullFieldsCodec.encode_value(present)
assert isinstance(absent_wire, dict) and isinstance(present_wire, dict)
assert 'flag' not in absent_wire and present_wire['flag'] is None


def guide(name: str) -> types.ModuleType:
    spec = importlib.util.spec_from_file_location('guide_' + name, ROOT / 'generated/python/examples' / (name + '.py'))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


guide('transport').mock_request()
guide('numbers_example').exact_numbers()
with httpx.MockTransport(echo) as transport:
    def sync_client(*args: Any, **kwargs: Any) -> Client:
        return Client(*args, transport=transport, **kwargs)
    def async_client(*args: Any, **kwargs: Any) -> AsyncClient:
        return AsyncClient(*args, transport=transport, **kwargs)
    quickstart = guide('quickstart')
    with patch.object(quickstart, 'Client', sync_client):
        assert quickstart.first_request('fixture').status == 200
    asynchronous = guide('async_client')
    with patch.object(asynchronous, 'AsyncClient', async_client):
        assert asyncio.run(asynchronous.first_request_async('fixture')).status == 200
    errors = guide('errors')
    with patch.object(errors, 'Client', sync_client):
        assert errors.request_with_errors('fixture').status == 200
    with Client(transport=transport) as client:
        guide('presence').send_variants(client)

print(json.dumps({'python': sys.version, 'installed': sdk.__file__, 'operations': 12, 'sync_async_requests_and_guides': len(requests), 'invalid_responses': 12, 'negative_mutations': 13, 'null_annotations': 'passed', 'owned_carriers': 'passed', 'native_guides': 'passed'}))
