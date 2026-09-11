from __future__ import annotations
import asyncio
import inspect
import json
import os
import sys
from pathlib import Path
from unittest.mock import patch
import httpx

lookups: list[str] = []
getenv=os.getenv
def guarded(name: str, default: str | None = None) -> str | None:
    if name=='OPENROUTER_API_KEY':
        lookups.append(name)
        raise OSError('unavailable test environment')
    return getenv(name,default)
with patch('os.getenv',side_effect=guarded):
    import openrouter_sdk as sdk
    from openrouter_sdk import Client, AsyncClient, SdkError, JsonNumber, operations
assert lookups==[], 'import read the key variable'
assert 'site-packages' in sdk.__file__ and Path(sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
responses=json.loads((Path(__file__).parent/'responses.json').read_text())
requests: list[str] = []
expected_token='controlled-normal-token'

def handle(request: httpx.Request) -> httpx.Response:
    assert request.method=='GET'
    assert str(request.url) in ('https://openrouter.ai/api/v1/key','https://openrouter.ai/api/v1/credits'),request.url
    assert request.headers['Authorization']=='Bearer '+expected_token
    operation='getCurrentKey' if request.url.path.endswith('/key') else 'getCredits'
    requests.append(operation)
    return httpx.Response(200,headers={'content-type':'application/json'},content=responses[operation].encode('utf-8'))

def current_key(client: Client) -> operations.GetCurrentKeySuccess:
    response=client.get_current_key()
    assert response.status==200
    assert isinstance(response.data.data.label,str)
    assert isinstance(response.data.data.usage,JsonNumber)
    assert response.data.data.is_management_key is False
    return response

async def current_key_async(client: AsyncClient) -> operations.GetCurrentKeySuccess:
    response=await client.get_current_key()
    assert response.status==200
    assert isinstance(response.data.data.label,str)
    assert isinstance(response.data.data.usage,JsonNumber)
    return response

failures=0
with patch.dict(os.environ,{'OPENROUTER_API_KEY':expected_token},clear=True), httpx.MockTransport(handle) as transport:
    client=Client(transport=transport)
    os.environ['OPENROUTER_API_KEY']='changed-after-creation'
    with client:
        current_key(client)
        credits=client.get_credits()
        assert credits.status==200 and isinstance(credits.data.data.total_credits,JsonNumber)
    expected_token='changed-after-creation'
    with Client(transport=transport) as newer:
        current_key(newer)
    # The bare public spelling also constructs an env-backed client. Intercept
    # its default transport for this controlled check; no account call occurs.
    with patch('httpx.HTTPTransport',return_value=transport):
        with Client() as bare:
            current_key(bare)
    expected_token='manual-token'
    with patch('os.getenv',side_effect=guarded):
        with Client(auth={'apiKey':expected_token},transport=transport) as explicit:
            current_key(explicit)
        empty_auth: tuple[dict[str, str] | None, ...] = (None,{})
        for auth in empty_auth:
            with Client(auth=auth,transport=transport) as explicit:
                before=len(requests)
                try:
                    explicit.get_current_key()
                except SdkError as error:
                    assert error.status is None and error.cause is None
                    assert expected_token not in str(error)+repr(error)
                    failures+=1
                else:
                    raise AssertionError('explicit empty credentials used the environment')
                assert len(requests)==before
    assert lookups==[]

for value in (None,''):
    with patch.dict(os.environ,{} if value is None else {'OPENROUTER_API_KEY':value},clear=True), httpx.MockTransport(handle) as transport:
        with Client(transport=transport) as client:
            before=len(requests)
            try:
                client.get_current_key()
            except SdkError as error:
                assert error.status is None and error.cause is None
                failures+=1
            else:
                raise AssertionError('missing runtime key was accepted')
            assert len(requests)==before

async def asynchronous() -> None:
    global expected_token
    expected_token='controlled-async-token'
    with patch.dict(os.environ,{'OPENROUTER_API_KEY':expected_token},clear=True), httpx.MockTransport(handle) as transport:
        client=AsyncClient(transport=transport)
        os.environ['OPENROUTER_API_KEY']='changed-after-async-creation'
        async with client:
            await current_key_async(client)
            assert (await client.get_credits()).status==200
        expected_token='changed-after-async-creation'
        with patch('httpx.AsyncHTTPTransport',return_value=transport):
            async with AsyncClient() as bare:
                await current_key_async(bare)
asyncio.run(asynchronous())

print(json.dumps({'python':sys.version,'installed':sdk.__file__,'requests':len(requests),'getCurrentKey':requests.count('getCurrentKey'),'getCredits':requests.count('getCredits'),'pre_http_failures':failures,'source_default_server':'https://openrouter.ai/api/v1','clientSignature':str(inspect.signature(Client)),'asyncSignature':str(inspect.signature(AsyncClient)),'controlled_transport_only':True}))
