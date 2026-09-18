"""Adversarial gate tests. Synthetic timings below are NOT benchmark evidence."""

import copy
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import compare

HERE = Path(__file__).resolve().parent
POLICY = compare.load(HERE / "policy-v2.json")
POLICY["required_fixtures"] = ["synthetic"]
POLICY["required_groups"] = ["all"]


def pin(path, size, tag):
    return {"path": path, "bytes": size, "sha256": compare.digest(tag)}


def synthetic_report(run_id="candidate", iterations=200, scale=1.0):
    """A controlled five-target workload, for checking decisions/exit codes only."""
    source_files = [pin("src/lib.rs", 100, "same-source")]
    harness_files = [pin("benchmark.rs", 100, "same-harness")]
    binary = pin("/fake/bin/bench", 100, "same-binary")
    inputs = [pin("input.json", 100, "same-input")]
    targets = [{"backend": name, "package_name": "BenchmarkSDK" if name == "swift-http" else "sdk-test", "package_version": "0.0.0", "import_name": None}
               for name in compare.CANONICAL_BACKENDS]
    config = {"targets": targets, "iterations": iterations, "warmups": 5, "operation_ids": [], "mode": "observational",
              "cache_entries": 4, "cache_bytes": 268435456, "owner": "test",
              "preparation": "private-canonical-json-v1", "schedule": "rotating-edit-revert-cycles-v1",
              "measurement": "system-allocator-generate-plus-owned-write-v1"}
    names = ("baseline", "schema-edit", "operation-edit", "docs-edit")
    oracles = [{"scenario": name, "files": [pin(f"{target}/client", 4096, name + target)
                for target in ("typescript", "rust", "python", "go", "swift")]} for name in names]
    hashes = {oracle["scenario"]: {item["path"]: item["sha256"] for item in oracle["files"]} for oracle in oracles}
    samples, warmups = [], []
    for cycle in range(iterations + 5):
        counters = {"compiles": 0, "renders": 0, "cache_hits": 0}
        previous = {}
        for scenario in compare.SCENARIOS:
            miss = scenario in ("cold", "schema-edit", "operation-edit", "docs-edit")
            delta = {"compiles": 1 if miss else 0, "renders": 5 if miss else 0, "cache_hits": 0 if miss else 1}
            for key, value in delta.items():
                counters[key] += value
            current = hashes[scenario if miss and scenario != "cold" else "baseline"]
            sample = {
                "cycle": cycle, "scenario": scenario,
                "generate": {"ms": 100.0 * scale, "allocation_calls": 1000, "allocated_bytes": 100000},
                "write": {"ms": 100.0 * scale, "allocation_calls": 1000, "allocated_bytes": 100000},
                "refresh_ms": 200.0 * scale, "allocation_calls": 2000, "allocated_bytes": 200000,
                "artifact_bytes": 20480, "artifact_files": 5, "artifact_sha256": compare.digest(current),
                "delta": delta, "stats": dict(counters), "new_documents": 1 if scenario == "cold" else 0,
                "changed_paths": sorted(path for path in previous.keys() | current.keys() if previous.get(path) != current.get(path)),
                "removed_files": 0, "redundant_rewrites": 0, "fresh_oracle_equal": True, "disk_current": True,
            }
            (warmups if cycle < 5 else samples).append(sample)
            previous = current
    probe = {"fresh_oracle_equal": True, "disk_current": True, "redundant_rewrites": 0}
    case = {"format": compare.BENCH_FORMAT, "fixture": "synthetic", "functional_status": "passed",
            "process": {"pid": 11, "started_at_ns": 20, "finished_at_ns": 800_000_000_000},
            "performance_status": "observational", "build_profile": "release", "binary": binary,
            "configuration": config, "configuration_sha256": compare.digest(config),
            "inputs": inputs, "input_sha256": compare.digest(inputs), "prepared_inputs": inputs,
            "prepared_input_sha256": compare.digest(inputs), "private_root": "/fake/work/private",
            "gates": {name: True for name in compare.GATES}, "oracles": oracles,
            "samples": samples, "warmup_samples": warmups,
            "configuration_probe": {
                "change": {**probe, "delta": {"compiles": 0, "renders": 0, "cache_hits": 4}, "removed_files": 1},
                "revert": {**probe, "delta": {"compiles": 0, "renders": 0, "cache_hits": 1}},
            }, "module_probe": {"package": "BenchmarkSDK", "default_module": "BenchmarkSDK", "explicit_module": "BenchmarkModule",
                "change": {**probe, "delta": {"compiles": 0, "renders": 1, "cache_hits": 4}, "removed_files": 1},
                "revert": {**probe, "delta": {"compiles": 0, "renders": 0, "cache_hits": 1}},
            }}
    actual = {"machine": "x86_64", "cpu": "fake-test-cpu", "machine_id_sha256": compare.digest("fake-machine")}
    return {"format": compare.SUITE_FORMAT, "run_id": run_id, "functional_status": "passed", "performance_status": "observational",
            "process_id": 10, "started_at_ns": 10, "finished_at_ns": 900_000_000_000,
            "integrity": {name: True for name in ("source_at_build", "binary", "tools", "inputs")},
            "identity": {
                "runner": {"declared": {"format": "suspect-sdk-session-runner-v2", "kind": "dedicated", "id": "fake-test-runner", "image": "fake-test-image",
                                        "expected_identity_sha256": compare.digest(actual)}, "actual": actual},
                "tools": {"rustc": {"version": "fake-test-version", "sha256": compare.digest("tool")}},
                "build": {"profile": "release"}, "work_root": "/fake/work",
                "harness": {"files": harness_files, "sha256": compare.digest(harness_files)},
            },
            "provenance": {"binary": binary, "source": {"files": source_files, "sha256": compare.digest(source_files)}},
            "cases": [{"id": "synthetic/all", "report": case}]}


def synthetic_collection(role="candidate", reports=None, scale=1.0, iterations=200):
    if reports is None:
        count = POLICY["minimum_runs"] if role == "baseline" else POLICY["minimum_candidate_runs"]
        reports = [synthetic_report(f"{role}-{index}", iterations, scale) for index in range(count)]
    start = 1 if role == "baseline" else 1_000_000_000_000_001
    attempts = []
    for index, report in enumerate(reports):
        begin = start + 1_000_000_000_000 * index
        report.update(process_id=100 + index, started_at_ns=begin + 10, finished_at_ns=begin + 900_000_000_000)
        report["cases"][0]["report"]["process"] = {"pid": 1000 + index, "started_at_ns": begin + 20, "finished_at_ns": begin + 800_000_000_000}
        attempts.append({"index": index, "pid": report["process_id"], "started_at_ns": begin, "finished_at_ns": begin + 950_000_000_000,
                         "exit_code": 0, "run_id": report["run_id"], "report_sha256": compare.digest(report)})
    return {"format": compare.COLLECTION_FORMAT, "role": role, "campaign_id": f"synthetic-{role}",
            "protocol": "sequential-fresh-suite-process-v1", "complete": True,
            "started_at_ns": start, "finished_at_ns": start + len(reports) * 1_000_000_000_000,
            "requested_runs": len(reports), "requested_iterations": reports[0]["cases"][0]["report"]["configuration"]["iterations"],
            "requested_warmups": 5, "attempts": attempts, "reports": reports}


def baseline_from(reports):
    collection = synthetic_collection("baseline", reports)
    return compare.build_baseline(reports, POLICY, compare.collection_evidence(collection))


class ComparisonTests(unittest.TestCase):
    def test_fixed_workload_is_valid_but_never_comparable_to_rotating_workload(self):
        rotating = synthetic_report("rotating")
        fixed = synthetic_report("fixed")
        configuration = fixed["cases"][0]["report"]["configuration"]
        configuration["schedule"] = "fixed-edit-revert-cycles-v1"
        fixed["cases"][0]["report"]["configuration_sha256"] = compare.digest(configuration)
        compare.validate_report(fixed)
        self.assertNotEqual(compare.compatibility(rotating), compare.compatibility(fixed))

    def test_cpu_attribution_cannot_disagree_with_its_cpu_phase_sum(self):
        report = synthetic_report()
        case = report["cases"][0]["report"]
        case["configuration"]["resource_attribution"] = "posix-process-rusage-outside-interval-v1"
        for sample in case["samples"] + case["warmup_samples"]:
            for phase in ("generate", "write"):
                sample[phase]["resources"] = {
                    "user_cpu_ms": 3.0, "system_cpu_ms": 2.0, "cpu_ms": 99.0,
                    "input_block_operations": 0, "output_block_operations": 0,
                    "minor_page_faults": 0, "major_page_faults": 0,
                    "voluntary_context_switches": 0, "involuntary_context_switches": 0,
                    "process_peak_rss_before_bytes": 1000, "process_peak_rss_after_bytes": 1000,
                }
        with self.assertRaisesRegex(compare.ReportError, "CPU phase sum"):
            compare.validate_report(report)

    @classmethod
    def setUpClass(cls):
        cls.runs = [synthetic_report(f"calibration-{index}") for index in range(5)]
        cls.baseline = baseline_from(cls.runs)

    def test_regression_is_detected_with_a_different_candidate_source_and_binary(self):
        reports = [synthetic_report(f"new-{index}", scale=1.20) for index in range(3)]
        for report in reports:
            report["provenance"]["source"]["files"] = [pin("src/lib.rs", 200, "new-source")]
            report["provenance"]["source"]["sha256"] = compare.digest(report["provenance"]["source"]["files"])
            new_binary = pin("/fake/bin/bench", 200, "new-binary")
            report["provenance"]["binary"] = new_binary
            report["cases"][0]["report"]["binary"] = new_binary
        candidate = synthetic_collection(reports=reports)
        result = compare.compare(self.baseline, candidate, gate=True)
        self.assertEqual(result["status"], "regressed")
        self.assertTrue(any(check["key"].endswith(":warm:refresh_ms") and check["decision"] == "regression" for check in result["checks"]))

    def test_relative_and_absolute_noise_tolerances_both_apply(self):
        # 10.1% exceeds relative 10%, but not relative 10% + the absolute floor.
        self.assertEqual(compare.compare(self.baseline, synthetic_collection(scale=1.101), gate=True)["status"], "passed")
        self.assertEqual(compare.compare(self.baseline, synthetic_collection(scale=1.11), gate=True)["status"], "regressed")

    def test_calibrated_repeatability_noise_is_added_without_pooling_runs(self):
        runs = [synthetic_report(f"noise-{index}", scale=scale) for index, scale in enumerate((0.98, 0.99, 1.0, 1.01, 1.02))]
        baseline = baseline_from(runs)
        self.assertTrue(baseline["gating_eligible"])
        self.assertEqual(baseline["limits"]["synthetic/all:warm:refresh_ms"]["noise_absolute"], 4)
        self.assertEqual(compare.compare(baseline, synthetic_collection(scale=1.14), gate=True)["status"], "passed")
        self.assertEqual(compare.compare(baseline, synthetic_collection(scale=1.141), gate=True)["status"], "regressed")

    def test_noisy_runner_cannot_be_promoted_to_a_gate(self):
        runs = [synthetic_report(f"noisy-{index}", scale=scale) for index, scale in enumerate((0.8, 1, 1, 1, 1.2))]
        baseline = baseline_from(runs)
        self.assertFalse(baseline["gating_eligible"])
        with self.assertRaises(compare.ReportError) as caught:
            compare.compare(baseline, synthetic_collection(), gate=True)
        self.assertEqual(caught.exception.status, "ineligible")

    def test_smoke_and_shared_runner_results_stay_observational(self):
        report = synthetic_report(iterations=3)
        report["identity"]["runner"]["declared"]["kind"] = "observational"
        baseline = compare.build_baseline([report], POLICY)
        candidate = copy.deepcopy(report)
        candidate["run_id"] = "independent-candidate"
        result = compare.compare(baseline, candidate)
        self.assertEqual(result["status"], "observational")
        self.assertFalse(result["gated"])
        self.assertGreaterEqual(len(baseline["ineligible_reasons"]), 3)
        with self.assertRaises(compare.ReportError):
            compare.compare(baseline, candidate, gate=True)

    def test_disabling_debug_assertions_cannot_qualify_a_debug_cargo_build(self):
        reports = copy.deepcopy(self.runs)
        for report in reports:
            # The executable can report cfg!(debug_assertions) == false even in
            # a modified dev profile. The actual Cargo invocation still matters.
            report["identity"]["build"]["profile"] = "debug"
        baseline = compare.build_baseline(reports, POLICY)
        self.assertFalse(baseline["gating_eligible"])
        self.assertTrue(any("Cargo's release profile" in reason for reason in baseline["ineligible_reasons"]))

    def test_runner_tool_input_configuration_and_harness_mismatches_are_rejected(self):
        mutations = [
            lambda report: report["identity"]["runner"]["actual"].update(cpu="different-cpu"),
            lambda report: report["identity"]["tools"]["rustc"].update(sha256=compare.digest("different-tool")),
            lambda report: report["cases"][0]["report"]["inputs"][0].update(sha256=compare.digest("different-input")),
            lambda report: report["cases"][0]["report"]["configuration"]["targets"][0].update(package_version="0.0.1"),
            lambda report: report["identity"]["harness"].update(sha256=compare.digest("different-harness")),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                report = synthetic_report()
                mutate(report)
                with self.assertRaises(compare.ReportError) as caught:
                    compare.compare(self.baseline, report)
                self.assertEqual(caught.exception.status, "incompatible")

    def test_missing_or_failing_functional_evidence_cannot_hide_behind_good_timings(self):
        mutations = [
            lambda report: report["cases"][0]["report"]["samples"].pop(),
            lambda report: report["cases"][0]["report"]["samples"][1]["delta"].update(compiles=1),
            lambda report: report["cases"][0]["report"]["samples"][1].update(redundant_rewrites=1),
            lambda report: report["cases"][0]["report"]["samples"][1].update(fresh_oracle_equal=False),
            lambda report: report["cases"][0]["report"]["samples"][1].update(changed_paths=["stale-file"]),
            lambda report: report["cases"][0]["report"]["samples"][1].update(artifact_bytes=1),
            lambda report: report["cases"][0]["report"]["gates"].pop("disk_current"),
            lambda report: report["integrity"].update(source_at_build=False),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                report = synthetic_report(scale=0.5)
                mutate(report)
                with self.assertRaises(compare.ReportError):
                    compare.compare(self.baseline, report)

    def test_duplicate_calibration_and_self_comparison_are_not_independent_evidence(self):
        with self.assertRaises(compare.ReportError):
            compare.build_baseline([self.runs[0]] * 5, POLICY)
        with self.assertRaises(compare.ReportError) as caught:
            compare.compare(self.baseline, self.runs[0], gate=True)
        self.assertEqual(caught.exception.status, "ineligible")

    def test_baseline_cannot_mix_candidate_binaries_or_tamper_with_limits(self):
        reports = copy.deepcopy(self.runs)
        reports[1]["provenance"]["binary"]["sha256"] = compare.digest("other-binary")
        with self.assertRaises(compare.ReportError):
            compare.build_baseline(reports, POLICY)
        baseline = copy.deepcopy(self.baseline)
        baseline["limits"]["synthetic/all:warm:refresh_ms"]["limit"] = 1e9
        with self.assertRaises(compare.ReportError):
            compare.compare(baseline, synthetic_report(scale=2), gate=True)

    def test_nonfinite_and_duplicate_json_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            for text in ('{"latency":NaN}', '{"latency":Infinity}', '{"latency":1,"latency":2}'):
                path.write_text(text)
                with self.assertRaises(compare.ReportError):
                    compare.load(path)

    def test_a_dedicated_flag_and_loose_reports_do_not_qualify_a_baseline(self):
        baseline = compare.build_baseline(copy.deepcopy(self.runs), POLICY)
        self.assertEqual(baseline["status"], "observational")
        self.assertTrue(any("collection evidence" in reason for reason in baseline["ineligible_reasons"]))
        reports = copy.deepcopy(self.runs)
        for report in reports:
            report["identity"]["runner"]["declared"].pop("expected_identity_sha256")
        baseline = baseline_from(reports)
        self.assertFalse(baseline["gating_eligible"])
        self.assertTrue(any("not been pinned" in reason for reason in baseline["ineligible_reasons"]))

    def test_exact_quantile_bounds_have_the_claimed_binomial_coverage(self):
        lower, upper, coverage = compare.quantile_ranks(200, .95)
        # Independent direct probability calculation at a numerically safe n.
        mass = lambda k: math.comb(200, k) * .95 ** k * .05 ** (200 - k)
        left = sum(mass(k) for k in range(lower))
        right = sum(mass(k) for k in range(upper, 201))
        self.assertLessEqual(left, .025)
        self.assertLessEqual(right, .025)
        self.assertGreater(sum(mass(k) for k in range(lower + 1)), .025)
        self.assertGreater(sum(mass(k) for k in range(upper - 1, 201)), .025)
        self.assertAlmostEqual(coverage, 1 - left - right)
        self.assertIsNone(compare.quantile_evidence([1, 2, 3], .95)["upper"])

    def test_wide_tail_uncertainty_and_within_run_drift_block_qualification(self):
        for pattern in ("tail", "drift"):
            reports = copy.deepcopy(self.runs)
            for report in reports:
                for sample in report["cases"][0]["report"]["samples"]:
                    cycle = sample["cycle"] - 5
                    total = (200 if cycle % 50 < 47 else 800) if pattern == "tail" else (200 if cycle < 100 else 240)
                    sample["generate"]["ms"] = sample["write"]["ms"] = total / 2
                    sample["refresh_ms"] = total
            baseline = baseline_from(reports)
            self.assertEqual(baseline["status"], "observational")
            self.assertTrue(any(("confidence interval" if pattern == "tail" else "stationarity") in reason for reason in baseline["ineligible_reasons"]))

    def test_confidence_interval_crossing_the_threshold_is_inconclusive_not_passed(self):
        reports = [synthetic_report(f"border-{index}") for index in range(3)]
        for report in reports:
            for sample in report["cases"][0]["report"]["samples"]:
                rank = ((sample["cycle"] - 5) * 73) % 200
                total = 216 if rank < 184 else 220 if rank < 195 else 224
                sample["generate"]["ms"] = sample["write"]["ms"] = total / 2
                sample["refresh_ms"] = total
        result = compare.compare(self.baseline, synthetic_collection(reports=reports), gate=True)
        self.assertEqual(result["status"], "inconclusive")

    def test_overlapped_missing_and_relabelled_collection_processes_are_rejected(self):
        for mutation in (
            lambda c: c["attempts"][1].update(started_at_ns=c["attempts"][0]["started_at_ns"]),
            lambda c: c.update(requested_runs=4),
            lambda c: c["attempts"][0].update(pid=999),
            lambda c: c["attempts"][0].update(report_sha256=compare.digest("wrong report")),
        ):
            candidate = synthetic_collection()
            mutation(candidate)
            with self.assertRaises(compare.ReportError):
                compare.compare(self.baseline, candidate, gate=True)

    def test_swift_module_probe_is_mandatory_and_old_four_target_sets_do_not_qualify(self):
        candidate = synthetic_report()
        candidate["cases"][0]["report"]["module_probe"]["change"]["delta"]["renders"] = 5
        with self.assertRaises(compare.ReportError):
            compare.validate_report(candidate)
        report = synthetic_report()
        report["cases"][0]["report"]["configuration"]["targets"].pop()
        self.assertTrue(any("all-five" in reason for reason in compare.eligibility(report, POLICY)))

    def test_more_samples_than_the_process_could_have_executed_are_rejected(self):
        report = synthetic_report()
        report["cases"][0]["report"]["process"]["finished_at_ns"] = 30
        with self.assertRaises(compare.ReportError):
            compare.validate_report(report)

    def test_cli_exit_status_and_report_cannot_mistake_observation_for_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compare.write_new(root / "baseline.json", self.baseline)
            compare.write_new(root / "candidate.json", synthetic_collection(scale=1.2))
            command = [sys.executable, str(HERE / "compare.py"), "compare", "--baseline", str(root / "baseline.json"),
                       "--candidate", str(root / "candidate.json")]
            observed = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(observed.returncode, 0, observed.stderr)
            self.assertEqual(json.loads(observed.stdout)["status"], "observational")
            gated = subprocess.run([*command, "--gate", "--out", str(root / "comparison.json")], capture_output=True, text=True)
            self.assertEqual(gated.returncode, 1, gated.stderr)
            self.assertEqual(compare.load(root / "comparison.json")["status"], "regressed")
            with self.assertRaises(FileExistsError):
                compare.write_new(root / "baseline.json", {})


if __name__ == "__main__":
    unittest.main()
