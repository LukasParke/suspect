from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock, patch

from controlled import CANARY, CHILD, ControlledRuntime, eventually
from catalog import CONSUMERS, LANGUAGES, REPO, excerpts
from jobs import JobManager, RequestError
from runtime import CAPTURE_LIMIT, OMITTED, AcceptedRuntime, accepted_runner, capture_process, child_environment, interpret


class NativeProcessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runner = accepted_runner()
        cls.work = REPO / "target/sdk-demo-web-20260911-01/test-work"
        cls.work.mkdir(exist_ok=True)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=self.work)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def capture(self, case, *, timeout=2.0, cancel=None, pid_path=None):
        argv = [sys.executable, "-B", str(CHILD), case, "0"]
        if pid_path:
            argv.append(str(pid_path))
        return capture_process(argv, CHILD.parent, child_environment({"environment": {}}, CANARY, "key", inherited={}), cancel or threading.Event(), timeout)

    def assert_stopped(self, pid):
        def stopped():
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return True
            # Orphaned grandchildren can briefly remain zombies under macOS init.
            state = subprocess.run(["/bin/ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
            return not state or state.startswith("Z")
        eventually(stopped)

    def test_real_preflight_all_thirteen_is_pin_only(self):
        with patch("subprocess.Popen", side_effect=AssertionError("Preflight must not start a process")):
            runtime = AcceptedRuntime()
        self.assertEqual(set(runtime.ready), set(CONSUMERS))
        self.assertFalse(runtime.errors)
        self.assertEqual(runtime.provenance()["nativeExecutionsDuringPreflight"], 0)
        self.assertEqual(sum(card["native"] and card["ready"] for card in runtime.cards()), 12)
        self.assertTrue(all(len(code.splitlines()) == 4 for code in excerpts().values()))

    def test_environment_uses_native_env_and_drops_verification_injection(self):
        info = {"environment": {"SDK_WEB_INSTALLED_RUNTIME": "retained", "SDK_DEMO_VERIFY": "1", "SDK_DEMO_TEST_URL": "http://unused.invalid"}}
        env = child_environment(info, CANARY, "key", inherited={"SDK_DEMO_VERIFY": "1", "SDK_DEMO_OPERATION": "credits", "NODE_OPTIONS": "unused", "PYTHONPATH": "unused", "HTTP_PROXY": "unused", "HTTPS_PROXY": "unused", "OPENROUTER_API_KEY": "unused"})
        raw = capture_process([sys.executable, "-B", str(CHILD), "environment"], CHILD.parent, env, threading.Event())
        record = json.loads(raw["stdout"])
        self.assertTrue(record["keyReceived"])
        self.assertEqual(record["operation"], "key")
        self.assertEqual(record["demoVariables"], ["SDK_DEMO_OPERATION"])
        self.assertFalse(record["injectionReceived"])
        self.assertEqual(record["runtimeMarker"], "retained")
        self.assertNotIn(CANARY, raw["stdout"])

    def test_token_and_json_escaped_token_redacted_before_projection(self):
        raw = self.capture("leak")
        safe = self.runner.sanitized_streams(raw, CANARY)
        for text in safe:
            self.assertNotIn(CANARY, text)
            self.assertNotIn(json.dumps(CANARY)[1:-1], text)
        result = interpret(raw, CANARY, "key", self.runner)
        self.assertTrue(result["ok"])
        self.assertNotIn("unknownField", result["confirmation"])
        self.assertNotIn("stderr", result)
        self.assertNotIn(CANARY, json.dumps(result))

    def test_token_in_error_kind_cannot_escape_to_ui(self):
        result = interpret(self.capture("leak-kind"), CANARY, "key", self.runner)
        self.assertEqual(result["httpStatus"], 401)
        self.assertEqual(result["kind"], "native-sdk-error")
        self.assertNotIn(CANARY, json.dumps(result))

    def test_unicode_escaped_key_in_decoded_kind_is_redacted(self):
        token = "controlled-unicode-redaction-canary"
        escaped = "".join("\\u%04x" % ord(char) for char in token)
        raw = {"stdout": '{"ok":false,"status":401,"kind":"' + escaped + '"}',
               "stderr": "", "exitCode": 1, "reason": None,
               "truncated": {"stdout": False, "stderr": False}}
        result = interpret(raw, token, "key", self.runner)
        self.assertEqual(result["kind"], "native-sdk-error")
        self.assertNotIn(token, json.dumps(result))

    def test_each_overflow_omits_entire_stream_and_never_parses_even_exit_zero(self):
        for name in ("stdout", "stderr"):
            with self.subTest(stream=name):
                raw = self.capture(f"{name}-overflow")
                self.assertTrue(raw["truncated"][name])
                self.assertEqual(raw[name], OMITTED)
                self.assertEqual(self.runner.sanitized_streams(raw, CANARY)[0 if name == "stdout" else 1], OMITTED)
                raw["exitCode"] = 0
                raw["reason"] = None
                validator = Mock(wraps=self.runner)
                result = interpret(raw, CANARY, "key", validator)
                validator.validate_response.assert_not_called()
                self.assertFalse(result["ok"])
                self.assertIsNone(result["httpStatus"])
                self.assertEqual(result["kind"], "output-truncated")

    def test_exact_capture_boundary_is_not_truncated(self):
        raw = self.capture("exact-boundary")
        self.assertEqual(len(raw["stdout"].encode()), CAPTURE_LIMIT)
        self.assertFalse(any(raw["truncated"].values()))
        self.assertTrue(interpret(raw, CANARY, "key", self.runner)["ok"])

    def test_malformed_and_nonzero_success_cannot_qualify(self):
        for case in ("malformed", "nonzero-success"):
            with self.subTest(case=case):
                result = interpret(self.capture(case), CANARY, "key", self.runner)
                self.assertFalse(result["ok"])
        self.assertIsNone(interpret(self.capture("malformed"), CANARY, "key", self.runner)["httpStatus"])

    def test_declared_failure_retains_http_status(self):
        result = interpret(self.capture("denied"), CANARY, "key", self.runner)
        self.assertFalse(result["ok"])
        self.assertEqual(result["httpStatus"], 401)
        self.assertEqual(result["exitCode"], 1)

    def test_timeout_cleans_parent_and_descendant_even_with_200_prefix(self):
        path = self.root / "tree.json"
        start = time.monotonic()
        raw = self.capture("tree", timeout=0.35, pid_path=path)
        self.assertLess(time.monotonic() - start, 2.5)
        self.assertEqual(raw["reason"], "deadline-exceeded")
        self.assertFalse(interpret(raw, CANARY, "key", self.runner)["ok"])
        for pid in json.loads(path.read_text()).values():
            self.assert_stopped(pid)

    def test_cancellation_cleans_process_group(self):
        path = self.root / "cancel.json"
        cancel = threading.Event()
        result = {}
        worker = threading.Thread(target=lambda: result.update(self.capture("tree", cancel=cancel, pid_path=path)))
        worker.start()
        eventually(path.exists)
        cancel.set()
        worker.join(timeout=3)
        self.assertFalse(worker.is_alive())
        self.assertEqual(result["reason"], "cancelled")
        for pid in json.loads(path.read_text()).values():
            self.assert_stopped(pid)

    def test_sigkill_escalation_for_stubborn_child(self):
        path = self.root / "stubborn.json"
        raw = self.capture("stubborn", timeout=0.2, pid_path=path)
        self.assertEqual(raw["reason"], "deadline-exceeded")
        self.assertEqual(raw["exitCode"], -signal.SIGKILL)
        self.assert_stopped(json.loads(path.read_text())["parent"])

    def test_exited_group_leader_does_not_orphan_live_descendant(self):
        path = self.root / "orphan.json"
        self.capture("orphan", pid_path=path)
        for pid in json.loads(path.read_text()).values():
            self.assert_stopped(pid)

    def test_cancel_before_spawn_and_launch_errors_are_secret_free(self):
        event = threading.Event()
        event.set()
        with patch("subprocess.Popen") as spawn:
            raw = self.capture("ok", cancel=event)
        spawn.assert_not_called()
        self.assertEqual(raw["reason"], "cancelled")
        with patch("subprocess.Popen", side_effect=OSError(CANARY)):
            raw = self.capture("ok")
        self.assertEqual(raw["reason"], "launch-failed")
        self.assertNotIn(CANARY, json.dumps(raw))

    def test_no_client_argv_operation_or_alias_dispatch(self):
        runtime = AcceptedRuntime()
        for language, operation in (("../../anything", "key"), ("py", "key"), ("all", "key"), ("python", "credits"), ("python", "delete")):
            with patch("subprocess.Popen") as spawn:
                result = runtime.execute(language, operation, CANARY, threading.Event())
            spawn.assert_not_called()
            self.assertFalse(result["ok"])

    def test_every_valid_dispatch_uses_accepted_direct_argv_without_global_mutation(self):
        runtime = AcceptedRuntime()
        globals_before = {name: getattr(runtime.runner, name) for name in ("ROOT", "SOURCES", "prepared", "sanitized_streams", "validate_response")}
        # Capture is replaced at its process seam: not one native program runs.
        raw = {"stdout": '{"ok":true,"status":200,"usage":"1","freeTier":false,"management":false}',
               "stderr": "", "exitCode": 0, "reason": None,
               "truncated": {"stdout": False, "stderr": False}}
        for language in CONSUMERS:
            expected = runtime.ready[language]
            with patch("runtime.capture_process", side_effect=lambda *_: {**raw, "truncated": dict(raw["truncated"])}) as capture:
                result = runtime.execute(language, "key", CANARY, threading.Event())
            self.assertTrue(result["ok"])
            actual = capture.call_args.args
            self.assertEqual(actual[0], expected["runArgv"])
            self.assertEqual(actual[1], Path(expected["consumer"]))
            self.assertEqual(actual[2]["OPENROUTER_API_KEY"], CANARY)
            self.assertEqual([name for name in actual[2] if name.startswith("SDK_DEMO_")], ["SDK_DEMO_OPERATION"])
            self.assertNotIn(CANARY, " ".join(actual[0]))
        self.assertEqual(globals_before, {name: getattr(runtime.runner, name) for name in globals_before})

    def test_manager_close_cleans_owned_native_style_processes(self):
        path = self.root / "shutdown.json"
        runtime = ControlledRuntime({"python": "tree"}, timeout=22, pid_path=path)
        manager = JobManager(runtime, self.root / "receipts", CANARY)
        self.addCleanup(manager.close)
        manager.start("python", "key")
        eventually(path.exists)
        manager.close()
        self.assertEqual(manager.snapshot()["jobs"][0]["state"], "cancelled")
        for pid in json.loads(path.read_text()).values():
            self.assert_stopped(pid)

    def test_receipts_are_redacted_projected_and_append_only(self):
        runtime = ControlledRuntime({"python": "leak"})
        manager = JobManager(runtime, self.root / "receipts", CANARY)
        self.addCleanup(manager.close)
        manager.start("python", "key")
        eventually(lambda: manager.snapshot()["counts"]["completed"] == 1)
        first = next((self.root / "receipts").iterdir())
        before = (first.read_bytes(), first.stat().st_mtime_ns, first.stat().st_ino)
        manager.start("python", "key")
        eventually(lambda: len(list((self.root / "receipts").iterdir())) == 2)
        self.assertEqual(before, (first.read_bytes(), first.stat().st_mtime_ns, first.stat().st_ino))
        for receipt in (self.root / "receipts").iterdir():
            text = receipt.read_text()
            self.assertNotIn(CANARY, text)
            self.assertNotIn(json.dumps(CANARY)[1:-1], text)
            self.assertNotIn("unknownField", text)
            self.assertLess(receipt.stat().st_size, 24_576)


if __name__ == "__main__":
    unittest.main()
