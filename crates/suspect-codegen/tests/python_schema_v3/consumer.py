from __future__ import annotations
import asyncio
import importlib.util
import json
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any
import httpx
import resource_python_sdk as sdk
from resource_python_sdk import Client, AsyncClient, CodecError, JsonNumber, SdkError, codecs, models, operations
from resource_python_sdk.json_runtime import parse_json, stringify_json

ROOT = Path(__file__).resolve().parent
assert Path(sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert 'site-packages' in sdk.__file__
requests: list[tuple[str,bytes]] = []
def echo(request: httpx.Request) -> httpx.Response:
    assert request.url.host == 'cdn.example'
    assert request.url.path.startswith('/api/')
    requests.append((request.url.path, request.content))
    if request.url.path.endswith('/bytes'):
        return httpx.Response(200, headers={'content-type':'application/octet-stream'}, content=b'\x00\xff')
    return httpx.Response(200, headers={'content-type':'application/json'}, content=request.content)

def invalid(action: Callable[[], object], source: str, path: str) -> None:
    before = len(requests)
    try:
        action()
    except CodecError as error:
        assert error.kind == 'invalid', error
        assert error.source == 'https://cdn.example/specs/api.json#/components/schemas/' + source, error
        assert error.path == path, error
    else:
        raise AssertionError(('invalid resource input accepted', source, path))
    assert len(requests) == before

tree: models.Strict = {'data':'root','children':[{'data':JsonNumber('1.2300')}]}
override: models.Override = {'payload':9007199254740993}
nested: models.NestedUse = 7
fallbacks: models.StaticFallbacks = {'dynamic':1,'pointer':'s','plain':'s','empty':'s','static':'s'}
number: models.Number = JsonNumber('9007199254740993.000000000000000001')
array: models.DynamicArray = ['head',JsonNumber('1.0'),2]
cycle: models.ContextCycle = {'null':None,'exact':JsonNumber('1.2300')}

with httpx.MockTransport(echo) as transport:
    with Client(transport=transport) as client:
        result: operations.WriteOverrideSuccess = client.write_override(body=override)
        assert isinstance(result.data,dict)
        payload = result.data['payload']
        assert isinstance(payload,JsonNumber) and payload.to_int() == 9007199254740993
        assert result.__class__.__module__ == 'resource_python_sdk.operations'
        strict = client.write_strict_tree(body=tree).data
        assert isinstance(strict,dict) and b'1.2300' in requests[-1][1]
        start = client.write_nested(body=nested).data
        assert isinstance(start,JsonNumber) and start.to_int() == 7
        client.write_fallbacks(body=fallbacks)
        amount = client.write_number(body=number).data
        assert isinstance(amount,JsonNumber) and amount.token == number.token
        client.write_array(body=array)
        client.write_context_cycle(body=cycle)
        assert client.resource_bytes().data == b'\x00\xff'
        invalid(lambda: client.write_override(body={'payload':'s'}),'Override/$defs/Binding/type','/payload')
        invalid(lambda: client.write_override(body={'payload':9007199254740992}),'Override/$defs/Binding/minimum','/payload')
        invalid(lambda: client.write_nested(body='s'),'Nested/$defs/Binding/type','')
        invalid(lambda: client.write_strict_tree(body={'children':[{'unexpected':1}]}),'Strict/unevaluatedProperties','/children/0/unexpected')
        invalid(lambda: client.write_fallbacks(body={'pointer':1}),'Fallback/$defs/Text/type','/pointer')
        invalid(lambda: client.write_fallbacks(body={'plain':1}),'Fallback/$defs/Text/type','/plain')
        invalid(lambda: client.write_fallbacks(body={'static':1}),'Fallback/$defs/Text/type','/static')
        invalid(lambda: client.write_fallbacks(body={'empty':1}),'Plain/type','/empty')
        invalid(lambda: client.write_array(body=['head',1,'extra']),'DynamicArray/unevaluatedItems','/2')
        invalid(lambda: client.write_number(body=-1),'Number/minimum','')

async def asynchronous() -> None:
    with httpx.MockTransport(echo) as transport:
        async with AsyncClient(transport=transport) as client:
            result: operations.WriteOverrideSuccess = await client.write_override(body=override)
            assert isinstance(result.data,dict)
            await client.write_strict_tree(body=tree)
            await client.write_nested(body=nested)
            await client.write_fallbacks(body=fallbacks)
            await client.write_number(body=number)
            await client.write_array(body=array)
            await client.write_context_cycle(body=cycle)
            assert (await client.resource_bytes()).data == b'\x00\xff'
asyncio.run(asynchronous())

# Mutations are revalidated; decoded carriers own their nested exact values.
owned = codecs.StrictCodec.decode_value(tree)
assert isinstance(owned,dict) and isinstance(tree,dict)
tree['children'] = [{'unexpected':1}]
assert 'unexpected' not in codecs.StrictCodec.encode(owned)
invalid(lambda: codecs.StrictCodec.encode(tree),'Strict/unevaluatedProperties','/children/0/unexpected')
override['payload'] = 's'
invalid(lambda: codecs.OverrideCodec.encode(override),'Override/$defs/Binding/type','/payload')

# Same dynamic entry under changed contexts succeeds; its unentered root const
# and fallback string schema are not detached codec checks.
assert codecs.NestedUseCodec.encode(nested) == '7'
assert codecs.ContextCycleCodec.encode(None) == 'null'
assert codecs.ContextCycleCodec.encode({}) == '{}'
assert sdk.ValidationError.__module__ == 'resource_python_sdk.validation'

fixture=json.loads((ROOT/'fixture.json').read_text(),parse_int=JsonNumber,parse_float=JsonNumber)
manifest=json.loads((Path(sdk.__file__).parent/'http-manifest.json').read_text())
assert manifest['validation']['version'] == 'suspect.validation.experimental.v3'
bad_responses=0
for case in fixture['cases']:
    if case['name']=='ContextCycle':
        continue
    codec=getattr(codecs,case['name']+'Codec')
    body=codec.decode_value(case['good'])
    method=next(op['method'] for op in manifest['operations'] if op['operationId']==case['operation'])
    def wrong(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200,headers={'content-type':'application/json'},content=stringify_json(case['bad']))
    with httpx.MockTransport(wrong) as transport:
        with Client(transport=transport) as client:
            try:
                getattr(client,method)(body=body)
            except SdkError as error:
                assert error.kind=='response-decoding' and isinstance(error.cause,CodecError),error
                assert error.cause.kind=='invalid' and error.cause.source.startswith('https://cdn.example/specs/api.json#')
                bad_responses+=1
            else:
                raise AssertionError(('invalid dynamic response',case['name']))

def guide(name: str) -> Any:
    directory = str(ROOT/'generated/python/examples')
    if directory not in sys.path:
        sys.path.insert(0,directory)
    spec=importlib.util.spec_from_file_location('guide_'+name,ROOT/'generated/python/examples'/(name+'.py'))
    assert spec is not None and spec.loader is not None
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
guide('transport').mock_request()
guide('numbers_example').exact_numbers()
validated=guide('validated')
with httpx.MockTransport(echo) as transport:
    with Client(transport=transport) as client:
        validated.run_sync(client)
    async def documented_async() -> None:
        async with AsyncClient(transport=transport) as client:
            await validated.run_async(client)
    asyncio.run(documented_async())
assert bad_responses==6
print(json.dumps({'python':sys.version,'installed':sdk.__file__,'requests':len(requests),'invalid_inputs_and_mutations':12,'invalid_responses':bad_responses,'dynamic_carriers':'passed','nested_entry_and_context_cycle':'passed','exact_numbers_and_bytes':'passed','native_guides':'passed'}))
