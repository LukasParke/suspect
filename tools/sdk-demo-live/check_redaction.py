#!/usr/bin/env python3
"""Focused output-cap secret controls: local child processes, no SDK or API calls."""
from __future__ import annotations
import contextlib
import io
import json
import os
from pathlib import Path
import sys
from unittest.mock import patch

from common import REPO, ROOT, fresh, save, sha
import run

def main() -> None:
    work = fresh(ROOT / "wrapper-redaction-fix-01", "checks")
    canary = "redaction-canary-NOT-A-REAL-TOKEN-" + "abcdefghijklmno" * 9
    prefix = canary[:48]
    success = json.dumps({"ok":True,"status":200,"freeTier":False,"management":False,"usage":"1.00"}) + "\n"
    results = []
    def child(stdout: bytes, stderr: bytes) -> dict:
        script = "import sys;sys.stdout.buffer.write("+repr(stdout)+");sys.stderr.buffer.write("+repr(stderr)+")"
        return run.capture([sys.executable,"-c",script],REPO,{},timeout=5)

    for stream in ("stdout","stderr"):
        lead = success.encode() if stream == "stdout" else b""
        payload = lead + b"x"*(65536-48-len(lead)) + canary.encode() + b"tail"
        other = ("independent "+canary+" "+json.dumps(canary)[1:-1]).encode() if stream == "stdout" else success.encode()
        raw = child(payload if stream == "stdout" else other, payload if stream == "stderr" else other)
        assert raw["exitCode"] == 0 and raw["truncated"][stream]
        assert prefix in raw[stream] and canary not in raw[stream]  # Reproduce the vulnerable retained prefix.
        stdout, stderr = run.sanitized_streams(raw,canary)
        sanitized = {"stdout":stdout,"stderr":stderr}
        assert sanitized[stream] == run.TRUNCATED_OUTPUT
        assert prefix not in stdout+stderr and canary not in stdout+stderr
        if stream == "stdout": assert "[REDACTED]" in stderr

        simulated = work / f"SIMULATED-{stream}-NO-NETWORK"
        fake = {"runArgv":[sys.executable,"-c","pass"],"consumer":str(REPO),"packageConfig":{"package_name":"control-only"},"environment":{}}
        transcript = io.StringIO()
        with patch.object(run,"ROOT",simulated), patch.object(run,"prepared",return_value=fake), patch.object(run,"capture",return_value=raw), patch.object(run,"validate_response",side_effect=AssertionError("truncated output must never be parsed")), patch.dict(os.environ,{"OPENROUTER_API_KEY":canary}), patch.object(sys,"argv",["demo-live.sh","python","--no-code"]), contextlib.redirect_stdout(transcript):
            code = run.main()
        assert code == 1 and "PASS GET" not in transcript.getvalue()
        reports=list(simulated.rglob("REPORT.json")); assert len(reports)==1
        report=json.loads(reports[0].read_text())
        assert report["passed"] == 0 and report["results"][0]["confirmation"] == {"ok":False,"kind":"output-truncated"}
        for path in simulated.rglob("*"):
            if path.is_file():
                text=path.read_text()
                assert prefix not in text and canary not in text
        (work/f"{stream}-transcript.txt").write_text("CONTROLLED WRAPPER TEST; NO SDK/API EXECUTED\n"+transcript.getvalue())
        results.append({"stream":stream,"rawCanaryCrossedCap":True,"entireTruncatedStreamDiscarded":True,"independentStreamRedacted":True,"confirmationParserNotCalled":True,"childExitZeroStillFails":True,"storedCanaryPrefixAbsent":True})

    raw = child(b"x"*(8192-20)+canary.encode(),b"full "+canary.encode())
    assert not any(raw["truncated"].values())
    cleaned=run.sanitized_streams(raw,canary)
    assert all(canary not in value and "[REDACTED]" in value for value in cleaned)
    escaped_token='canary-"-\\-\u96ea-full-only'
    raw_text=escaped_token+' '+json.dumps(escaped_token)[1:-1]
    assert run.redact(raw_text,escaped_token) == '[REDACTED] [REDACTED]'
    invalid = child(b"\xff"*65536+canary.encode(),b"\xfe"*65536+canary.encode())
    assert run.sanitized_streams(invalid,canary) == (run.TRUNCATED_OUTPUT,run.TRUNCATED_OUTPUT)
    save(work/"REPORT.json",{"mode":"controlled-wrapper-only","sdkExecuted":False,"apiContacted":False,"passed":True,
        "runnerSha256":sha(REPO/"tools/sdk-demo-live/run.py"),"cases":results,
        "nontruncatedCrossChunkRedaction":True,"rawAndJsonEscapedTokenRedaction":True,"invalidUtf8TruncatedStreamsDiscarded":True})
    print(f"Focused redaction guards passed: {work}")

if __name__ == "__main__": main()
