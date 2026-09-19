#!/usr/bin/env python3
"""Bounded new-cohort checks for configured credentials; not the live entry point."""
from __future__ import annotations
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import sys
import threading

REPO = Path(__file__).resolve().parents[3]
PARENT = REPO / "target/sdk-demo-live-20260911-01/environment-01"

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language")
    parser.add_argument("--candidate", required=True)
    args = parser.parse_args()
    root = PARENT / args.candidate
    if root.parent != PARENT: raise SystemExit("Invalid candidate")
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(REPO / "tools/sdk-demo-live"))
    from common import LANGUAGES, fresh, save, sha
    from run import capture, validate_response
    spec = importlib.util.spec_from_file_location("installed_program_pins", REPO / "tools/sdk-demo-live/verify.py")
    assert spec is not None and spec.loader is not None
    support = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(support)
    languages = LANGUAGES if args.language == "all" else (args.language,)
    fixtures = json.loads((REPO / "examples/sdk-demo-live/verification/responses.json").read_text())
    canary = "environment-cohort-controlled-canary-not-a-real-token"
    for language in languages:
        if language not in LANGUAGES: raise SystemExit("Unknown language")
        choices = sorted((root / "native").glob(f"{language}-*/ready.json"), key=lambda p:int(p.parent.name.rsplit("-",1)[1]), reverse=True)
        if not choices: raise SystemExit(f"Not built: {language}")
        info = json.loads(choices[0].read_text())
        if (choices[0].parent / "environment-verification.json").exists():
            raise SystemExit("This successful check is already retained; use a fresh attempt for changed code")
        receipt = fresh(root / "verification", language)
        wire = []
        state = {"path":"/api/v1/key", "fixture":"key"}
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self):
                valid = self.path == state["path"] and self.headers.get("Authorization") == "Bearer " + canary
                wire.append({"method":"GET","path":self.path,"authMatchesCanary":self.headers.get("Authorization") == "Bearer "+canary,"valid":valid})
                payload = fixtures[state["fixture"]].encode()
                self.send_response(401 if state["fixture"] == "denied" else 200 if valid else 500)
                self.send_header("Content-Type","application/json")
                self.send_header("Content-Length",str(len(payload)))
                self.end_headers(); self.wfile.write(payload)
        server = ThreadingHTTPServer(("127.0.0.1",0),Handler)
        server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever,daemon=True)
        thread.start()
        outcomes = []
        try:
            consumers = [(language,info["argv"])] + ([("javascript",info["javascript"])] if info.get("javascript") else [])
            for name, argv in consumers:
                for operation, case in (("key","key"),("credits","credits"),("key","denied"),("key","missing-env")):
                    state["path"] = "/api/v1/key" if operation == "key" else "/api/v1/credits"
                    state["fixture"] = "key" if case == "missing-env" else case
                    env = {key:value for key,value in os.environ.items() if key != "OPENROUTER_API_KEY" and not key.startswith("SDK_DEMO_")}
                    env.update(info["environment"])
                    env.update(SDK_DEMO_OPERATION=operation,SDK_DEMO_VERIFY="1",SDK_DEMO_TEST_URL=f"http://127.0.0.1:{server.server_port}/api/v1")
                    if case != "missing-env": env["OPENROUTER_API_KEY"] = canary
                    count = len(wire)
                    raw = capture(argv,Path(info["consumer"]),env)
                    label = f"{name}-{case}"
                    (receipt/f"{label}.stdout").write_text(raw["stdout"])
                    (receipt/f"{label}.stderr").write_text(raw["stderr"])
                    assert canary not in raw["stdout"]+raw["stderr"]
                    response = validate_response(raw["stdout"],operation)
                    expected_exit = 1 if case in ("denied","missing-env") else 0
                    assert raw["exitCode"] == expected_exit and raw["reason"] is None,(name,case,raw)
                    if case == "missing-env":
                        assert response["ok"] is False and response.get("status") in (None,0)
                        assert len(wire) == count, (name,case,wire[count:])
                    else:
                        assert len(wire) == count+1 and wire[-1]["valid"]
                        if case == "denied": assert response["ok"] is False and response["status"] == 401
                        else:
                            assert response["ok"] is True and response["status"] == 200
                            assert response["usage"] == ("9007199254740993.000000000000000001" if case == "key" else "25.75")
                    outcomes.append({"consumer":name,"case":case,"operation":operation,"exitCode":raw["exitCode"],"response":response,"requests":len(wire)-count,"passed":True})
        finally:
            server.shutdown();server.server_close();thread.join()
            save(receipt/"wire.json",wire)
            save(receipt/"REPORT.json",{"mode":"controlled-new-env-cohort","realApiContacted":False,"outcomes":outcomes})
        proof = {"passed":True,"candidate":str(root),"packageCohortPinsSha256":sha(root/"package-pins.json"),"report":str(receipt/"REPORT.json"),"outcomes":len(outcomes),"executionPins":support.execution_pins(info),"liveExecuted":False}
        save(choices[0].parent/"environment-verification.json",proof)
        print(f"ENV controlled checks passed: {language}, {len(outcomes)} outcomes including missing env with zero HTTP")

if __name__ == "__main__": main()
