#!/usr/bin/env python3
"""Controlled preparation verification. This is never called by demo-live.sh."""
from __future__ import annotations
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import threading

from common import LANGUAGES, REPO, ROOT, SOURCES, fresh, save, sha
from run import capture, validate_response

CANARY = "sdk-live-verification-canary-not-a-real-token"

def execution_pins(info: dict) -> dict[str,str]:
    pins = {}
    attempt = Path(info["attempt"])
    for argv in [info["argv"], *([info["javascript"]] if info["javascript"] else [])]:
        for arg in argv:
            for text in arg.split(os.pathsep):
                path = Path(text)
                if path.is_absolute() and path.is_file(): pins[str(path)] = sha(path)
                elif path.is_absolute() and path.is_dir() and path.is_relative_to(attempt):
                    for file in path.rglob("*"):
                        if file.is_file(): pins[str(file)] = sha(file)
    consumer = Path(info["consumer"])
    package = Path(info["package"])
    # Pin imported native package/runtime data without compiler/cache intermediates.
    roots = [consumer / "node_modules/@openrouter/sdk", consumer / "vendor/openrouter/sdk",
             consumer / "bin/Release/net8.0", package / "lib", attempt / "gems/gems/openrouter-0.1.0/lib"]
    roots += list((attempt / "venv/lib").glob("python*/site-packages/openrouter"))
    for root in roots:
        if root.is_dir():
            for path in root.rglob("*"):
                if path.is_file() and "__pycache__" not in path.parts: pins[str(path)] = sha(path)
    return pins

def verify(language: str) -> None:
    paths = sorted((ROOT / "native").glob(f"{language}-*/ready.json"), key=lambda p:int(p.parent.name.rsplit("-",1)[1]), reverse=True)
    if not paths: raise RuntimeError(f"{language} has not built")
    info = json.loads(paths[0].read_text())
    if (paths[0].parent / "verification.json").exists(): raise RuntimeError("Verification already retained; use a new native attempt for changed code")
    fixtures = json.loads((SOURCES / "verification/responses.json").read_text())
    receipt = fresh(ROOT / "verification", language)
    wire = []
    expected_path = "/api/v1/key"
    selected_fixture = "key"
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*args): pass
        def do_GET(self):
            valid = self.path == expected_path and self.headers.get("Authorization") == "Bearer " + CANARY
            wire.append({"method":"GET","path":self.path,"bearerMatchesCanary":self.headers.get("Authorization") == "Bearer " + CANARY,"valid":valid})
            body = fixtures[selected_fixture].encode()
            self.send_response(401 if selected_fixture == "denied" else 200 if valid else 500)
            self.send_header("Content-Type","application/json")
            self.send_header("Content-Length",str(len(body)))
            self.end_headers(); self.wfile.write(body)
    server = ThreadingHTTPServer(("127.0.0.1",0),Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever,daemon=True); thread.start()
    results = []
    try:
        commands = [(language,info["argv"])] + ([("javascript",info["javascript"])] if info["javascript"] else [])
        for name, argv in commands:
            for operation, fixture in (("key","key"),("credits","credits"),("key","denied")):
                selected_fixture = fixture
                expected_path = "/api/v1/key" if operation == "key" else "/api/v1/credits"
                env = {key:value for key,value in os.environ.items() if key != "OPENROUTER_API_KEY" and not key.startswith("SDK_DEMO_")}
                env.update(info["environment"])
                env.update(OPENROUTER_API_KEY=CANARY, SDK_DEMO_OPERATION=operation, SDK_DEMO_VERIFY="1", SDK_DEMO_TEST_URL=f"http://127.0.0.1:{server.server_port}/api/v1")
                before = len(wire)
                raw = capture(argv, Path(info["consumer"]), env)
                label = f"{name}-{fixture}"
                (receipt / f"{label}.stdout").write_text(raw["stdout"])
                (receipt / f"{label}.stderr").write_text(raw["stderr"])
                response = validate_response(raw["stdout"],operation)
                okay = raw["exitCode"] == (1 if fixture == "denied" else 0) and raw["reason"] is None
                okay &= len(wire) == before+1 and wire[-1]["valid"]
                if fixture == "denied": okay &= response.get("ok") is False and response.get("status") == 401
                else:
                    okay &= response.get("ok") is True and response.get("status") == 200
                    okay &= response.get("usage") == ("9007199254740993.000000000000000001" if fixture == "key" else "25.75")
                results.append({"consumer":name,"fixture":fixture,"operation":operation,"exitCode":raw["exitCode"],"passed":okay,"response":response})
                if not okay: raise RuntimeError(f"Controlled check failed: {label}: {raw}")
    finally:
        server.shutdown(); server.server_close(); thread.join()
        save(receipt / "wire.json",wire)
        save(receipt / "REPORT.json",{"mode":"controlled-verification","realApiContacted":False,"results":results})
    proof = {"passed":True,"mode":"controlled-verification-not-live","receipt":str(receipt),"checks":len(results),"executionPins":execution_pins(info),"sourceDefaultServer":"https://openrouter.ai/api/v1","liveExecuted":False}
    save(paths[0].parent / "verification.json", proof)
    print(f"CONTROLLED VERIFIED {language}: key/credits/typed401; no real API contacted")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language",choices=(*LANGUAGES,"all"))
    args = parser.parse_args()
    for language in LANGUAGES if args.language == "all" else (args.language,): verify(language)
