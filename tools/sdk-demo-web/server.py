#!/usr/bin/env python3
"""A loopback-only local showcase for the accepted native OpenRouter SDKs."""
from __future__ import annotations

import argparse
import hmac
import json
import os
import re
import secrets
import signal
import socket
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

sys.dont_write_bytecode = True

from catalog import LANGUAGES, REFERENCE, REPO, SOURCE_SERVER, document_paths, source_paths
from jobs import MAX_CONCURRENT, SESSION_LIMIT, JobManager, RequestError
from runtime import AcceptedRuntime, PROCESS_DEADLINE

STATIC = Path(__file__).resolve().parent / "static"
MAX_BODY = 8192
REQUEST_TIMEOUT = 4.0
JOB_ID = re.compile(r"[a-f0-9]{32}\Z")
CSP = "; ".join((
    "default-src 'self'", "script-src 'self'", "style-src 'self'",
    "connect-src 'self'", "img-src 'self'", "font-src 'self'",
    "object-src 'none'", "base-uri 'none'", "frame-ancestors 'none'", "form-action 'self'",
))


def fresh_root() -> Path:
    parent = REPO / "target"
    index = 1
    while True:
        path = parent / f"sdk-demo-web-20260911-{index:02}"
        try:
            path.mkdir(mode=0o700)
            return path
        except FileExistsError:
            index += 1


def save_new(path: Path, data: Any) -> None:
    with path.open("x") as stream:
        json.dump(data, stream, indent=2)
        stream.write("\n")


class LocalServer(ThreadingHTTPServer):
    daemon_threads = True
    block_on_close = False
    request_queue_size = 16
    allow_reuse_address = True

    def __init__(self, port: int, runtime: Any, evidence: Path, token: str = "") -> None:
        self.nonce = secrets.token_urlsafe(32)
        self.runtime = runtime
        self.proof = runtime.provenance()
        self.evidence = evidence
        self._slots = threading.BoundedSemaphore(16)
        self.assets = {
            "/": (STATIC / "index.html", "text/html; charset=utf-8"),
            "/app.js": (STATIC / "app.js", "text/javascript; charset=utf-8"),
            "/style.css": (STATIC / "style.css", "text/css; charset=utf-8"),
            "/favicon.svg": (STATIC / "favicon.svg", "image/svg+xml"),
            "/reference": (REFERENCE, "text/plain; charset=utf-8"),
            **{f"/source/{key}": (path, "text/plain; charset=utf-8") for key, path in source_paths().items()},
            **{f"/docs/{key}": (path, "text/plain; charset=utf-8") for key, path in document_paths().items()},
        }
        super().__init__(("127.0.0.1", port), Handler)
        self.manager = JobManager(runtime, evidence / "jobs", token)
        self.port = self.server_address[1]
        self.hosts = {f"127.0.0.1:{self.port}", f"localhost:{self.port}"}
        if self.port == 80:
            self.hosts.update(("127.0.0.1", "localhost"))
        self.origins = {f"http://{host}" for host in self.hosts}

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(REQUEST_TIMEOUT)
        return connection, address

    def process_request(self, request, client_address):
        if not self._slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self._slots.release()
            self.shutdown_request(request)

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._slots.release()

    def handle_error(self, request, client_address):
        # No request bodies, URL text, native output, or exception strings in logs.
        pass


class Handler(BaseHTTPRequestHandler):
    server: LocalServer
    server_version = "SDKLocal"
    sys_version = ""
    protocol_version = "HTTP/1.0"

    def handle(self):
        # Absolute connection deadline, including slowly supplied request headers.
        def expire():
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        deadline = threading.Timer(REQUEST_TIMEOUT, expire)
        deadline.daemon = True
        deadline.start()
        try:
            super().handle()
        finally:
            deadline.cancel()

    def log_message(self, format, *args):
        pass

    def send_error(self, code, message=None, explain=None):
        self._json(code, {"error": "invalid-request", "message": "The local server could not accept that request."})

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Security-Policy", CSP)
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, status: int, value: Any) -> None:
        self._send(status, json.dumps(value, ensure_ascii=True).encode(), "application/json; charset=utf-8")

    def _check_local(self, api: bool = False, mutation: bool = False) -> None:
        hosts = self.headers.get_all("Host", [])
        if len(hosts) != 1 or hosts[0].lower() not in self.server.hosts:
            raise RequestError(403, "local-host-required", "Open the loopback URL printed by ./demo-web.sh.")
        origins = self.headers.get_all("Origin", [])
        if len(origins) > 1 or (origins and origins[0] not in self.server.origins) or (mutation and not origins):
            raise RequestError(403, "same-origin-required", "Use the controls on this local page.")
        site = self.headers.get("Sec-Fetch-Site")
        if site is not None and site not in ("same-origin", "none"):
            raise RequestError(403, "same-origin-required", "Use the controls on this local page.")
        if len(self.path) > 2048 or "?" in self.path or "#" in self.path:
            raise RequestError(400, "invalid-path", "Use the local page's fixed routes.")
        if api:
            nonces = self.headers.get_all("X-SDK-Demo-Nonce", [])
            if len(nonces) != 1 or not nonces[0].isascii() or not hmac.compare_digest(nonces[0], self.server.nonce):
                raise RequestError(403, "reload-required", "Reload the local page to reconnect to this server session.")

    def _body(self) -> dict:
        lengths = self.headers.get_all("Content-Length", [])
        if self.headers.get("Transfer-Encoding") is not None or len(lengths) != 1 or not lengths[0].isascii() or not lengths[0].isdigit():
            raise RequestError(400, "invalid-body", "A bounded JSON request is required.")
        if len(lengths[0]) > 8:
            raise RequestError(413, "request-too-large", "The local request is too large.")
        length = int(lengths[0])
        if length > MAX_BODY:
            raise RequestError(413, "request-too-large", "The local request is too large.")
        types = self.headers.get_all("Content-Type", [])
        if len(types) != 1 or types[0].split(";", 1)[0].strip().lower() != "application/json":
            raise RequestError(415, "json-required", "Use the local page's JSON controls.")
        raw = self.rfile.read(length)
        if len(raw) != length:
            raise RequestError(400, "incomplete-body", "The local request was interrupted.")
        try:
            def unique(pairs):
                result = {}
                for name, value in pairs:
                    if name in result:
                        raise ValueError("Duplicate field")
                    result[name] = value
                return result
            value = json.loads(raw, object_pairs_hook=unique)
            if not isinstance(value, dict):
                raise ValueError("Expected object")
            return value
        except (ValueError, UnicodeError, RecursionError):
            raise RequestError(400, "invalid-json", "The local request must be a JSON object.") from None

    def _dispatch(self, method: str) -> None:
        try:
            self._check_local(api=self.path.startswith("/api/"), mutation=method == "POST")
            if method in ("GET", "HEAD"):
                if self.path == "/api/readiness":
                    self._json(200, {
                        "mode": self.server.runtime.mode, "cards": self.server.manager.cards,
                        "nativeTargets": len(LANGUAGES), "sourceServer": SOURCE_SERVER,
                        "operation": "getCurrentKey", "route": "/key",
                        "limits": {"concurrency": MAX_CONCURRENT, "deadlineSeconds": PROCESS_DEADLINE, "sessionJobs": SESSION_LIMIT},
                        "credential": self.server.manager.snapshot()["credential"],
                        "preflight": {"nativeExecutions": 0, "liveRequests": 0},
                    })
                elif self.path == "/api/jobs":
                    self._json(200, self.server.manager.snapshot())
                elif self.path == "/api/provenance":
                    self._json(200, self.server.proof)
                elif self.path.startswith("/api/jobs/") and JOB_ID.fullmatch(self.path[10:]):
                    self._json(200, self.server.manager.get(self.path[10:]))
                elif self.path in self.server.assets:
                    path, kind = self.server.assets[self.path]
                    body = path.read_bytes()
                    if self.path == "/":
                        body = body.replace(b"__BOOTSTRAP_NONCE__", self.server.nonce.encode())
                    self._send(200, body, kind)
                else:
                    raise RequestError(404, "not-found", "That local resource was not found.")
            elif method == "POST":
                if self.path not in ("/api/credential", "/api/jobs", "/api/cancel"):
                    raise RequestError(404, "not-found", "That local action was not found.")
                body = self._body()
                if self.path == "/api/credential" and set(body) == {"token"}:
                    credential = self.server.manager.set_token(body.pop("token"))
                    self._json(200, {"credential": credential})
                elif self.path == "/api/jobs" and set(body) == {"language", "operation"}:
                    self._json(202, self.server.manager.start(body["language"], body["operation"]))
                elif self.path == "/api/cancel" and set(body) == {"all"} and body["all"] is True:
                    self.server.manager.cancel()
                    self._json(200, {"cancelRequested": True})
                elif self.path == "/api/cancel" and set(body) == {"id"} and isinstance(body["id"], str) and JOB_ID.fullmatch(body["id"]):
                    self.server.manager.cancel(body["id"])
                    self._json(200, {"cancelRequested": True})
                else:
                    raise RequestError(400, "invalid-fields", "Use the fixed actions provided by this page.")
            else:
                raise RequestError(405, "method-not-allowed", "Use the local page's controls.")
        except RequestError as error:
            self._json(error.status, {"error": error.code, "message": error.message})
        except (BrokenPipeError, ConnectionResetError, socket.timeout):
            self.close_connection = True
        except Exception:
            self._json(500, {"error": "local-server-error", "message": "The local action could not finish. Reload this page and try again."})

    def do_GET(self):
        self._dispatch("GET")

    def do_HEAD(self):
        self._dispatch("HEAD")

    def do_POST(self):
        self._dispatch("POST")

    def do_OPTIONS(self):
        self._dispatch("OPTIONS")

    def do_PUT(self):
        self._dispatch("PUT")

    def do_DELETE(self):
        self._dispatch("DELETE")


def serve(server: LocalServer) -> None:
    stop = threading.Event()
    previous = {}
    for sig in (signal.SIGINT, signal.SIGTERM):
        previous[sig] = signal.signal(sig, lambda *_: stop.set())
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True, name="sdk-demo-http")
    thread.start()
    try:
        print(f"SDK showcase: http://127.0.0.1:{server.port}", flush=True)
        print(f"Prepared: {sum(card['ready'] and card['native'] for card in server.manager.cards)}/12 native SDKs. Live requests start only when you click Run.", flush=True)
        print(f"Receipts: {server.evidence.relative_to(REPO)}", flush=True)
        stop.wait()
    finally:
        server.shutdown()
        server.manager.close()
        server.server_close()
        thread.join(timeout=2)
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        print("Local SDK server stopped; owned jobs cancelled.", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765, help="Loopback port (default: 8765; 0 chooses an available port)")
    parser.add_argument("--preflight", action="store_true", help="Verify all prepared SDK pins without executing a native request")
    args = parser.parse_args()
    if not 0 <= args.port <= 65535:
        parser.error("Port must be between 0 and 65535")
    token = os.environ.pop("OPENROUTER_API_KEY", "")
    try:
        runtime = AcceptedRuntime()
        evidence = fresh_root()
        save_new(evidence / "preflight.json", runtime.provenance())
        ready = sum(language in runtime.ready for language in LANGUAGES)
        if args.preflight:
            for card in runtime.cards():
                print(f"{card['name']}: {'READY' if card['ready'] else 'UNAVAILABLE'} — pinned preflight; no native request executed")
            print(f"Preparation: {ready}/12 native SDKs. Evidence: {evidence.relative_to(REPO)}")
            return int(ready != 12)
        server = LocalServer(args.port, runtime, evidence, token)
        token = ""
        save_new(evidence / "server.json", {"url": f"http://127.0.0.1:{server.port}", "mode": runtime.mode,
                                          "pid": os.getpid(), "liveRequestsAtStartup": 0, "tokenRecorded": False})
        serve(server)
        return 0
    except (OSError, RuntimeError, ValueError, KeyError):
        print("The local showcase could not start. Check the prepared runtime pins and choose an available port with --port.", file=sys.stderr)
        return 1
    finally:
        token = ""


if __name__ == "__main__":
    raise SystemExit(main())
