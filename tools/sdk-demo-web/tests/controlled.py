"""Internal test seams. The production launcher has no controlled/test mode."""
from __future__ import annotations

from functools import lru_cache
from pathlib import Path
import sys
import threading
import time

WEB = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(WEB))

from catalog import CONSUMERS
from runtime import AcceptedRuntime, accepted_runner, capture_process, child_environment, failed_result, interpret

CHILD = Path(__file__).with_name("controlled_child.py")
CANARY = 'web-test-canary-"quoted"-\\-not-an-api-key'


@lru_cache(maxsize=1)
def metadata():
    # Pin-checking only. No accepted native consumer is ever executed here.
    return AcceptedRuntime()


class ControlledRuntime:
    mode = "controlled-test"

    def __init__(self, cases=None, delay=0.0, timeout=3.0, pid_path=None):
        self.cases = cases or {}
        self.delay = delay
        self.timeout = timeout
        self.pid_path = pid_path
        self.calls = []
        self.runner = accepted_runner()

    def cards(self):
        return metadata().cards()

    def provenance(self):
        return {"mode": self.mode, "controlledChild": str(CHILD), "nativeSdkExecutions": 0, "openRouterRequests": 0}

    def execute(self, language, operation, token, cancel):
        if language not in CONSUMERS or operation != "key":
            return failed_result("unsupported-request")
        self.calls.append(language)
        argv = [sys.executable, "-B", str(CHILD), self.cases.get(language, "ok"), str(self.delay)]
        if self.pid_path:
            argv.append(str(self.pid_path))
        env = child_environment({"environment": {}}, token, operation, inherited={})
        raw = capture_process(argv, CHILD.parent, env, cancel, timeout=self.timeout)
        try:
            return interpret(raw, token, operation, self.runner)
        finally:
            raw.clear()


class GateRuntime(ControlledRuntime):
    def __init__(self):
        super().__init__()
        self.release = threading.Event()
        self.started = []
        self.tokens = []
        self.active = 0
        self.high_water = 0
        self.lock = threading.Lock()

    def execute(self, language, operation, token, cancel):
        with self.lock:
            self.active += 1
            self.high_water = max(self.high_water, self.active)
            self.started.append(language)
            self.tokens.append(token)
        try:
            while not self.release.is_set():
                if cancel.wait(0.005):
                    return failed_result("cancelled")
            return {"ok": True, "httpStatus": 200, "decoded": True,
                    "confirmation": {"ok": True, "status": 200, "usage": "1", "freeTier": False, "management": False},
                    "kind": "confirmed", "exitCode": 0, "reason": None,
                    "outputTruncated": {"stdout": False, "stderr": False}}
        finally:
            with self.lock:
                self.active -= 1


def eventually(predicate, timeout=4.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(0.01)
    raise AssertionError("Controlled condition did not finish before its deadline")
