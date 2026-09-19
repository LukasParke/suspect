from __future__ import annotations

import json
from pathlib import Path
import tempfile
import time
import unittest

from controlled import CANARY, GateRuntime, eventually
from catalog import CONSUMERS, LANGUAGES, REPO
from jobs import JobManager, RequestError


class QueueTests(unittest.TestCase):
    def setUp(self):
        work = REPO / "target/sdk-demo-web-20260911-01/test-work"
        work.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=work)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def manager(self, runtime=None, **options):
        runtime = runtime or GateRuntime()
        manager = JobManager(runtime, self.root / "receipts", CANARY, **options)
        self.addCleanup(manager.close)
        return manager, runtime

    def test_run_all_queues_quickly_three_workers_and_twelve_native_only(self):
        manager, runtime = self.manager()
        start = time.monotonic()
        result = manager.start("all", "key")
        self.assertLess(time.monotonic() - start, 0.2)
        self.assertEqual(result["accepted"], 12)
        eventually(lambda: manager.snapshot()["counts"]["running"] == 3)
        snapshot = manager.snapshot()
        self.assertEqual(snapshot["counts"]["queued"], 9)
        self.assertNotIn("javascript", {job["language"] for job in snapshot["jobs"]})
        self.assertEqual(manager.start("all", "key")["accepted"], 0)
        runtime.release.set()
        eventually(lambda: manager.snapshot()["counts"]["completed"] == 12)
        self.assertEqual(runtime.high_water, 3)
        self.assertEqual(set(runtime.started), set(LANGUAGES))

    def test_fifo_and_queued_cancellation_never_launches_cancelled_job(self):
        manager, runtime = self.manager(concurrency=1)
        manager.start("typescript", "key")
        eventually(lambda: runtime.started == ["typescript"])
        cancelled = manager.start("python", "key")["jobIds"][0]
        manager.start("go", "key")
        manager.cancel(cancelled)
        queued = next(job for job in manager.snapshot()["jobs"] if job["language"] == "go")
        self.assertEqual(queued["queuePosition"], 1)
        runtime.release.set()
        eventually(lambda: manager.snapshot()["counts"]["completed"] == 2)
        self.assertEqual(runtime.started, ["typescript", "go"])
        self.assertEqual(manager.get(cancelled)["state"], "cancelled")

    def test_clear_key_cancels_running_and_queued_and_requires_new_click(self):
        manager, runtime = self.manager(concurrency=1)
        manager.start("all", "key")
        eventually(lambda: len(runtime.started) == 1)
        response = manager.set_token("")
        self.assertEqual(response, {"ready": False})
        eventually(lambda: manager.snapshot()["counts"]["cancelled"] == 12)
        self.assertEqual(len(runtime.started), 1)
        with self.assertRaises(RequestError) as error:
            manager.start("python", "key")
        self.assertEqual(error.exception.code, "key-required")
        self.assertNotIn(CANARY, json.dumps(manager.snapshot()))

    def test_replacing_key_cancels_old_jobs_and_snapshots_new_key_only_on_click(self):
        manager, runtime = self.manager(concurrency=1)
        manager.start("all", "key")
        eventually(lambda: len(runtime.started) == 1)
        replacement = "new-controlled-key-only"
        manager.set_token(replacement)
        eventually(lambda: manager.snapshot()["counts"]["cancelled"] == 12)
        self.assertEqual(runtime.tokens, [CANARY])
        runtime.release.set()
        manager.start("python", "key")
        eventually(lambda: manager.snapshot()["counts"]["completed"] == 1)
        self.assertEqual(runtime.tokens[-1], replacement)
        self.assertNotIn(replacement, json.dumps(manager.snapshot()))

    def test_allowlist_validation_and_fixed_budget_do_not_enqueue(self):
        manager, runtime = self.manager(session_limit=1)
        for language, operation in ((["python"], "key"), ("python;anything", "key"), ("python", {"argv": []}), ("python", "credits")):
            with self.assertRaises(RequestError):
                manager.start(language, operation)
        self.assertEqual(manager.snapshot()["submitted"], 0)
        self.assertEqual(runtime.started, [])
        with self.assertRaises(RequestError):
            manager.start("all", "key")
        self.assertEqual(manager.snapshot()["submitted"], 0)
        manager.start("python", "key")
        with self.assertRaises(RequestError) as error:
            manager.start("go", "key")
        self.assertEqual(error.exception.status, 429)

    def test_token_validation_never_echoes_input(self):
        manager, _ = self.manager()
        for token in ("too-short" + "\n", "x" * 4097, " spaces not allowed ", {"secret": CANARY}, None):
            with self.assertRaises(RequestError) as error:
                manager.set_token(token)
            self.assertNotIn(CANARY, str(error.exception))
            self.assertEqual(error.exception.status, 400)

    def test_history_is_bounded_and_receipts_keep_prior_terminal_results(self):
        manager, runtime = self.manager(history_limit=len(CONSUMERS), session_limit=20)
        runtime.release.set()
        for number in range(20):
            job_id = manager.start("python", "key")["jobIds"][0]
            eventually(lambda: manager.get(job_id)["state"] == "completed")
        self.assertEqual(manager.snapshot()["historyCount"], len(CONSUMERS))
        self.assertEqual(len(list((self.root / "receipts").iterdir())), 20)
        with self.assertRaises(RequestError):
            manager.start("python", "key")

    def test_exception_with_token_is_generic_in_memory_and_receipt(self):
        runtime = GateRuntime()
        def failed(*_):
            raise RuntimeError(CANARY)
        runtime.execute = failed
        manager, _ = self.manager(runtime)
        manager.start("python", "key")
        eventually(lambda: manager.snapshot()["counts"]["failed"] == 1)
        self.assertEqual(manager.snapshot()["jobs"][0]["result"]["kind"], "execution-failed")
        self.assertNotIn(CANARY, json.dumps(manager.snapshot()))
        self.assertNotIn(CANARY, next((self.root / "receipts").iterdir()).read_text())


if __name__ == "__main__":
    unittest.main()
