#!/usr/bin/env python3
"""No-network runner controls. Simulated results live only in a labeled check root."""
from __future__ import annotations
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import sys
import threading
from unittest.mock import patch

from common import REPO, ROOT, fresh, save
import run

def main() -> None:
    checks = fresh(ROOT / "checks", "runner")
    canary = "runner-control-canary-not-a-real-token"
    log = []
    marker = {"ok": True, "status": 200, "freeTier": False, "management": False, "usage": "1.00000000000000001"}
    fake = {"runArgv": [sys.executable, "-c", "pass"], "consumer": str(REPO), "packageConfig": {"package_name": "control-only"}, "environment": {}}
    responses = iter([
        {"exitCode":1,"stdout":'{"ok":false,"status":401,"kind":"declared-api-error"}',"stderr":canary,"reason":None,"truncated":{"stdout":False,"stderr":False}},
        {"exitCode":0,"stdout":json.dumps(marker),"stderr":"","reason":None,"truncated":{"stdout":False,"stderr":False}},
    ])
    def controlled_capture(argv, cwd, env):
        assert env["OPENROUTER_API_KEY"] == canary
        assert "SDK_DEMO_VERIFY" not in env and "SDK_DEMO_TEST_URL" not in env
        log.append({"argv":argv,"operation":env["SDK_DEMO_OPERATION"],"verificationSettingsRemoved":True})
        return next(responses)
    output = io.StringIO()
    with patch.object(run, "ROOT", checks / "SIMULATED-NO-NETWORK"), patch.object(run,"LANGUAGES",("typescript","python")), patch.object(run,"prepared",return_value=fake), patch.object(run,"capture",side_effect=controlled_capture), patch.dict(os.environ,{"OPENROUTER_API_KEY":canary,"SDK_DEMO_VERIFY":"1","SDK_DEMO_TEST_URL":"http://127.0.0.1:1"}), patch.object(sys,"argv",["demo-live.sh","all","--no-code"]), contextlib.redirect_stdout(output):
        code = run.main()
    assert code == 1 and len(log) == 2
    assert "1/2 SDKs confirmed success" in output.getvalue()
    assert canary not in output.getvalue()
    for file in (checks / "SIMULATED-NO-NETWORK").rglob("*"):
        if file.is_file(): assert canary not in file.read_text()
    (checks / "simulated-transcript.txt").write_text("CONTROLLED RUNNER TEST — NO NATIVE API EXECUTED\n"+output.getvalue())
    assert run.redact('plain '+canary+' escaped '+json.dumps(canary)[1:-1],canary).count("[REDACTED]") == 2

    timeout = run.capture([sys.executable,"-c","import time; time.sleep(60)"], REPO, {}, timeout=0.15)
    assert timeout["reason"] == "deadline-exceeded" and timeout["exitCode"] != 0
    child_pid = checks / "child.pid"
    def interrupt(): os.kill(os.getpid(), signal.SIGINT)
    timer = threading.Timer(0.2, interrupt)
    timer.start()
    cancelled = run.capture([sys.executable,"-c",f"import os,time; open({str(child_pid)!r},'w').write(str(os.getpid())); time.sleep(60)"], REPO, {}, timeout=5)
    timer.join()
    assert cancelled["reason"] == "cancelled" and cancelled["exitCode"] != 0
    try: os.kill(int(child_pid.read_text()),0)
    except ProcessLookupError: pass
    else: raise AssertionError("Cancelled native process survived")
    oversized = run.capture([sys.executable,"-c","print('x'*200000)"], REPO, {}, timeout=5)
    assert oversized["truncated"]["stdout"] and len(oversized["stdout"]) == 65536
    output = io.StringIO()
    environment = {key:value for key,value in os.environ.items() if key != "OPENROUTER_API_KEY"}
    with patch.object(run,"prepared",return_value=fake), patch.dict(os.environ,environment,clear=True), patch.object(sys,"argv",["demo-live.sh","python","--no-prompt"]), contextlib.redirect_stderr(output):
        missing = run.main()
    assert missing == 2 and "OPENROUTER_API_KEY" in output.getvalue()
    save(checks / "REPORT.json", {"mode":"controlled-runner-verification","realApiContacted":False,"passed":True,
        "failureContinuation":True,"nonzeroOnPartialFailure":True,"testOverridesRemoved":True,"secretRedaction":True,
        "deadlineCleanup":True,"ctrlCChildCleanup":True,"boundedOutput":True,"missingCredentialNoPrompt":True})
    print(f"Runner controls passed without network: {checks}")

if __name__ == "__main__": main()
