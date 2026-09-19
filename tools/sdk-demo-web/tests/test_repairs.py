"""New repair-boundary tests only; no native SDK or network execution."""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

from controlled import CANARY
from catalog import REPO
from repair_runtime import REPAIR_ROOT, RepairRegistry, RepairRuntime, RepairUnavailable, diagnostic_result
from runtime import AcceptedRuntime, accepted_runner, sha


class RepairTests(unittest.TestCase):
    def setUp(self):
        (REPAIR_ROOT / "tests").mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=REPAIR_ROOT / "tests")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.registry = RepairRegistry(self.root)
        self.executable = self.root / "controlled-not-executed"
        self.executable.write_text("This file is only a pinned data marker, never executed.\n")
        self.source = self.root / "source.txt"
        self.source.write_text("Controlled source pin\n")
        self.receipt = self.root / "owner.json"
        self.receipt.write_text('{"mode":"controlled-test"}\n')

    def descriptor(self, **changes):
        result = {"version": 1, "language": "dart", "operation": "key", "mode": "repair",
                  "runArgv": [str(self.executable)], "cwd": str(self.root), "environmentDelta": {},
                  "sourcePins": {str(self.source): sha(self.source)},
                  "executionPins": {str(self.executable): sha(self.executable)},
                  "ownerReceipt": {"path": str(self.receipt), "sha256": sha(self.receipt)}}
        result.update(changes)
        return result

    def approve(self, descriptor):
        manifest = (json.dumps(descriptor, sort_keys=True, indent=2) + "\n").encode()
        import hashlib
        digest = hashlib.sha256(manifest).hexdigest()
        directory = self.root / "manifests"
        directory.mkdir(exist_ok=True)
        with (directory / f"{digest}.json").open("xb") as stream:
            stream.write(manifest)
        with (self.root / "approvals.jsonl").open("a") as stream:
            stream.write(json.dumps({"language": descriptor["language"], "manifestSha256": digest}) + "\n")
        return digest

    def test_only_locally_approved_dart_or_kotlin_can_replace(self):
        self.assertIsNone(self.registry.load("dart", {}))
        digest = self.approve(self.descriptor())
        loaded = self.registry.load("dart", {})
        self.assertEqual(loaded["manifestSha256"], digest)
        self.assertIsNone(self.registry.load("python", {}))
        self.assertIsNone(self.registry.load("kotlin", {}))

    def test_source_execution_and_receipt_drift_fail_closed(self):
        self.approve(self.descriptor())
        for path in (self.source, self.executable, self.receipt):
            original = path.read_bytes()
            path.write_bytes(original + b"changed")
            with self.assertRaises(RepairUnavailable):
                self.registry.load("dart", {})
            path.write_bytes(original)

    def test_fixed_operation_environment_and_argv_are_enforced(self):
        for changes in ({"operation": "credits"}, {"environmentDelta": {"OPENROUTER_API_KEY": CANARY}},
                        {"runArgv": [str(self.executable), "extra-argument"]},
                        {"executionPins": {str(self.source): sha(self.source)}}):
            with self.subTest(fields=list(changes)):
                self.approve(self.descriptor(**changes))
                with self.assertRaises(RepairUnavailable):
                    self.registry.load("dart", {})

    def test_hot_adoption_is_rechecked_and_append_only(self):
        first = self.approve(self.descriptor())
        initial = (self.root / "manifests" / f"{first}.json").read_bytes()
        self.assertEqual(self.registry.load("dart", {})["mode"], "repair")
        second = self.approve(self.descriptor(mode="diagnostic"))
        self.assertEqual(self.registry.load("dart", {})["manifestSha256"], second)
        self.assertEqual((self.root / "manifests" / f"{first}.json").read_bytes(), initial)

    def test_kotlin_requires_same_java_and_all_classpath_files_pinned(self):
        classes = self.root / "classes"
        classes.mkdir()
        main = classes / "Main.class"
        main.write_bytes(b"controlled class data, never executed")
        argv = [str(self.executable), "-cp", str(classes), "demo.MainKt"]
        descriptor = self.descriptor(language="kotlin", runArgv=argv,
                                     executionPins={str(self.executable): sha(self.executable), str(main): sha(main)})
        self.approve(descriptor)
        self.assertEqual(self.registry.load("kotlin", {"runArgv": argv})["runArgv"], argv)
        with self.assertRaises(RepairUnavailable):
            self.registry.load("kotlin", {"runArgv": ["/unapproved/java"]})
        (classes / "Unpinned.class").write_bytes(b"unapproved class data")
        with self.assertRaises(RepairUnavailable):
            self.registry.load("kotlin", {"runArgv": argv})

    def test_diagnostic_200_never_passes_and_is_scalar_redacted(self):
        runner = accepted_runner()
        raw = {"stdout": json.dumps({"ok": True, "status": 200, "usage": "1", "freeTier": False,
                                       "management": False, "headerNames": ["server", CANARY],
                                       "causeClasses": ["java.net.ProtocolException", CANARY],
                                       "body": CANARY, "headers": {"secret": CANARY}}),
               "stderr": CANARY, "truncated": {"stdout": False, "stderr": False}, "reason": None, "exitCode": 0}
        result = diagnostic_result(raw, CANARY, runner)
        self.assertFalse(result["ok"])
        self.assertFalse(result["decoded"])
        self.assertEqual(result["httpStatus"], 200)
        self.assertEqual(result["confirmation"]["headerNames"], ["server"])
        self.assertEqual(result["confirmation"]["causeClasses"], ["java.net.ProtocolException"])
        self.assertNotIn(CANARY, json.dumps(result))
        self.assertNotIn("body", result["confirmation"])
        raw["truncated"]["stderr"] = True
        result = diagnostic_result(raw, CANARY, runner)
        self.assertFalse(result["ok"])
        self.assertIsNone(result["httpStatus"])
        self.assertEqual(result["kind"], "output-truncated")

    def test_other_native_dispatch_is_unchanged_without_execution(self):
        runtime = object.__new__(RepairRuntime)
        runtime.registry = self.registry
        with patch.object(AcceptedRuntime, "execute", return_value={"delegated": True}) as original:
            result = runtime.execute("go", "key", CANARY, threading.Event())
        self.assertEqual(result, {"delegated": True})
        self.assertEqual(original.call_args.args[:2], ("go", "key"))

    def test_effective_preflight_is_unavailable_when_repair_pins_drift(self):
        self.approve(self.descriptor())
        runtime = object.__new__(RepairRuntime)
        runtime.registry = self.registry
        runtime.ready = {"dart": {}}
        with patch.object(AcceptedRuntime, "cards", side_effect=lambda: [{"id": "dart", "ready": True, "native": True}]):
            self.assertTrue(runtime.cards()[0]["ready"])
            self.source.write_text("changed")
            self.assertFalse(runtime.cards()[0]["ready"])

    def test_approved_direct_dispatch_is_pinned_and_bad_revision_never_spawns(self):
        digest = self.approve(self.descriptor())
        runtime = object.__new__(RepairRuntime)
        runtime.registry = self.registry
        runtime.runner = Mock(wraps=accepted_runner())
        runtime.runner.prepared.return_value = {"environment": {}}
        raw = {"stdout": '{"ok":true,"status":200,"usage":"1","freeTier":false,"management":false}',
               "stderr": CANARY, "exitCode": 0, "reason": None,
               "truncated": {"stdout": False, "stderr": False}}
        with patch("repair_runtime.capture_process", return_value=raw) as capture:
            result = runtime.execute("dart", "key", CANARY, threading.Event())
        self.assertTrue(result["ok"])
        self.assertEqual(result["runtimeRevision"]["manifestSha256"], digest)
        self.assertEqual(capture.call_args.args[0], [str(self.executable)])
        self.assertEqual(capture.call_args.args[2]["OPENROUTER_API_KEY"], CANARY)
        self.assertNotIn(CANARY, json.dumps(result))
        self.source.write_text("changed")
        with patch("repair_runtime.capture_process") as capture:
            result = runtime.execute("dart", "key", CANARY, threading.Event())
        capture.assert_not_called()
        self.assertFalse(result["ok"])
        self.assertEqual(result["kind"], "repair-pin-mismatch")


if __name__ == "__main__":
    unittest.main()
