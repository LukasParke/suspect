"""Pinned native execution and bounded, cancellation-aware process ownership.

The web server never calls the accepted command-line runner's main(). Its loader
is composed once; prepared(), sanitized_streams(), and validate_response() are
then used without changing that module's globals between jobs.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import selectors
import signal
import subprocess
import sys
import threading
import time
from functools import lru_cache
from pathlib import Path
from typing import Any

from catalog import CONSUMERS, LANGUAGES, META, OPERATIONS, PREPARED_ROOT, REFERENCE, REPO, SOURCE_SERVER, excerpts, source_paths

CAPTURE_LIMIT = 65_536
PROCESS_DEADLINE = 22.0
TERMINATION_GRACE = 0.4
OMITTED = "[output omitted: capture limit exceeded]\n"
ACCEPTANCE = REPO / "target/sdk-credential-env-integration-20260911-01/live-env-delivery-owner-receipt-01.json"
PINNED_FILES = {
    "LIVE-ENV-DEMO-README.md": "680457b1b4a4b1794e4d26d47d2bb77b72eb06554c431bfb079b1beca7fcf13b",
    "tools/sdk-demo-live/environment/run.py": "9da5e3482a5bcd5bf8893b61af40756d8a306a34b04d3f3deccebbf7e938c52c",
    "tools/sdk-demo-live/environment/support.py": "6007c540042e9c01f5669ca0b40fb3f570b7bbda768ac73cbd957fc9922d868a",
    "tools/sdk-demo-live/run.py": "578b79a4a34bf5a55966e217c96d3322ac31955ad9bf2015de09d9bcb85d3ee7",
    "tools/sdk-demo-live/common.py": "5d54dac883af066714f9d3d0dbaa9d8141f21af6f1c3050358c1253fb2d733b6",
    "target/sdk-demo-live-20260911-01/environment-01/candidate-02/ready.json": "c3390f438a9d85db5f07061c77031b7d010301186c1edfefdfa8476b682cf9f3",
    "target/sdk-credential-env-integration-20260911-01/live-env-delivery-owner-receipt-01.json": "3ba90129fd219ac246e0d5b028fe9123d9e22e911a91985248ec57305558c3c9",
}
SAFE_KIND = re.compile(r"[A-Za-z0-9_.:*+/-]{1,96}\Z")


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1_048_576), b""):
            digest.update(block)
    return digest.hexdigest()


@lru_cache(maxsize=1)
def accepted_runner() -> Any:
    for filename, digest in PINNED_FILES.items():
        if sha(REPO / filename) != digest:
            raise RuntimeError("Accepted web runtime dependency changed")
    path = REPO / "tools/sdk-demo-live/environment/run.py"
    old_path = sys.path[:]
    try:
        sys.path.insert(0, str(path.parent))
        spec = importlib.util.spec_from_file_location("sdk_demo_web_accepted_env", path)
        if spec is None or spec.loader is None:
            raise RuntimeError("Accepted environment adapter unavailable")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.load_runner()
    finally:
        sys.path[:] = old_path


def child_environment(info: dict, token: str, operation: str, inherited: dict[str, str] | None = None) -> dict[str, str]:
    # Only platform necessities plus the pinned installed runtime environment.
    # In particular, verification endpoints, proxies, and runtime injection hooks
    # are not inherited from the web server's shell.
    inherited = os.environ if inherited is None else inherited
    base_names = ("HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "SYSTEMROOT")
    result = {name: inherited[name] for name in base_names if name in inherited}
    result.update(info["environment"])
    result = {name: value for name, value in result.items() if not name.startswith("SDK_DEMO_") and name != "OPENROUTER_API_KEY"}
    result.update(
        OPENROUTER_API_KEY=token, SDK_DEMO_OPERATION=operation,
        PYTHONDONTWRITEBYTECODE="1", PYTHONNOUSERSITE="1",
    )
    return result


def _group_exists(pid: int) -> bool:
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False


def _signal_group(pid: int, sig: int) -> None:
    try:
        os.killpg(pid, sig)
    except ProcessLookupError:
        pass


def _cleanup_group(process: subprocess.Popen, grace: float = TERMINATION_GRACE) -> None:
    # Do this even when the group leader exited: a descendant may still own pipes.
    process.poll()
    if _group_exists(process.pid):
        _signal_group(process.pid, signal.SIGTERM)
        end = time.monotonic() + grace
        while time.monotonic() < end:
            process.poll()
            if not _group_exists(process.pid):
                break
            time.sleep(0.01)
        if _group_exists(process.pid):
            _signal_group(process.pid, signal.SIGKILL)
    process.wait(timeout=1.0)


def capture_process(argv: list[str], cwd: Path, env: dict[str, str], cancel: threading.Event,
                    timeout: float = PROCESS_DEADLINE) -> dict:
    """Capture each stream up to 64 KiB; own and clean the entire native group."""
    empty = {"exitCode": None, "stdout": "", "stderr": "", "truncated": {"stdout": False, "stderr": False}, "reason": None}
    if cancel.is_set():
        return {**empty, "reason": "cancelled"}
    try:
        process = subprocess.Popen(
            argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True, close_fds=True,
        )
    except (OSError, ValueError):
        # Never stringify launch exceptions: they can include environment data.
        return {**empty, "exitCode": 127, "reason": "launch-failed"}

    chunks = {"stdout": bytearray(), "stderr": bytearray()}
    truncated = {"stdout": False, "stderr": False}
    reason = None
    selector = selectors.DefaultSelector()
    for name, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
        os.set_blocking(pipe.fileno(), False)
        selector.register(pipe, selectors.EVENT_READ, name)

    def drain(wait: float) -> None:
        for key, _ in selector.select(wait):
            block = os.read(key.fd, 8192)
            name = key.data
            if not block:
                selector.unregister(key.fileobj)
                key.fileobj.close()
            elif not truncated[name]:
                if len(chunks[name]) + len(block) > CAPTURE_LIMIT:
                    # Discard the entire prefix immediately. It may end mid-key.
                    chunks[name].clear()
                    truncated[name] = True
                else:
                    chunks[name].extend(block)

    end = time.monotonic() + timeout
    try:
        while True:
            if cancel.is_set():
                reason = "cancelled"
                break
            if time.monotonic() >= end:
                reason = "deadline-exceeded"
                break
            drain(min(0.04, max(0, end - time.monotonic())))
            if any(truncated.values()):
                reason = "output-truncated"
                break
            if process.poll() is not None:
                break
    except (OSError, ValueError):
        reason = "capture-failed"
    finally:
        try:
            _cleanup_group(process)
            drain_end = time.monotonic() + 0.5
            while selector.get_map() and time.monotonic() < drain_end:
                drain(0.02)
        finally:
            # An unfinished pipe is also an incomplete stream, never a safe prefix.
            for key in list(selector.get_map().values()):
                truncated[key.data] = True
                chunks[key.data].clear()
                selector.unregister(key.fileobj)
                key.fileobj.close()
            selector.close()

    return {
        "exitCode": process.returncode, "reason": reason,
        "truncated": truncated,
        **{name: OMITTED if truncated[name] else chunks[name].decode(errors="replace") for name in chunks},
    }


def failed_result(kind: str) -> dict:
    return {"ok": False, "httpStatus": None, "decoded": False, "confirmation": None,
            "kind": kind, "exitCode": None, "reason": kind,
            "outputTruncated": {"stdout": False, "stderr": False}}


def interpret(raw: dict, token: str, operation: str, runner: Any) -> dict:
    # Both streams are sanitized before any parsing. Raw streams are never sent to
    # the UI or receipts, even after redaction; only bounded typed metadata leaves.
    stdout, _stderr = runner.sanitized_streams(raw, token)
    result = failed_result(raw["reason"] or "invalid-or-missing-native-confirmation")
    result.update(exitCode=raw["exitCode"], reason=raw["reason"], outputTruncated=dict(raw["truncated"]))
    if any(raw["truncated"].values()):
        result["kind"] = "output-truncated"
        return result
    try:
        response = runner.validate_response(stdout, operation)
        if response.get("ok") not in (True, False) or type(response.get("ok")) is not bool:
            raise ValueError("Invalid confirmation")
        status = response.get("status")
        status = status if type(status) is int and 100 <= status <= 599 else None
        confirmation: dict[str, Any] = {"ok": response["ok"]}
        if status is not None:
            confirmation["status"] = status
        if response["ok"]:
            if status != 200:
                raise ValueError("Invalid success status")
            # The accepted validator checks declared 200, exact decimals and bools.
            for name in ("usage", "credits"):
                if name in response:
                    if not isinstance(response[name], str) or len(response[name]) > 4096:
                        raise ValueError("Confirmation too large")
                    confirmation[name] = response[name]
            if operation == "key":
                confirmation.update(freeTier=response["freeTier"], management=response["management"])
        else:
            kind = response.get("kind")
            # JSON can encode otherwise-visible key characters as \uXXXX. Redact
            # again after decoding before selecting the public scalar error kind.
            if isinstance(kind, str):
                kind = runner.redact(kind, token)
            confirmation["kind"] = kind if isinstance(kind, str) and SAFE_KIND.fullmatch(kind) else "native-sdk-error"
        result.update(httpStatus=status, decoded=response["ok"], confirmation=confirmation)
        result["ok"] = raw["exitCode"] == 0 and response["ok"] and raw["reason"] is None
        result["kind"] = "confirmed" if result["ok"] else raw["reason"] or confirmation.get("kind", "native-process-failed")
    except (ValueError, TypeError, KeyError, RecursionError):
        pass
    return result


class AcceptedRuntime:
    mode = "live"

    def __init__(self) -> None:
        self.runner = accepted_runner()
        self.ready: dict[str, dict] = {}
        self.errors: dict[str, str] = {}
        self.code = excerpts()
        self.pins = dict(PINNED_FILES)
        index = json.loads((PREPARED_ROOT / "native-index.json").read_text())
        self.execution_pins: dict[str, dict] = {}
        for language in CONSUMERS:
            try:
                self.ready[language] = self.runner.prepared(language)
                native = "typescript" if language == "javascript" else language
                proof = json.loads(Path(index[native]["verification"]).read_text())
                self.execution_pins[language] = proof["executionPins"]
            except (OSError, ValueError, RuntimeError, KeyError):
                self.errors[language] = "The prepared source, package, or executable could not be verified. See the local setup guide."

    def cards(self) -> list[dict]:
        cards = []
        paths = source_paths()
        for language in CONSUMERS:
            info = self.ready.get(language, {})
            config = info.get("packageConfig", {})
            native = "typescript" if language == "javascript" else language
            cards.append({
                "id": language, **META[language], "code": self.code[language],
                "package": config.get("package_name", "Package unavailable"),
                "identity": "openrouter::openrouter" if language == "cpp" else config.get("package_name", "Package unavailable"),
                "version": config.get("package_version", "0.1.0"),
                "importName": config.get("import_name"), "native": language in LANGUAGES,
                "ready": language in self.ready, "error": self.errors.get(language),
                "sourceUrl": f"/source/{language}", "docsUrl": f"/docs/{language}",
                "sourcePath": str(paths[language].relative_to(REPO)),
                "sourceSha256": sha(paths[language]),
                "packagePath": str((PREPARED_ROOT / "packages" / native).relative_to(REPO)),
            })
        return cards

    def provenance(self) -> dict:
        return {
            "mode": self.mode, "nativeTargets": len(LANGUAGES),
            "readyNativeTargets": sum(language in self.ready for language in LANGUAGES),
            "javascriptReady": "javascript" in self.ready,
            "preparedRoot": str(PREPARED_ROOT.relative_to(REPO)),
            "sourceServer": SOURCE_SERVER, "operation": "getCurrentKey", "route": "/key",
            "acceptedControlledOutcomes": 52, "reusedOutcomes": 44, "newGoDartOutcomes": 8,
            "missingEnvZeroHttpConsumers": 13, "nativeExecutionsDuringPreflight": 0,
            "liveCallsDuringPreflight": 0, "scriptAndAcceptancePins": self.pins,
            "sourcePins": {card["sourcePath"]: card["sourceSha256"] for card in self.cards()},
            "runtimes": {
                language: {"runArgv": info["runArgv"], "cwd": info["consumer"],
                           "executionPins": self.execution_pins[language]}
                for language, info in self.ready.items()
            },
        }

    def execute(self, language: str, operation: str, token: str, cancel: threading.Event) -> dict:
        if language not in CONSUMERS or operation not in OPERATIONS:
            return failed_result("unsupported-request")
        try:
            # This runs in a worker: HTTP and UI stay responsive during rechecking.
            info = self.runner.prepared(language)
        except (OSError, ValueError, RuntimeError, KeyError):
            return failed_result("runtime-pin-mismatch")
        raw = capture_process(info["runArgv"], Path(info["consumer"]), child_environment(info, token, operation), cancel)
        try:
            return interpret(raw, token, operation, self.runner)
        finally:
            raw.clear()
