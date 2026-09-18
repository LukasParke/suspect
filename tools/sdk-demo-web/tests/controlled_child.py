"""TEST ONLY. Deterministic local child; never imports an SDK or uses a network."""
from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def no_network(event, _args):
    if event.startswith("socket."):
        raise RuntimeError("Network forbidden in the controlled child")


sys.addaudithook(no_network)
case = sys.argv[1]
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0
if delay:
    time.sleep(delay)
confirmation = {"ok": True, "status": 200, "usage": "9007199254740993.000000000000000001", "freeTier": False, "management": False}
token = os.environ.get("OPENROUTER_API_KEY", "")

if case == "ok":
    print(json.dumps(confirmation), flush=True)
elif case == "denied":
    print(json.dumps({"ok": False, "status": 401, "kind": "controlled-declared-api-error"}), flush=True)
    sys.exit(1)
elif case == "leak":
    print(json.dumps({**confirmation, "unknownField": token}), flush=True)
    print(token, file=sys.stderr, flush=True)
    print(json.dumps(token)[1:-1], file=sys.stderr, flush=True)
elif case == "leak-kind":
    print(json.dumps({"ok": False, "status": 401, "kind": token, "message": token}), flush=True)
    sys.exit(1)
elif case in ("stdout-overflow", "stderr-overflow"):
    stream = sys.stdout if case == "stdout-overflow" else sys.stderr
    print(json.dumps(confirmation), flush=True)
    # A credential straddles the exact 64-KiB boundary. Prefix retention is unsafe.
    prefix = ("prefix-" + token) * 2000
    stream.write(prefix + token)
    stream.flush()
elif case == "exact-boundary":
    text = json.dumps(confirmation)
    sys.stdout.write(text + " " * (65_536 - len(text)))
    sys.stdout.flush()
elif case == "malformed":
    print('{"ok":true,"status":200,"usage":', flush=True)
elif case == "nonzero-success":
    print(json.dumps(confirmation), flush=True)
    sys.exit(7)
elif case == "environment":
    print(json.dumps({
        **confirmation, "keyReceived": bool(token),
        "demoVariables": sorted(name for name in os.environ if name.startswith("SDK_DEMO_")),
        "operation": os.environ.get("SDK_DEMO_OPERATION"),
        "injectionReceived": any(name in os.environ for name in ("NODE_OPTIONS", "PYTHONPATH", "HTTP_PROXY", "HTTPS_PROXY")),
        "runtimeMarker": os.environ.get("SDK_WEB_INSTALLED_RUNTIME"),
    }), flush=True)
elif case in ("hang", "stubborn", "sleep-child"):
    if case == "stubborn":
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
    if len(sys.argv) > 3:
        Path(sys.argv[3]).write_text(json.dumps({"parent": os.getpid()}))
    time.sleep(60)
elif case in ("tree", "orphan"):
    child = subprocess.Popen([sys.executable, "-B", __file__, "sleep-child"])
    Path(sys.argv[3]).write_text(json.dumps({"parent": os.getpid(), "child": child.pid}))
    if case == "tree":
        def stopped(*_):
            child.wait(timeout=1)
            sys.exit(0)
        signal.signal(signal.SIGTERM, stopped)
    print(json.dumps(confirmation), flush=True)
    if case == "tree":
        time.sleep(60)
else:
    raise SystemExit("Unknown controlled test case")
