#!/usr/bin/env python3
"""A prepared TypeScript AsyncIterable over a normative, loopback-only OAS 3.2 fixture."""
from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import threading

from demo import REPO, owned_root, record, sha, verify_pins, write_json
from native import tools


def prepare(root: Path) -> None:
    work = root / "standard-stream"
    work.mkdir(exist_ok=False)
    source = REPO / "examples/sdk-demo-all/stream.openapi.json"
    t = tools()
    env = {"PATH": str(t["node"].parent) + os.pathsep + os.environ["PATH"]}

    def run(label: str, argv: list[str | Path], cwd: Path = work) -> str:
        return record(work, label, list(map(str, argv)), cwd=cwd, env=env).stdout

    run("generate", [root / "bin/suspect", "codegen", source, "--profile", "typescript-http",
        "--package-name", "@demo/standard-stream-sdk", "--package-version", "0.1.0", "--operation-id", "events",
        "--out", work / "generated", "--format", "json"])
    package = work / "package"
    shutil.copytree(work / "generated/typescript", package)
    run("npm-ci", [t["npm"], "ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], package)
    run("sdk-build", [t["npm"], "run", "build"], package)
    run("pack", [t["npm"], "pack", "--ignore-scripts", "--pack-destination", work], package)
    consumer = work / "consumer"
    consumer.mkdir()
    archive = next(work.glob("*.tgz"))
    write_json(consumer / "package.json", {"private": True, "type": "module", "dependencies": {
        "@demo/standard-stream-sdk": f"file:{archive}"}})
    run("install", [t["npm"], "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], consumer)
    snippet = REPO / "examples/sdk-demo-all/stream/main.ts"
    shutil.copy2(snippet, consumer / "main.ts")
    run("strict-types", [t["node"], package / "node_modules/typescript/bin/tsc", "--strict", "--target", "ES2022",
        "--module", "NodeNext", "--moduleResolution", "NodeNext", "--lib", "ES2023,DOM", "--outDir", "build", "main.ts"], consumer)
    write_json(work / "prepared.json", {"source": str(source), "sourceSha256": sha(source),
        "snippet": str(snippet), "snippetSha256": sha(snippet), "archiveSha256": sha(archive),
        "argv": [str(t["node"]), str(consumer / "build/main.js")], "cwd": str(consumer),
        "scope": "normative OpenAPI 3.2 SSE envelopes; no vendor conventions"})
    print(f"STANDARD STREAM PREPARED: {work}")


def run_demo(root: Path) -> None:
    work = root / "standard-stream"
    info = json.loads((work / "prepared.json").read_text())
    if sha(Path(info["source"])) != info["sourceSha256"] or sha(Path(info["snippet"])) != info["snippetSha256"]:
        raise SystemExit("Stream source/snippet changed since preparation.")
    wire = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args: object) -> None:
            pass

        def do_GET(self) -> None:
            wire.append(self.path)
            body = b'data: hello\n\ndata: [DONE]\n\ndata: {"value":1}\n\n'
            self.send_response(200 if self.path == "/api/v1/events" else 404)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        result = record(work, "offline-stream", info["argv"], cwd=Path(info["cwd"]),
            env={"SDK_DEMO_URL": f"http://127.0.0.1:{server.server_port}/api/v1"}, timeout=20)
        if "stream OFFLINE OK" not in result.stdout or wire != ["/api/v1/events"]:
            raise SystemExit("Stream fixture failed.")
        print(result.stdout.strip())
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("action", choices=("prepare", "run"))
    args = parser.parse_args()
    root = owned_root(args.root)
    verify_pins(root)
    prepare(root) if args.action == "prepare" else run_demo(root)


if __name__ == "__main__":
    main()
