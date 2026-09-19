"""Installed-package tests at the documented httpx transport boundary."""
import asyncio
import importlib
import json
from pathlib import Path
import sys
import unittest

import httpx

CONFIG = json.loads(Path("consumer-config.json").read_text())
SDK = importlib.import_module(CONFIG["package"])
BODY = CONFIG["valid_body"].encode("utf-8")
ARGUMENTS = CONFIG.get("arguments", {})
AUTH = {"apiKey": "explicit-regression-credential"}
CLOSE_MARKER = "private-close-diagnostic"
READ_MARKER = "private-read-diagnostic"


def call(client):
    return getattr(client, CONFIG["method"])(**ARGUMENTS)


class SyncBody(httpx.SyncByteStream):
    def __init__(self, read_error=None):
        self.read_error = read_error
        self.close_error = RuntimeError(CLOSE_MARKER)
        self.closed = False

    def __iter__(self):
        if self.read_error is not None:
            yield b'{'
            raise self.read_error
        yield BODY

    def close(self):
        self.closed = True
        raise self.close_error


class SyncTransport(httpx.BaseTransport):
    def __init__(self, stream):
        self.stream = stream

    def handle_request(self, request):
        return httpx.Response(200, headers={"Content-Type": "application/json"}, stream=self.stream)


class AsyncBody(httpx.AsyncByteStream):
    def __init__(self, read_error=None, started=None):
        self.read_error = read_error
        self.started = started
        self.close_error = RuntimeError(CLOSE_MARKER)
        self.closed = False

    async def __aiter__(self):
        if self.read_error is not None:
            yield b'{'
            raise self.read_error
        if self.started is not None:
            yield b'{'
            self.started.set()
            await asyncio.Event().wait()
            return
        yield BODY

    async def aclose(self):
        self.closed = True
        raise self.close_error


class AsyncTransport(httpx.AsyncBaseTransport):
    def __init__(self, stream):
        self.stream = stream

    async def handle_async_request(self, request):
        return httpx.Response(200, headers={"Content-Type": "application/json"}, stream=self.stream)


def assert_redacted(test, error):
    for marker in (CLOSE_MARKER, READ_MARKER, AUTH["apiKey"]):
        test.assertNotIn(marker, str(error))
        test.assertNotIn(marker, repr(error))


class SyncCleanup(unittest.TestCase):
    def test_close_failure_preserves_primary_response_limit(self):
        stream = SyncBody()
        with SDK.Client(auth=AUTH, transport=SyncTransport(stream),
                        max_response_bytes=4, max_capture_bytes=2) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                call(client)
        error = caught.exception
        self.assertEqual(error.kind, "resource-limit")
        self.assertEqual(error.status, 200)
        self.assertTrue(error.truncated)
        self.assertLessEqual(len(error.capture), 2)
        self.assertTrue(stream.closed)
        assert_redacted(self, error)

    def test_close_failure_preserves_primary_read_error(self):
        primary = httpx.ReadError(READ_MARKER)
        stream = SyncBody(primary)
        with SDK.Client(auth=AUTH, transport=SyncTransport(stream), max_capture_bytes=2) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                call(client)
        error = caught.exception
        self.assertEqual(error.kind, "transport")
        self.assertIs(error.cause, primary)
        self.assertTrue(error.truncated)
        self.assertLessEqual(len(error.capture), 2)
        self.assertTrue(stream.closed)
        assert_redacted(self, error)

    def test_standalone_close_failure_is_a_redacted_transport_error(self):
        stream = SyncBody()
        with SDK.Client(auth=AUTH, transport=SyncTransport(stream)) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                call(client)
        self.assertEqual(caught.exception.kind, "transport")
        self.assertIs(caught.exception.cause, stream.close_error)
        self.assertTrue(stream.closed)
        assert_redacted(self, caught.exception)


class AsyncCleanup(unittest.IsolatedAsyncioTestCase):
    async def test_close_failure_preserves_primary_response_limit(self):
        stream = AsyncBody()
        async with SDK.AsyncClient(auth=AUTH, transport=AsyncTransport(stream),
                                   max_response_bytes=4, max_capture_bytes=2) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                await call(client)
        error = caught.exception
        self.assertEqual(error.kind, "resource-limit")
        self.assertEqual(error.status, 200)
        self.assertTrue(error.truncated)
        self.assertLessEqual(len(error.capture), 2)
        self.assertTrue(stream.closed)
        assert_redacted(self, error)

    async def test_close_failure_preserves_primary_read_error(self):
        primary = httpx.ReadError(READ_MARKER)
        stream = AsyncBody(primary)
        async with SDK.AsyncClient(auth=AUTH, transport=AsyncTransport(stream), max_capture_bytes=2) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                await call(client)
        error = caught.exception
        self.assertEqual(error.kind, "transport")
        self.assertIs(error.cause, primary)
        self.assertTrue(error.truncated)
        self.assertLessEqual(len(error.capture), 2)
        self.assertTrue(stream.closed)
        assert_redacted(self, error)

    async def test_standalone_close_failure_is_a_redacted_transport_error(self):
        stream = AsyncBody()
        async with SDK.AsyncClient(auth=AUTH, transport=AsyncTransport(stream)) as client:
            with self.assertRaises(SDK.SdkError) as caught:
                await call(client)
        self.assertEqual(caught.exception.kind, "transport")
        self.assertIs(caught.exception.cause, stream.close_error)
        self.assertTrue(stream.closed)
        assert_redacted(self, caught.exception)

    async def test_close_failure_preserves_caller_cancellation(self):
        started = asyncio.Event()
        stream = AsyncBody(started=started)
        async with SDK.AsyncClient(auth=AUTH, transport=AsyncTransport(stream)) as client:
            task = asyncio.create_task(call(client))
            try:
                # These waits detect deadlock; they are not latency assertions.
                await asyncio.wait_for(started.wait(), 5)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await asyncio.wait_for(task, 5)
                self.assertTrue(task.cancelled())
                self.assertTrue(stream.closed)
            finally:
                if not task.done():
                    task.cancel()
                await asyncio.gather(task, return_exceptions=True)


INVALID_SERVERS = (
    "http://127.0.0.1:bad/api",
    "http://127.0.0.1:-1/api",
    "http://127.0.0.1:65536/api",
    "http://127.0.0.1:999999/api",
)


class ServerPorts(unittest.TestCase):
    def test_invalid_ports_are_sdk_representation_failures_before_transport(self):
        seen = []
        def respond(request):
            seen.append(request)
            return httpx.Response(200, headers={"Content-Type": "application/json"}, content=BODY)
        with httpx.MockTransport(respond) as transport:
            for server in INVALID_SERVERS:
                with self.subTest(server=server):
                    before = len(seen)
                    with self.assertRaises(SDK.SdkError) as caught:
                        with SDK.Client(auth=AUTH, transport=transport, server_url=server) as client:
                            call(client)
                    self.assertEqual(caught.exception.kind, "request-representation")
                    self.assertEqual(len(seen), before)
                    assert_redacted(self, caught.exception)
            for port in (1, 65535):
                with self.subTest(valid_port=port):
                    with SDK.Client(auth=AUTH, transport=transport,
                                    server_url=f"http://127.0.0.1:{port}/api") as client:
                        result = call(client)
                    self.assertEqual(getattr(result.data, CONFIG["value_attribute"]), CONFIG["value_expected"])
                    self.assertEqual(seen[-1].url.port, port)


class AsyncServerPorts(unittest.IsolatedAsyncioTestCase):
    async def test_invalid_ports_are_sdk_representation_failures_before_transport(self):
        seen = []
        def respond(request):
            seen.append(request)
            return httpx.Response(200, headers={"Content-Type": "application/json"}, content=BODY)
        async with httpx.MockTransport(respond) as transport:
            for server in INVALID_SERVERS:
                with self.subTest(server=server):
                    before = len(seen)
                    with self.assertRaises(SDK.SdkError) as caught:
                        async with SDK.AsyncClient(auth=AUTH, transport=transport, server_url=server) as client:
                            await call(client)
                    self.assertEqual(caught.exception.kind, "request-representation")
                    self.assertEqual(len(seen), before)
                    assert_redacted(self, caught.exception)
            for port in (1, 65535):
                with self.subTest(valid_port=port):
                    async with SDK.AsyncClient(auth=AUTH, transport=transport,
                                               server_url=f"http://127.0.0.1:{port}/api") as client:
                        result = await call(client)
                    self.assertEqual(getattr(result.data, CONFIG["value_attribute"]), CONFIG["value_expected"])
                    self.assertEqual(seen[-1].url.port, port)


if __name__ == "__main__":
    print("NATIVE_PYTHON=" + json.dumps({"executable": sys.executable, "version": sys.version,
                                        "package": SDK.__file__, "httpx": httpx.__version__}), flush=True)
    unittest.main()
