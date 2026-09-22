#!/usr/bin/env python3
"""Collect or verify the sdk-full native-costs contract using installed tools only."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
from typing import Any
import zipfile

from bindings import load_binding
from evidence import (Commands, DEFAULT_RUN_POLICY, EvidenceError, FORMAT, ITERATIONS, METHODOLOGY, PHASES, PRECISION,
                      RecordingServer, RUN_KINDS, dimensions, file_record, read_json, require, required_environment,
                      run_policy, run_statistics, sha, sha_bytes, summary_view, targets_from, validate_report, witness,
                      write_json, write_new)
from inputs import Inputs, artifact_files
from native import Context, Tools, WORKSPACE, prepare


def reserve_output(env: dict[str, str]) -> Path:
    path = Path(env["SUSPECT_SDK_FULL_MEASUREMENTS"])
    require(path.parent.is_dir(), f"output parent does not exist: {path.parent}")
    require(not os.path.lexists(path), f"native output must be new; preserving existing attempt: {path}")
    root = path.parent.resolve(strict=True) / path.name
    for key in ("SUSPECT_SDK_FULL_PACKAGES", "SUSPECT_SDK_FULL_NATIVE_ROOT"):
        source = Path(env[key]).resolve(strict=True)
        require(not root.is_relative_to(source) and not source.is_relative_to(root), "native output overlaps immutable inputs")
    root.mkdir()
    return root


def selected(only: list[str], language: str, tier: str) -> bool:
    return not only or language in only or language + "/" + tier in only


def collect(env: dict[str, str], only: list[str] | None = None, policy: dict[str, int] | None = None,
            report_view: str = "raw") -> tuple[Path, dict[str, Any]]:
    required_environment(env)
    only = only or []
    counts = policy or DEFAULT_RUN_POLICY
    policy = run_policy(counts["cold"], counts["warmup"], counts["steady"])
    require(report_view in ("raw", "summary"), f"unknown --report view: {report_view}")
    output = reserve_output(env)
    commands = Commands(output)
    report: dict[str, Any] = {
        "format": FORMAT, "complete": False, "methodology": METHODOLOGY,
        "sourceFingerprint": env["SUSPECT_SDK_FULL_SOURCE_SHA256"], "cliSha256": env["SUSPECT_SDK_FULL_BINARY_SHA256"],
        "measurements": [], "failures": [], "selection": only,
        "measurementPolicy": {"clock": "Python time.perf_counter_ns monotonic subprocess wall time",
                              "methodology": METHODOLOGY, "runsPerPhase": dict(policy), "iterations": ITERATIONS,
                              "scope": "raw native observations; every cold/warmup/steady run retained with full attribution; "
                                       "import measures process/runtime startup separately from the codec/request operations",
                              "dependencyPolicy": "already installed tools and warm offline dependency source/plugin caches; private new build outputs",
                              "statistics": "median, p90, p99, min, max, mean and population stddev recomputed from the retained raw "
                                            "steady-state samples during every validation; no calibration, regression threshold or qualification"},
        "inputInventory": "input-inventory.json",
    }
    targets: list[dict[str, Any]] = []
    inputs: Inputs | None = None
    progress = (output / "observations.jsonl").open("x", encoding="utf-8")

    def journal(value: dict[str, Any]) -> None:
        progress.write(json.dumps(value, sort_keys=True) + "\n")
        progress.flush()
        os.fsync(progress.fileno())

    def failure(language: str, tier: str, phase: str, error: BaseException) -> None:
        value = {"language": language, "tier": tier, "phase": phase, "error": f"{type(error).__name__}: {error}"}
        report["failures"].append(value)
        journal({"failure": value})
        print(f"UNMET {language}/{tier}/{phase}: {error}", file=sys.stderr, flush=True)

    def row(target: dict[str, Any], tier: str, phase: str, runs: list[dict[str, Any]], artifacts: list[Path], scope: str) -> None:
        language = target["language"]
        manifest = f"artifacts/{language}/{tier}/{phase}.json"
        payload = artifact_files(artifacts)
        write_json(output / manifest, payload)
        samples = []
        for run in runs:
            record = run["record"]
            sample = {key: record[key] for key in ("nanoseconds", "exitCode", "command", "programSha256", "stdout",
                                                  "stdoutSha256", "stderr", "stderrSha256", "record")}
            sample["iterations"] = ITERATIONS[phase]
            sample["runKind"] = run["kind"]
            sample.update(run.get("extra") or {})
            samples.append(sample)
        value = {"language": language, "tier": tier, "phase": phase,
                 "packageManifestSha256": sha(Path(env["SUSPECT_SDK_FULL_PACKAGES"]) / language / target["manifest"]),
                 "artifactBytes": sum(p["bytes"] for p in payload), "artifactManifest": manifest,
                 "artifactManifestSha256": sha(output / manifest), "scope": scope,
                 "methodology": METHODOLOGY, "runs": dict(policy),
                 "summary": run_statistics([s["nanoseconds"] for s in samples if s["runKind"] == "steady"]),
                 "samples": samples}
        report["measurements"].append(value)
        journal({"measurement": value})
        summary = value["summary"]
        print(f"OBSERVED {language}/{tier}/{phase}: {len(samples)} retained runs (cold {policy['cold']}, "
              f"warmup {policy['warmup']}, steady {policy['steady']}); steady median {summary['median']:.0f} ns, "
              f"p90 {summary['p90']:.0f} ns; {value['artifactBytes']} payload bytes", flush=True)

    try:
        write_json(output / "invocation.json", {"environment": {key: value for key, value in env.items() if key.startswith("SUSPECT_")},
                                                "selection": only, "collector": file_record(Path(__file__)),
                                                "interpreter": file_record(Path(sys.executable)), "cwd": str(Path.cwd())})
        targets = targets_from(Path(env["SUSPECT_SDK_FULL_TARGETS"]))
        allowed = {t["language"] for t in targets} | {t["language"] + "/" + tier for t in targets for tier in t["toolchain_tiers"]}
        require(len(only) == len(set(only)) and set(only) <= allowed, "unknown/duplicate --only selection")
        inputs = Inputs(Path(env["SUSPECT_SDK_FULL_PACKAGES"]), Path(env["SUSPECT_SDK_FULL_NATIVE_ROOT"]), output)
        inputs.remember(Path(env["SUSPECT_SDK_FULL_TARGETS"]))
        for path in Path(__file__).parent.glob("*.py"):
            inputs.remember(path)
        for path in (Path(__file__).parent / "drivers").glob("*.in"):
            inputs.remember(path)
        inputs.remember(Path(sys.executable))
        inputs.generated()
        # The six-field contract supplies fingerprints. The runner also exposes
        # its frozen CLI as SUSPECT_TEST_BINARY and under out/bin/suspect. Verify
        # those actual bytes when present instead of trusting a version string.
        binary = Path(env.get("SUSPECT_TEST_BINARY", str(inputs.packages.parent / "bin/suspect")))
        if binary.is_file():
            require(sha(binary) == report["cliSha256"], "frozen CLI bytes differ from runner fingerprint")
            inputs.remember(binary)
            report["cliPath"] = str(binary)
        report["fingerprintAuthority"] = "explicit frozen source census and CLI SHA-256 supplied by sdk-full"
        fixture = WORKSPACE / "crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json"
        inputs.remember(fixture)
        fixture_data = read_json(fixture)
        credits = fixture_data["credits"]
        require(isinstance(credits, str) and f'"total_credits":{PRECISION}' in credits and '"total_usage":25.75' in credits,
                "independent getCredits fixture changed; update the native assertions explicitly")
        write_new(output / "fixtures/openrouter-five-responses.json", fixture.read_bytes())
        write_new(output / "fixtures/credits.json", credits)
        report["fixture"] = {"path": "fixtures/openrouter-five-responses.json", "sha256": sha(fixture),
                              "credits": "fixtures/credits.json", "creditsSha256": sha_bytes(credits.encode()),
                              "origin": "independent hand-authored OpenRouter five-response wire fixture"}
        tools = Tools(env, inputs, commands)
        for target in targets:
            language = target["language"]
            if not any(selected(only, language, tier) for tier in target["toolchain_tiers"]):
                continue
            try:
                inputs.package_identity(target)
                binding = load_binding(inputs.packages / language, target)
                write_json(output / f"bindings/{language}.json", binding)
            except (EvidenceError, OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
                failure(language, "all", "package-bindings", error)
                continue
            for index, tier in enumerate(target["toolchain_tiers"]):
                if not selected(only, language, tier):
                    continue
                slot = "declared" if language == "cpp" else ("floor" if index == 0 else "current")
                ctx = None
                try:
                    installed = inputs.installed(target, slot)
                    ctx = Context(target, tier, slot, binding, installed, credits, inputs, tools, commands,
                                  lambda runs, files, scope, t=target, version=tier: row(t, version, "build", runs, files, scope),
                                  policy)
                    prepare(ctx)
                except (EvidenceError, OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
                    failure(language, tier, ctx.stage if ctx else "installed-input", error)
                    continue
                for phase in PHASES[1:]:
                    try:
                        runs: list[dict[str, Any]] = []
                        # Run-set order per the measurement plan: repeated cold
                        # runs first, then excluded warmups, then the
                        # steady-state batch. Import stays the separate
                        # startup measurement; codec/request repeat the
                        # operation. Prep is never part of a measured run.
                        for kind in RUN_KINDS:
                            for index in range(policy[kind]):
                                argv = [*ctx.command, phase, str(ITERATIONS[phase])]
                                label = f"measured {phase} ({kind} {index + 1}/{policy[kind]})"
                                extra: dict[str, Any] = {}
                                if phase == "request":
                                    server = RecordingServer(credits.encode(), ITERATIONS[phase])
                                    wire = f"wire/{language}/{tier}.{index}.{kind}.json"
                                    try:
                                        with server:
                                            record = ctx.run([*argv, server.url], label=label, timeout=45)
                                    finally:
                                        write_json(output / wire, {"url": server.url, "requests": server.records,
                                                                   "errors": server.errors, "threadTerminated": not server.worker.is_alive()})
                                    server.verify()
                                    extra = {"wire": wire, "wireSha256": sha(output / wire)}
                                else:
                                    record = ctx.run(argv, label=label, timeout=45)
                                extra["witness"] = witness(commands.stdout(record), phase, ITERATIONS[phase])
                                runs.append({"kind": kind, "record": record, "extra": extra})
                        scope = ctx.import_scope if phase == "import" else ctx.runtime_scope + (
                            "; each iteration decodes the independent fixture into the allocated public model and encodes it with the SDK codec"
                            if phase == "codec" else "; sequential getCredits SDK calls using an owned HTTP/1.1 loopback recording server")
                        row(target, tier, phase, runs, ctx.runtime_artifacts, scope)
                    except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
                        failure(language, tier, phase, error)
    except (EvidenceError, OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
        failure("all", "all", "configuration", error)
    except KeyboardInterrupt as error:
        failure("all", "all", "interrupted", error)
    finally:
        if inputs:
            try:
                inputs.verify()
            except (EvidenceError, OSError) as error:
                failure("all", "all", "input-integrity", error)
            inputs.save()
        else:
            write_json(output / "input-inventory.json", {"trees": {}, "files": []})
        expected = dimensions(targets)
        observed = {(r["language"], r["tier"], r["phase"]) for r in report["measurements"]}
        report["requiredDimensionCount"] = len(expected)
        report["missing"] = [dict(zip(("language", "tier", "phase"), key)) for key in sorted(expected - observed)]
        report["complete"] = bool(expected) and observed == expected and not report["failures"]
        write_json(output / "candidate-report.json", report)
        if targets and report["measurements"]:
            try:
                validate_report(output, targets, Path(env["SUSPECT_SDK_FULL_PACKAGES"]), report["sourceFingerprint"], report["cliSha256"],
                                allow_incomplete=True, report_name="candidate-report.json")
            except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
                failure("all", "all", "report-integrity", error)
                report["complete"] = False
        if report_view == "summary" and report["measurements"]:
            # The raw evidence document stays complete on disk; report.json
            # becomes a derived summary view that validation must cross-check
            # against the raw document. Aggregates are never the only copy.
            write_json(output / "raw-evidence.json", report)
            view = summary_view(report, "raw-evidence.json", sha(output / "raw-evidence.json"))
            write_json(output / "report.json", view)
            if targets:
                try:
                    validate_report(output, targets, Path(env["SUSPECT_SDK_FULL_PACKAGES"]), report["sourceFingerprint"],
                                    report["cliSha256"], allow_incomplete=True, report_name="report.json")
                except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
                    failure("all", "all", "report-integrity", error)
                    report["complete"] = False
        else:
            write_json(output / "report.json", report)
        progress.close()
    return output, report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    collect_parser = commands.add_parser("collect", help="run all 92 required native phase run sets")
    collect_parser.add_argument("--only", action="append", default=[], metavar="LANGUAGE[/TIER]",
                                help="development observation subset; report remains incomplete and exits nonzero")
    collect_parser.add_argument("--cold-runs", type=int, default=DEFAULT_RUN_POLICY["cold"], metavar="N",
                                help="retained cold runs per measured phase (default %(default)s)")
    collect_parser.add_argument("--warmups", type=int, default=DEFAULT_RUN_POLICY["warmup"], metavar="W",
                                help="excluded warmup runs before the steady batch (default %(default)s)")
    collect_parser.add_argument("--steady-runs", type=int, default=DEFAULT_RUN_POLICY["steady"], metavar="M",
                                help="steady-state runs summarized per phase (default %(default)s)")
    collect_parser.add_argument("--report", choices=("raw", "summary"), default="raw",
                                help="report.json content: full raw evidence, or a summary view beside raw-evidence.json")
    verify = commands.add_parser("validate", help="verify existing raw evidence and immutable inputs")
    verify.add_argument("--allow-incomplete", action="store_true", help="inspect retained partial evidence without claiming acceptance")
    args = parser.parse_args()
    try:
        policy = run_policy(args.cold_runs, args.warmups, args.steady_runs) if args.command == "collect" else None
        env = dict(os.environ)
        required_environment(env)
        if args.command == "collect":
            output, report = collect(env, args.only, policy, args.report)
            print(f"Native cost report: {output / 'report.json'}; complete={report['complete']}; rows={len(report['measurements'])}")
            return 0 if report["complete"] else 1
        validate_report(Path(env["SUSPECT_SDK_FULL_MEASUREMENTS"]), targets_from(Path(env["SUSPECT_SDK_FULL_TARGETS"])),
                        Path(env["SUSPECT_SDK_FULL_PACKAGES"]), env["SUSPECT_SDK_FULL_SOURCE_SHA256"], env["SUSPECT_SDK_FULL_BINARY_SHA256"],
                        allow_incomplete=args.allow_incomplete)
        print("Native report bytes, dimensions, commands, logs, artifacts and installed inputs verified")
        return 0
    except (EvidenceError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"Native costs failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
