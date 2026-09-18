#!/usr/bin/env python3
"""Coordinated replacement server; adds only pinned Dart/Kotlin repair adoption."""
from __future__ import annotations

import argparse
import os
import sys

sys.dont_write_bytecode = True

from repair_runtime import RepairRuntime
from jobs import valid_token
from repair_history import restore_previous_results
from server import LocalServer, fresh_root, save_new, serve


class RepairServer(LocalServer):
    @property
    def proof(self):
        # Handler reads this only after its existing Host/Origin/nonce guards.
        return self.runtime.provenance()

    @proof.setter
    def proof(self, value):
        self._startup_proof = value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--require-environment-key", action="store_true")
    parser.add_argument("--require-repairs", action="store_true")
    parser.add_argument("--restore-previous-results", action="store_true", help="Same-key transition only: preserve unchanged SDK results from the prior fixed session")
    args = parser.parse_args()
    if not 0 <= args.port <= 65535:
        parser.error("Port must be between 0 and 65535")
    token = os.environ.pop("OPENROUTER_API_KEY", "")
    try:
        if args.require_environment_key and not valid_token(token):
            print("The original environment key is unavailable. Keep the existing server; coordinate user key re-entry.", file=sys.stderr)
            return 2
        runtime = RepairRuntime()
        proof = runtime.provenance()
        if args.require_repairs and any(proof["nativeRepairs"].get(language, {}).get("mode") != "repair" for language in ("dart", "kotlin")):
            print("Both corrected native programs must be approved and pinned before this transition.", file=sys.stderr)
            return 2
        evidence = fresh_root()
        save_new(evidence / "preflight.json", {**proof, "credential": {"ready": valid_token(token)}})
        if args.preflight:
            print("Pinned repair preflight complete; no native request executed.")
            print(f"Environment key available: {str(valid_token(token)).lower()}")
            print(f"Evidence: {evidence}")
            return int(any(not card["ready"] for card in runtime.cards()))
        server = RepairServer(args.port, runtime, evidence, token)
        token = ""
        if args.restore_previous_results:
            save_new(evidence / "retained-results.json", restore_previous_results(server.manager))
        save_new(evidence / "server.json", {"url": f"http://127.0.0.1:{server.port}", "mode": "live", "edition": "pinned-dart-kotlin-repairs", "pid": os.getpid(), "liveRequestsAtStartup": 0, "tokenRecorded": False})
        serve(server)
        return 0
    except (OSError, RuntimeError, ValueError, KeyError):
        print("The pinned repair server could not start. Verify approved artifacts and choose an available port.", file=sys.stderr)
        return 1
    finally:
        token = ""


if __name__ == "__main__":
    raise SystemExit(main())
