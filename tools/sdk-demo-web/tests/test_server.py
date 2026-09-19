from __future__ import annotations

import http.client
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from controlled import CANARY, GateRuntime, eventually
from catalog import REPO
from server import LocalServer, MAX_BODY, REQUEST_TIMEOUT


class LocalHTTPTests(unittest.TestCase):
    def setUp(self):
        work = REPO / "target/sdk-demo-web-20260911-01/test-work"
        work.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=work)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime = GateRuntime()
        self.server = LocalServer(0, self.runtime, self.root)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop)

    def stop(self):
        self.server.shutdown()
        self.server.manager.close()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self, method="GET", path="/api/readiness", body=None, headers=None, *, nonce=True, origin=True):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.port, timeout=5)
        headers = dict(headers or {})
        if nonce:
            headers.setdefault("X-SDK-Demo-Nonce", self.server.nonce)
        if origin:
            headers.setdefault("Origin", f"http://127.0.0.1:{self.server.port}")
        if body is not None and not isinstance(body, (bytes, str)):
            body = json.dumps(body)
        if body is not None:
            headers.setdefault("Content-Type", "application/json")
        connection.request(method, path, body, headers)
        response = connection.getresponse()
        data = response.read()
        result = (response.status, dict(response.getheaders()), data)
        connection.close()
        return result

    def test_bootstrap_html_fixed_assets_and_no_initial_success(self):
        status, headers, body = self.request(path="/", nonce=False, origin=False)
        self.assertEqual(status, 200)
        self.assertIn(self.server.nonce.encode(), body)
        self.assertNotIn(b"__BOOTSTRAP_NONCE__", body)
        self.assertIn(b'type="password"', body)
        self.assertIn("frame-ancestors 'none'", headers["Content-Security-Policy"])
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(headers["Referrer-Policy"], "no-referrer")
        self.assertNotIn("Access-Control-Allow-Origin", headers)
        status, _, body = self.request()
        self.assertEqual(status, 200)
        ready = json.loads(body)
        self.assertEqual(sum(card["native"] for card in ready["cards"]), 12)
        self.assertFalse(ready["credential"]["ready"])
        self.assertEqual(self.server.manager.snapshot()["counts"]["completed"], 0)
        self.assertEqual(self.runtime.started, [])
        for path in ("/app.js", "/style.css", "/favicon.svg", "/source/python", "/docs/cpp", "/reference"):
            self.assertEqual(self.request(path=path, nonce=False)[0], 200)

    def test_api_requires_nonce_and_foreign_origin_is_rejected(self):
        self.assertEqual(self.request(nonce=False)[0], 403)
        self.assertEqual(self.request(headers={"X-SDK-Demo-Nonce": "incorrect"})[0], 403)
        self.assertEqual(self.request(headers={"Host": "untrusted.invalid"})[0], 403)
        for origin in ("https://untrusted.invalid", "null", "http://127.0.0.1:1"):
            status, _, data = self.request("POST", "/api/credential", {"token": CANARY}, headers={"Origin": origin})
            self.assertEqual(status, 403)
            self.assertNotIn(CANARY.encode(), data)
        self.assertEqual(self.request("POST", "/api/credential", {"token": CANARY}, origin=False)[0], 403)
        self.assertFalse(self.server.manager.snapshot()["credential"]["ready"])

    def test_cross_site_fetch_is_rejected_even_with_correct_host(self):
        for site in ("cross-site", "same-site"):
            self.assertEqual(self.request(headers={"Sec-Fetch-Site": site})[0], 403)

    def test_token_post_returns_readiness_only_and_clear_is_memory_only(self):
        status, _, data = self.request("POST", "/api/credential", {"token": CANARY})
        self.assertEqual(status, 200)
        self.assertTrue(json.loads(data)["credential"]["ready"])
        self.assertEqual(set(json.loads(data)["credential"]), {"ready"})
        self.assertNotIn(CANARY.encode(), data)
        self.assertNotIn(json.dumps(CANARY)[1:-1].encode(), data)
        self.assertEqual(self.runtime.started, [])
        self.assertEqual(list((self.root / "jobs").iterdir()), [])
        status, _, data = self.request("POST", "/api/credential", {"token": ""})
        self.assertEqual(status, 200)
        self.assertFalse(json.loads(data)["credential"]["ready"])

    def test_fixed_fields_and_read_only_operation_never_dispatch_arbitrary_input(self):
        self.request("POST", "/api/credential", {"token": CANARY})
        attempts = (
            {"language": "python", "operation": "key", "argv": ["unused"]},
            {"language": "python", "operation": "key", "cwd": "unused"},
            {"language": "python", "operation": "credits"},
            {"language": "python", "operation": "delete"},
            {"language": "../../python", "operation": "key"},
            {"language": ["python"], "operation": "key"},
        )
        for body in attempts:
            self.assertEqual(self.request("POST", "/api/jobs", body)[0], 400)
        self.assertEqual(self.server.manager.snapshot()["submitted"], 0)
        self.assertEqual(self.runtime.started, [])
        for path in ("/source/../server.py", "/source/%2e%2e/server.py", "/api/jobs?token=unused", "/anything"):
            self.assertIn(self.request(path=path)[0], (400, 404))

    def test_body_size_content_type_duplicate_fields_and_errors_are_bounded(self):
        status, _, body = self.request("POST", "/api/credential", b"x" * (MAX_BODY + 1))
        self.assertEqual(status, 413)
        self.assertLess(len(body), 512)
        self.assertEqual(self.request("POST", "/api/credential", {"token": CANARY}, headers={"Content-Type": "text/plain"})[0], 415)
        duplicate = '{"token":"one-controlled-key","token":"second-controlled-key"}'
        self.assertEqual(self.request("POST", "/api/credential", duplicate)[0], 400)
        for body in ("not-json", "[]", '{"token":null}', '{"token":"short"}', '{"token":"bad\\nkey-value"}'):
            status, _, response = self.request("POST", "/api/credential", body)
            self.assertEqual(status, 400)
            self.assertNotIn(CANARY.encode(), response)

    def test_async_start_keeps_readiness_and_cancel_responsive(self):
        self.request("POST", "/api/credential", {"token": CANARY})
        start = time.monotonic()
        status, _, body = self.request("POST", "/api/jobs", {"language": "all", "operation": "key"})
        self.assertEqual(status, 202)
        self.assertLess(time.monotonic() - start, 0.25)
        self.assertEqual(json.loads(body)["accepted"], 12)
        eventually(lambda: self.server.manager.snapshot()["counts"]["running"] == 3)
        for path in ("/api/jobs", "/api/readiness", "/api/provenance"):
            start = time.monotonic()
            self.assertEqual(self.request(path=path)[0], 200)
            self.assertLess(time.monotonic() - start, 0.25)
        self.assertEqual(self.request("POST", "/api/cancel", {"all": True})[0], 200)
        eventually(lambda: self.server.manager.snapshot()["counts"]["cancelled"] == 12)

    def test_absolute_request_deadline_closes_incomplete_headers(self):
        client = socket.create_connection(("127.0.0.1", self.server.port), timeout=REQUEST_TIMEOUT + 2)
        self.addCleanup(client.close)
        start = time.monotonic()
        client.sendall(f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{self.server.port}\r\n".encode())
        try:
            data = client.recv(4096)
        except ConnectionResetError:
            data = b""
        self.assertLess(time.monotonic() - start, REQUEST_TIMEOUT + 1)
        self.assertEqual(data, b"")

    def test_server_signal_shutdown_cancels_owned_child_group(self):
        evidence = self.root / "signal-server"
        environment = {name: value for name, value in os.environ.items() if name != "OPENROUTER_API_KEY" and not name.startswith("SDK_DEMO_")}
        process = subprocess.Popen([sys.executable, "-B", str(Path(__file__).with_name("controlled_server.py")), "--port", "0", "--scenario", "shutdown", "--evidence-dir", str(evidence)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment, start_new_session=True)
        self.addCleanup(lambda: process.kill() if process.poll() is None else None)
        eventually(lambda: (evidence / "server.json").exists(), timeout=10)
        port = int(json.loads((evidence / "server.json").read_text())["url"].rsplit(":", 1)[1])
        def request(path, payload=None, nonce=None):
            client = http.client.HTTPConnection("127.0.0.1", port, timeout=4)
            headers = {"Origin": f"http://127.0.0.1:{port}"}
            if nonce:
                headers["X-SDK-Demo-Nonce"] = nonce
            if payload is not None:
                headers["Content-Type"] = "application/json"
            client.request("GET" if payload is None else "POST", path, None if payload is None else json.dumps(payload), headers)
            response = client.getresponse()
            body = response.read()
            self.assertIn(response.status, (200, 202))
            client.close()
            return body
        html = request("/").decode()
        nonce = re.search(r'name="sdk-demo-nonce" content="([^"]+)"', html).group(1)
        request("/api/credential", {"token": CANARY}, nonce)
        request("/api/jobs", {"language": "python", "operation": "key"}, nonce)
        eventually(lambda: (evidence / "child-pids.json").exists())
        pids = json.loads((evidence / "child-pids.json").read_text())
        process.send_signal(signal.SIGTERM)
        stdout, stderr = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0)
        self.assertNotIn(CANARY.encode(), stdout + stderr)
        for pid in pids.values():
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
        receipts = list((evidence / "jobs").glob("*.json"))
        self.assertEqual(len(receipts), 1)
        self.assertEqual(json.loads(receipts[0].read_text())["job"]["state"], "cancelled")


if __name__ == "__main__":
    unittest.main()
