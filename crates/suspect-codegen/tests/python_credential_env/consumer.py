from __future__ import annotations
import asyncio
import inspect
import json
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any
from unittest.mock import patch
import httpx

NAMES = ('SUSPECT_PY_BEARER_ENV','SUSPECT_PY_HEADER_ENV','SUSPECT_PY_QUERY_ENV','SUSPECT_PY_COOKIE_ENV')
old_getenv = os.getenv
reads: list[str] = []
def forbidden_read(name: str, default: str | None = None) -> str | None:
    if name in NAMES:
        reads.append(name)
        raise OSError('SENSITIVE_ENV_ERROR_VALUE')
    return old_getenv(name,default)

with patch('os.getenv',side_effect=forbidden_read):
    import credential_python_sdk as sdk
    from credential_python_sdk import Client, AsyncClient, SdkError, UNSET
assert reads == [], 'import read a credential variable'
assert Path(sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert 'site-packages' in sdk.__file__
assert inspect.signature(Client).parameters['auth'].default is not None

wire: list[httpx.Request] = []
failures = 0
def handle(request: httpx.Request) -> httpx.Response:
    assert request.url.scheme == 'https' and request.url.host == 'credentials.example.test'
    assert request.url.path.startswith('/v1/')
    wire.append(request)
    return httpx.Response(200,headers={'content-type':'application/json'},content=b'"ok"')

def missing(action: Callable[[], object]) -> None:
    global failures
    before=len(wire)
    try:
        action()
    except SdkError as error:
        assert error.kind in ('request-validation','request-representation'),error
        assert error.status is None and error.cause is None
        assert all(value not in str(error)+repr(error) for value in ('SENSITIVE_ENV_ERROR_VALUE','environment-bearer','manual-bearer'))
    else:
        raise AssertionError('unavailable or invalid explicit credentials were accepted')
    assert len(wire)==before, 'credential failure reached HTTP'
    failures+=1

initial = dict(zip(NAMES,('environment-bearer','environment-header','environment-query','environment-cookie')))
with patch.dict(os.environ,initial,clear=True), httpx.MockTransport(handle) as transport:
    with Client(transport=transport) as client:
        assert client.get_current_key().data == 'ok'
        assert wire[-1].headers['Authorization']=='Bearer environment-bearer'
        client.header_key()
        assert wire[-1].headers['X-Api-Key']=='environment-header'
        client.query_key()
        assert wire[-1].url.params['key']=='environment-query'
        client.cookie_key()
        assert wire[-1].headers['Cookie']=='session=environment-cookie'
        client.together()
        request=wire[-1]
        assert request.headers['Authorization']=='Bearer environment-bearer'
        assert request.headers['X-Api-Key']=='environment-header'
        assert request.headers['Cookie']=='session=environment-cookie'
        assert request.url.params['key']=='environment-query'
        client.either()
        assert wire[-1].headers['Authorization']=='Bearer environment-bearer'
        assert 'X-Api-Key' not in wire[-1].headers
        client.optional_auth()
        assert 'Authorization' not in wire[-1].headers
        client.anonymous()
        client.credential_env()
        client.init()
    # Creation snapshots all mapped variables once, not at first use or per call.
    with patch('os.getenv',wraps=old_getenv) as getter:
        client=Client(transport=transport)
        assert sorted(call.args[0] for call in getter.call_args_list if call.args[0] in NAMES) == sorted(NAMES)
        count=len(getter.call_args_list)
        os.environ[NAMES[0]]='new-environment-bearer'
        with client:
            client.get_current_key()
            client.get_current_key()
        assert len(getter.call_args_list)==count
        assert wire[-1].headers['Authorization']=='Bearer environment-bearer'
    with Client(transport=transport) as new_client:
        new_client.get_current_key()
        assert wire[-1].headers['Authorization']=='Bearer new-environment-bearer'

    # Any explicitly supplied auth argument wins as a whole and performs no reads.
    reads.clear()
    with patch('os.getenv',side_effect=forbidden_read):
        with Client(auth={'apiKey':'manual-bearer'},transport=transport) as client:
            client.get_current_key()
            assert wire[-1].headers['Authorization']=='Bearer manual-bearer'
            missing(client.together)
        for explicit in (None, {}, {'apiKey':None}, {'apiKey':UNSET}, {'apiKey':''}):
            value: Any=explicit
            with Client(auth=value,transport=transport) as client:
                client.anonymous()
                missing(client.get_current_key)
        with Client(auth={'headerKey':'manual-header'},transport=transport) as client:
            missing(client.get_current_key)
            client.either()
            assert 'Authorization' not in wire[-1].headers and wire[-1].headers['X-Api-Key']=='manual-header'
            missing(client.together)
        with Client(auth={'headerKey':''},transport=transport) as client:
            client.header_key()
            assert wire[-1].headers['X-Api-Key']==''
    assert reads==[]

for environment in ({},dict.fromkeys(NAMES,'')):
    with patch.dict(os.environ,environment,clear=True), httpx.MockTransport(handle) as transport:
        with Client(transport=transport) as client:
            client.anonymous()
            client.optional_auth()
            missing(client.get_current_key)
            missing(client.header_key)
            missing(client.either)
            missing(client.together)
        with Client(transport=transport,auth_alternative=1) as client:
            missing(client.optional_auth)

with patch.dict(os.environ,{NAMES[1]:'only-header'},clear=True), httpx.MockTransport(handle) as transport:
    with Client(transport=transport) as client:
        client.either()
        assert wire[-1].headers['X-Api-Key']=='only-header' and 'Authorization' not in wire[-1].headers
        missing(client.together)

with patch.dict(os.environ,initial,clear=True), httpx.MockTransport(handle) as transport:
    with Client(transport=transport,auth_alternative=1) as client:
        client.either()
        assert wire[-1].headers['X-Api-Key']=='environment-header' and 'Authorization' not in wire[-1].headers
        client.optional_auth()
        assert wire[-1].headers['Authorization']=='Bearer environment-bearer'

reads.clear()
with patch('os.getenv',side_effect=forbidden_read), httpx.MockTransport(handle) as transport:
    with Client(transport=transport) as client:
        assert len(reads)==4
        client.anonymous()
        missing(client.get_current_key)
        assert len(reads)==4

async def asynchronous() -> None:
    global failures
    with patch.dict(os.environ,initial,clear=True), httpx.MockTransport(handle) as transport:
        client=AsyncClient(transport=transport)
        os.environ[NAMES[0]]='after-async-construction'
        async with client:
            await client.get_current_key()
            assert wire[-1].headers['Authorization']=='Bearer environment-bearer'
            await client.together()
            await client.anonymous()
        async with AsyncClient(transport=transport) as new_client:
            await new_client.get_current_key()
            assert wire[-1].headers['Authorization']=='Bearer after-async-construction'
        for explicit in (None, {}, {'apiKey':None}, {'apiKey':UNSET}, {'apiKey':''}, {'headerKey':'manual-header'}):
            value: Any=explicit
            reads.clear()
            with patch('os.getenv',side_effect=forbidden_read):
                async with AsyncClient(auth=value,transport=transport) as client:
                    await client.anonymous()
                    before=len(wire)
                    try:
                        await client.get_current_key()
                    except SdkError as error:
                        assert error.status is None and error.cause is None
                        assert 'SENSITIVE_ENV_ERROR_VALUE' not in str(error)+repr(error)
                    else:
                        raise AssertionError('async explicit credential was supplemented')
                    assert len(wire)==before
                    failures+=1
            assert reads==[]
    with patch.dict(os.environ,{},clear=True), httpx.MockTransport(handle) as transport:
        async with AsyncClient(transport=transport) as client:
            await client.anonymous()
            await client.optional_auth()
            before=len(wire)
            try:
                await client.together()
            except SdkError:
                failures+=1
            else:
                raise AssertionError('async missing credentials passed')
            assert len(wire)==before

asyncio.run(asynchronous())
print(json.dumps({'python':sys.version,'installed':sdk.__file__,'requests':len(wire),'pre_http_failures':failures,'import_reads':0,'creation_snapshot':True,'explicit_argument_precedence':True,'source_default_https':True,'unavailable_environment':True}))
