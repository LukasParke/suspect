#!/usr/bin/env python3
"""Verify the additive adapter and its inherited contracts without SDK execution."""
from __future__ import annotations
import ast
import contextlib
import hashlib
import inspect
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from types import SimpleNamespace
from unittest.mock import patch

from support import FILENAMES, LANGUAGES, OBSERVABILITY, REPO, ROOT, RUNNER_SHA, SOURCES, check_pins, fresh, load_base, metadata, preservation, save, sha
from run import FACTORY, load_runner, prepared

def main() -> None:
    work = fresh(ROOT / "checks", "entry")
    base, reference = load_runner(), load_base()
    assert base.ROOT == ROOT and base.SOURCES == SOURCES
    inherited = {}
    for name in ("main", "capture", "redact", "sanitized_streams", "validate_response", "fresh", "save"):
        source = inspect.getsource(getattr(base, name))
        assert source == inspect.getsource(getattr(reference, name)), name
        inherited[name] = {"file":inspect.getsourcefile(getattr(base, name)), "functionSourceSha256":hashlib.sha256(source.encode()).hexdigest()}
    assert base.capture.__defaults__ == (22,)
    factories = {}
    for language in (*LANGUAGES, "javascript"):
        excerpt = base.source_excerpt(language)
        assert FACTORY[language] in excerpt
        factories[language] = {"source":str(SOURCES / language / FILENAMES[language]), "excerpt":excerpt}
    save(work / "displayed-factories.json", factories)

    def sdk_branch(text):
        text = text.split("} else if (operations.isSdkError(error)) {", 1)[1].split("} else {", 1)[0]
        text = re.sub(r"//[^\n]*", "", text)
        return re.sub(r"\s+", "", text)
    old_ts = (REPO / "examples/sdk-demo-live/observability/typescript/main.ts").read_text()
    new_ts = (SOURCES / "typescript/main.ts").read_text()
    new_js = (SOURCES / "javascript/main.mjs").read_text()
    assert sdk_branch(old_ts) == sdk_branch(new_ts) == sdk_branch(new_js)
    def handlers(path):
        tree = ast.parse(path.read_text())
        return [ast.dump(handler) for node in tree.body if isinstance(node, ast.Try) for handler in node.handlers]
    assert handlers(REPO / "examples/sdk-demo-live/observability/python/main.py") == handlers(SOURCES / "python/main.py")
    legacy = REPO / "target/sdk-demo-live-20260911-01/candidate-02"
    inherited_receipts = [legacy / "checks/runner-01/REPORT.json",
                          legacy / "wrapper-redaction-fix-01/checks-01/REPORT.json",
                          legacy / "wrapper-redaction-fix-01/independent-green.json",
                          OBSERVABILITY / "verification.json",
                          REPO / "target/sdk-credential-env-integration-20260911-01/live-observability-owner-receipt-01.json"]
    inherited_proof = json.loads((OBSERVABILITY / "verification.json").read_text())
    assert inherited_proof["status"] == "passed" and inherited_proof["checks"] == 16
    save(work / "inherited-contract.json", {"status":"passed", "baseRunnerSha256":RUNNER_SHA, "unchangedFunctions":inherited, "receiptPins":{str(p):sha(p) for p in inherited_receipts},
        "pythonErrorHandlersAstEqual":True, "typescriptSdkGuardEqualIgnoringWhitespaceAndComments":True, "javascriptUsesSameBoundedSdkGuard":True,
        "f10ControlledOutcomesInherited":16, "f10OutcomesReexecuted":0, "transportHook":"accepted Python HTTPTransport with socket-level network blocking", "discardedSetupAttempt":inherited_proof["discardedSetupAttempt"], "fullNativeMatrixReplayed":False})

    environment = {key:value for key, value in os.environ.items() if key != "OPENROUTER_API_KEY" and not key.startswith("SDK_DEMO_")}
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    command_records = []
    for label, argv, expected in (("shell-syntax", ["/bin/sh", "-n", str(REPO / "demo-live-env.sh")], 0),
                                  ("env-preflight", [str(REPO / "demo-live-env.sh"), "all", "--preflight"], 0),
                                  ("env-no-prompt", [str(REPO / "demo-live-env.sh"), "python", "--no-prompt"], 2)):
        result = subprocess.run(argv, cwd=REPO, env=environment, capture_output=True, text=True, timeout=60)
        (work / f"{label}.stdout").write_text(result.stdout)
        (work / f"{label}.stderr").write_text(result.stderr)
        row = {"label":label, "argv":argv, "exitCode":result.returncode, "expected":expected, "stdoutSha256":sha(work / f"{label}.stdout"), "stderrSha256":sha(work / f"{label}.stderr")}
        command_records.append(row)
        assert result.returncode == expected, row
        if label == "env-preflight": assert "Preparation: 12/12 ready" in result.stdout
        if label == "env-no-prompt": assert "OPENROUTER_API_KEY" in result.stderr

    canary = "env-adapter-controlled-canary-not-a-real-token"
    known = {language:prepared(language) for language in ("typescript", "python")}
    simulations = []
    for operation in ("key", "credits"):
        runner = load_runner()
        sandbox = work / f"SIMULATED-{operation}-NO-NETWORK"
        calls = []
        def controlled_capture(argv, cwd, env):
            assert env["OPENROUTER_API_KEY"] == canary
            assert env["SDK_DEMO_OPERATION"] == operation
            assert set(name for name in env if name.startswith("SDK_DEMO_")) == {"SDK_DEMO_OPERATION"}
            assert canary not in " ".join(argv)
            selected = "typescript" if operation == "key" and not calls else "python"
            assert argv == known[selected]["runArgv"] and str(cwd) == known[selected]["consumer"]
            calls.append({"language":selected, "argv":argv, "cwd":str(cwd), "runtimeEnvironmentOnly":True, "verificationOverridesRemoved":True})
            if operation == "key" and len(calls) == 1:
                response = {"ok":False, "status":401, "kind":"declared-api-error"}
            elif operation == "key":
                response = {"ok":True, "status":200, "freeTier":False, "management":False, "usage":"1.000000000000000001"}
            else:
                response = {"ok":True, "status":200, "credits":"100.50", "usage":"25.75"}
            return {"exitCode":0 if response["ok"] else 1, "stdout":json.dumps(response), "stderr":canary, "reason":None, "truncated":{"stdout":False, "stderr":False}}
        fake_env = {**environment, "SDK_DEMO_VERIFY":"1", "SDK_DEMO_TEST_URL":"http://127.0.0.1:1"}
        if operation == "key": fake_env["OPENROUTER_API_KEY"] = canary
        argv = ["demo-live-env.sh", "all" if operation == "key" else "python", "--operation", operation]
        transcript = io.StringIO()
        with patch.object(runner, "ROOT", sandbox), patch.object(runner, "LANGUAGES", ("typescript", "python")), patch.object(runner, "prepared", side_effect=lambda language:known[language]), patch.object(runner, "capture", side_effect=controlled_capture), patch.object(runner.getpass, "getpass", return_value=canary), patch.object(sys, "stdin", SimpleNamespace(isatty=lambda:True)), patch.dict(os.environ, fake_env, clear=True), patch.object(sys, "argv", argv), contextlib.redirect_stdout(transcript):
            code = runner.main()
        assert code == (1 if operation == "key" else 0)
        assert len(calls) == (2 if operation == "key" else 1)
        report_path = next(sandbox.glob("live-runs/run-*/REPORT.json"))
        report = json.loads(report_path.read_text())
        assert report["credentialSource"] == ("environment" if operation == "key" else "secure-prompt")
        if operation == "key": assert FACTORY["typescript"] in transcript.getvalue() and FACTORY["python"] in transcript.getvalue()
        else: assert "Source: examples/sdk-demo-live/environment/python/" in transcript.getvalue()
        for path in sandbox.rglob("*"):
            if path.is_file(): assert canary not in path.read_text()
        assert canary not in transcript.getvalue()
        (work / f"{operation}-simulated-transcript.txt").write_text("ADAPTER CONTROL ONLY — NO NATIVE SDK OR API EXECUTED\n" + transcript.getvalue())
        simulations.append({"operation":operation, "exitCode":code, "calls":calls, "receipt":str(report_path), "source":"simulated adapter response"})

    for path in (REPO / "tools/sdk-demo-live/environment").glob("*.py"):
        ast.parse(path.read_text(), filename=str(path))
    assert not (ROOT / "live-runs").exists()
    owner = REPO / "target/sdk-credential-env-integration-20260911-01/env-provisional-native-owner-receipt-01.json"
    check_pins({str(owner):"d9674545d65a002bb91df3226dfb91f25edefea6716bbfd16fa6bb4c0656720e"})
    owner_receipt = json.loads(owner.read_text())
    for filename, expected in owner_receipt["verified"].items():
        actual = metadata(Path(filename))
        assert all(actual[key] == value for key, value in expected.items()), filename
    destination = ROOT / "provenance/main-provisional-native-owner-receipt.json"
    if not destination.exists(): shutil.copy2(owner, destination)
    else: assert sha(destination) == sha(owner)
    save(work / "REPORT.json", {"status":"passed", "scope":"new adapter only; previous runtime contracts inherited by exact source", "commandRecords":command_records, "simulations":simulations,
        "newFactoryExcerptsChecked":13, "inheritedContract":str(work / "inherited-contract.json"), "sourcePins":{str(REPO / "demo-live-env.sh"):sha(REPO / "demo-live-env.sh"), str(REPO / "tools/sdk-demo-live/environment/run.py"):sha(REPO / "tools/sdk-demo-live/environment/run.py"), str(REPO / "tools/sdk-demo-live/environment/support.py"):sha(REPO / "tools/sdk-demo-live/environment/support.py")},
        "mainProvisionalReceiptSha256":sha(owner), "nativeConsumersExecuted":False, "apiContacted":False, "realTokenUsed":False, "preservation":preservation()})
    print(f"ENV adapter checked: new 12-target preflight, credential/receipt/source routing, exact inherited redaction/cancellation/F10 linkage; no native SDK/API execution; {work}")

if __name__ == "__main__": main()
