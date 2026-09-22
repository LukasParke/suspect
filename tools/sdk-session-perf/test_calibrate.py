"""Collector control-flow checks use fake subprocess reports, never real timings."""

import argparse
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

import calibrate
import compare
import run
from test_compare import POLICY, synthetic_report


def options(root):
    compare.write_new(root / "policy.json", POLICY)
    return argparse.Namespace(
        policy=root / "policy.json", out=root / "collection", role="baseline",
        repeats=5, iterations=200, warmups=5, gate=False, require_qualified=True,
        baseline=None, suite="public", runner=None, openrouter_root=None, offline=True,
        work_root=root / "work", build_dir=root / "build", group=None,
    )


class CollectorTests(unittest.TestCase):
    def test_collection_launches_every_repeat_and_retains_raw_identity_evidence(self):
        with tempfile.TemporaryDirectory(prefix="sdk-session-collector-test-", dir=run.ROOT / "target") as directory:
            root = Path(directory)
            args = options(root)
            commands = []

            def launch(command, **_kwargs):
                index = len(commands)
                commands.append(command)
                start = time.time_ns()
                # Artificial tiny values fit the synthetic subprocess interval.
                report = synthetic_report(f"fake-collector-{index}", scale=1e-9)
                finish = time.time_ns()
                report.update(process_id=200 + index, started_at_ns=start, finished_at_ns=finish)
                report["cases"][0]["report"]["process"] = {
                    "pid": 300 + index, "started_at_ns": start + 1, "finished_at_ns": finish - 1,
                }
                destination = Path(command[command.index("--out") + 1])
                destination.mkdir()
                compare.write_new(destination / "report.json", report)
                return type("FakeProcess", (), {"pid": 200 + index, "wait": lambda self: 0})()

            with patch.object(calibrate.subprocess, "Popen", side_effect=launch):
                collection, result = calibrate.collect(args)
            self.assertEqual(len(commands), 5)
            self.assertTrue(all(command[command.index("--iterations") + 1] == "200" for command in commands))
            self.assertTrue(all(command[command.index("--warmups") + 1] == "5" for command in commands))
            self.assertEqual(result["status"], "qualified")
            self.assertTrue(collection["complete"])
            stored = compare.load(args.out / "baseline.json")
            self.assertEqual(len(stored["calibration_runs"]), 5)
            self.assertEqual(stored["collection_evidence"]["attempts"], collection["attempts"])

    def test_failed_attempt_is_retained_and_cannot_be_skipped_to_get_a_baseline(self):
        with tempfile.TemporaryDirectory(prefix="sdk-session-collector-test-", dir=run.ROOT / "target") as directory:
            args = options(Path(directory))
            failed = type("FailedProcess", (), {"pid": 321, "wait": lambda self: 1})()
            with patch.object(calibrate.subprocess, "Popen", return_value=failed) as launch:
                with self.assertRaises(compare.ReportError):
                    calibrate.collect(args)
            self.assertEqual(launch.call_count, 1)
            self.assertFalse((args.out / "baseline.json").exists())
            collection = compare.load(args.out / "collection.json")
            self.assertFalse(collection["complete"])
            self.assertEqual(collection["requested_runs"], 5)
            self.assertEqual(collection["attempts"][0]["exit_code"], 1)


if __name__ == "__main__":
    unittest.main()
