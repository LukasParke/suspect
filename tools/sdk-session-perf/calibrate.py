#!/usr/bin/env python3
"""Collect independent Session runs and evaluate evidence-derived qualification."""

from __future__ import annotations

import argparse
import fcntl
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

import compare
import run


def collect(args: argparse.Namespace) -> tuple[dict, dict]:
    policy = compare.load(args.policy)
    compare.validate_policy(policy)
    count = args.repeats if args.repeats is not None else policy["minimum_runs" if args.role == "baseline" else "minimum_candidate_runs"]
    iterations = args.iterations if args.iterations is not None else policy["minimum_samples"]
    warmups = args.warmups if args.warmups is not None else policy["minimum_warmups"]
    compare.require(1 <= count <= 100 and 1 <= iterations <= 10000 and 0 <= warmups <= 100, "invalid repeat/sample protocol")
    compare.require(not args.gate or args.role == "candidate" and args.baseline is not None, "--gate requires candidate role and an explicit baseline")
    out = run.below_target(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.mkdir()
    compare.write_new(out / "policy.json", policy)
    work = run.below_target(args.work_root)
    work.parent.mkdir(parents=True, exist_ok=True)
    # Serializes collectors using this benchmark workspace. It is not evidence
    # that unrelated processes are absent; host reservation remains explicit.
    with work.with_name(f".{work.name}.collection.lock").open("a+") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise compare.ReportError("another collection owns this benchmark workspace") from error
        collection = {
            "format": compare.COLLECTION_FORMAT, "protocol": "sequential-fresh-suite-process-v1",
            "role": args.role, "campaign_id": uuid.uuid4().hex,
            "started_at_ns": time.time_ns(), "finished_at_ns": None, "complete": False,
            "requested_runs": count, "requested_iterations": iterations, "requested_warmups": warmups,
            "attempts": [], "reports": [],
        }
        try:
            for index in range(count):
                destination = out / f"run-{index + 1:03d}"
                command = [sys.executable, str(run.HERE / "run.py"), "--out", str(destination),
                           "--work-root", str(work), "--build-dir", str(args.build_dir),
                           "--suite", args.suite, "--profile", "release",
                            "--iterations", str(iterations), "--warmups", str(warmups)]
                command += ["--schedule", getattr(args, "schedule", "rotating")]
                if getattr(args, "attribution", False):
                    command.append("--attribution")
                if args.runner:
                    command += ["--runner", str(args.runner)]
                if args.openrouter_root:
                    command += ["--openrouter-root", str(args.openrouter_root)]
                if args.offline:
                    command += ["--offline"]
                for group in args.group or []:
                    command += ["--group", group]
                started = time.time_ns()
                with (out / f"run-{index + 1:03d}.stdout.log").open("wb") as stdout, (out / f"run-{index + 1:03d}.stderr.log").open("wb") as stderr:
                    process = subprocess.Popen(command, cwd=run.ROOT, stdout=stdout, stderr=stderr)
                    code = process.wait()
                attempt = {"index": index, "command": command, "pid": process.pid,
                           "started_at_ns": started, "finished_at_ns": time.time_ns(), "exit_code": code}
                collection["attempts"].append(attempt)
                compare.require(code == 0, f"run {index + 1} failed; every attempt is retained and no baseline is qualified")
                report = compare.load(destination / "report.json")
                compare.validate_report(report)
                attempt.update(run_id=report["run_id"], report_sha256=compare.digest(report))
                collection["reports"].append(report)
                print(f"Collected independent {args.role} suite {index + 1}/{count}", flush=True)
            collection["complete"] = True
        finally:
            collection["finished_at_ns"] = time.time_ns()
            compare.write_new(out / "collection.json", collection)
        evidence = compare.collection_evidence(collection)
        if args.role == "baseline":
            result = compare.build_baseline(collection["reports"], policy, evidence)
            compare.write_new(out / "baseline.json", result)
        elif args.baseline:
            baseline = compare.load(args.baseline)
            compare.require(baseline["policy"] == policy, "candidate collection policy differs from baseline policy", "incompatible")
            result = compare.compare(baseline, collection, args.gate)
            compare.write_new(out / "comparison.json", result)
        else:
            reasons, summaries = compare.assess(collection["reports"], policy, evidence, "candidate")
            result = {"status": "qualified" if not reasons else "observational", "gating_eligible": not reasons,
                      "ineligible_reasons": reasons, "summaries": summaries}
            compare.write_new(out / "qualification.json", result)
        if args.require_qualified:
            compare.require(result.get("gating_eligible") is True, "; ".join(result.get("ineligible_reasons", [])), "ineligible")
        return collection, result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--role", choices=("baseline", "candidate"), default="baseline")
    parser.add_argument("--policy", type=Path, default=run.HERE / "policy-v2.json")
    parser.add_argument("--baseline", type=Path, help="explicit baseline artifact for candidate comparison")
    parser.add_argument("--gate", action="store_true")
    parser.add_argument("--require-qualified", action="store_true", help="fail after retaining evidence if baseline/candidate collection is only observational")
    parser.add_argument("--suite", choices=("compact", "public"), default="public")
    parser.add_argument("--openrouter-root", type=Path)
    parser.add_argument("--group", choices=(*run.BACKENDS, "all"), action="append")
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--work-root", type=Path, default=run.ROOT / "target/sdk-session-perf-work")
    parser.add_argument("--build-dir", type=Path, default=run.ROOT / "target/sdk-session-perf-build")
    parser.add_argument("--repeats", type=int, help="default: five baseline or three candidate suites")
    parser.add_argument("--iterations", type=int, help="default: policy minimum, 200 samples/scenario")
    parser.add_argument("--warmups", type=int, help="default: policy minimum, five complete cycles")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--attribution", action="store_true", help="record CPU/IO/fault/scheduler counters; changes measurement identity")
    parser.add_argument("--schedule", choices=("rotating", "fixed"), default="rotating", help="prospective workload order, pinned for every suite")
    args = parser.parse_args()
    try:
        _, result = collect(args)
        print(json.dumps({key: result[key] for key in ("status", "gating_eligible", "ineligible_reasons", "confidence_decision") if key in result}, indent=2))
        return 1 if result["status"] == "regressed" else 2 if result["status"] == "inconclusive" else 0
    except (compare.ReportError, KeyError, TypeError, OSError, ValueError, subprocess.SubprocessError) as error:
        failure = {"status": getattr(error, "status", "invalid"), "error": str(error), "gated": False}
        if args.out.is_dir() and not (args.out / "failure.json").exists():
            compare.write_new(args.out / "failure.json", failure)
        print(json.dumps(failure, indent=2), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
