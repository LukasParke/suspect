#!/usr/bin/env python3
"""Verify/import final-tree v2 evidence into a fresh M3/M6 acceptance snapshot.

No generation or measurement occurs here. Imported verdicts are recomputed from
hash-pinned raw samples, then bound to actual source, tool and input bytes.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import subprocess
import sys
import time

import compare
import run

FORMAT = "suspect-sdk-session-acceptance-v1"
PINS_FORMAT = "suspect-sdk-session-acceptance-inputs-v1"
KINDS = ("baseline", "candidate", "comparison", "policy", "runner")
FIXTURES = {"small": "m2-small", "split-recursive": "split-recursive", "openrouter": "openrouter-public-five"}
M2 = "crates/suspect-codegen/tests/fixtures/m2"
FIXTURE_FILES = {
    "m2-small": ("canonical.openapi.yaml",),
    "split-recursive": ("split.openapi.yaml", "split-resources.yaml"),
    "openrouter-public-five": ("openapi.yaml",),
}


def byte_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def decode(data: bytes):
    def invalid(value):
        raise compare.ReportError(f"non-finite JSON number: {value}")
    return json.loads(data, object_pairs_hook=compare._object, parse_constant=invalid)


def relative(name: str) -> Path:
    path = PurePosixPath(name)
    compare.require(bool(name) and not path.is_absolute() and "\\" not in name
                    and all(part not in ("", ".", "..") for part in name.split("/")), f"invalid relative evidence path: {name}")
    return Path(*path.parts)


def contained(root: Path, name: str) -> Path:
    path = root / relative(name)
    compare.require(path.resolve().is_relative_to(root.resolve()) and path.is_file(), f"missing/escaping file: {path}")
    return path


def read_pin(pin: dict) -> tuple[bytes, dict]:
    compare.require(set(pin) == {"path", "sha256"}, "pins require exactly path and sha256")
    path = Path(pin["path"])
    compare.require(path.is_absolute() and path.is_file(), f"pinned input must be an absolute regular file: {path}")
    compare.sha(pin["sha256"], str(path))
    data = path.read_bytes()
    compare.require(byte_hash(data) == pin["sha256"], f"pinned input changed: {path}", "incompatible")
    return data, decode(data)


def load_inputs(path: Path, digest: str) -> tuple[dict, dict, dict]:
    data, pins = read_pin({"path": str(path), "sha256": digest})
    compare.require(pins["format"] == PINS_FORMAT and set(pins) == {"format", *KINDS}, "unsupported acceptance pin manifest")
    raw, values = {"pins": data}, {}
    for kind in KINDS:
        raw[kind], values[kind] = read_pin(pins[kind])
    compare.require(values["baseline"]["policy"] == values["policy"], "pinned policy differs from baseline")
    result = compare.compare(values["baseline"], values["candidate"], gate=True)
    compare.require(result == values["comparison"], "pinned comparison differs from actual v2 recomputation")
    compare.require(result["status"] == result["confidence_decision"] == "passed" and result["gated"] is True
                    and not result["would_regress"] and not result["candidate_ineligible_reasons"],
                    "comparison is not a qualified, gated pass with zero regressions", "ineligible")
    compare.require(all(check["decision"] == "within-tolerance" and check["point_regression"] is False for check in result["checks"]),
                    "comparison contains a regression or unresolved confidence interval", "ineligible")
    policy = values["policy"]
    compare.require(set(FIXTURE_FILES) <= set(policy["required_fixtures"])
                    and {"typescript-http", "all"} <= set(policy["required_groups"]), "acceptance requires all three fixture classes and single/all-five groups")
    return pins, raw, values


def implementation_path(name: str) -> bool:
    return name in ("Cargo.toml", "Cargo.lock") or name.startswith(("crates/", ".cargo/"))


def rust_inventory_digest(inventory: dict) -> str:
    # Matches xtask's BTreeMap<String, FileRecord> serde field ordering. This is
    # distinct from the performance source manifest's canonical JSON digest.
    ordered = {name: {key: record[key] for key in ("kind", "sha256", "bytes", "mode", "link")}
               for name, record in sorted(inventory.items())}
    return byte_hash(json.dumps(ordered, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode())


def verify_record(root: Path, name: str, record: dict) -> None:
    path = root / relative(name)
    if record["kind"] == "deleted":
        compare.require(not path.exists() and not path.is_symlink(), f"deleted source reappeared: {path}")
        return
    metadata = path.lstat()
    compare.require(stat.S_IMODE(metadata.st_mode) & 0o777 == record["mode"], f"source permissions differ: {path}")
    if record["kind"] == "symlink":
        compare.require(path.is_symlink() and os.readlink(path) == record["link"]
                        and path.resolve().is_relative_to(root.resolve()), f"source symlink differs/escapes: {path}")
        data = os.readlink(path).encode()
    else:
        compare.require(record["kind"] == "file" and stat.S_ISREG(metadata.st_mode), f"source kind differs: {path}")
        data = path.read_bytes()
    compare.require(len(data) == record["bytes"] and byte_hash(data) == record["sha256"], f"source snapshot changed: {path}", "incompatible")


def verify_manifest(root: Path, manifest: dict) -> None:
    compare.fingerprints(manifest["files"], "source/harness")
    compare.require(manifest["sha256"] == compare.digest(manifest["files"]), "source/harness aggregate hash differs")
    compare.require([item["path"] for item in manifest["files"]] == sorted(item["path"] for item in manifest["files"]), "source/harness paths are not canonical")
    for item in manifest["files"]:
        path = contained(root, item["path"])
        compare.require(run.file_fingerprint(path, item["path"]) == item, f"source/harness bytes differ: {path}", "incompatible")


@contextmanager
def recorded_environment(values: dict):
    before = {name: os.environ.get(name) for name in values}
    try:
        for name, value in values.items():
            compare.require(isinstance(name, str) and (value is None or isinstance(value, str)), "invalid recorded build environment")
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
        yield
    finally:
        for name, value in before.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


def verify_tools(tools: dict, original: Path) -> None:
    compare.require({"rustc", "cargo", "python", "git", "cc"} <= tools.keys(), "incomplete tool identity")
    for name, tool in tools.items():
        compare.require(name in {"rustc", "cargo", "python", "git", "cc", "rustc_wrapper", "rustc_workspace_wrapper"}, "unknown tool identity")
        path = Path(tool["path"])
        compare.require(path.is_absolute() and run.file_fingerprint(path) == {key: tool[key] for key in ("path", "bytes", "sha256")}, f"tool bytes changed: {name}", "incompatible")
        flags = ["-Vv"] if name in ("rustc", "cargo") else ["-VV"] if name == "python" else ["--version"]
        query = [str(path), *flags]
        compare.require(tool.get("query") == query and run.output(query, original) == tool["version"], f"tool version/query differs: {name}", "incompatible")
        if "launcher" in tool:
            launcher = tool["launcher"]
            compare.require(run.file_fingerprint(Path(launcher["path"])) == launcher, f"tool dispatcher changed: {name}", "incompatible")


def snapshot_source(args, reports: list[dict]) -> tuple[dict, dict]:
    original, snapshot = args.original.resolve(), args.snapshot.resolve()
    manifest_path = args.acceptance_root / "source-manifest.json"
    inventory_bytes = manifest_path.read_bytes()
    inventory = decode(inventory_bytes)
    compare.sha(args.source_sha256, "acceptance source fingerprint")
    compare.require(rust_inventory_digest(inventory) == args.source_sha256, "xtask source fingerprint does not match its manifest")
    for name, record in inventory.items():
        if implementation_path(name) or name in run.HARNESS_FILES:
            verify_record(original, name, record)
            verify_record(snapshot, name, record)
    selected = sorted(name for name, record in inventory.items() if implementation_path(name)
                      and record["kind"] != "deleted" and (snapshot / name).is_file())
    compare.require(selected, "acceptance implementation census is empty")
    current = run.source_manifest(original)
    compare.require([item["path"] for item in current["files"]] == selected,
                    "original Git source census differs from the acceptance snapshot", "incompatible")
    for report in reports:
        source = report["provenance"]["source"]
        compare.require(source == current, "numerical evidence was not collected from the final acceptance implementation", "incompatible")
        verify_manifest(original, source)
        verify_manifest(snapshot, source)
        harness = report["identity"]["harness"]
        compare.require({item["path"] for item in harness["files"]} == set(run.HARNESS_FILES), "performance harness census differs")
        verify_manifest(original, harness)
        verify_manifest(snapshot, harness)
    cli_path = args.acceptance_root / "bin/suspect"
    compare.sha(args.cli_sha256, "acceptance CLI fingerprint")
    compare.require(run.file_fingerprint(cli_path)["sha256"] == args.cli_sha256, "acceptance frozen CLI changed", "incompatible")
    return inventory, {"acceptance_source_sha256": args.source_sha256, "manifest_sha256": byte_hash(inventory_bytes),
                       "performance_source_sha256": current["sha256"], "source_files": len(selected),
                       "original_and_snapshot_match": True, "acceptance_cli_sha256": args.cli_sha256}


def verify_inputs(args, report: dict) -> tuple[list[dict], list[tuple[str, bytes]]]:
    checked, copies = [], []
    provenance = report["provenance"]["inputs"]
    for case in report["cases"]:
        data = case["report"]
        fixture = data["fixture"]
        compare.require(fixture in FIXTURE_FILES, "unknown acceptance fixture")
        compare.require(tuple(sorted(item["path"] for item in data["inputs"])) == tuple(sorted(FIXTURE_FILES[fixture])), "fixture closure differs from the accepted corpus")
        public = fixture == "openrouter-public-five"
        original_root = args.corpus / Path(run.PUBLIC_ENTRY).parent if public else args.original / M2
        snapshot_root = args.acceptance_root / "inputs" / Path(run.PUBLIC_ENTRY).parent if public else args.snapshot / M2
        compare.require(Path(data["input_root"]).resolve() == original_root.resolve(), "measured original input root differs")
        for item in data["inputs"]:
            source, snapshot = contained(original_root, item["path"]), contained(snapshot_root, item["path"])
            compare.require(run.file_fingerprint(source, item["path"]) == item
                            and run.file_fingerprint(snapshot, item["path"]) == item, "original/private acceptance input differs from measurement", "incompatible")
            if public:
                current = run.git_provenance(source, True)
                recorded = [entry for entry in provenance if entry["case"] == case["id"]]
                compare.require(len(recorded) == 1 and {key: value for key, value in recorded[0].items() if key != "case"} == current
                                and current["provenance"] == "tracked-head", "tracked public input provenance differs", "incompatible")
            checked.append({"case": case["id"], "path": item["path"], "sha256": item["sha256"], "bytes": item["bytes"], "original_and_snapshot_match": True})
        prepared_root = Path(case["prepared_input_root"])
        suite_root = Path(report["provenance"]["binary"]["path"]).parent.parent
        compare.require(prepared_root == suite_root / "prepared-inputs" / case["id"].replace("/", "-"), "prepared archive root differs from suite output")
        compare.require({item["path"] for item in data["prepared_inputs"]} == set(FIXTURE_FILES[fixture]), "prepared closure differs")
        for item in data["prepared_inputs"]:
            path = contained(prepared_root, item["path"])
            content = path.read_bytes()
            compare.require(len(content) == item["bytes"] and byte_hash(content) == item["sha256"], "prepared input archive differs", "incompatible")
            copies.append(("input", content))
    return checked, copies


def verify_configuration(report: dict) -> None:
    build = report["identity"]["build"]
    compare.require(build["profile"] == "release" and build["locked"] is True and build["features"] == [], "unaccepted build configuration")
    command = report["commands"]["build"]
    prefix = ["cargo", "build", "--locked", "-p", "suspect-codegen", "--example", "sdk_session_bench", "--target-dir"]
    compare.require(command[:len(prefix)] == prefix and len(command) in (10, 11)
                    and command[len(prefix) + 1:] in (["--release"], ["--offline", "--release"]), "build command differs from the recorded release configuration")
    commands = report["commands"]["cases"]
    compare.require(len(commands) == len(report["cases"]), "case command census differs")
    for case, command in zip(report["cases"], commands):
        data = case["report"]
        config = data["configuration"]
        fixture = data["fixture"]
        group = case["id"].split("/")[-1]
        names = run.BACKENDS if group == "all" else (group,)
        compare.require(all(name in run.BACKENDS for name in names), "unknown target group")
        targets = [{"backend": name, "package_name": "BenchmarkSDK" if name == "swift-http" else "example.com/sdk-session-perf" if name == "go-http" else "sdk-session-perf",
                    "package_version": "0.0.0", "import_name": "sdk_session_perf" if name == "python-http" else None} for name in names]
        operations = list(run.PUBLIC_OPERATIONS) if fixture == "openrouter-public-five" else []
        compare.require(config["targets"] == targets and config["operation_ids"] == operations
                        and config["owner"] == "suspect-sdk:session-performance"
                        and compare.digest(config) == data["configuration_sha256"], "actual session configuration differs from the canonical fixture workload")
        entry = FIXTURE_FILES[fixture][0]
        expected = [report["provenance"]["binary"]["path"], "--spec", str(Path(data["input_root"]) / entry),
                    "--out", str(Path(data["private_root"]).parent), "--fixture", fixture,
                    "--iterations", str(config["iterations"]), "--warmups", str(config["warmups"]),
                    "--targets", ",".join(names), "--cache-entries", str(config["cache_entries"]), "--cache-bytes", str(config["cache_bytes"])]
        for operation in operations:
            expected += ["--operation-id", operation]
        compare.require(command == expected, "recorded execution command does not match actual case configuration")


def summary(reports: list[dict], fixture: str) -> dict:
    groups = {case["id"].split("/")[-1] for case in reports[0]["cases"] if case["report"]["fixture"] == fixture}
    compare.require({"typescript-http", "all"} <= groups, "fixture lacks single/all-five coverage")
    result = {name: {"samples": 0, "compiles": 0, "renders": 0, "writes": 0} for name in ("cold", "warm", "sourceChange", "configChange")}
    for report in reports:
        for case in report["cases"]:
            raw = case["report"]
            if raw["fixture"] != fixture:
                continue
            classified = [(sample["scenario"], sample) for sample in raw["samples"]
                          if sample["scenario"] in ("cold", "warm", "schema-edit", "operation-edit", "docs-edit")]
            classified.append(("configChange", raw["configuration_probe"]["change"]))
            if raw["module_probe"]:
                classified.append(("configChange", raw["module_probe"]["change"]))
            for name, sample in classified:
                bucket = result["sourceChange" if name.endswith("-edit") else name]
                compare.require(sample["fresh_oracle_equal"] is True and sample["disk_current"] is True and sample["redundant_rewrites"] == 0,
                                "stale output or redundant writes in accepted samples")
                for phase in ("generate", "write"):
                    compare.require(compare.number(sample[phase]["ms"], f"{name}/{phase}/ms") > 0, "required scenario was not measured")
                if name == "warm":
                    compare.require(not sample["changed_paths"] and sample["delta"]["compiles"] == sample["delta"]["renders"] == 0, "warm refresh did redundant work")
                bucket["samples"] += 1
                bucket["compiles"] += sample["delta"]["compiles"]
                bucket["renders"] += sample["delta"]["renders"]
                # Warm has neither semantic changes nor same-byte rewrites; the
                # native gate also compares bytes, inode, mtime and ctime.
                bucket["writes"] += len(sample["changed_paths"]) + sample["redundant_rewrites"]
    compare.require(all(item["samples"] > 0 for item in result.values()), "required measurement category is empty")
    return result


def copy_blob(root: Path, kind: str, content: bytes) -> dict:
    digest = byte_hash(content)
    relative_path = f"performance/raw/{kind}-{digest}"
    destination = root / relative_path
    destination.parent.mkdir(parents=True, exist_ok=True)
    compare.require(destination.parent.resolve().is_relative_to(root.resolve() / "performance"), "evidence archive escaped its output root")
    if destination.exists():
        compare.require(destination.is_file() and not destination.is_symlink() and destination.read_bytes() == content, "existing content-addressed evidence differs")
    else:
        with destination.open("xb") as stream:
            stream.write(content)
        destination.chmod(0o444)
    return {"path": relative_path, "sha256": digest, "bytes": len(content)}


def import_evidence(args) -> dict:
    pins, raw, values = load_inputs(args.evidence, args.evidence_sha256)
    baseline, candidate = values["baseline"], values["candidate"]
    reports = [*baseline["calibration_runs"], *candidate["reports"]]
    _, source = snapshot_source(args, reports)
    reference = candidate["reports"][0]
    identity = reference["identity"]
    compare.require(identity["runner"]["declared"] == values["runner"], "pinned runner declaration differs from measurements")
    # Import runs under xtask's isolated test environment. Restore only the
    # recorded build selectors for read-only tool/config queries, never a build.
    with recorded_environment(identity["build"]["environment"]):
        verify_tools(identity["tools"], args.original)
        compare.require(run.cargo_configuration(args.original) == identity["build"]["cargo_config_files"], "actual Cargo configuration differs from measurements", "incompatible")
        work = Path(identity["work_root"])
        while not work.exists():
            work = work.parent
        compare.require(run.runner_identity(Path(pins["runner"]["path"]), work) == identity["runner"], "actual machine/power/runner identity differs", "incompatible")
    expected_config = reference["identity"]["build"]
    compare.require(expected_config["profile"] == "release" and expected_config["locked"] is True and expected_config["features"] == [], "unaccepted build configuration")
    checked_inputs, blobs = [], list(raw.items())
    for report in reports:
        compare.require(report["identity"] == identity, "tool/runner/config identity differs within final evidence")
        verify_configuration(report)
        binary = report["provenance"]["binary"]
        binary_path = Path(binary["path"])
        compare.require(binary_path.resolve().is_relative_to((args.original / "target").resolve()), "benchmark binary is not an original-workspace artifact")
        content = binary_path.read_bytes()
        compare.require(len(content) == binary["bytes"] and byte_hash(content) == binary["sha256"], "frozen measured binary differs", "incompatible")
        blobs.append(("binary", content))
        inputs, prepared = verify_inputs(args, report)
        checked_inputs.extend(inputs)
        blobs.extend(prepared)
    # Recheck every external pin after expensive verification, before publication.
    read_pin({"path": str(args.evidence), "sha256": args.evidence_sha256})
    for kind in KINDS:
        read_pin(pins[kind])
    copied = {(kind, byte_hash(content)): copy_blob(args.acceptance_root, kind, content) for kind, content in blobs}
    output = {
        "format": FORMAT, "stage": args.stage, "complete": True, "qualification": "qualified",
        "verified_at_ns": time.time_ns(), "measurement_origin": "verified-original-workspace-v2-import",
        "source": source, "tool_identity_sha256": compare.digest(identity["tools"]),
        "runner_identity_sha256": compare.digest(identity["runner"]),
        "build_configuration_sha256": compare.digest(expected_config), "policy_sha256": compare.digest(values["policy"]),
        "inputs": checked_inputs, "evidence_manifest_sha256": args.evidence_sha256,
        "copied_evidence": sorted(copied.values(), key=lambda item: item["path"]),
        "verification": {"source_and_snapshot": True, "actual_tools": True, "actual_inputs": True,
                         "prepared_inputs": True, "build_configuration": True, "measured_binaries": True,
                         "runner_identity": True, "v2_qualification_recomputed": True, "comparison_recomputed": True},
    }
    if args.stage in FIXTURES:
        fixture = FIXTURES[args.stage]
        output.update(fixture=fixture, performance_status="observational", summary=summary(candidate["reports"], fixture))
    else:
        result = values["comparison"]
        cases = {check["key"].rsplit(":", 2)[0] for check in result["checks"]}
        fixture_classes = {case["report"]["fixture"] for case in reference["cases"]}
        compare.require(set(FIXTURE_FILES) <= fixture_classes, "comparison omits a required fixture class")
        output.update(gated=result["gated"], verdict=result["status"], regressions=0, comparedCases=len(cases),
                      comparedFixtureClasses=len(fixture_classes), comparison_sha256=pins["comparison"]["sha256"])
    return output


def plan(args) -> dict:
    pins, _, values = load_inputs(args.evidence, args.evidence_sha256)
    python = values["candidate"]["reports"][0]["identity"]["tools"]["python"]["path"]
    inputs = [{"path": str(args.evidence), "sha256": args.evidence_sha256}, *[pins[kind] for kind in KINDS]]
    for report in [*values["baseline"]["calibration_runs"], *values["candidate"]["reports"]]:
        binary = report["provenance"]["binary"]
        inputs.append({key: binary[key] for key in ("path", "sha256")})
        for item in report["identity"]["build"]["cargo_config_files"]:
            inputs.append({key: item[key] for key in ("path", "sha256")})
        for case in report["cases"]:
            for item in case["report"]["prepared_inputs"]:
                inputs.append({"path": str(Path(case["prepared_input_root"]) / relative(item["path"])), "sha256": item["sha256"]})
    unique = {}
    for item in inputs:
        compare.require(item["path"] not in unique or unique[item["path"]] == item, "one external path has inconsistent evidence pins")
        unique[item["path"]] = item
    stages = []
    for name in (*FIXTURES, "compare"):
        claims = {"complete": "/complete"}
        if name == "compare":
            claims.update(gated="/gated", verdict="/verdict", regressions="/regressions", comparedCases="/comparedCases")
        else:
            claims.update(measurementStatus="/performance_status", warmCompiles="/summary/warm/compiles",
                          warmRenders="/summary/warm/renders", warmWrites="/summary/warm/writes",
                          coldSamples="/summary/cold/samples", warmSamples="/summary/warm/samples",
                          sourceChangeSamples="/summary/sourceChange/samples", configChangeSamples="/summary/configChange/samples")
        report_path = f"{{out}}/performance/{name}.json"
        stages.append({"id": f"performance-{name}", "program": python, "cwd": "{workspace}", "environment": {},
                       "args": ["{workspace}/tools/sdk-session-perf/accept.py", "import", "--stage", name,
                                "--evidence", str(args.evidence), "--evidence-sha256", args.evidence_sha256,
                                "--original", "{original}", "--snapshot", "{workspace}",
                                "--corpus", str(args.corpus), "--acceptance-root", "{out}", "--out", report_path],
                       "report": report_path, "claims": claims,
                       "assertions": {"/format": FORMAT, "/complete": True, "/qualification": "qualified",
                                      "/verification/comparison_recomputed": True, "/verification/source_and_snapshot": True,
                                      "/policy_sha256": compare.digest(values["policy"])}})
    return {"format": "suspect.sdk.m3-m6.performance-plan.v1", "inputs": list(unique.values()), "stages": stages}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    for name in ("plan", "import"):
        mode = modes.add_parser(name)
        mode.add_argument("--evidence", type=Path, required=True)
        mode.add_argument("--evidence-sha256", required=True)
        mode.add_argument("--corpus", type=Path, required=True, help="original tracked OpenRouter checkout")
        mode.add_argument("--out", type=Path, required=True)
        if name == "import":
            mode.add_argument("--stage", choices=(*FIXTURES, "compare"), required=True)
            mode.add_argument("--original", type=Path, required=True)
            mode.add_argument("--snapshot", type=Path, required=True)
            mode.add_argument("--acceptance-root", type=Path, required=True)
            mode.add_argument("--source-sha256", default=os.environ.get("SUSPECT_M3_M6_SOURCE_SHA256"))
            mode.add_argument("--cli-sha256", default=os.environ.get("SUSPECT_M3_M6_BINARY_SHA256"))
    args = parser.parse_args()
    safe_output = False
    try:
        compare.require(args.evidence.is_absolute() and args.corpus.is_absolute(), "evidence and original corpus paths must be absolute")
        if args.mode == "import":
            args.acceptance_root = args.acceptance_root.resolve()
            compare.require(args.out.resolve().is_relative_to(args.acceptance_root / "performance") and not args.out.exists(), "acceptance report must be new under performance/")
            safe_output = True
            compare.require(Path(__file__).resolve() == (args.snapshot / "tools/sdk-session-perf/accept.py").resolve(), "run the adapter from the actual acceptance execution copy")
            result = import_evidence(args)
        else:
            result = plan(args)
        compare.write_new(args.out, result)
        if args.mode == "import":
            args.out.chmod(0o444)
        print(json.dumps({"format": result["format"], "complete": result.get("complete"), "out": str(args.out)}, indent=2))
        return 0
    except (compare.ReportError, KeyError, TypeError, ValueError, OSError, subprocess.SubprocessError) as error:
        failure = {"format": FORMAT, "complete": False, "status": getattr(error, "status", "invalid"), "error": str(error)}
        if safe_output and not args.out.exists() and args.out.parent.is_dir():
            compare.write_new(args.out, failure)
        print(json.dumps(failure, indent=2), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
