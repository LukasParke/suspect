"""Additive, local-owner-approved Dart/Kotlin replacements.

No HTTP request can register a path or command. An append-only local approval
record selects a SHA-256-pinned manifest; every execution rechecks its pins. This
module is used only by repair_server.py after a coordinated server transition.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import threading

from catalog import REPO
from runtime import AcceptedRuntime, SAFE_KIND, capture_process, child_environment, failed_result, interpret, sha

REPAIR_ROOT = REPO / "target/sdk-demo-web-20260911-05-repairs"
REPAIR_LANGUAGES = frozenset(("dart", "kotlin"))
DIGEST = re.compile(r"[0-9a-f]{64}\Z")
MAIN_CLASS = re.compile(r"[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*\Z")
HEADER_NAME = re.compile(r"[A-Za-z0-9!#$%&'*+.^_`|~-]{1,64}\Z")


class RepairUnavailable(Exception):
    pass


def _bounded_json(path: Path, limit: int = 524288):
    with path.open("rb") as stream:
        value = stream.read(limit + 1)
    if len(value) > limit:
        raise RepairUnavailable("Repair manifest exceeds its local limit")
    return json.loads(value)


def _pin_map(pins) -> dict[str, str]:
    if not isinstance(pins, dict) or not pins or len(pins) > 2048:
        raise RepairUnavailable("Missing bounded repair pins")
    for filename, digest in pins.items():
        if not isinstance(filename, str) or not Path(filename).is_absolute() or not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            raise RepairUnavailable("Invalid repair pin")
        if not Path(filename).is_file() or sha(Path(filename)) != digest:
            raise RepairUnavailable("Repair pin mismatch")
    return pins


class RepairRegistry:
    def __init__(self, root: Path = REPAIR_ROOT):
        self.root = root

    def load(self, language: str, baseline: dict) -> dict | None:
        if language not in REPAIR_LANGUAGES:
            return None
        approvals = self.root / "approvals.jsonl"
        if not approvals.exists():
            return None
        try:
            with approvals.open("rb") as stream:
                data = stream.read(65537)
            if len(data) > 65536:
                raise RepairUnavailable("Approval log exceeds its local limit")
            selected = None
            for line in data.splitlines():
                entry = json.loads(line)
                if not isinstance(entry, dict) or set(entry) != {"language", "manifestSha256"}:
                    raise RepairUnavailable("Invalid local approval")
                if entry["language"] not in REPAIR_LANGUAGES or not isinstance(entry["manifestSha256"], str) or not DIGEST.fullmatch(entry["manifestSha256"]):
                    raise RepairUnavailable("Invalid local approval target")
                if entry["language"] == language:
                    selected = entry["manifestSha256"]
            if selected is None:
                return None
            path = self.root / "manifests" / f"{selected}.json"
            if sha(path) != selected:
                raise RepairUnavailable("Approved manifest changed")
            manifest = _bounded_json(path)
            required = {"version", "language", "operation", "mode", "runArgv", "cwd", "environmentDelta", "sourcePins", "executionPins", "ownerReceipt"}
            if not isinstance(manifest, dict) or set(manifest) != required:
                raise RepairUnavailable("Invalid approved manifest fields")
            if manifest["version"] != 1 or manifest["language"] != language or manifest["operation"] != "key" or manifest["mode"] not in ("repair", "diagnostic"):
                raise RepairUnavailable("Invalid approved operation")
            argv = manifest["runArgv"]
            if not isinstance(argv, list) or not argv or any(not isinstance(value, str) or not value or len(value) > 16384 or "\x00" in value for value in argv):
                raise RepairUnavailable("Invalid approved native command")
            target_root = (REPO / "target").resolve()
            cwd = Path(manifest["cwd"])
            if not cwd.is_absolute() or not cwd.resolve().is_relative_to(target_root) or not cwd.is_dir():
                raise RepairUnavailable("Replacement must use a private target directory")
            if language == "dart":
                if len(argv) != 1 or not Path(argv[0]).resolve().is_relative_to(target_root):
                    raise RepairUnavailable("Dart requires one private native executable")
            else:
                if len(argv) != 4 or argv[0] != baseline["runArgv"][0] or argv[1] != "-cp" or not MAIN_CLASS.fullmatch(argv[3]):
                    raise RepairUnavailable("Kotlin requires the accepted Java launcher and a fixed classpath/main")
            # These consumers need the already-pinned runtime environment only.
            if manifest["environmentDelta"] != {}:
                raise RepairUnavailable("Unapproved runtime environment change")
            sources = _pin_map(manifest["sourcePins"])
            execution = _pin_map(manifest["executionPins"])
            if argv[0] not in execution:
                raise RepairUnavailable("Native launcher lacks an execution pin")
            if language == "kotlin":
                for component in argv[2].split(os.pathsep):
                    entry_path = Path(component)
                    if not entry_path.is_absolute() or not entry_path.resolve().is_relative_to(target_root):
                        raise RepairUnavailable("Classpath must use pinned local target artifacts")
                    if entry_path.is_dir():
                        files = [item for item in entry_path.rglob("*") if item.is_file()]
                        if not files or any(str(item) not in execution for item in files):
                            raise RepairUnavailable("Classpath directory is not fully pinned")
                    elif not entry_path.is_file() or str(entry_path) not in execution:
                        raise RepairUnavailable("Classpath file lacks an execution pin")
            receipt = manifest["ownerReceipt"]
            if not isinstance(receipt, dict) or set(receipt) != {"path", "sha256"}:
                raise RepairUnavailable("Missing native-owner receipt")
            _pin_map({receipt["path"]: receipt["sha256"]})
            return {**manifest, "manifestSha256": selected, "sourcePinCount": len(sources), "executionPinCount": len(execution)}
        except (OSError, ValueError, TypeError, KeyError, RecursionError):
            raise RepairUnavailable("Approved native replacement is unavailable") from None


def diagnostic_result(raw: dict, token: str, runner) -> dict:
    """A diagnostic observation can never become a successful live card."""
    result = interpret(raw, token, "key", runner)
    if any(raw["truncated"].values()) or raw["reason"]:
        result["ok"] = False
        return result
    result = failed_result("diagnostic-only")
    result["exitCode"] = raw["exitCode"]
    stdout, _ = runner.sanitized_streams(raw, token)
    try:
        lines = [line for line in stdout.splitlines() if line.strip()]
        if len(lines) != 1:
            return result
        observation = json.loads(lines[0])
        if not isinstance(observation, dict):
            return result
        confirmation = {"ok": False, "kind": "diagnostic-only"}
        status = observation.get("status")
        if type(status) is int and 100 <= status <= 599:
            result["httpStatus"] = status
            confirmation["status"] = status
        for name in ("kind", "category"):
            value = observation.get(name)
            if isinstance(value, str):
                value = runner.redact(value, token)
                if SAFE_KIND.fullmatch(value):
                    confirmation["nativeKind" if name == "kind" else name] = value
        for name, pattern, limit in (("headerNames", HEADER_NAME, 64), ("causeClasses", SAFE_KIND, 8)):
            values = observation.get(name)
            if isinstance(values, list) and len(values) <= limit:
                sanitized = [runner.redact(value, token) for value in values if isinstance(value, str)]
                confirmation[name] = [value for value in sanitized if pattern.fullmatch(value)]
        result["confirmation"] = confirmation
    except (ValueError, TypeError, RecursionError):
        pass
    return result


class RepairRuntime(AcceptedRuntime):
    def __init__(self, registry: RepairRegistry | None = None):
        super().__init__()
        self.registry = registry or RepairRegistry()

    def cards(self) -> list[dict]:
        cards = super().cards()
        for card in cards:
            language = card["id"]
            if language not in REPAIR_LANGUAGES or language not in self.ready:
                continue
            try:
                repair = self.registry.load(language, self.ready[language])
                if repair:
                    card["runtimeRevision"] = {"mode": repair["mode"], "manifestSha256": repair["manifestSha256"]}
            except RepairUnavailable:
                card["ready"] = False
                card["error"] = "The approved native replacement failed its pin checks."
        return cards

    def execute(self, language: str, operation: str, token: str, cancel: threading.Event) -> dict:
        if language not in REPAIR_LANGUAGES or operation != "key":
            return super().execute(language, operation, token, cancel)
        try:
            baseline = self.runner.prepared(language)
            replacement = self.registry.load(language, baseline)
        except (OSError, ValueError, RuntimeError, KeyError, RepairUnavailable):
            return failed_result("repair-pin-mismatch")
        if replacement is None:
            return super().execute(language, operation, token, cancel)
        raw = capture_process(replacement["runArgv"], Path(replacement["cwd"]), child_environment(baseline, token, "key"), cancel)
        try:
            result = diagnostic_result(raw, token, self.runner) if replacement["mode"] == "diagnostic" else interpret(raw, token, "key", self.runner)
            result["runtimeRevision"] = {"mode": replacement["mode"], "manifestSha256": replacement["manifestSha256"], "ownerReceiptSha256": replacement["ownerReceipt"]["sha256"]}
            return result
        finally:
            raw.clear()

    def provenance(self) -> dict:
        proof = super().provenance()
        repairs = {}
        for language in sorted(REPAIR_LANGUAGES):
            if language not in self.ready:
                continue
            try:
                manifest = self.registry.load(language, self.ready[language])
                if manifest:
                    repairs[language] = {name: manifest[name] for name in ("mode", "manifestSha256", "runArgv", "cwd", "sourcePins", "executionPins", "ownerReceipt")}
            except RepairUnavailable:
                repairs[language] = {"status": "repair-pin-mismatch"}
        return {**proof, "readyNativeTargets": sum(card["native"] and card["ready"] for card in self.cards()),
                "nativeRepairs": repairs, "repairApprovalMode": "local-owner-append-only; no HTTP registration"}
