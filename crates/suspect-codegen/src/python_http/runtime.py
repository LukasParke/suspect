"""Bounded sync/async HTTP exchanges over the admitted protocol descriptors."""
from __future__ import annotations

import inspect
import math
import re
import sys
from collections.abc import AsyncIterable, AsyncIterator, Iterable, Iterator, Mapping
from types import MappingProxyType
from typing import Any, Self, cast
from urllib.parse import unquote
import httpx
from . import json_runtime as J
from .codec_runtime import CodecError
from .models import UNSET, Unset
from ._types import ApiError as ApiError, SdkError as SdkError, Source as Source
from ._types import Credential, Link, RawHeaders
from ._registry import Operation as Operation
from ._urls import resolve_server
from . import _auth as auth_runtime, _media as media, _parts as parts, _registry as registry, _streams as streams
from ._wire import Budget, TOKEN, header_value, location, parse_scalar, percent, scalar, serialize, text_bytes

_MANAGED = {'host', 'content-length', 'transfer-encoding', 'connection', 'trailer', 'upgrade', 'accept-encoding'}


def server_url(op: Operation, override: str | None, selected: int | str, variables: Mapping[str, str], document_url: str | None) -> str:
    base: str | None = None
    if override is not None:
        result = override
    else:
        candidates = op.wire['servers']['candidates']
        if type(selected) is int:
            if not 0 <= selected < len(candidates):
                raise SdkError('request-validation', op.source, code='http-server-choice')
            server = candidates[selected]
        else:
            choices = [candidate for candidate in candidates if candidate['name'] is not None and candidate['name']['value'] == selected]
            if len(choices) != 1:
                raise SdkError('request-validation', op.source, code='http-server-choice')
            server = choices[0]
        defined = {variable['name']: variable for variable in server['variables']}
        if any(key not in defined for key in variables):
            raise SdkError('request-validation', op.source, code='http-server-override-unknown')
        values: dict[str, str] = {}
        for name, variable in defined.items():
            value = variables.get(name, variable['default']['value'])
            if type(value) is not str or variable['values'] is not None and value not in [entry['value'] for entry in variable['values']]:
                raise SdkError('request-validation', location(variable['source']), code='http-server-override-enum')
            values[name] = value
        result = re.sub(r'\{([^{}]+)\}', lambda match: values[match.group(1)], server['template'])
        base = document_url if document_url is not None else location(server['document_base']).document
    text_bytes(result, op.max_request_bytes, op.source)
    if base is not None:
        text_bytes(base, op.max_request_bytes, op.source)
    try:
        result = resolve_server(result, base)
    except (ValueError, TypeError) as error:
        raise SdkError('request-representation', op.source, cause=error, code='http-server-url') from None
    text_bytes(result, op.max_request_bytes, op.source)
    return result


def body_media(op: Operation, body: object, content_type: str | Unset) -> tuple[int, dict[str, Any], str] | None:
    declaration = op.wire['body']
    if isinstance(body, Unset):
        if declaration is not None and declaration['required']:
            raise SdkError('request-validation', op.source, code='http-required-body')
        return None
    if declaration is None:
        raise SdkError('request-validation', op.source, code='http-undeclared-body')
    if isinstance(content_type, Unset):
        choices = declaration['media']
        if len(choices) != 1 or choices[0]['media_type']['range']['kind'] != 'concrete':
            raise SdkError('request-representation', op.source, code='http-request-content-type-required')
        content_type = choices[0]['media_type']['declared']
    if type(content_type) is not str:
        raise SdkError('request-representation', op.source, code='http-request-content-type')
    index, selected = media.select(declaration['media'], content_type, op.source)
    return index, selected, content_type


def encode_body(op: Operation, body: object, selected: tuple[int, dict[str, Any], str] | None) -> tuple[bytes | None, str | None]:
    if selected is None:
        return None, None
    index, definition, content_type = selected
    rep = definition['representation']
    at = location(definition['source'])
    kind = rep['kind']
    maximum = op.max_request_bytes
    if kind == 'binary':
        if type(body) is not bytes:
            raise SdkError('request-representation', at, code='http-byte-body-required')
        if len(body) > min(maximum, rep['bytes']['max_bytes']):
            raise SdkError('resource-limit', at)
        return body, content_type
    if kind in ('json', 'text'):
        wire = registry.codec(rep['codec']).encode_value(body) if rep['codec'] is not None else cast(J.JsonValue, body)
        text = J.stringify_json(wire) if kind == 'json' else scalar(wire, rep['scalar'], at)
        return text_bytes(text, maximum, at), content_type
    if kind in ('form', 'multipart'):
        group_name = op.binding['body']['media'][index]['group']
        values = parts.group_values(group_name, body, at)
        if kind == 'form':
            return parts.encode_form(values, rep['form'], maximum, op.max_parts, at), content_type
        return parts.encode_multipart(group_name, values, rep['multipart'], content_type, maximum, op.max_part_bytes, op.max_parts, at)
    if kind == 'stream':
        if isinstance(body, (str, bytes, bytearray, dict)) or not isinstance(body, Iterable):
            raise SdkError('request-representation', at, code='http-item-iterable-required')
        stream = rep['stream']
        encoded = bytearray()
        iterator = iter(body)
        try:
            for value in iterator:
                wire = registry.codec(stream['item_codec']).encode_value(value)
                frame = streams.encode(wire, stream['framing'], min(stream['max_item_bytes'], maximum - len(encoded)), at)
                if len(frame) > maximum - len(encoded):
                    raise SdkError('resource-limit', at)
                encoded.extend(frame)
        finally:
            primary = sys.exception()
            try:
                close = getattr(iterator, 'close', None)
                if close is not None:
                    close()
            except Exception:
                if primary is None:
                    raise
        return bytes(encoded), content_type
    raise SdkError('request-representation', at, code='http-body-descriptor')


async def encode_body_async(op: Operation, body: object, selected: tuple[int, dict[str, Any], str] | None) -> tuple[bytes | None, str | None]:
    if selected is None or selected[1]['representation']['kind'] != 'stream' or not isinstance(body, AsyncIterable):
        return encode_body(op, body, selected)
    _, definition, content_type = selected
    stream = definition['representation']['stream']
    source = location(definition['source'])
    encoded = bytearray()
    iterator = body.__aiter__()
    try:
        async for value in iterator:
            wire = registry.codec(stream['item_codec']).encode_value(value)
            frame = streams.encode(wire, stream['framing'], min(stream['max_item_bytes'], op.max_request_bytes - len(encoded)), source)
            if len(frame) > op.max_request_bytes - len(encoded):
                raise SdkError('resource-limit', source)
            encoded.extend(frame)
    finally:
        primary = sys.exception()
        try:
            close = getattr(iterator, 'aclose', None)
            if close is not None:
                await close()
        except Exception:
            if primary is None:
                raise
    return bytes(encoded), content_type


def parameters(op: Operation, arguments: Mapping[str, object]) -> list[tuple[str, str, str]]:
    output = []
    for binding, descriptor in zip(op.binding['parameters'], op.wire['parameters']):
        value = arguments.get(binding['member'], UNSET)
        at = location(descriptor['source'])
        if isinstance(value, Unset):
            if descriptor['required']:
                raise SdkError('request-validation', at, code='http-required-parameter')
            continue
        wire = registry.codec(descriptor['codec']).encode_value(value)
        content = descriptor.get('content_media')
        encoded: str | None
        if descriptor['location'] == 'querystring' and content is not None and content['representation']['kind'] == 'form':
            if type(wire) is not dict:
                raise SdkError('request-representation', at, code='http-querystring-object')
            encoded = parts.encode_form(cast(dict[str, object], wire), content['representation']['form'], op.max_request_bytes, op.max_parts, at, wire_values=True).decode('ascii')
        else:
            encoded = serialize(descriptor['name'], descriptor['location'], descriptor['serialization'], wire, op.max_request_bytes, at, optional=not descriptor['required'])
        if encoded is not None:
            output.append((descriptor['location'], descriptor['name'], encoded))
    return output


def links(response: dict[str, Any]) -> tuple[Link, ...]:
    result = []
    for link in response['links']:
        target = link['target']
        request = link['request_body']
        result.append(Link(name=link['name'], source=location(link['source']), target_source=location(target['operation']),
                           operation_id=target['value']['value'] if target['kind'] == 'operation-id' else None,
                           operation_ref=target['value']['value'] if target['kind'] == 'operation-ref' else None,
                           parameters=MappingProxyType({name: J.parse_json(J.stringify_json(value['value'])) for name, value in link['parameters'].items()}),
                           request_body=UNSET if request is None else J.parse_json(J.stringify_json(request['value'])),
                           description=None if link['description'] is None else link['description']['value'],
                           server=cast(Mapping[str, J.JsonValue] | None, link['server'])))
    return tuple(result)


class RawResponse:
    def __init__(self, op: Operation, response: httpx.Response, maximum: int, capture: int) -> None:
        self.operation, self.response = op, response
        self.status = response.status_code
        self.headers: RawHeaders = tuple(response.headers.multi_items())
        self.maximum, self.capture_limit, self.total = maximum, capture, 0
        self.capture = bytearray()
        self.body = b''
        self.closed = False
        self.response_index = 0
        self.media_index: int | None = None
        self.media: dict[str, Any] | None = None

    def failure(self, kind: str, cause: BaseException | None = None, *, code: str | None = None, truncated: bool = False) -> SdkError:
        return SdkError(kind, self.operation.source, status=self.status, headers=self.headers, capture=bytes(self.capture), truncated=truncated or self.total > len(self.capture), cause=cause, code=code)

    def chunk(self, value: bytes) -> bytes:
        if type(value) is not bytes:
            raise self.failure('transport', TypeError('transport yielded non-byte body'))
        available = self.capture_limit - len(self.capture)
        self.capture.extend(value[:min(available, max(0, self.maximum - self.total + 1))])
        self.total += len(value)
        if self.total > self.maximum:
            raise self.failure('resource-limit', truncated=True)
        return value

    def select(self) -> None:
        types = [value for name, value in self.headers if name.lower() == 'content-type']
        self.response_index, self.media_index, self.media = media.response(self.operation.wire, self.status, types, self.operation.source)
        response = self.operation.wire['responses'][self.response_index]
        self.maximum = min(self.maximum, response['max_body_bytes'])
        if self.media is not None and self.media['representation']['kind'] == 'binary':
            self.maximum = min(self.maximum, self.media['representation']['bytes']['max_bytes'])

    def decode(self) -> object:
        if media.forbidden(self.operation.wire['method'], self.status):
            return None
        if self.media is None:
            return self.body
        rep = self.media['representation']
        kind = rep['kind']
        if kind == 'binary':
            return self.body
        if kind == 'json':
            return registry.codec(rep['codec']).decode(self.body) if rep['codec'] is not None else J.parse_json(self.body)
        if kind == 'text':
            value = parse_scalar(self.body.decode('utf-8'), rep['scalar'], self.operation.source)
            return registry.codec(rep['codec']).decode_value(value) if rep['codec'] is not None else value
        binding = self.operation.binding['responses'][self.response_index]
        group = binding['media'][self.media_index]['group']
        if kind == 'form':
            return parts.decode_form(group, self.body, rep['form'], self.operation.max_parts, self.operation.source)
        if kind == 'multipart':
            content_type = next(value for name, value in self.headers if name.lower() == 'content-type')
            return parts.decode_multipart(group, self.body, rep['multipart'], content_type, self.operation.max_part_bytes, self.operation.max_parts, self.operation.source)
        raise self.failure('response-decoding', code='http-response-descriptor')

    def result(self, data: object, asynchronous: bool) -> object:
        binding = self.operation.binding['responses'][self.response_index]
        descriptor = self.operation.wire['responses'][self.response_index]
        typed = parts.decode_headers(binding['headerGroup'], self.headers, self.operation.source)
        content_type = next((value for name, value in self.headers if name.lower() == 'content-type'), None)
        success = 200 <= self.status < 300
        name = binding['asyncClass' if asynchronous else 'class'] if success else binding['asyncErrorClass' if asynchronous else 'errorClass']
        arguments = dict(data=data, headers=self.headers, content_type=content_type, links=links(descriptor))
        if binding['headerGroup'] is not None:
            arguments['typed_headers'] = typed
        if not success or descriptor['status']['kind'] != 'exact':
            arguments['status'] = self.status
        if not success:
            arguments['source'] = self.operation.source
        value = registry.native_class(name)(**arguments)
        if not success:
            raise cast(ApiError[object], value)
        return value


class SyncResponse(RawResponse):
    def __init__(self, owner: SyncClient, op: Operation, response: httpx.Response, maximum: int, capture: int) -> None:
        super().__init__(op, response, maximum, capture)
        self.owner = owner
        owner._responses.add(self)

    def chunks(self) -> Iterator[bytes]:
        try:
            if self.closed:
                return
            if not isinstance(self.response.stream, httpx.SyncByteStream):
                raise TypeError('sync transport returned a non-sync body')
            iterator = iter(self.response.stream)
            while not self.closed:
                try:
                    chunk = next(iterator)
                except StopIteration:
                    return
                yield self.chunk(chunk)
        except SdkError:
            raise
        except Exception as error:
            raise self.failure('transport', error, truncated=True) from None

    def close(self, primary: BaseException | None = None) -> None:
        if self.closed:
            return
        self.closed = True
        self.owner._responses.discard(self)
        try:
            self.response.close()
        except Exception as error:
            if primary is None:
                raise self.failure('transport', error) from None

    def read(self) -> None:
        data = bytearray()
        try:
            for chunk in self.chunks():
                data.extend(chunk)
            self.body = bytes(data)
        finally:
            self.close(sys.exception())


class AsyncResponse(RawResponse):
    def __init__(self, owner: AsyncClientBase, op: Operation, response: httpx.Response, maximum: int, capture: int) -> None:
        super().__init__(op, response, maximum, capture)
        self.owner = owner
        owner._responses.add(self)

    async def chunks(self) -> AsyncIterator[bytes]:
        try:
            if self.closed:
                return
            if not isinstance(self.response.stream, httpx.AsyncByteStream):
                raise TypeError('async transport returned a non-async body')
            iterator = self.response.stream.__aiter__()
            while not self.closed:
                try:
                    chunk = await iterator.__anext__()
                except StopAsyncIteration:
                    return
                yield self.chunk(chunk)
        except SdkError:
            raise
        except Exception as error:
            raise self.failure('transport', error, truncated=True) from None

    async def close(self, primary: BaseException | None = None) -> None:
        if self.closed:
            return
        self.closed = True
        self.owner._responses.discard(self)
        try:
            await self.response.aclose()
        except Exception as error:
            if primary is None:
                raise self.failure('transport', error) from None

    async def read(self) -> None:
        data = bytearray()
        try:
            async for chunk in self.chunks():
                data.extend(chunk)
            self.body = bytes(data)
        finally:
            await self.close(sys.exception())


class _Base:
    def _configure(self, auth: Mapping[str, Credential] | None, override: str | None, selected: int | str,
                   variables: Mapping[str, str] | None, document: str | None, security: int | None,
                   maximum: int | None, capture: int, timeout: float | None) -> None:
        if maximum is not None and (type(maximum) is not int or maximum <= 0):
            raise ValueError('max_response_bytes must be positive')
        if type(capture) is not int or capture < 0:
            raise ValueError('max_capture_bytes must be nonnegative')
        if timeout is not None and (not math.isfinite(timeout) or timeout <= 0):
            raise ValueError('timeout must be finite and positive')
        self._auth = dict(auth or {})
        self._server_url, self._server, self._variables = override, selected, dict(variables or {})
        self._document_url, self._security = document, security
        self._max_response, self._max_capture = maximum, capture
        self._timeout = httpx.Timeout(timeout).as_dict()
        self._closed = False

    def _prepare(self, op: Operation, encoded_parameters: list[tuple[str, str, str]], body: bytes | None,
                 content_type: str | None, attachments: list[tuple[str, str, str]], base: str) -> tuple[httpx.Request, int]:
        if self._closed:
            raise SdkError('request-validation', op.source, code='http-client-closed')
        maximum = self._max_response if self._max_response is not None else op.max_response_bytes
        if maximum > op.max_response_bytes:
            raise SdkError('resource-limit', op.source)
        route: str = op.wire['path']
        query: list[str] = []
        cookies: list[str] = []
        types = list(dict.fromkeys(entry['media_type']['declared'] for response in op.wire['responses'] for entry in response['media']))
        headers = [('Accept', ', '.join(types) if types else '*/*'), ('Accept-Encoding', 'identity')]
        if content_type is not None:
            headers.append(('Content-Type', content_type))
        used_headers = {key.lower() for key, _ in headers}
        used_query: dict[str, str] = {}
        used_cookies: dict[str, str] = {}
        whole_query = False
        budget = Budget(op.max_request_bytes, op.source)
        budget.charge(len(base) + len(route))
        for where, name, value in encoded_parameters:
            if where == 'path':
                if value in ('', '.', '..') or any(unquote(segment) in ('.', '..') for segment in value.split('/')):
                    raise SdkError('request-representation', op.source, code='http-path-segment')
                marker = '{' + name + '}'
                if marker not in route:
                    raise SdkError('request-representation', op.source, code='http-path-parameter')
                route = route.replace(marker, value)
            elif where == 'header':
                if name.lower() in used_headers or name.lower() in _MANAGED or TOKEN.fullmatch(name) is None:
                    raise SdkError('request-representation', op.source, code='http-header-conflict')
                headers.append((name, header_value(value, op.source)))
                used_headers.add(name.lower())
            elif where == 'querystring':
                if query:
                    raise SdkError('request-representation', op.source, code='http-querystring-conflict')
                query.append(value)
                whole_query = True
            else:
                target, used, delimiter = (cookies, used_cookies, '; ') if where == 'cookie' else (query, used_query, '&')
                for pair in value.split(delimiter):
                    key = unquote(pair.partition('=')[0])
                    if key in used and used[key] != name:
                        raise SdkError('request-representation', op.source, code='http-expanded-parameter-conflict')
                    used[key] = name
                target.append(value)
            budget.charge(len(name) + len(value) + 4)
        for where, name, value in attachments:
            if where == 'header':
                if name.lower() in used_headers or name.lower() in _MANAGED or name.lower() == 'cookie':
                    raise SdkError('request-representation', op.source, code='http-security-attachment-conflict')
                headers.append((name, value))
                used_headers.add(name.lower())
            elif where == 'query':
                if whole_query or name in used_query:
                    raise SdkError('request-representation', op.source, code='http-security-attachment-conflict')
                used_query[name] = 'credential'
                query.append(percent(name, 'uri-component', op.max_request_bytes, op.source) + '=' + value)
            else:
                if name in used_cookies:
                    raise SdkError('request-representation', op.source, code='http-security-attachment-conflict')
                used_cookies[name] = 'credential'
                cookies.append(name + '=' + value)
            budget.charge(len(name) + len(value) + 4)
        if cookies:
            headers.append(('Cookie', '; '.join(cookies)))
        url = (base[:-1] if base.endswith('/') else base) + route + ('?' + '&'.join(query) if query else '')
        text_bytes(url, op.max_request_bytes, op.source)
        try:
            request = httpx.Request(op.wire['method'], url, headers=headers, content=body, extensions={'timeout': self._timeout})
            # httpx normalizes methods in Request.__init__; OAS 3.2 custom
            # method tokens are case-sensitive and must retain their spelling.
            request.method = op.wire['method']
        except (httpx.HTTPError, httpx.InvalidURL, ValueError) as error:
            raise SdkError('request-representation', op.source, cause=error) from None
        return request, maximum


class SyncClient(_Base):
    def __init__(self, *, auth: Mapping[str, Credential] | None = None, server_url: str | None = None,
                 server: int | str = 0, server_variables: Mapping[str, str] | None = None,
                 document_url: str | None = None, auth_alternative: int | None = None,
                 transport: httpx.BaseTransport | None = None, max_response_bytes: int | None = None,
                 max_capture_bytes: int = 4096, timeout: float | None = 30) -> None:
        self._configure(auth, server_url, server, server_variables, document_url, auth_alternative, max_response_bytes, max_capture_bytes, timeout)
        self._owned = transport is None
        self._transport = httpx.HTTPTransport(trust_env=False, retries=0) if transport is None else transport
        self._responses: set[SyncResponse] = set()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, kind: object, error: BaseException | None, traceback: object) -> None:
        self._close(error)

    def close(self) -> None:
        self._close(None)

    def _close(self, primary: BaseException | None) -> None:
        if self._closed:
            return
        self._closed = True
        failure: BaseException | None = primary
        for response in tuple(self._responses):
            try:
                response.close(failure)
            except BaseException as error:
                if failure is None:
                    failure = error
        if self._owned:
            try:
                self._transport.close()
            except BaseException as error:
                if failure is None:
                    failure = error
        if primary is None and failure is not None:
            raise failure

    def _call(self, op: Operation, arguments: Mapping[str, object], body: object = UNSET, content_type: str | Unset = UNSET) -> object:
        params = parameters(op, arguments)
        payload, content = encode_body(op, body, body_media(op, body, content_type))
        base = server_url(op, self._server_url, self._server, self._variables, self._document_url)
        attachments = auth_runtime.resolve(self._auth, op.wire['security'], self._security, op.source, op.max_request_bytes, effective_server_url=base)
        request, maximum = self._prepare(op, params, payload, content, attachments, base)
        try:
            response = self._transport.handle_request(request)
        except Exception as error:
            raise SdkError('transport', op.source, cause=error) from None
        raw = SyncResponse(self, op, response, maximum, self._max_capture)
        streaming = False
        try:
            try:
                raw.select()
            except SdkError as error:
                raw.read()
                raise raw.failure(error.kind, error.cause, code=error.code) from None
            if raw.media is not None and raw.media['representation']['kind'] == 'stream':
                stream = raw.media['representation']['stream']
                data: object = streams.SyncStream(raw.chunks(), registry.codec(stream['item_codec']), stream['framing'], stream['max_item_bytes'], location(stream['source']), raw.close, raw.failure, lambda: raw.closed)
                streaming = True
            else:
                if media.forbidden(op.wire['method'], raw.status):
                    raw.close()
                else:
                    raw.read()
                data = raw.decode()
            return raw.result(data, False)
        except ApiError:
            if not streaming:
                raw.close(sys.exception())
            raise
        except BaseException as error:
            raw.close(error)
            if isinstance(error, (CodecError, J.JsonError, UnicodeError, ValueError, TypeError)):
                kind = 'resource-limit' if isinstance(error, (CodecError, J.JsonError)) and error.kind in ('resource', J.RESOURCE_LIMIT) else 'response-decoding'
                raise raw.failure(kind, error) from None
            if isinstance(error, SdkError) and error.status is None:
                raise raw.failure(error.kind if error.kind == 'resource-limit' else 'response-decoding', error, code=error.code) from None
            raise


class AsyncClientBase(_Base):
    def __init__(self, *, auth: Mapping[str, Credential] | None = None, server_url: str | None = None,
                 server: int | str = 0, server_variables: Mapping[str, str] | None = None,
                 document_url: str | None = None, auth_alternative: int | None = None,
                 transport: httpx.AsyncBaseTransport | None = None, max_response_bytes: int | None = None,
                 max_capture_bytes: int = 4096, timeout: float | None = 30) -> None:
        self._configure(auth, server_url, server, server_variables, document_url, auth_alternative, max_response_bytes, max_capture_bytes, timeout)
        self._owned = transport is None
        self._transport = httpx.AsyncHTTPTransport(trust_env=False, retries=0) if transport is None else transport
        self._responses: set[AsyncResponse] = set()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, error: BaseException | None, traceback: object) -> None:
        await self._close(error)

    async def aclose(self) -> None:
        await self._close(None)

    async def _close(self, primary: BaseException | None) -> None:
        if self._closed:
            return
        self._closed = True
        failure: BaseException | None = primary
        for response in tuple(self._responses):
            try:
                await response.close(failure)
            except BaseException as error:
                if failure is None:
                    failure = error
        if self._owned:
            try:
                await self._transport.aclose()
            except BaseException as error:
                if failure is None:
                    failure = error
        if primary is None and failure is not None:
            raise failure

    async def _call(self, op: Operation, arguments: Mapping[str, object], body: object = UNSET, content_type: str | Unset = UNSET) -> object:
        params = parameters(op, arguments)
        payload, content = await encode_body_async(op, body, body_media(op, body, content_type))
        base = server_url(op, self._server_url, self._server, self._variables, self._document_url)
        attachments = await auth_runtime.resolve_async(self._auth, op.wire['security'], self._security, op.source, op.max_request_bytes, effective_server_url=base)
        request, maximum = self._prepare(op, params, payload, content, attachments, base)
        try:
            response = await self._transport.handle_async_request(request)
        except Exception as error:
            raise SdkError('transport', op.source, cause=error) from None
        raw = AsyncResponse(self, op, response, maximum, self._max_capture)
        streaming = False
        try:
            try:
                raw.select()
            except SdkError as error:
                await raw.read()
                raise raw.failure(error.kind, error.cause, code=error.code) from None
            if raw.media is not None and raw.media['representation']['kind'] == 'stream':
                stream = raw.media['representation']['stream']
                data: object = streams.AsyncStream(raw.chunks(), registry.codec(stream['item_codec']), stream['framing'], stream['max_item_bytes'], location(stream['source']), raw.close, raw.failure, lambda: raw.closed)
                streaming = True
            else:
                if media.forbidden(op.wire['method'], raw.status):
                    await raw.close()
                else:
                    await raw.read()
                data = raw.decode()
            return raw.result(data, True)
        except ApiError:
            if not streaming:
                await raw.close(sys.exception())
            raise
        except BaseException as error:
            await raw.close(error)
            if isinstance(error, (CodecError, J.JsonError, UnicodeError, ValueError, TypeError)):
                kind = 'resource-limit' if isinstance(error, (CodecError, J.JsonError)) and error.kind in ('resource', J.RESOURCE_LIMIT) else 'response-decoding'
                raise raw.failure(kind, error) from None
            if isinstance(error, SdkError) and error.status is None:
                raise raw.failure(error.kind if error.kind == 'resource-limit' else 'response-decoding', error, code=error.code) from None
            raise
