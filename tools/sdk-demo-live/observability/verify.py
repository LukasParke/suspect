#!/usr/bin/env python3
"""F10 controls through actual installed SDK transports; never account HTTP."""
from __future__ import annotations
import argparse
import json
from pathlib import Path

from support import ROOT, fresh, record, save, sha, verify_preservation

CANARY = "f10-private-canary-not-an-api-token"

def main(selected: str) -> None:
    work = fresh(ROOT / "checks", "sdk-errors")
    plans = json.loads((ROOT / "prepared.json").read_text())
    fixtures = work / "hooks"
    fixtures.mkdir()
    node = fixtures / "fetch.mjs"
    node.write_text("""import assert from 'node:assert/strict';
let calls = 0;
globalThis.fetch = async (url, init) => {
  calls++;
  const route = process.env.SDK_DEMO_OPERATION === 'credits' ? '/credits' : '/key';
  assert.equal(String(url), 'https://openrouter.ai/api/v1' + route);
  assert.equal(init.method, 'GET');
  assert.equal(new Headers(init.headers).get('authorization'), 'Bearer f10-private-canary-not-an-api-token');
  if (process.env.F10_CASE === 'transport') throw new TypeError('f10-private-canary-not-an-api-token transport cause');
  const status = process.env.F10_CASE === 'malformed200' ? 200 : 401;
  const body = process.env.F10_CASE === 'declared401' ? '{"error":{"code":401,"message":"f10-private-canary-not-an-api-token body"}}' : '{}';
  return new Response(body, {status, headers:{'content-type':'application/json'}});
};
process.on('beforeExit', () => assert.equal(calls, 1));
""")
    python = fixtures / "python"
    python.mkdir()
    (python / "sitecustomize.py").write_text("""import atexit
import os
import socket
import httpx
calls = 0
def no_network(*args, **kwargs):
    raise AssertionError('network access is forbidden in this controlled verification')
socket.create_connection = no_network
socket.socket.connect = no_network
socket.socket.connect_ex = no_network
def handler(request):
    global calls
    calls += 1
    route = '/credits' if os.environ['SDK_DEMO_OPERATION'] == 'credits' else '/key'
    assert str(request.url) == 'https://openrouter.ai/api/v1' + route
    assert request.method == 'GET'
    assert request.headers['authorization'] == 'Bearer f10-private-canary-not-an-api-token'
    if os.environ['F10_CASE'] == 'transport':
        raise httpx.ConnectError('f10-private-canary-not-an-api-token transport cause', request=request)
    status = 200 if os.environ['F10_CASE'] == 'malformed200' else 401
    body = '{"error":{"code":401,"message":"f10-private-canary-not-an-api-token body"}}' if os.environ['F10_CASE'] == 'declared401' else '{}'
    return httpx.Response(status, headers={'content-type':'application/json'}, content=body, request=request)
class ControlledTransport(httpx.BaseTransport):
    def __init__(self, **kwargs):
        pass
    def handle_request(self, request):
        return handler(request)
httpx.HTTPTransport = ControlledTransport
@atexit.register
def complete():
    assert calls == 1
""")
    outcomes = []
    for language, plan in plans.items():
        if selected != "all" and language != selected:
            continue
        for operation in ("key", "credits"):
            for case in ("malformed200", "malformed401", "declared401", "transport"):
                env = {"OPENROUTER_API_KEY":CANARY,"SDK_DEMO_OPERATION":operation,"F10_CASE":case}
                argv = plan["argv"]
                if language == "typescript":
                    argv = [argv[0], "--import", str(node), *argv[1:]]
                else:
                    env["PYTHONPATH"] = str(python)
                result = record(work, f"{language}-{operation}-{case}", argv, Path(plan["consumer"]), env, expected=1)
                value = json.loads(result["stdout"])
                expected_status = 200 if case == "malformed200" else None if case == "transport" else 401
                assert value["ok"] is False and value["status"] == expected_status, (language, case, value)
                if case.startswith("malformed"):
                    assert value["kind"] == "response-decoding", value
                elif case == "transport":
                    assert value["kind"] == "transport", value
                elif language == "typescript":
                    assert value["kind"] == "declared-api-error", value
                else:
                    assert value["kind"] in ("GetCurrentKeyStatus401", "GetCreditsStatus401"), value
                assert set(value) == {"ok", "status", "kind"}, value
                assert result["stderr"] == "", result["stderr"]
                assert CANARY not in result["stdout"] + result["stderr"]
                outcomes.append({"language":language,"operation":operation,"case":case,"result":value,"exitCode":result["exitCode"],"receipt":result["receipt"]})
    proof = {"status":"passed","scope":"two additive consumers, actual installed SDKs, controlled default-HTTPS transports","realApiContacted":False,"sdkImplementationModified":False,"outcomes":outcomes,"checks":len(outcomes),"sourcePins":{str(p):sha(p) for p in (node, python / "sitecustomize.py")},"preservation":verify_preservation()}
    save(work / "REPORT.json", proof)
    save(ROOT / f"verification-{selected}.json", {"status":"passed","report":str(work / "REPORT.json"),"checks":len(outcomes),"preparedSha256":sha(ROOT / "prepared.json")})
    print(f"F10 native SDK controls passed: {len(outcomes)} outcomes across two consumers; {work}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--language", choices=("all", "typescript", "python"), default="all")
    main(parser.parse_args().language)
