"""Explicit credential attachment; callbacks stay on the caller task."""
from __future__ import annotations
import base64
import inspect
import re
from collections.abc import Mapping
from types import MappingProxyType
from typing import Any, cast
from ._types import AuthValue, Authorization, BasicAuth, Credential, CredentialRequest, OAuthFlow, Source, SdkError
from ._wire import TOKEN, cookie_value, header_value, location, percent, text_bytes

_BEARER = re.compile(r'[A-Za-z0-9._~+/-]+=*', re.ASCII)


def context(operation: Source, requirement: dict[str, Any], effective_server_url: str | None = None) -> CredentialRequest:
    credential = requirement['credential']
    flows = tuple(OAuthFlow(kind=flow['kind'], source=location(flow['source']),
                           authorization_url=value(flow.get('authorization_url')),
                           token_url=value(flow.get('token_url')),
                           refresh_url=value(flow.get('refresh_url')),
                           device_authorization_url=value(flow.get('device_authorization_url')),
                           scopes=MappingProxyType({key: entry['value'] for key, entry in flow['scopes'].items()}))
                  for flow in credential.get('flows', []))
    return CredentialRequest(name=requirement['name'], operation=operation,
                             source=location(requirement['source']), scheme_source=location(requirement['scheme']),
                             kind=credential['kind'], permissions_kind=requirement['permissions']['kind'],
                             permissions=tuple(entry['value'] for entry in requirement['permissions']['names']),
                             flows=flows, metadata_url=value(credential.get('metadata_url')),
                             discovery_url=value(credential.get('discovery_url')),
                             effective_server_url=effective_server_url)


def value(located: dict[str, Any] | None) -> str | None:
    return cast(str, located['value']) if located is not None else None


def alternatives(security: dict[str, Any], choice: int | None, source: Source) -> list[dict[str, Any]]:
    if security['kind'] != 'alternatives':
        if choice is not None:
            raise SdkError('request-validation', source, code='http-auth-alternative')
        return [{'requirements': []}]
    options: list[dict[str, Any]] = security['alternatives']
    if choice is not None:
        if type(choice) is not int or not 0 <= choice < len(options):
            raise SdkError('request-validation', source, code='http-auth-alternative')
        return [options[choice]]
    return options


def attach(requirement: dict[str, Any], credential: AuthValue, maximum: int) -> tuple[str, str, str]:
    at = location(requirement['source'])
    kind = requirement['credential']['kind']
    where, name = 'header', 'Authorization'
    if kind == 'bearer':
        if type(credential) is not str or _BEARER.fullmatch(credential) is None:
            raise SdkError('request-validation', at, code='http-bearer-credential')
        result = 'Bearer ' + credential
    elif kind == 'basic':
        if type(credential) is not BasicAuth or ':' in credential.username or credential.encoding not in ('utf-8', 'latin-1'):
            raise SdkError('request-validation', at, code='http-basic-credential')
        if any(ord(char) < 32 or ord(char) == 127 for char in credential.username + credential.password):
            raise SdkError('request-representation', at, code='http-basic-control')
        try:
            plain = text_bytes(credential.username + ':' + credential.password, maximum, at)
            if credential.encoding == 'latin-1':
                plain = plain.decode('utf-8').encode('latin-1')
        except (UnicodeError, TypeError) as error:
            raise SdkError('request-representation', at, cause=error, code='http-basic-charset') from None
        result = 'Basic ' + base64.b64encode(plain).decode('ascii')
    elif kind == 'api-key':
        if type(credential) is not str:
            raise SdkError('request-validation', at, code='http-api-key-credential')
        where, name = requirement['credential']['location'], requirement['credential']['name']['value']
        result = percent(credential, 'uri-component', maximum, at) if where == 'query' else cookie_value(credential, at) if where == 'cookie' else header_value(credential, at)
    elif kind in ('o-auth2', 'open-id-connect', 'oauth2', 'openid-connect'):
        if type(credential) is not Authorization or not credential.value.partition(' ')[1] or TOKEN.fullmatch(credential.value.partition(' ')[0]) is None:
            raise SdkError('request-validation', at, code='http-explicit-authorization-required')
        result = header_value(credential.value, at)
    else:
        raise SdkError('request-validation', at, code='http-credential-descriptor')
    if len(result) > maximum:
        raise SdkError('resource-limit', at)
    if where == 'header':
        header_value(result, at)
    return where, name, result


def resolve(auth: Mapping[str, Credential], security: dict[str, Any], choice: int | None,
            source: Source, maximum: int, *, effective_server_url: str | None = None) -> list[tuple[str, str, str]]:
    cached: dict[tuple[str, Source], AuthValue | None] = {}
    for alternative in alternatives(security, choice, source):
        requirements = alternative['requirements']
        if any(requirement['name'] not in auth for requirement in requirements):
            continue
        result = []
        for requirement in requirements:
            name = requirement['name']
            key = name, location(requirement['source'])
            if key not in cached:
                supplied = auth[name]
                try:
                    item = supplied(context(source, requirement, effective_server_url)) if callable(supplied) else supplied
                    if inspect.isawaitable(item):
                        if inspect.iscoroutine(item):
                            item.close()
                        raise SdkError('request-validation', source, code='http-async-credential-in-sync-client')
                except SdkError:
                    raise
                except Exception as error:
                    raise SdkError('authentication', location(requirement['source']), cause=error) from None
                cached[key] = item
            credential = cached[key]
            if credential is None:
                break
            result.append(attach(requirement, credential, maximum))
        else:
            return result
    raise SdkError('request-validation', source, code='http-credentials-unavailable')


async def resolve_async(auth: Mapping[str, Credential], security: dict[str, Any], choice: int | None,
                        source: Source, maximum: int, *, effective_server_url: str | None = None) -> list[tuple[str, str, str]]:
    cached: dict[tuple[str, Source], AuthValue | None] = {}
    for alternative in alternatives(security, choice, source):
        requirements = alternative['requirements']
        if any(requirement['name'] not in auth for requirement in requirements):
            continue
        result = []
        for requirement in requirements:
            name = requirement['name']
            key = name, location(requirement['source'])
            if key not in cached:
                supplied = auth[name]
                try:
                    item = supplied(context(source, requirement, effective_server_url)) if callable(supplied) else supplied
                    if inspect.isawaitable(item):
                        item = await item
                except Exception as error:
                    raise SdkError('authentication', location(requirement['source']), cause=error) from None
                cached[key] = item
            credential = cached[key]
            if credential is None:
                break
            result.append(attach(requirement, credential, maximum))
        else:
            return result
    raise SdkError('request-validation', source, code='http-credentials-unavailable')
