"""Adversarial contract tests. Synthetic report fixtures here are NOT observations.

Actual SDK measurements are produced only by run.py collect and the twelve
native drivers. Unit fixtures are private, disposable, and visibly named UNIT.
"""

from __future__ import annotations

import copy
import http.client
import json
import math
import os
from pathlib import Path
import socket
import sys
import tempfile
import time
import unittest

from bindings import load_binding
from evidence import (Commands, DEFAULT_RUN_POLICY, EvidenceError, FORMAT, ITERATIONS, MARKER, METHODOLOGY, PHASES,
                      PRECISION, REQUIRED_ENV, RecordingServer, RUN_KINDS, TIERS, TOKEN, check_summary, dimensions,
                      file_record, inventory, read_json, required_environment, run_policy, run_statistics, run_set_summary,
                      sha, sha_bytes, summary_view, targets_from, validate_report, verify_run_set, verify_summary,
                      witness, write_json, write_new)
from inputs import Inputs
from native import native_environment
from run import reserve_output


class Scratch(unittest.TestCase):
    def setUp(self) -> None:
        workspace = Path(__file__).resolve().parents[2]
        parent = Path(os.environ.get("SUSPECT_TEST_TMPDIR", str(workspace / "target")))
        self.temp = tempfile.TemporaryDirectory(prefix="sdk-native-costs-UNIT-", dir=parent)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def put(self, path: Path, value: object) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))


class ReportContract(Scratch):
    """Synthetic 4-dimension report fixture.

    ``plan`` selects the run-set shape: ``None`` keeps the historical
    single-sample receipt shape; a run-kind list emits repeated-v1 rows with
    one attributed raw sample per declared run.
    """

    plan: list[str] | None = None

    def setUp(self) -> None:
        super().setUp()
        self.packages = self.root / "packages"
        self.output = self.root / "report"
        self.output.mkdir()
        self.target = {"language": "python", "manifest": "pyproject.toml", "toolchain_tiers": ["3.11"]}
        write_new(self.packages / "python/pyproject.toml", "UNIT fixture manifest\n")
        self.program = self.root / "UNIT-fixture-program"
        write_new(self.program, "#!/bin/sh\n# UNIT fixture only\nexit 0\n")
        self.program.chmod(0o755)
        artifact = self.root / "UNIT-artifact"
        write_new(artifact, "UNIT compiler payload fixture\n")
        self.installed = self.root / "installed"
        write_new(self.installed / "UNIT-module", "original UNIT installed bytes\n")
        write_json(self.output / "input-inventory.json", {"trees": {str(self.installed): inventory(self.installed)}, "files": []})
        self.report = {"format": FORMAT, "complete": True, "sourceFingerprint": "a" * 64, "cliSha256": "b" * 64,
                       "measurements": [], "failures": [], "inputInventory": "input-inventory.json",
                       "requiredDimensionCount": len(PHASES), "missing": [], "selection": [],
                       "fixture": {"creditsSha256": "c" * 64}}
        for phase in PHASES:
            samples = []
            plan = self.plan if self.plan is not None else [None]
            for index, kind in enumerate(plan):
                suffix = "" if kind is None else f"-{index}"
                record = {"command": [str(self.program), phase], "nanoseconds": 17, "exitCode": 0,
                          "programSha256": sha(self.program), "programUnchanged": True,
                          "record": phase + suffix + ".json", "stdout": phase + suffix + ".stdout",
                          "stderr": phase + suffix + ".stderr"}
                value = {"phase": phase, "iterations": ITERATIONS[phase], "loaded": True,
                         "precision": PRECISION, "encodedBytes": 80}
                write_new(self.output / record["stdout"], "UNIT synthetic build\n" if phase == "build" else MARKER + json.dumps(value) + "\n")
                write_new(self.output / record["stderr"], "")
                record["stdoutSha256"], record["stderrSha256"] = sha(self.output / record["stdout"]), sha(self.output / record["stderr"])
                write_json(self.output / record["record"], record)
                sample = {**record, "iterations": ITERATIONS[phase]}
                if phase == "request":
                    wire_name = "wire.json" if kind is None else f"wire-{index}.json"
                    wire = {"errors": [], "threadTerminated": True, "requests": [
                        {"method": "GET", "target": "/api/v1/credits", "bodyHex": "", "responseSha256": "c" * 64,
                         "headers": {"authorization": ["Bearer " + TOKEN], "accept": ["application/json"]}}
                        for _ in range(ITERATIONS[phase])]}
                    write_json(self.output / wire_name, wire)
                    sample.update(wire=wire_name, wireSha256=sha(self.output / wire_name))
                if kind is not None:
                    sample["runKind"] = kind
                samples.append(sample)
            artifact_manifest = phase + ".artifacts.json"
            write_json(self.output / artifact_manifest, [file_record(artifact)])
            row = {"language": "python", "tier": "3.11", "phase": phase,
                   "artifactBytes": artifact.stat().st_size, "artifactManifest": artifact_manifest,
                   "artifactManifestSha256": sha(self.output / artifact_manifest),
                   "packageManifestSha256": sha(self.packages / "python/pyproject.toml"), "samples": samples}
            if self.plan is not None:
                self.report["methodology"] = METHODOLOGY
                row["methodology"] = METHODOLOGY
                row["runs"] = run_policy(self.plan.count("cold"), self.plan.count("warmup"), self.plan.count("steady"))
                row["summary"] = run_statistics([s["nanoseconds"] for s in samples if s["runKind"] == "steady"])
            self.report["measurements"].append(row)

    def check(self, report: dict | None = None, *, allow_incomplete: bool = False) -> None:
        self.put(self.output / "report.json", report or self.report)
        validate_report(self.output, [self.target], self.packages, "a" * 64, "b" * 64, allow_incomplete=allow_incomplete)

    def sample(self, phase: str = "codec") -> dict:
        return next(r for r in self.report["measurements"] if r["phase"] == phase)["samples"][0]

    def alter_log(self, phase: str, text: str) -> None:
        sample = self.sample(phase)
        (self.output / sample["stdout"]).write_text(text)
        sample["stdoutSha256"] = sha(self.output / sample["stdout"])
        record = read_json(self.output / sample["record"])
        record["stdoutSha256"] = sample["stdoutSha256"]
        self.put(self.output / sample["record"], record)

    def test_valid_synthetic_contract_is_not_a_real_collection(self) -> None:
        self.check()
        self.assertEqual(len(dimensions([self.target])), 4)

    def test_missing_duplicate_and_unknown_dimensions_fail(self) -> None:
        for mode in ("missing", "duplicate", "unknown-tier", "unknown-phase"):
            report = copy.deepcopy(self.report)
            if mode == "missing": report["measurements"].pop()
            elif mode == "duplicate": report["measurements"].append(report["measurements"][0])
            elif mode == "unknown-tier": report["measurements"][0]["tier"] = "future"
            else: report["measurements"][0]["phase"] = "test-duration"
            with self.subTest(mode=mode), self.assertRaises(EvidenceError):
                self.check(report)

    def test_source_and_cli_fingerprints_cannot_be_substituted(self) -> None:
        for key in ("sourceFingerprint", "cliSha256"):
            report = copy.deepcopy(self.report); report[key] = "0" * 64
            with self.subTest(key=key), self.assertRaises(EvidenceError): self.check(report)

    def test_failed_zero_negative_boolean_and_unmaintained_samples_fail(self) -> None:
        for key, values in (("nanoseconds", [0, -1, True]), ("iterations", [0, -1, True, 100000]), ("exitCode", [1, False])):
            for value in values:
                report = copy.deepcopy(self.report)
                report["measurements"][1]["samples"][0][key] = value
                with self.subTest(key=key, value=value), self.assertRaises(EvidenceError): self.check(report)

    def test_no_samples_is_not_a_measurement(self) -> None:
        self.report["measurements"][0]["samples"] = []
        with self.assertRaises(EvidenceError): self.check()

    def test_package_manifest_and_installed_tree_drift_fail(self) -> None:
        for path in (self.packages / "python/pyproject.toml", self.installed / "UNIT-module"):
            original = path.read_bytes()
            path.write_bytes(original + b"tampered")
            with self.subTest(path=path), self.assertRaises(EvidenceError): self.check()
            path.write_bytes(original)

    def test_executable_and_artifact_drift_fail(self) -> None:
        for path in (self.program, self.root / "UNIT-artifact"):
            original = path.read_bytes(); path.write_bytes(original + b"tampered")
            with self.subTest(path=path), self.assertRaises(EvidenceError): self.check()
            path.write_bytes(original)

    def test_raw_log_tampering_fails(self) -> None:
        (self.output / self.sample()["stdout"]).write_text("changed bytes")
        with self.assertRaises(EvidenceError): self.check()

    def test_sample_cannot_borrow_other_command_duration(self) -> None:
        self.sample()["nanoseconds"] += 1
        with self.assertRaisesRegex(EvidenceError, "attribution"): self.check()

    def test_wrong_artifact_size_or_inventory_hash_fails(self) -> None:
        for key, value in (("artifactBytes", 999), ("artifactManifestSha256", "0" * 64)):
            report = copy.deepcopy(self.report); report["measurements"][0][key] = value
            with self.subTest(key=key), self.assertRaises(EvidenceError): self.check(report)

    def test_empty_exit_zero_or_wrong_phase_native_output_fails(self) -> None:
        for text in ("", "all tests passed\n", MARKER + json.dumps({"phase": "request", "iterations": 16}) + "\n"):
            self.alter_log("codec", text)
            with self.subTest(text=text), self.assertRaises(EvidenceError): self.check()

    def test_precision_loss_and_unexecuted_encode_fail(self) -> None:
        for key, value in (("precision", "100.5"), ("encodedBytes", 0), ("iterations", 1)):
            payload = {"phase": "codec", "iterations": 16, "precision": PRECISION, "encodedBytes": 80}
            payload[key] = value
            self.alter_log("codec", MARKER + json.dumps(payload) + "\n")
            with self.subTest(key=key), self.assertRaises(EvidenceError): self.check()

    def test_traversal_absolute_and_symlink_log_escape_fail(self) -> None:
        outside = self.root / "escape.log"; outside.write_text("outside")
        (self.output / "escape-link").symlink_to(outside)
        for path in ("../escape.log", str(outside), "escape-link", "./codec.stdout", "a/../codec.stdout"):
            report = copy.deepcopy(self.report)
            sample = report["measurements"][2]["samples"][0]
            sample["stdout"] = path; sample["stdoutSha256"] = sha(outside)
            command = read_json(self.output / sample["record"])
            command["stdout"] = path; command["stdoutSha256"] = sha(outside)
            self.put(self.output / sample["record"], command)
            with self.subTest(path=path), self.assertRaises(EvidenceError): self.check(report)

    def test_forged_or_missing_request_records_fail(self) -> None:
        sample = self.sample("request")
        original = read_json(self.output / sample["wire"])
        for mode in ("missing", "auth", "body", "cookie", "response", "cleanup"):
            wire = copy.deepcopy(original)
            if mode == "missing": wire["requests"].pop()
            elif mode == "auth": wire["requests"][0]["headers"]["authorization"] = ["Bearer other"]
            elif mode == "body": wire["requests"][0]["bodyHex"] = "00"
            elif mode == "cookie": wire["requests"][0]["headers"]["cookie"] = ["saved=yes"]
            elif mode == "response": wire["requests"][0]["responseSha256"] = "f" * 64
            else: wire["threadTerminated"] = False
            self.put(self.output / sample["wire"], wire)
            self.sample("request")["wireSha256"] = sha(self.output / sample["wire"])
            with self.subTest(mode=mode), self.assertRaises(EvidenceError): self.check()

    def test_partial_report_stays_incomplete_and_needs_explicit_validation_option(self) -> None:
        self.report["complete"] = False; self.report["measurements"].pop()
        with self.assertRaisesRegex(EvidenceError, "incomplete"): self.check()
        self.check(allow_incomplete=True)

    def test_single_sample_receipts_summarize_as_their_own_one_run_set(self) -> None:
        self.check()
        row = self.report["measurements"][2]
        summary = run_set_summary(row)
        self.assertEqual(summary["runs"], 1)
        self.assertEqual(summary["median"], float(row["samples"][0]["nanoseconds"]))
        self.assertEqual(summary["p99"], float(row["samples"][0]["nanoseconds"]))

    def test_run_kind_tags_without_a_declared_methodology_fail(self) -> None:
        # Old receipts must not carry run-kind tags; declared run-set rows must
        # keep their declared run counts. Either contract violation is rejected.
        self.report["measurements"][2]["samples"][0]["runKind"] = "steady"
        with self.assertRaisesRegex(EvidenceError, "run-kind|declared run set"): self.check()


class RepeatedRunContract(ReportContract):
    """The full adversarial battery, replayed against repeated-v1 run-set rows."""

    plan = (["cold"] * DEFAULT_RUN_POLICY["cold"] + ["warmup"] * DEFAULT_RUN_POLICY["warmup"]
            + ["steady"] * DEFAULT_RUN_POLICY["steady"])

    def steady(self, phase: str = "codec") -> dict:
        row = next(r for r in self.report["measurements"] if r["phase"] == phase)
        return next(s for s in row["samples"] if s["runKind"] == "steady")

    def retime(self, phase: str, nanoseconds: int, *, consistent: bool = True) -> None:
        """Rewrite a steady run's duration in the sample and, when consistent, its subprocess record."""
        sample = self.steady(phase)
        sample["nanoseconds"] = nanoseconds
        if consistent:
            record = read_json(self.output / sample["record"])
            record["nanoseconds"] = nanoseconds
            self.put(self.output / sample["record"], record)

    def retime_steady_batch(self, phase: str = "codec", base: int = 1000) -> list[int]:
        row = next(r for r in self.report["measurements"] if r["phase"] == phase)
        values = []
        for index, sample in enumerate(s for s in row["samples"] if s["runKind"] == "steady"):
            value = base + index * 37
            sample["nanoseconds"] = value
            record = read_json(self.output / sample["record"])
            record["nanoseconds"] = value
            self.put(self.output / sample["record"], record)
            values.append(value)
        return values

    def test_single_sample_receipts_summarize_as_their_own_one_run_set(self) -> None:
        # Overridden for run-set rows: they summarize as their declared steady
        # batch, while historical single-sample rows summarize as one run.
        self.check()
        for row in self.report["measurements"]:
            self.assertEqual(run_set_summary(row), row["summary"])

    def test_summary_covers_only_the_steady_batch_and_keeps_all_raw_runs(self) -> None:
        row = self.report["measurements"][3]
        kinds = {kind: [s["nanoseconds"] for s in row["samples"] if s["runKind"] == kind] for kind in RUN_KINDS}
        self.assertEqual(row["summary"]["runs"], len(kinds["steady"]))
        self.assertEqual(row["runs"], run_policy(len(kinds["cold"]), len(kinds["warmup"]), len(kinds["steady"])))
        self.assertEqual(len(row["samples"]), sum(row["runs"].values()))
        verify_run_set(row)

    def test_tampered_summary_fields_are_recomputed_and_rejected(self) -> None:
        for key in ("runs", "min", "max", "median", "p90", "p99", "mean", "stddev"):
            report = copy.deepcopy(self.report)
            report["measurements"][2]["summary"][key] += 1 if key == "runs" else 0.5
            with self.subTest(key=key), self.assertRaisesRegex(EvidenceError, "summary"): self.check(report)

    def test_tampered_raw_steady_sample_breaks_the_summary_after_attribution(self) -> None:
        # The duration is rewritten consistently in the sample and its command
        # record, so attribution passes and only the recomputed summary can
        # reject the receipt.
        self.retime("codec", 4242)
        with self.assertRaisesRegex(EvidenceError, "summary"): self.check()

    def test_retimed_batch_with_recomputed_summary_verifies(self) -> None:
        values = self.retime_steady_batch()
        row = self.report["measurements"][2]
        row["summary"] = run_statistics(values)
        self.check()
        self.assertEqual(run_set_summary(row), row["summary"])
        self.assertEqual(row["summary"]["median"], (1000.0 + 37 * 9 + 1000.0 + 37 * 10) / 2)

    def test_dropped_extra_and_miscounted_runs_fail(self) -> None:
        row = self.report["measurements"][2]
        dropped = copy.deepcopy(self.report)
        dropped["measurements"][2]["samples"] = [s for s in row["samples"] if s["runKind"] != "warmup"]
        with self.subTest(mode="dropped"), self.assertRaisesRegex(EvidenceError, "run set"): self.check(dropped)
        duplicated = copy.deepcopy(self.report)
        duplicated["measurements"][2]["samples"].append(copy.deepcopy(row["samples"][0]))
        with self.subTest(mode="extra"), self.assertRaisesRegex(EvidenceError, "run set"): self.check(duplicated)
        for key in ("cold", "warmup", "steady"):
            miscounted = copy.deepcopy(self.report)
            miscounted["measurements"][2]["runs"][key] += 1
            with self.subTest(mode="count", key=key), self.assertRaisesRegex(EvidenceError, "run set"): self.check(miscounted)

    def test_unknown_methodology_missing_summary_and_untagged_runs_fail(self) -> None:
        report = copy.deepcopy(self.report)
        report["measurements"][1]["methodology"] = "repeated-v0"
        with self.subTest(mode="methodology"), self.assertRaisesRegex(EvidenceError, "methodology"): self.check(report)
        report = copy.deepcopy(self.report)
        del report["measurements"][1]["summary"]
        with self.subTest(mode="no-summary"), self.assertRaisesRegex(EvidenceError, "summary"): self.check(report)
        report = copy.deepcopy(self.report)
        del report["measurements"][1]["runs"]
        with self.subTest(mode="no-runs"), self.assertRaisesRegex(EvidenceError, "cold, warmup and steady"): self.check(report)
        report = copy.deepcopy(self.report)
        report["measurements"][1]["samples"][3]["runKind"] = "invented"
        with self.subTest(mode="tag"), self.assertRaisesRegex(EvidenceError, "runKind"): self.check(report)

    def test_run_set_summary_normalization_matches_the_stored_summary(self) -> None:
        self.check()
        for row in self.report["measurements"]:
            self.assertEqual(run_set_summary(row), row["summary"])


class SummaryViewContract(RepeatedRunContract):
    """--report summary views: derived aggregates cross-checked against full raw evidence."""

    def view(self) -> dict:
        raw = copy.deepcopy(self.report)
        path = self.output / "raw-evidence.json"
        if path.exists():
            path.unlink()
        write_json(path, raw)
        return summary_view(raw, "raw-evidence.json", sha(path))

    def validate_view(self, view: dict) -> None:
        self.put(self.output / "report.json", view)
        validate_report(self.output, [self.target], self.packages, "a" * 64, "b" * 64)

    def test_derived_summary_view_verifies_against_its_raw_report(self) -> None:
        self.validate_view(self.view())
        written = read_json(self.output / "report.json")
        self.assertEqual(written["reportView"], "summary")
        self.assertEqual(len(written["measurements"]), len(self.report["measurements"]))
        self.assertNotIn("samples", written["measurements"][0])
        self.assertEqual(written["measurements"][0]["summary"], self.report["measurements"][0]["summary"])
        self.assertEqual(written["measurements"][3]["runs"], self.report["measurements"][3]["runs"])

    def test_view_row_header_and_pointer_tampering_fails(self) -> None:
        view = self.view(); view["measurements"][2]["summary"]["median"] += 1.0
        with self.subTest(mode="row"), self.assertRaisesRegex(EvidenceError, "summary view row"): self.validate_view(view)
        view = self.view(); view["rawReportSha256"] = "0" * 64
        with self.subTest(mode="digest"), self.assertRaisesRegex(EvidenceError, "digest"): self.validate_view(view)
        view = self.view(); view["complete"] = False
        with self.subTest(mode="header"), self.assertRaisesRegex(EvidenceError, "header"): self.validate_view(view)
        view = self.view(); view["measurements"].pop()
        with self.subTest(mode="count"), self.assertRaisesRegex(EvidenceError, "row count"): self.validate_view(view)
        view = self.view(); view["measurements"][0]["samples"] = [{"nanoseconds": 1}]
        with self.subTest(mode="smuggled"), self.assertRaisesRegex(EvidenceError, "raw samples"): self.validate_view(view)
        view = self.view(); view["measurements"][1]["methodology"] = "repeated-v0"
        with self.subTest(mode="methodology"), self.assertRaisesRegex(EvidenceError, "summary view row"): self.validate_view(view)

    def test_view_must_point_at_a_raw_report_not_another_view(self) -> None:
        first = self.view()
        write_json(self.output / "other-view.json", first)
        chained = summary_view(first, "other-view.json", sha(self.output / "other-view.json"))
        with self.assertRaisesRegex(EvidenceError, "not another view"): self.validate_view(chained)


class RunSetPolicyAndStatistics(Scratch):
    def test_flag_bounds_are_enforced(self) -> None:
        self.assertEqual(run_policy(1, 0, 1), {"cold": 1, "warmup": 0, "steady": 1})
        self.assertEqual(run_policy(**DEFAULT_RUN_POLICY), DEFAULT_RUN_POLICY)
        for cold, warmup, steady in ((0, 2, 20), (-1, 2, 20), (5, -1, 20), (5, 2, 0), (True, 2, 20),
                                     (5, True, 20), (5, 2, True), (1.5, 2, 20), (5, 2, 20.0), (None, 2, 20)):
            with self.subTest(cold=cold, warmup=warmup, steady=steady), self.assertRaises(EvidenceError):
                run_policy(cold, warmup, steady)

    def test_statistics_are_exact_and_self_consistent(self) -> None:
        stats = run_statistics([5, 1, 9, 7, 3])
        self.assertEqual((stats["min"], stats["max"]), (1, 9))
        self.assertEqual(stats["median"], 5.0)
        self.assertEqual(stats["mean"], 5.0)
        self.assertEqual(stats["stddev"], math.sqrt(8.0))
        self.assertAlmostEqual(stats["p90"], 8.2, places=12)
        self.assertAlmostEqual(stats["p99"], 8.92, places=12)
        self.assertEqual(run_statistics([10, 20])["median"], 15.0)
        single = run_statistics([42])
        self.assertEqual((single["runs"], single["min"], single["max"], single["median"], single["p90"], single["p99"],
                          single["mean"], single["stddev"]), (1, 42, 42, 42.0, 42.0, 42.0, 42.0, 0.0))
        self.assertEqual(run_set_summary({"samples": [{"nanoseconds": 42}]}), single)
        for values in ([], [0], [-5], [True], [1, 2.5], [3, None]):
            with self.subTest(values=values), self.assertRaises(EvidenceError): run_statistics(values)

    def test_summary_shape_and_consistency_checks(self) -> None:
        stats = run_statistics([3, 1, 2])
        check_summary(stats)
        verify_summary(copy.deepcopy(stats), stats, "UNIT")
        base = dict(stats)
        for key in ("runs", "min", "max", "median", "p90", "p99", "mean", "stddev"):
            broken = dict(base); del broken[key]
            with self.subTest(mode="missing", key=key), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["extra"] = 1
        with self.subTest(mode="extra"), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["min"] = broken["max"] + 1
        with self.subTest(mode="order"), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["stddev"] = -1.0
        with self.subTest(mode="stddev"), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["mean"] = 0.0
        with self.subTest(mode="mean"), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["runs"] = True
        with self.subTest(mode="runs-type"), self.assertRaises(EvidenceError): check_summary(broken)
        broken = dict(base); broken["median"] = int(broken["median"])
        with self.subTest(mode="type"), self.assertRaisesRegex(EvidenceError, "disagrees"): verify_summary(broken, stats, "UNIT")


class ConfigurationAndBindings(Scratch):
    def test_all_six_environment_fields_are_required(self) -> None:
        env = {key: str(self.root / key) for key in REQUIRED_ENV}
        env["SUSPECT_SDK_FULL_SOURCE_SHA256"] = "a" * 64
        env["SUSPECT_SDK_FULL_BINARY_SHA256"] = "b" * 64
        required_environment(env)
        for key in REQUIRED_ENV:
            missing = dict(env); missing.pop(key)
            with self.subTest(key=key), self.assertRaisesRegex(EvidenceError, key): required_environment(missing)

    def test_maintained_configuration_has_exactly_92_dimensions(self) -> None:
        targets = [{"language": language, "manifest": "UNIT.json", "package_name": "UNIT", "toolchain_tiers": tiers}
                   for language, tiers in TIERS.items()]
        path = self.root / "targets.json"; self.put(path, targets)
        self.assertEqual(len(dimensions(targets_from(path))), 92)
        for mode in ("duplicate", "removed", "tier"):
            changed = copy.deepcopy(targets)
            if mode == "duplicate": changed[0] = changed[1]
            elif mode == "removed": changed.pop()
            else: changed[0]["toolchain_tiers"] = ["invented"]
            self.put(path, changed)
            with self.subTest(mode=mode), self.assertRaises(EvidenceError): targets_from(path)

    def test_output_is_create_once_and_cannot_overlap_inputs(self) -> None:
        packages, native = self.root / "packages", self.root / "native"
        packages.mkdir(); native.mkdir()
        output = self.root / "new-report"
        env = {"SUSPECT_SDK_FULL_PACKAGES": str(packages), "SUSPECT_SDK_FULL_NATIVE_ROOT": str(native), "SUSPECT_SDK_FULL_MEASUREMENTS": str(output)}
        reserve_output(env); (output / "sentinel").write_text("user evidence")
        with self.assertRaises(EvidenceError): reserve_output(env)
        self.assertEqual((output / "sentinel").read_text(), "user evidence")
        env["SUSPECT_SDK_FULL_MEASUREMENTS"] = str(packages / "nested")
        with self.assertRaises(EvidenceError): reserve_output(env)

    def test_generated_manifest_and_symlink_adversaries_fail(self) -> None:
        generated = self.root / "generated"; native = self.root / "native"; native.mkdir()
        write_new(generated / "sdk/source", "UNIT actual source")
        self.put(self.root / "generated-manifest.json", {"sdk/source": {"kind": "file", "sha256": "0" * 64, "bytes": 18}})
        inputs = Inputs(generated, native, self.root)
        with self.assertRaises(EvidenceError): inputs.generated()
        (generated / "escape").symlink_to(self.root / "generated-manifest.json")
        with self.assertRaises(EvidenceError): inventory(generated)

    def test_metadata_allocations_drive_symbols_without_reading_native_source(self) -> None:
        package = self.root / "python"
        target = {"language": "python", "import_name": "sdk_full", "package_name": "sdk-full"}
        operation = {"operationId": "getCredits", "method": "allocated_credits_method", "source": {"document": "file:///UNIT.yaml", "pointer": "/paths/~1credits/get"},
                     "responses": [{"status": 200, "model": "AllocatedCredits777"}]}
        path = package / "src/sdk_full/http-manifest.json"
        self.put(path, {"operations": [operation]})
        binding = load_binding(package, target)
        self.assertEqual(binding["model"], "AllocatedCredits777")
        self.assertEqual(binding["codec"], "AllocatedCredits777Codec")
        self.assertEqual(binding["method"], "allocated_credits_method")
        operation["responses"][0]["model"] = "Injected();not_a_symbol"
        self.put(path, {"operations": [operation]})
        with self.assertRaises(EvidenceError): load_binding(package, target)

    def test_duplicate_metadata_rows_and_wrong_operation_source_fail(self) -> None:
        target = {"language": "typescript", "package_name": "UNIT"}
        path = self.root / "http-manifest.json"
        operation = {"operationId": "getCredits", "export": "credits", "source": {"document": "file:///UNIT", "pointer": "/paths/~1credits/get"}, "responses": [{"status": 200, "model": "Credits"}]}
        self.put(path, {"operations": [operation, operation]})
        with self.assertRaises(EvidenceError): load_binding(self.root, target)
        operation["source"]["pointer"] = "/paths/~1unrelated/get"
        self.put(path, {"operations": [operation]})
        with self.assertRaises(EvidenceError): load_binding(self.root, target)

    def test_hostile_runtime_options_and_ambient_proxy_are_not_inherited(self) -> None:
        env = native_environment({"HOME": str(self.root), "PATH": os.defpath, "HTTP_PROXY": "https://bad.invalid", "NODE_OPTIONS": "--require bad.js", "PYTHONPATH": "/bad", "JAVA_TOOL_OPTIONS": "-javaagent:bad.jar"}, self.root / "work")
        for key in ("HTTP_PROXY", "NODE_OPTIONS", "PYTHONPATH", "JAVA_TOOL_OPTIONS"):
            self.assertNotIn(key, env)
        self.assertEqual(env["GOPROXY"], "off")
        self.assertEqual(env["CARGO_NET_OFFLINE"], "true")


class ProcessAndWire(Scratch):
    def test_real_subprocess_transcript_and_timeout_survive(self) -> None:
        commands = Commands(self.root)
        env = {"PATH": os.defpath, "PYTHONDONTWRITEBYTECODE": "1"}
        record = commands.run([str(Path(sys.executable).absolute()), "-c", "print('UNIT child executed', flush=True)"], self.root, env, label="UNIT process")
        self.assertGreater(record["nanoseconds"], 0)
        self.assertEqual(commands.stdout(record).strip(), "UNIT child executed")
        self.assertEqual(sha(self.root / record["stdout"]), record["stdoutSha256"])
        before = time.monotonic()
        with self.assertRaises(EvidenceError):
            commands.run([str(Path(sys.executable).absolute()), "-c", "import time; print('UNIT partial', flush=True); time.sleep(30)"], self.root, env, label="UNIT timeout", timeout=0.15)
        self.assertLess(time.monotonic() - before, 5)
        partial = commands.records[-1]
        self.assertTrue(partial["timedOut"])
        self.assertIn("UNIT partial", commands.stdout(partial))
        self.assertTrue((self.root / partial["record"]).is_file())

    def test_exit_zero_without_native_witness_is_rejected(self) -> None:
        with self.assertRaises(EvidenceError): witness("test result: ok\n", "codec", 16)
        output = MARKER + json.dumps({"phase": "import", "iterations": 1, "loaded": True})
        with self.assertRaises(EvidenceError): witness(output + "\n" + output, "import", 1)

    def request(self, server: RecordingServer, headers: dict[str, str] | None = None, body: bytes | None = None) -> bytes:
        connection = http.client.HTTPConnection("127.0.0.1", server.port, timeout=3)
        try:
            connection.request("GET", "/api/v1/credits", body=body,
                               headers=headers or {"Authorization": "Bearer " + TOKEN, "Accept": "application/json"})
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            return response.read()
        finally:
            connection.close()

    def test_owned_loopback_records_exact_auth_body_response_and_cleanup(self) -> None:
        response = b'{"UNIT":"independent response"}'
        server = RecordingServer(response, 2)
        with server:
            for _ in range(2): self.assertEqual(self.request(server), response)
        server.verify()
        self.assertFalse(server.worker.is_alive())
        self.assertEqual(server.records[0]["responseSha256"], sha_bytes(response))

    def test_wire_auth_body_cookie_and_extra_request_adversaries(self) -> None:
        valid = {"Authorization": "Bearer " + TOKEN, "Accept": "application/json"}
        for mode in ("auth", "body", "cookie", "extra"):
            server = RecordingServer(b"{}", 0 if mode == "extra" else 1)
            headers = dict(valid)
            if mode == "auth": headers["Authorization"] = "Bearer wrong"
            if mode == "cookie": headers["Cookie"] = "retained=yes"
            with self.subTest(mode=mode), server:
                with self.assertRaises(http.client.RemoteDisconnected): self.request(server, headers, b"body" if mode == "body" else None)
            with self.assertRaises(EvidenceError): server.verify()
            self.assertFalse(server.worker.is_alive())

    def test_incomplete_socket_client_cannot_hang_server_cleanup(self) -> None:
        server = RecordingServer(b"{}", 1)
        before = time.monotonic()
        with server:
            connection = socket.create_connection(("127.0.0.1", server.port), timeout=1)
            connection.sendall(b"GET /api/v1/credits HTTP/1.1\r\nHost:")
        connection.close()
        self.assertFalse(server.worker.is_alive())
        self.assertLess(time.monotonic() - before, 4)
        with self.assertRaises(EvidenceError): server.verify()


if __name__ == "__main__":
    unittest.main()
