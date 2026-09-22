"""Caller-owned, context-managed item streams with bounded incremental framing."""
from __future__ import annotations
from collections.abc import AsyncIterator, Awaitable, Callable, Iterator
from typing import Generic, TypeVar, cast
from . import json_runtime as J
from .codec_runtime import CodecError, ModelCodec
from ._types import Source, SdkError
from ._wire import text_bytes

T = TypeVar('T')
Failure = Callable[[str, BaseException | None], SdkError]


class Framer:
    def __init__(self, framing: str, limit: int, source: Source) -> None:
        self.framing, self.limit, self.source = framing, limit, source
        self.line = bytearray()
        self.frame_bytes = 0
        self.skip_lf = False
        self.first = True
        self.data: list[str] = []
        self.event: str | None = None
        self.last_id: str | None = None
        self.retry: int | None = None

    def feed(self, chunk: bytes) -> Iterator[J.JsonValue]:
        for byte in chunk:
            if self.framing == 'server-sent-events' and self.skip_lf:
                self.skip_lf = False
                if byte == 10:
                    continue
            self.frame_bytes += 1
            if self.frame_bytes > self.limit:
                raise SdkError('resource-limit', self.source, code='http-stream-item-limit')
            newline = byte == 10 or self.framing == 'server-sent-events' and byte == 13
            if not newline:
                self.line.append(byte)
                continue
            self.skip_lf = self.framing == 'server-sent-events' and byte == 13
            line = bytes(self.line)
            self.line.clear()
            yield from self.accept(line)

    def accept(self, raw: bytes) -> Iterator[J.JsonValue]:
        if self.framing == 'json-lines':
            if raw.endswith(b'\r'):
                raw = raw[:-1]
            self.frame_bytes = 0
            # A blank line is not a JSON value. Final LF is optional; no RS or
            # sentinel convention is introduced by this framing.
            yield J.parse_json(raw)
            return
        # HTML event-stream uses UTF-8 replacement decoding and ignores one
        # leading BOM. Line bytes are retained across arbitrary chunk splits.
        text = raw.decode('utf-8', 'replace')
        if self.first:
            self.first = False
            if text.startswith('\ufeff'):
                text = text[1:]
        if not text:
            self.frame_bytes = 0
            if self.data:
                event: dict[str, J.JsonValue] = {'data': '\n'.join(self.data)}
                if self.last_id is not None:
                    event['id'] = self.last_id
                if self.event is not None:
                    event['event'] = self.event or 'message'
                if self.retry is not None:
                    event['retry'] = self.retry
                yield event
            self.data.clear()
            self.event = None
            return
        if text.startswith(':'):
            return
        name, separator, value = text.partition(':')
        if not separator:
            value = ''
        elif value.startswith(' '):
            value = value[1:]
        if name == 'data':
            self.data.append(value)
        elif name == 'event':
            self.event = value
        elif name == 'id' and '\0' not in value:
            self.last_id = value
        elif name == 'retry' and value and all('0' <= c <= '9' for c in value):
            self.retry = J.JsonNumber(value.lstrip('0') or '0').to_int()

    def finish(self) -> Iterator[J.JsonValue]:
        if self.framing == 'json-lines' and self.line:
            yield from self.accept(bytes(self.line))
        # HTML discards an unfinished event at EOF, even after a data line.
        self.line.clear()
        self.data.clear()


class SyncStream(Generic[T], Iterator[T]):
    """Validate one item per pull. Use `with` or close() when stopping early."""
    def __init__(self, chunks: Iterator[bytes], codec: ModelCodec[T], framing: str,
                 limit: int, source: Source, close: Callable[[BaseException | None], None], failure: Failure,
                 owner_closed: Callable[[], bool] | None = None) -> None:
        self._chunks, self._codec = chunks, codec
        self._framer = Framer(framing, limit, source)
        self._close, self._failure = close, failure
        self._items: Iterator[J.JsonValue] = iter(())
        self._finished = False
        self._closed = False
        self._owner_closed = owner_closed

    def __iter__(self) -> SyncStream[T]:
        return self

    def __next__(self) -> T:
        if self._closed or self._owner_closed is not None and self._owner_closed():
            self._closed = True
            raise StopIteration
        try:
            while True:
                try:
                    value = next(self._items)
                except StopIteration:
                    if self._finished:
                        self.close()
                        raise
                    try:
                        chunk = next(self._chunks)
                    except StopIteration:
                        self._finished = True
                        self._items = self._framer.finish()
                    else:
                        self._items = self._framer.feed(chunk)
                    continue
                return self._codec.decode_value(value)
        except StopIteration:
            raise
        except BaseException as error:
            self._closed = True
            self._close(error)
            if isinstance(error, (CodecError, J.JsonError)):
                kind = 'resource-limit' if error.kind in ('resource', J.RESOURCE_LIMIT) else 'response-decoding'
                raise self._failure(kind, error) from None
            if isinstance(error, SdkError) and error.status is None:
                failure = self._failure(error.kind, error)
                failure.code = error.code
                raise failure from None
            raise

    def close(self) -> None:
        if not self._closed:
            self._closed = True
            self._close(None)

    def __enter__(self) -> SyncStream[T]:
        return self

    def __exit__(self, kind: object, error: BaseException | None, traceback: object) -> None:
        if not self._closed:
            self._closed = True
            self._close(error)


class AsyncStream(Generic[T], AsyncIterator[T]):
    """Await one item per pull. Async context exit closes an early-stopped body."""
    def __init__(self, chunks: AsyncIterator[bytes], codec: ModelCodec[T], framing: str,
                 limit: int, source: Source, close: Callable[[BaseException | None], Awaitable[None]], failure: Failure,
                 owner_closed: Callable[[], bool] | None = None) -> None:
        self._chunks, self._codec = chunks, codec
        self._framer = Framer(framing, limit, source)
        self._close, self._failure = close, failure
        self._items: Iterator[J.JsonValue] = iter(())
        self._finished = False
        self._closed = False
        self._owner_closed = owner_closed

    def __aiter__(self) -> AsyncStream[T]:
        return self

    async def __anext__(self) -> T:
        if self._closed or self._owner_closed is not None and self._owner_closed():
            self._closed = True
            raise StopAsyncIteration
        try:
            while True:
                try:
                    value = next(self._items)
                except StopIteration:
                    if self._finished:
                        await self.aclose()
                        raise StopAsyncIteration from None
                    try:
                        chunk = await self._chunks.__anext__()
                    except StopAsyncIteration:
                        self._finished = True
                        self._items = self._framer.finish()
                    else:
                        self._items = self._framer.feed(chunk)
                    continue
                return self._codec.decode_value(value)
        except StopAsyncIteration:
            raise
        except BaseException as error:
            self._closed = True
            await self._close(error)
            if isinstance(error, (CodecError, J.JsonError)):
                kind = 'resource-limit' if error.kind in ('resource', J.RESOURCE_LIMIT) else 'response-decoding'
                raise self._failure(kind, error) from None
            if isinstance(error, SdkError) and error.status is None:
                failure = self._failure(error.kind, error)
                failure.code = error.code
                raise failure from None
            raise

    async def aclose(self) -> None:
        if not self._closed:
            self._closed = True
            await self._close(None)

    async def __aenter__(self) -> AsyncStream[T]:
        return self

    async def __aexit__(self, kind: object, error: BaseException | None, traceback: object) -> None:
        if not self._closed:
            self._closed = True
            await self._close(error)


def encode(value: J.JsonValue, framing: str, maximum: int, source: Source) -> bytes:
    if framing == 'json-lines':
        return text_bytes(J.stringify_json(value) + '\n', maximum, source)
    if type(value) is not dict:
        raise SdkError('request-representation', source, code='http-sse-envelope')
    event = value
    if 'data' not in event or any(key not in ('data', 'event', 'id', 'retry') for key in event):
        raise SdkError('request-representation', source, code='http-sse-envelope')
    pieces: list[str] = []
    for key in ('id', 'event', 'retry', 'data'):
        if key not in event:
            continue
        item = event[key]
        if key == 'retry':
            if type(item) is J.JsonNumber:
                item = item.to_int()
            if type(item) is not int or item < 0:
                raise SdkError('request-representation', source, code='http-sse-retry')
            pieces.append('retry:' + J.stringify_json(item) + '\n')
        else:
            if type(item) is not str or '\r' in item or key != 'data' and ('\n' in item or '\0' in item):
                raise SdkError('request-representation', source, code='http-sse-field')
            # One separator space is removed by the HTML parser; adding it
            # preserves any leading spaces that belong to the field value.
            pieces.extend(key + ': ' + line + '\n' for line in item.split('\n'))
    return text_bytes(''.join(pieces) + '\n', maximum, source)
