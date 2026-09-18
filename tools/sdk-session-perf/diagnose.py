#!/usr/bin/env python3
"""Read-only timing attribution and process/position/correlation diagnostics.

This produces exploratory observations, never a replacement qualification policy.
It accepts complete raw suites or collections and keeps incompatible workloads in
separate cohorts. Original report bytes and qualification outcomes are untouched.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import math
from pathlib import Path
import random
import statistics
import sys

import compare

FORMAT = "suspect-sdk-session-diagnosis-v1"


def summary(values: list[float]) -> dict:
    compare.require(bool(values), "empty diagnostic series")
    for value in values:
        compare.number(value, "diagnostic value")
    return {"samples": len(values), "minimum": min(values), "maximum": max(values),
            "mean": statistics.fmean(values), "median": statistics.median(values), "p95": compare.p95(values)}


def lag_correlation(values: list[float], lag: int) -> float | None:
    """Pearson correlation of observed pairs separated by `lag` sample positions.

    Constant series have undefined correlation, rather than evidence of independence.
    No effective sample size or AR(1) assumption is inferred from this coefficient.
    """
    compare.integer(lag, "lag", 1)
    if len(values) - lag < 2:
        return None
    for value in values:
        compare.number(value, "correlation value")
    scale = max(values)
    if scale == 0:
        return None
    left = [value / scale for value in values[:-lag]]
    right = [value / scale for value in values[lag:]]
    a, b = statistics.fmean(left), statistics.fmean(right)
    xx = math.fsum((value - a) ** 2 for value in left)
    yy = math.fsum((value - b) ** 2 for value in right)
    if xx == 0 or yy == 0:
        return None
    xy = math.fsum((x - a) * (y - b) for x, y in zip(left, right, strict=True))
    return min(1.0, max(-1.0, xy / math.sqrt(xx * yy)))


def block_interval(processes: list[list[float]], block_length: int, replicates: int, seed: int) -> dict:
    """Exploratory cluster/circular-block bootstrap of median process p95.

    Select entire process identities with replacement, then adjacent circular
    blocks within each selected process. This retains short-range ordering and
    between-process variation. It neither proves stationarity nor establishes
    nominal coverage; the result can never promote a baseline.
    """
    compare.integer(block_length, "block length", 1)
    compare.integer(replicates, "bootstrap replicates", 200)
    compare.require(replicates <= 10000, "bootstrap replicates exceed 10000")
    compare.require(bool(processes) and all(len(values) >= block_length for values in processes),
                    "each process needs at least one complete bootstrap block")
    for values in processes:
        summary(values)
    rng = random.Random(seed)
    estimates = []
    for _ in range(replicates):
        quantiles = []
        for _ in processes:
            values = processes[rng.randrange(len(processes))]
            sample = []
            while len(sample) < len(values):
                start = rng.randrange(len(values))
                length = min(block_length, len(values) - len(sample))
                sample.extend(values[(start + index) % len(values)] for index in range(length))
            quantiles.append(compare.p95(sample))
        estimates.append(statistics.median(quantiles))
    estimates.sort()
    return {"method": "cluster-circular-block-percentile-v1", "statistic": "median-process-nearest-rank-p95",
            "block_length": block_length, "replicates": replicates, "seed": seed,
            "nominal_level": 0.95, "coverage_established": False,
            "lower": estimates[math.floor(0.025 * (replicates - 1))],
            "upper": estimates[math.ceil(0.975 * (replicates - 1))]}


def analyze(reports: list[dict], *, metrics: tuple[str, ...] = tuple(compare.METRICS),
            bootstrap_replicates: int = 0, block_length: int = 5, seed: int = 20260910) -> dict:
    compare.require(bool(reports), "diagnosis needs at least one complete report")
    compare.require(bool(metrics) and len(set(metrics)) == len(metrics)
                    and all(name in compare.METRICS for name in metrics), "unknown or duplicate diagnostic metric")
    compare.integer(bootstrap_replicates, "bootstrap replicates")
    compare.require(bootstrap_replicates == 0 or 200 <= bootstrap_replicates <= 10000,
                    "bootstrap replicates must be zero or 200..10000")
    compare.integer(block_length, "block length", 1)
    cohorts = defaultdict(list)
    run_ids = set()
    for report in reports:
        compare.validate_report(report)
        compare.require(report["performance_status"] == "observational", "untimed functional evidence has no timing diagnosis")
        compare.require(report["run_id"] not in run_ids, "duplicate raw suite: do not count the same observations twice")
        run_ids.add(report["run_id"])
        cohorts[compare.digest(compare.compatibility(report))].append(report)
    dimensions = []
    cohort_records = []
    for cohort, runs in sorted(cohorts.items()):
        cohort_records.append({"identity": cohort, "runs": [run["run_id"] for run in runs],
                               "compatibility": compare.compatibility(runs[0])})
        for case_id in sorted(case["id"] for case in runs[0]["cases"]):
            cases = [(run["run_id"], next(case["report"] for case in run["cases"] if case["id"] == case_id))
                     for run in runs]
            for scenario in compare.SCENARIOS:
                process_samples = [(run_id, case, [(index % len(compare.SCENARIOS), sample)
                                   for index, sample in enumerate(case["samples"]) if sample["scenario"] == scenario])
                                   for run_id, case in cases]
                for metric in metrics:
                    positions = defaultdict(list)
                    processes = []
                    series = []
                    for run_id, case, samples in process_samples:
                        values = [compare.metric(sample, metric) for _, sample in samples]
                        series.append(values)
                        for position, sample in samples:
                            positions[position].append(compare.metric(sample, metric))
                        record = {"run_id": run_id, "process": case["process"], "summary": summary(values),
                                  "constant": min(values) == max(values),
                                  "lag_correlations": [{"lag": lag, "pairs": len(values) - lag,
                                                        "pearson": lag_correlation(values, lag)}
                                                       for lag in range(1, min(10, len(values) // 2) + 1)]}
                        if case["configuration"].get("resource_attribution", "disabled") != "disabled":
                            attribution = {}
                            for phase in ("generate", "write"):
                                resources = [sample[phase]["resources"] for _, sample in samples]
                                attribution[phase] = {key: summary([item[key] for item in resources]) for key in
                                                      ("cpu_ms", "user_cpu_ms", "system_cpu_ms", *compare.RESOURCE_COUNTERS)}
                                ratios = [sample[phase]["resources"]["cpu_ms"] / sample[phase]["ms"]
                                          for _, sample in samples if sample[phase]["ms"] > 0]
                                attribution[phase]["cpu_to_wall_ratio"] = summary(ratios) if ratios else None
                            record["resources"] = attribution
                        processes.append(record)
                    quantiles = [process["summary"]["p95"] for process in processes]
                    medians = [statistics.median(values) for values in positions.values()]
                    row = {"cohort": cohort, "case": case_id, "scenario": scenario, "metric": metric,
                           "pooled": summary([value for values in series for value in values]), "processes": processes,
                           "between_process_p95_spread": max(quantiles) - min(quantiles),
                           "position_median_spread": max(medians) - min(medians),
                           "positions": [{"position": position, "summary": summary(values)}
                                         for position, values in sorted(positions.items())]}
                    if bootstrap_replicates:
                        row["block_bootstrap"] = block_interval(series, block_length, bootstrap_replicates, seed)
                    dimensions.append(row)
    return {"format": FORMAT, "status": "observational", "gating_eligible": False,
            "methodology": {"warmups_included": False, "block_length": block_length,
                            "bootstrap_replicates": bootstrap_replicates, "seed": seed,
                            "position_basis": "actual order in each raw cycle; no samples dropped or reweighted"},
            "cohorts": cohort_records, "dimensions": dimensions,
            "limitations": [
                "No qualification threshold, policy or recorded outcome is changed by this analysis.",
                "Process and position strata must be inspected before interpreting pooled quantiles.",
                "Lag correlations describe observed pairs; independence, stationarity and effective sample size are not established.",
                "Optional block-bootstrap intervals are exploratory; nominal coverage is not established and depends on the chosen block length.",
                "CPU counters include all current-process threads and a slightly wider interval than wall timing. CPU/wall may exceed one.",
                "Block operations are not physical IO bytes. Cumulative process peak RSS cannot be attributed as a per-refresh peak.",
                "This evidence alone cannot identify thermal throttling, another process, or the cause of a scheduling delay.",
            ]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True, action="append", help="raw suite or completed collection; repeatable")
    parser.add_argument("--out", type=Path, required=True, help="new diagnostic JSON file; never overwritten")
    parser.add_argument("--metric", choices=tuple(compare.METRICS), action="append")
    parser.add_argument("--block-length", type=int, default=5)
    parser.add_argument("--bootstrap-replicates", type=int, default=0, help="zero disables exploratory bootstrap; otherwise 200..10000")
    parser.add_argument("--seed", type=int, default=20260910)
    args = parser.parse_args()
    try:
        reports, inputs = [], []
        for path in args.input:
            data = path.read_bytes()
            value = compare.load(path)
            inputs.append({"path": str(path.resolve()), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
            if value.get("format") == compare.COLLECTION_FORMAT:
                compare.collection_evidence(value)
                reports.extend(value["reports"])
            else:
                reports.append(value)
        result = analyze(reports, metrics=tuple(args.metric or compare.METRICS), block_length=args.block_length,
                         bootstrap_replicates=args.bootstrap_replicates, seed=args.seed)
        result["inputs"] = inputs
        compare.write_new(args.out, result)
        print(f"{len(result['dimensions'])} observational dimensions in {len(result['cohorts'])} cohorts: {args.out}")
        return 0
    except (compare.ReportError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"Timing diagnosis failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
