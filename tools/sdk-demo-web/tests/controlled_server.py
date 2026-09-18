#!/usr/bin/env python3
"""TEST ONLY server. No command-line switch can enable this in demo-web.sh."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True

from controlled import ControlledRuntime
from catalog import REPO
from server import LocalServer, save_new, serve


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8766)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--scenario", choices=("browser", "shutdown"), default="browser")
    args = parser.parse_args()
    evidence = args.evidence_dir.resolve()
    relative = evidence.relative_to(REPO / "target")
    if not relative.parts[0].startswith("sdk-demo-web-20260911-"):
        parser.error("Use a fresh web-owned evidence directory")
    evidence.mkdir(parents=True, exist_ok=True)
    os.environ.pop("OPENROUTER_API_KEY", None)
    if args.scenario == "browser":
        runtime = ControlledRuntime({"python": "denied"}, delay=1.1)
    else:
        runtime = ControlledRuntime({"python": "tree"}, timeout=22, pid_path=evidence / "child-pids.json")
    server = LocalServer(args.port, runtime, evidence)
    save_new(evidence / "server.json", {"mode": "controlled-test", "url": f"http://127.0.0.1:{server.port}",
                                      "pid": os.getpid(), "nativeSdkExecutions": 0, "openRouterRequests": 0})
    print("CONTROLLED TEST SERVER — local test children only; zero SDK / OpenRouter requests.", flush=True)
    serve(server)


if __name__ == "__main__":
    main()
