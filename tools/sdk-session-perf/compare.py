#!/usr/bin/env python3
"""Versioned SDK Session baselines and fail-closed comparisons (stdlib only)."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from functools import lru_cache
from pathlib import Path
import re
import statistics
import sys
from typing import Any

SUITE_FORMAT = "suspect-sdk-session-suite-v2"
BENCH_FORMAT = "suspect-sdk-session-bench-v2"
BASELINE_FORMAT = "suspect-sdk-session-baseline-v2"
POLICY_FORMAT = "suspect-sdk-session-policy-v2"
COLLECTION_FORMAT = "suspect-sdk-session-collection-v2"
CANONICAL_BACKENDS = ("typescript-http", "rust-http", "python-http", "go-http", "swift-http")
SCENARIOS = (
    "cold", "warm", "schema-edit", "schema-revert", "operation-edit",
    "operation-revert", "docs-edit", "docs-revert",
)
METRICS = {
    "refresh_ms": ("refresh_ms",),
    "generate_ms": ("generate", "ms"),
    "write_ms": ("write", "ms"),
    "allocation_calls": ("allocation_calls",),
    "allocated_bytes": ("allocated_bytes",),
    "artifact_bytes": ("artifact_bytes",),
}
RESOURCE_COUNTERS = (
    "input_block_operations", "output_block_operations", "minor_page_faults", "major_page_faults",
    "voluntary_context_switches", "involuntary_context_switches",
    "process_peak_rss_before_bytes", "process_peak_rss_after_bytes",
)
GATES = {
    "zero_redundant_work", "zero_redundant_rewrites", "fresh_oracle_equal",
    "disk_current", "reverts_reuse_snapshots", "docs_only_executable_stable",
    "configuration_reuses_contract", "original_inputs_unchanged", "private_edits_reverted",
    "module_configuration_uses_target_cache",
}


class ReportError(ValueError):
    def __init__(self, message: str, status: str = "invalid") -> None:
        super().__init__(message)
        self.status = status


def require(condition: bool, message: str, status: str = "invalid") -> None:
    if not condition:
        raise ReportError(message, status)


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=False, allow_nan=False).encode()).hexdigest()


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load(path: Path) -> Any:
    def constant(value: str) -> None:
        raise ReportError(f"non-finite JSON number in {path}: {value}")
    with path.open(encoding="utf-8") as stream:
        return json.load(stream, object_pairs_hook=_object, parse_constant=constant)


def write_new(path: Path, value: Any) -> None:
    # An explicit baseline is never silently overwritten or refreshed by a candidate.
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def number(value: Any, label: str) -> float:
    require(type(value) in (int, float), f"{label}: expected a number")
    require(math.isfinite(value) and value >= 0, f"{label}: expected a finite nonnegative number")
    return value


def integer(value: Any, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label}: expected integer >= {minimum}")
    return value


def sha(value: Any, label: str) -> str:
    require(isinstance(value, str) and re.fullmatch(r"[a-f0-9]{64}", value) is not None,
            f"{label}: missing SHA-256")
    return value


def fingerprints(values: Any, label: str) -> dict[str, str]:
    require(isinstance(values, list) and len(values) > 0, f"{label}: empty fingerprint manifest")
    paths: dict[str, str] = {}
    for item in values:
        path = item["path"]
        require(isinstance(path, str) and path and path not in paths, f"{label}: duplicate/empty path")
        integer(item["bytes"], f"{label}/{path}/bytes")
        paths[path] = sha(item["sha256"], f"{label}/{path}")
    return paths


def metric(sample: dict[str, Any], name: str) -> float:
    value: Any = sample
    for part in METRICS[name]:
        value = value[part]
    return number(value, name)


def p95(values: list[float]) -> float:
    """Nearest-rank p95. Small samples remain observations, never gate evidence."""
    require(bool(values), "p95 has no samples")
    return sorted(values)[math.ceil(0.95 * len(values)) - 1]


def validate_case(case: dict[str, Any]) -> None:
    label, report = case["id"], case["report"]
    require(isinstance(label, str) and re.fullmatch(r"[a-zA-Z0-9._/-]+", label) is not None, "invalid case ID")
    require(report["format"] == BENCH_FORMAT, f"{label}: unsupported benchmark format")
    require(report["functional_status"] == "passed", f"{label}: functional gates did not pass")
    require(report["performance_status"] in ("observational", "not-measured"), f"{label}: raw timings cannot claim qualification")
    require(GATES <= report["gates"].keys() and all(value is True for value in report["gates"].values()),
            f"{label}: a mandatory functional gate is missing or failed")
    require(report["build_profile"] in ("release", "debug"), f"{label}: unknown build profile")
    sha(report["binary"]["sha256"], f"{label}/binary")
    sha(report["configuration_sha256"], f"{label}/configuration")
    fingerprints(report["inputs"], f"{label}/inputs")
    fingerprints(report["prepared_inputs"], f"{label}/prepared_inputs")
    sha(report["input_sha256"], f"{label}/input_sha256")
    sha(report["prepared_input_sha256"], f"{label}/prepared_input_sha256")
    config = report["configuration"]
    count = integer(config["iterations"], f"{label}/iterations", 1)
    warmups = integer(config["warmups"], f"{label}/warmups")
    functional = config["mode"] == "functional-only"
    require(config["mode"] in ("functional-only", "observational"), f"{label}: unknown mode")
    require(report["performance_status"] == ("not-measured" if functional else "observational"), f"{label}: mode/status differ")
    if functional:
        require(count == 1 and warmups == 0, f"{label}: functional-only must be one cycle")
    process = report["process"]
    integer(process["pid"], f"{label}/pid", 1)
    require(integer(process["finished_at_ns"], f"{label}/finished_at_ns", 1) > integer(process["started_at_ns"], f"{label}/started_at_ns", 1), f"{label}: invalid process interval")
    integer(config["cache_entries"], f"{label}/cache_entries", 2)
    integer(config["cache_bytes"], f"{label}/cache_bytes", 1)
    require(config["owner"] and config["preparation"] == "private-canonical-json-v1"
            and config["schedule"] in ("rotating-edit-revert-cycles-v1", "fixed-edit-revert-cycles-v1")
            and config["measurement"] == "system-allocator-generate-plus-owned-write-v1",
            f"{label}: unsupported workload/measurement identity")
    attribution = config.get("resource_attribution", "disabled")
    require(attribution in ("disabled", "posix-process-rusage-outside-interval-v1"),
            f"{label}: unknown resource attribution")
    require(not functional or attribution == "disabled", f"{label}: functional-only cannot attribute timing")
    targets = config["targets"]
    require(isinstance(targets, list) and len(targets) > 0, f"{label}: missing targets")
    require(len({item["backend"] for item in targets}) == len(targets), f"{label}: duplicate target")
    oracle_list = report["oracles"]
    oracles = {item["scenario"]: item["files"] for item in oracle_list}
    require(len(oracles) == len(oracle_list) and set(oracles) == {"baseline", "schema-edit", "operation-edit", "docs-edit"},
            f"{label}: missing/duplicate fresh oracles")
    oracle_hashes = {name: fingerprints(files, f"{label}/{name}") for name, files in oracles.items()}
    for group, start, cycles in (("samples", warmups, count), ("warmup_samples", 0, warmups)):
        samples = report[group]
        require(len(samples) == cycles * len(SCENARIOS), f"{label}/{group}: missing or extra samples")
        seen: set[tuple[int, str]] = set()
        last_cycle = None
        previous: dict[str, str] = {}
        cumulative = {"compiles": 0, "renders": 0, "cache_hits": 0}
        previous_elapsed = None
        position = 0
        for sample in samples:
            cycle = integer(sample["cycle"], f"{label}/cycle")
            scenario = sample["scenario"]
            require(start <= cycle < start + cycles and scenario in SCENARIOS, f"{label}: invalid cycle/scenario")
            require((cycle, scenario) not in seen, f"{label}: duplicated sample")
            seen.add((cycle, scenario))
            if cycle != last_cycle:
                require(scenario == "cold" and (last_cycle is None or cycle == last_cycle + 1),
                        f"{label}: cycles must start cold, in order")
                previous = {}
                cumulative = {"compiles": 0, "renders": 0, "cache_hits": 0}
                last_cycle = cycle
                position = 0
            if config["schedule"] == "fixed-edit-revert-cycles-v1" or "position_in_cycle" in sample:
                edits = ["schema", "operation", "docs"]
                offset = cycle % 3 if config["schedule"] == "rotating-edit-revert-cycles-v1" else 0
                edits = edits[offset:] + edits[:offset]
                schedule = ["cold", "warm", *[f"{edit}-{action}" for edit in edits for action in ("edit", "revert")]]
                require(scenario == schedule[position], f"{label}: recorded scenario order differs from schedule")
            if "position_in_cycle" in sample:
                require(integer(sample["position_in_cycle"], "position_in_cycle") == position
                        and integer(sample["ordinal"], "ordinal") == cycle * len(SCENARIOS) + position,
                        f"{label}: sample position/ordinal differs from execution order")
                elapsed = number(sample["observed_at_elapsed_ms"], "observed_at_elapsed_ms")
                require(previous_elapsed is None or elapsed >= previous_elapsed,
                        f"{label}: elapsed observation time moved backwards")
                previous_elapsed = elapsed
            position += 1
            for name in METRICS:
                metric(sample, name)
                if functional and name != "artifact_bytes":
                    require(metric(sample, name) == 0, f"{label}: functional-only contains performance observations")
            require(math.isclose(sample["refresh_ms"], sample["generate"]["ms"] + sample["write"]["ms"],
                                 rel_tol=1e-12, abs_tol=1e-9), f"{label}: refresh phase sum differs")
            for name in ("allocation_calls", "allocated_bytes"):
                value = integer(sample[name], f"{label}/{name}")
                total = sum(integer(sample[phase][name], f"{label}/{phase}/{name}") for phase in ("generate", "write"))
                require(value == total, f"{label}: allocation phase sum differs")
            for phase in ("generate", "write"):
                resources = sample[phase].get("resources")
                if attribution == "disabled":
                    require(resources is None, f"{label}: undeclared resource observations")
                    continue
                require(isinstance(resources, dict), f"{label}: missing resource attribution")
                cpu = number(resources["cpu_ms"], "cpu_ms")
                parts = sum(number(resources[key], key) for key in ("user_cpu_ms", "system_cpu_ms"))
                require(math.isclose(cpu, parts, rel_tol=1e-12, abs_tol=1e-9), f"{label}: CPU phase sum differs")
                for key in RESOURCE_COUNTERS:
                    integer(resources[key], key)
                require(resources["process_peak_rss_after_bytes"] >= resources["process_peak_rss_before_bytes"],
                        f"{label}: cumulative process peak RSS moved backwards")
            miss = scenario == "cold" or scenario.endswith("-edit")
            delta = {"compiles": int(miss), "renders": len(targets) if miss else 0, "cache_hits": int(not miss)}
            require(sample["delta"] == delta, f"{label}/{scenario}: redundant/missing compile, render or cache reuse")
            for key, value in delta.items():
                cumulative[key] += value
            require(sample["stats"] == cumulative, f"{label}/{scenario}: inconsistent cumulative counters")
            require(sample["new_documents"] == (len(report["prepared_inputs"]) if scenario == "cold" else 0),
                    f"{label}/{scenario}: inconsistent closure membership")
            require(sample["fresh_oracle_equal"] is True and sample["disk_current"] is True
                    and sample["redundant_rewrites"] == 0, f"{label}/{scenario}: stale output or redundant rewrites")
            oracle = scenario if scenario.endswith("-edit") else "baseline"
            hashes = oracle_hashes[oracle]
            require(sample["artifact_files"] == len(hashes)
                    and sample["artifact_bytes"] == sum(item["bytes"] for item in oracles[oracle])
                    and sample["artifact_sha256"] == digest(hashes), f"{label}/{scenario}: artifact oracle differs")
            expected_changed = sorted(path for path in previous.keys() | hashes.keys() if previous.get(path) != hashes.get(path))
            require(sample["changed_paths"] == expected_changed, f"{label}/{scenario}: incomplete changed_paths")
            require(sample["removed_files"] == len(previous.keys() - hashes.keys()), f"{label}/{scenario}: stale-file count differs")
            previous = hashes
        require(len(seen) == cycles * len(SCENARIOS), f"{label}: incomplete cycles")
    probe = report["configuration_probe"]
    for sample in (probe["change"], probe["revert"]):
        require(sample["fresh_oracle_equal"] is True and sample["disk_current"] is True
                and sample["redundant_rewrites"] == 0, f"{label}: configuration probe failed")
    require(probe["change"]["delta"] == {"compiles": 0, "renders": int(len(targets) == 1), "cache_hits": max(0, len(targets) - 1)},
            f"{label}: configuration must reuse the Contract and unchanged target artifacts")
    require(probe["revert"]["delta"] == {"compiles": 0, "renders": 0, "cache_hits": 1}, f"{label}: configuration revert missed cache")
    if len(targets) > 1:
        integer(probe["change"]["removed_files"], f"{label}: obsolete target files removed", 1)
    swift = [target for target in targets if target["backend"] == "swift-http"]
    if swift:
        require(swift[0]["package_name"] == "BenchmarkSDK" and swift[0]["import_name"] is None
                and swift[0]["package_version"] == "0.0.0", f"{label}: unexpected default Swift package/module")
        probe = report["module_probe"]
        require(probe["package"] == probe["default_module"] == "BenchmarkSDK"
                and probe["explicit_module"] == "BenchmarkModule", f"{label}: missing Swift module identities")
        require(probe["change"]["delta"] == {"compiles": 0, "renders": 1, "cache_hits": len(targets) - 1},
                f"{label}: Swift module rename must reuse other target caches")
        require(probe["revert"]["delta"] == {"compiles": 0, "renders": 0, "cache_hits": 1}, f"{label}: Swift module revert missed cache")
        integer(probe["change"]["removed_files"], f"{label}: obsolete Swift module files removed", 1)
        for sample in (probe["change"], probe["revert"]):
            require(sample["fresh_oracle_equal"] is True and sample["disk_current"] is True
                    and sample["redundant_rewrites"] == 0, f"{label}: Swift module probe failed")
    else:
        require(report["module_probe"] is None, f"{label}: unselected Swift probe")
    measured = sum(sample["refresh_ms"] for sample in report["samples"] + report["warmup_samples"])
    require(measured <= (process["finished_at_ns"] - process["started_at_ns"]) / 1_000_000 + 1,
            f"{label}: claimed samples exceed the native process execution interval")


def validate_report(report: dict[str, Any]) -> None:
    require(report["format"] == SUITE_FORMAT, "unsupported suite format")
    require(isinstance(report["run_id"], str) and report["run_id"], "missing independent run ID")
    require(report["functional_status"] == "passed" and report["performance_status"] in ("observational", "not-measured"), "suite did not complete functional checks")
    require(set(report["integrity"]) == {"source_at_build", "binary", "tools", "inputs"}
            and all(value is True for value in report["integrity"].values()), "source/binary/tool/input integrity not established")
    identity = report["identity"]
    runner = identity["runner"]
    require(runner["declared"]["format"] == "suspect-sdk-session-runner-v2", "unknown runner declaration")
    require(runner["declared"]["kind"] in ("observational", "dedicated") and runner["declared"]["id"]
            and runner["declared"]["image"] and runner["actual"]["machine"] and runner["actual"]["cpu"], "incomplete runner identity")
    sha(identity["harness"]["sha256"], "harness fingerprint")
    fingerprints(identity["harness"]["files"], "harness")
    require(identity["tools"] and identity["build"] and identity["work_root"], "missing tool/build/path identity")
    for name, tool in identity["tools"].items():
        require(tool["version"], f"{name}: missing tool version")
        sha(tool["sha256"], f"{name}: tool binary")
    provenance = report["provenance"]
    sha(provenance["source"]["sha256"], "generator source")
    fingerprints(provenance["source"]["files"], "generator source")
    sha(provenance["binary"]["sha256"], "generator binary")
    require(len(report["cases"]) > 0, "suite has no cases")
    start = integer(report["started_at_ns"], "suite started_at_ns", 1)
    end = integer(report["finished_at_ns"], "suite finished_at_ns", 1)
    integer(report["process_id"], "suite process_id", 1)
    require(end > start, "invalid suite interval")
    ids: set[str] = set()
    for case in report["cases"]:
        require(case["id"] not in ids, "duplicate benchmark case")
        ids.add(case["id"])
        validate_case(case)
        require(case["report"]["binary"]["sha256"] == provenance["binary"]["sha256"], "case ran a different executable")
        process = case["report"]["process"]
        require(start <= process["started_at_ns"] < process["finished_at_ns"] <= end, "case process is outside the suite's execution interval")
        require(case["report"]["performance_status"] == report["performance_status"], "suite/case observation mode differs")


def compatibility(report: dict[str, Any]) -> dict[str, Any]:
    # Source and generator binary hashes deliberately are provenance, not equality
    # constraints: the candidate is allowed to contain an optimization/regression.
    return {
        "identity": report["identity"],
        "cases": [{"id": case["id"], **{key: case["report"][key] for key in (
            "fixture", "configuration", "configuration_sha256", "build_profile", "inputs",
            "input_sha256", "prepared_inputs", "prepared_input_sha256", "private_root",
        )}} for case in sorted(report["cases"], key=lambda case: case["id"])],
    }


def series(report: dict[str, Any]) -> dict[str, list[float]]:
    values: dict[str, list[float]] = {}
    for case in report["cases"]:
        for scenario in SCENARIOS:
            samples = [sample for sample in case["report"]["samples"] if sample["scenario"] == scenario]
            for name in METRICS:
                values[f"{case['id']}:{scenario}:{name}"] = [metric(sample, name) for sample in samples]
    return values


@lru_cache(maxsize=64)
def quantile_ranks(count: int, confidence: float) -> tuple[int, int, float]:
    """Exact binomial order-statistic bounds for q=.95, with infinite endpoints.

    For B~Binomial(n,.95), [X_(L),X_(U)] covers q when L <= B < U.
    L=0/U=n+1 are unbounded. Log probabilities avoid underflow at large n.
    """
    tail = (1 - confidence) / 2
    masses = [math.exp(math.lgamma(count + 1) - math.lgamma(k + 1) - math.lgamma(count - k + 1)
                       + k * math.log(.95) + (count - k) * math.log(.05)) for k in range(count + 1)]
    total = math.fsum(masses)
    cdf = [0.0]
    for mass in masses:
        cdf.append(cdf[-1] + mass / total)
    lower = max(k for k in range(count + 1) if cdf[k] <= tail)
    upper = min(k for k in range(1, count + 2) if 1 - cdf[k] <= tail)
    return lower, upper, min(1.0, cdf[upper] - cdf[lower])


def quantile_evidence(values: list[float], confidence: float) -> dict[str, Any]:
    ordered = sorted(values)
    count = len(values)
    lower, upper, coverage = quantile_ranks(count, confidence)
    estimate = p95(values)
    low = ordered[lower - 1] if lower > 0 else None
    high = ordered[upper - 1] if upper <= count else None
    half_width = max(estimate - low, high - estimate) if low is not None and high is not None else None
    middle = count // 2
    drift = abs(statistics.median(values[:middle]) - statistics.median(values[middle:])) if middle else None
    return {"samples": count, "p95": estimate, "lower": low, "upper": high,
            "lower_rank": lower, "upper_rank": upper, "coverage": coverage,
            "half_width": half_width, "early_late_median_delta": drift}


def validate_policy(policy: dict[str, Any]) -> None:
    require(policy["format"] == POLICY_FORMAT and policy["statistic"] == "nearest-rank-p95", "unsupported baseline policy")
    require(policy["policy_status"] in ("candidate", "approved"), "policy must state candidate/approved")
    integer(policy["minimum_runs"], "minimum_runs", 5)
    integer(policy["minimum_candidate_runs"], "minimum_candidate_runs", 3)
    integer(policy["minimum_samples"], "minimum_samples", 200)
    integer(policy["minimum_warmups"], "minimum_warmups", 5)
    require(policy["confidence_method"] == "binomial-order-statistic-v1"
            and .95 <= number(policy["confidence_level"], "confidence_level") < 1, "at least 95% quantile confidence required")
    require(policy["required_fixtures"] and policy["required_groups"], "policy must name its workload")
    require(tuple(policy["canonical_backends"]) == CANONICAL_BACKENDS, "policy must cover all five canonical backends")
    require(number(policy["noise_multiplier"], "noise_multiplier") >= 1, "noise multiplier must be >= 1")
    require(set(policy["metrics"]) == set(METRICS), "policy must explicitly cover every measured metric")
    for name, tolerances in policy["metrics"].items():
        require(set(tolerances) == {"relative", "absolute", "maximum_noise_relative", "maximum_confidence_relative", "maximum_drift_relative"}, f"{name}: incomplete tolerances")
        for field, value in tolerances.items():
            number(value, f"{name}/{field}")


def eligibility(report: dict[str, Any], policy: dict[str, Any]) -> list[str]:
    reasons: list[str] = []
    if report["identity"]["runner"]["declared"]["kind"] != "dedicated":
        reasons.append("runner is observational, not a declared dedicated runner")
    if report["identity"]["build"]["profile"] != "release":
        reasons.append("suite must have been built with Cargo's release profile")
    runner = report["identity"]["runner"]
    if runner["declared"].get("expected_identity_sha256") != digest(runner["actual"]):
        reasons.append("observed runner identity has not been pinned in its declaration")
    if "expected_tools_sha256" in runner["declared"] and runner["declared"]["expected_tools_sha256"] != digest(report["identity"]["tools"]):
        reasons.append("observed tools differ from the runner's explicit tool pin")
    if not runner["actual"].get("machine_id_sha256"):
        reasons.append("stable machine identity evidence is unavailable")
    if report["performance_status"] != "observational":
        reasons.append("functional-only execution contains no performance evidence")
    actual_cases = {case["id"] for case in report["cases"]}
    required_cases = {f"{fixture}/{group}" for fixture in policy["required_fixtures"] for group in policy["required_groups"]}
    if not required_cases <= actual_cases:
        reasons.append("required fixture/target groups are missing: " + ", ".join(sorted(required_cases - actual_cases)))
    for case in report["cases"]:
        config = case["report"]["configuration"]
        if case["id"].endswith("/all") and {target["backend"] for target in config["targets"]} != set(CANONICAL_BACKENDS):
            reasons.append(f"{case['id']}: all-five backend coverage required")
        if case["report"]["build_profile"] != "release":
            reasons.append(f"{case['id']}: release build required")
        if config["iterations"] < policy["minimum_samples"]:
            reasons.append(f"{case['id']}: fewer than {policy['minimum_samples']} samples/scenario")
        if config["warmups"] < policy["minimum_warmups"]:
            reasons.append(f"{case['id']}: insufficient warmup cycles")
    return reasons


def collection_evidence(collection: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in collection.items() if key != "reports"}


def validate_collection(reports: list[dict[str, Any]], evidence: dict[str, Any], role: str) -> None:
    require(evidence["format"] == COLLECTION_FORMAT and evidence["role"] == role
            and evidence["protocol"] == "sequential-fresh-suite-process-v1", "unsupported collection protocol/role")
    require(evidence["complete"] is True and evidence["campaign_id"], "collection did not complete every requested run")
    require(integer(evidence["requested_runs"], "requested_runs", 1) == len(reports) == len(evidence["attempts"]),
            "collection is missing requested runs")
    previous_end = integer(evidence["started_at_ns"], "collection start", 1)
    finish = integer(evidence["finished_at_ns"], "collection finish", 1)
    for index, (report, attempt) in enumerate(zip(reports, evidence["attempts"])):
        require(attempt["index"] == index and attempt["exit_code"] == 0
                and attempt["run_id"] == report["run_id"] and attempt["report_sha256"] == digest(report),
                "collection attempt does not match its raw report")
        require(integer(attempt["pid"], "suite subprocess PID", 1) == report["process_id"], "report came from a different suite process")
        require(previous_end <= attempt["started_at_ns"] <= report["started_at_ns"]
                < report["finished_at_ns"] <= attempt["finished_at_ns"] <= finish,
                "suite processes overlapped, were reused, or fall outside the collection interval")
        previous_end = attempt["finished_at_ns"]
        for case in report["cases"]:
            config = case["report"]["configuration"]
            require(config["iterations"] == evidence["requested_iterations"]
                    and config["warmups"] == evidence["requested_warmups"], "collection sample protocol differs from actual samples")


def assess(reports: list[dict[str, Any]], policy: dict[str, Any], evidence: dict[str, Any] | None,
           role: str) -> tuple[list[str], dict[str, Any]]:
    validate_policy(policy)
    require(bool(reports), "collection has no reports")
    for report in reports:
        validate_report(report)
        require(report["performance_status"] == "observational", "functional-only reports contain no performance evidence", "ineligible")
    require(len({report["run_id"] for report in reports}) == len(reports), "reports must be independent runs")
    processes = [(case["report"]["process"]["pid"], case["report"]["process"]["started_at_ns"])
                 for report in reports for case in report["cases"]]
    require(len(processes) == len(set(processes)), "native process observations were reused across runs")
    expected = compatibility(reports[0])
    source = reports[0]["provenance"]["source"]["sha256"]
    binary = reports[0]["provenance"]["binary"]["sha256"]
    for report in reports:
        require(compatibility(report) == expected, "collection runner/tool/harness/build/config/input identity mismatch", "incompatible")
        require(report["provenance"]["source"]["sha256"] == source and report["provenance"]["binary"]["sha256"] == binary,
                "a collection must repeat the same generator source AND binary", "incompatible")
    reasons = eligibility(reports[0], policy)
    if evidence is None:
        reasons.append("sequential fresh-process collection evidence is missing")
    else:
        validate_collection(reports, evidence, role)
    minimum = policy["minimum_runs"] if role == "baseline" else policy["minimum_candidate_runs"]
    if len(reports) < minimum:
        reasons.append(f"fewer than {minimum} independent {role} runs")
    raw = [series(report) for report in reports]
    summaries = {}
    for key in sorted(raw[0]):
        runs = [quantile_evidence(item[key], policy["confidence_level"]) for item in raw]
        reference = statistics.median(run["p95"] for run in runs)
        noise = max(abs(run["p95"] - reference) for run in runs)
        tolerance = policy["metrics"][key.rsplit(":", 1)[1]]
        if noise > max(tolerance["absolute"], reference * tolerance["maximum_noise_relative"]):
            reasons.append(f"{key}: excessive between-process p95 noise")
        for index, run in enumerate(runs):
            if run["half_width"] is None or run["half_width"] > max(tolerance["absolute"], run["p95"] * tolerance["maximum_confidence_relative"]):
                reasons.append(f"{key}/run-{index}: p95 confidence interval is unbounded or too wide")
            if run["early_late_median_delta"] is None or run["early_late_median_delta"] > max(tolerance["absolute"], run["p95"] * tolerance["maximum_drift_relative"]):
                reasons.append(f"{key}/run-{index}: early/late stationarity screen failed")
        summaries[key] = {"reference_p95": reference, "run_p95s": [run["p95"] for run in runs],
                          "noise_absolute": noise, "runs": runs}
    return reasons, summaries


def build_baseline(reports: list[dict[str, Any]], policy: dict[str, Any],
                   evidence: dict[str, Any] | None = None) -> dict[str, Any]:
    reasons, summaries = assess(reports, policy, evidence, "baseline")
    expected = compatibility(reports[0])
    source = reports[0]["provenance"]["source"]["sha256"]
    binary = reports[0]["provenance"]["binary"]["sha256"]
    limits: dict[str, Any] = {}
    for key, summary in summaries.items():
        reference = summary["reference_p95"]
        tolerance = policy["metrics"][key.rsplit(":", 1)[1]]
        allowance = reference * tolerance["relative"] + max(tolerance["absolute"], summary["noise_absolute"] * policy["noise_multiplier"])
        limits[key] = {**summary, "allowed_delta": allowance, "limit": reference + allowance}
    return {
        "format": BASELINE_FORMAT,
        "status": "qualified" if not reasons else "observational",
        "gating_eligible": not reasons, "ineligible_reasons": reasons,
        "policy": policy, "policy_sha256": digest(policy), "compatibility": expected,
        "generator_source_sha256": source, "generator_binary_sha256": binary,
        "limits": limits,
        # Self-contained baseline artifacts retain every raw calibration sample.
        "calibration_runs": reports,
        "collection_evidence": evidence,
    }


def compare(baseline: dict[str, Any], candidate: dict[str, Any], gate: bool = False) -> dict[str, Any]:
    require(baseline["format"] == BASELINE_FORMAT, "unsupported baseline format")
    rebuilt = build_baseline(baseline["calibration_runs"], baseline["policy"], baseline["collection_evidence"])
    require(rebuilt == baseline, "baseline does not match its raw calibration reports/policy")
    if candidate["format"] == COLLECTION_FORMAT:
        reports = candidate["reports"]
        evidence = collection_evidence(candidate)
    else:
        reports, evidence = [candidate], None
    reasons, summaries = assess(reports, baseline["policy"], evidence, "candidate")
    require(compatibility(reports[0]) == baseline["compatibility"],
            "candidate runner/tool/harness/build/config/input identity is incompatible with the baseline", "incompatible")
    if gate:
        require(baseline["gating_eligible"], "; ".join(baseline["ineligible_reasons"]), "ineligible")
        require(not reasons, "; ".join(reasons), "ineligible")
        require(not {run["run_id"] for run in reports} & {run["run_id"] for run in baseline["calibration_runs"]},
                "a calibration run cannot also be its own gated candidate", "ineligible")
        executions = lambda runs: {(case["report"]["process"]["pid"], case["report"]["process"]["started_at_ns"])
                                   for run in runs for case in run["cases"]}
        require(not executions(reports) & executions(baseline["calibration_runs"]), "candidate reused a calibration native process", "ineligible")
    checks = []
    for key, limit in baseline["limits"].items():
        summary = summaries[key]
        observed = summary["reference_p95"]
        bounded = all(run["lower"] is not None and run["upper"] is not None for run in summary["runs"])
        lower = min(run["lower"] for run in summary["runs"]) if bounded else None
        upper = max(run["upper"] for run in summary["runs"]) if bounded else None
        decision = "within-tolerance" if bounded and upper <= limit["limit"] else "regression" if bounded and lower > limit["limit"] else "inconclusive"
        checks.append({"key": key, **limit, "candidate_p95": observed,
                       "candidate_runs": summary["runs"], "candidate_lower": lower, "candidate_upper": upper,
                       "delta": observed - limit["reference_p95"], "point_regression": observed > limit["limit"],
                       "decision": decision})
    regressions = [check for check in checks if check["decision"] == "regression"]
    inconclusive = any(check["decision"] == "inconclusive" for check in checks)
    decision = "regressed" if regressions else "inconclusive" if inconclusive else "passed"
    return {
        "format": "suspect-sdk-session-comparison-v2",
        "status": decision if gate else "observational",
        "gated": gate, "policy_status": baseline["policy"]["policy_status"],
        "functional_status": "passed", "would_regress": any(check["point_regression"] for check in checks),
        "confidence_decision": decision, "candidate_ineligible_reasons": reasons,
        "baseline_sha256": digest(baseline), "candidate_sha256": digest(candidate),
        "candidate_source_sha256": reports[0]["provenance"]["source"]["sha256"],
        "candidate_binary_sha256": reports[0]["provenance"]["binary"]["sha256"],
        "checks": checks,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    baseline = commands.add_parser("baseline", help="create a new explicit baseline with raw calibration samples")
    inputs = baseline.add_mutually_exclusive_group(required=True)
    inputs.add_argument("--report", type=Path, action="append", help="loose reports remain observational without collection evidence")
    inputs.add_argument("--collection", type=Path, help="fresh-process collection from calibrate.py")
    baseline.add_argument("--policy", type=Path, required=True)
    baseline.add_argument("--out", type=Path, required=True)
    comparison = commands.add_parser("compare", help="compare compatible raw samples against an explicit baseline")
    comparison.add_argument("--baseline", type=Path, required=True)
    comparison.add_argument("--candidate", type=Path, required=True)
    comparison.add_argument("--gate", action="store_true", help="require calibrated dedicated-runner evidence and fail regressions")
    comparison.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "baseline":
            if args.collection:
                collection = load(args.collection)
                result = build_baseline(collection["reports"], load(args.policy), collection_evidence(collection))
            else:
                result = build_baseline([load(path) for path in args.report], load(args.policy))
        else:
            result = compare(load(args.baseline), load(args.candidate), args.gate)
        if args.out:
            write_new(args.out, result)
        print(json.dumps({key: value for key, value in result.items() if key in (
            "format", "status", "gated", "gating_eligible", "ineligible_reasons", "functional_status", "would_regress",
        )}, indent=2))
        return 1 if result["status"] == "regressed" else 2 if result["status"] == "inconclusive" else 0
    except (ReportError, KeyError, TypeError, OSError, json.JSONDecodeError, OverflowError) as error:
        result = {"format": "suspect-sdk-session-comparison-v2", "status": getattr(error, "status", "invalid"),
                  "gated": False, "error": str(error)}
        if args.out and not args.out.exists():
            write_new(args.out, result)
        print(json.dumps(result, indent=2), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
