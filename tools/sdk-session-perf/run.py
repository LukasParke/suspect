#!/usr/bin/env python3
"""Build/freeze the SDK Session benchmark and collect attributable raw samples."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import uuid

import compare

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
BACKENDS = compare.CANONICAL_BACKENDS
PUBLIC_ENTRY = "projects/docs/openapi/openapi.yaml"
PUBLIC_OPERATIONS = ("getCredits", "createKeys", "updateKeys", "listContainerFiles", "getContainerFile")
HARNESS_FILES = (
    "crates/suspect-codegen/examples/sdk_session_bench.rs",
    "crates/suspect-codegen/examples/sdk_session_bench/attribution.rs",
    "tools/sdk-session-perf/run.py",
    "tools/sdk-session-perf/compare.py",
    "tools/sdk-session-perf/calibrate.py",
    "tools/sdk-session-perf/accept.py",
)


def output(command: list[str], cwd: Path = ROOT) -> str:
    completed = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    return (completed.stdout + completed.stderr).decode("utf-8", errors="replace").strip()


def file_fingerprint(path: Path, label: str | None = None) -> dict:
    hasher = hashlib.sha256()
    length = 0
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            length += len(data)
            hasher.update(data)
    return {"path": label if label is not None else str(path), "bytes": length, "sha256": hasher.hexdigest()}


def manifest(paths: list[str], root: Path = ROOT) -> dict:
    files = [file_fingerprint(root / path, path) for path in sorted(set(paths))]
    return {"sha256": compare.digest(files), "files": files}


def source_manifest(root: Path = ROOT) -> dict:
    # Include dirty and new source/runtime/template/build files. A Git commit alone
    # cannot identify a binary compiled from an active working tree.
    names = subprocess.check_output([
        "git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--",
        "Cargo.toml", "Cargo.lock", ".cargo", "crates",
    ], cwd=root).decode().split("\0")
    return manifest([name for name in names if name and (root / name).is_file()], root)


def executable(name: str) -> Path:
    found = shutil.which(name)
    compare.require(found is not None, f"required tool missing: {name}")
    return Path(found).resolve()


def rust_tool(name: str, root: Path = ROOT) -> Path:
    path = executable(name)
    rustup = shutil.which("rustup")
    # rustup proxies can be hard links as well as symlinks. Fingerprint the
    # selected tool, not just the dispatcher. RUSTC may select a different
    # compiler from Cargo's active toolchain, so resolve them independently.
    if rustup and os.path.samefile(path, rustup):
        return Path(output([rustup, "which", Path(name).name], root)).resolve()
    return path


def tool_identity(root: Path = ROOT) -> dict:
    rustc = os.environ.get("RUSTC") or "rustc"
    tools = {
        "rustc": (rust_tool(rustc, root), [rustc, "-Vv"]),
        "cargo": (rust_tool("cargo", root), ["cargo", "-Vv"]),
        "python": (Path(sys.executable).resolve(), [sys.executable, "-VV"]),
        "git": (executable("git"), ["git", "--version"]),
        "cc": (executable(os.environ.get("CC", "cc")), [os.environ.get("CC", "cc"), "--version"]),
    }
    for variable in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
        if os.environ.get(variable):
            path = executable(os.environ[variable])
            tools[variable.lower()] = (path, [str(path), "--version"])
    identities = {}
    for name, (path, command) in tools.items():
        launcher = None
        if platform.system() == "Darwin" and name in ("git", "cc") and path.parent == Path("/usr/bin"):
            # /usr/bin/git and /usr/bin/cc are identical Xcode dispatchers on this
            # Mac. Pin the selected implementation as well as that dispatcher.
            launcher = file_fingerprint(path)
            path = Path(output(["/usr/bin/xcrun", "--find", "clang" if name == "cc" else "git"], root)).resolve()
        query = [str(path.resolve()), *command[1:]]
        identities[name] = {**file_fingerprint(path.resolve()), "version": output(query, root), "query": query}
        if launcher:
            identities[name]["launcher"] = launcher
    return identities


def cargo_configuration(root: Path = ROOT) -> list[dict]:
    # Cargo also reads parent-directory and user configuration, outside Git.
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    directories = [cargo_home, *[parent / ".cargo" for parent in (root, *root.parents)]]
    paths = {directory / name for directory in directories for name in ("config", "config.toml")}
    return [file_fingerprint(path.resolve()) for path in sorted(paths) if path.is_file()]


def runner_identity(spec: Path | None, work: Path) -> dict:
    declared = compare.load(spec) if spec else {
        "format": "suspect-sdk-session-runner-v2", "kind": "observational",
        "id": platform.node() or "local", "image": platform.platform(),
        "cpu_policy": "uncontrolled", "notes": "Uncalibrated local/shared runner observations.",
    }
    compare.require(declared["format"] == "suspect-sdk-session-runner-v2"
                    and declared["kind"] in ("observational", "dedicated")
                    and declared["id"] and declared["image"] and declared["cpu_policy"], "invalid runner declaration")
    if platform.system() == "Darwin":
        cpu = output(["sysctl", "-n", "machdep.cpu.brand_string"])
        memory = int(output(["sysctl", "-n", "hw.memsize"]))
        physical_cpus = int(output(["sysctl", "-n", "hw.physicalcpu"]))
        hardware = re.search(r'"IOPlatformUUID"\s*=\s*"([^"]+)"', output(["ioreg", "-rd1", "-c", "IOPlatformExpertDevice"]))
        machine_id = compare.digest(hardware.group(1)) if hardware else None
        version = output(["/usr/bin/sw_vers", "-productVersion"])
        build = output(["/usr/bin/sw_vers", "-buildVersion"])
        power = output(["/usr/bin/pmset", "-g", "custom"])
        power_source = re.search(r"Now drawing from '([^']+)'", output(["/usr/bin/pmset", "-g", "batt"]))
        macos = {"version": version, "build": build, "power_settings": power,
                 "power_source": power_source.group(1) if power_source else None}
    elif platform.system() == "Linux":
        lines = Path("/proc/cpuinfo").read_text().splitlines()
        cpu = next((line.split(":", 1)[1].strip() for line in lines if line.startswith(("model name", "Hardware"))), platform.machine())
        memory = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        physical_cpus = None
        machine_value = Path("/etc/machine-id").read_text().strip() if Path("/etc/machine-id").is_file() else ""
        machine_id = compare.digest(machine_value) if machine_value else None
    else:
        cpu, memory, physical_cpus = platform.processor() or platform.machine(), None, None
        machine_id = None
    if platform.system() != "Darwin":
        macos = None
    governors = {str(path): path.read_text().strip() for path in Path("/sys/devices/system/cpu").glob("cpu[0-9]*/cpufreq/scaling_governor")}
    boost = {str(path): path.read_text().strip() for path in (
        Path("/sys/devices/system/cpu/intel_pstate/no_turbo"), Path("/sys/devices/system/cpu/cpufreq/boost")) if path.is_file()}
    return {"declared": declared, "actual": {
        "system": platform.system(), "release": platform.release(), "version": platform.version(),
        "machine": platform.machine(), "hostname": platform.node(), "cpu": cpu,
        "machine_id_sha256": machine_id, "cpu_governors": governors, "cpu_boost": boost,
        "macos": macos,
        "logical_cpus": os.cpu_count(), "physical_cpus": physical_cpus, "memory_bytes": memory,
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "filesystem_block_bytes": os.statvfs(work).f_bsize if hasattr(os, "statvfs") else None,
    }}


def git_provenance(path: Path, require_tracked: bool) -> dict:
    try:
        root = Path(output(["git", "rev-parse", "--show-toplevel"], path.parent))
        relative = path.relative_to(root).as_posix()
        tracked = subprocess.run(["git", "ls-files", "--error-unmatch", "--", relative], cwd=root,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        compare.require(tracked or not require_tracked, f"public input is not tracked: {path}")
        head = output(["git", "rev-parse", "HEAD"], root)
        blob = subprocess.run(["git", "show", f"HEAD:{relative}"], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        head_sha256 = hashlib.sha256(blob.stdout).hexdigest() if blob.returncode == 0 else None
        current = file_fingerprint(path)
        at_head = head_sha256 == current["sha256"]
        return {"path": relative, "repository": str(root), "repository_head": head,
                "source_revision": head if tracked and at_head else None,
                "provenance": "tracked-head" if tracked and at_head else "tracked-modified" if tracked else "untracked",
                "head_sha256": head_sha256, "sha256": current["sha256"]}
    except subprocess.CalledProcessError:
        compare.require(not require_tracked, f"tracked input repository unavailable: {path}")
        return {"path": str(path), "provenance": "outside-git", "source_revision": None}


def below_target(path: Path) -> Path:
    path = path.resolve()
    compare.require(path != ROOT / "target" and path.is_relative_to((ROOT / "target").resolve()),
                    f"benchmark outputs/build/work directories must be beneath {ROOT / 'target'}")
    return path


def command_logged(command: list[str], logs: Path, name: str) -> None:
    with (logs / f"{name}.stdout.log").open("wb") as stdout, (logs / f"{name}.stderr.log").open("wb") as stderr:
        completed = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    compare.require(completed.returncode == 0,
                    f"{name} exited {completed.returncode}; see {logs / (name + '.stderr.log')}\n"
                    + (logs / f"{name}.stderr.log").read_text(errors="replace")[-6000:])


def fixtures(args: argparse.Namespace) -> list[dict]:
    directory = ROOT / "crates/suspect-codegen/tests/fixtures/m2"
    result = [
        {"id": "m2-small", "entry": directory / "canonical.openapi.yaml", "operations": [], "tracked": False},
        {"id": "split-recursive", "entry": directory / "split.openapi.yaml", "operations": [], "tracked": False},
    ]
    if args.suite == "public":
        compare.require(args.openrouter_root is not None, "--suite public requires --openrouter-root (or OPENROUTER_WEB_ROOT)")
        entry = (args.openrouter_root / PUBLIC_ENTRY).resolve(strict=True)
        git_provenance(entry, True)
        result.append({"id": "openrouter-public-five", "entry": entry, "operations": list(PUBLIC_OPERATIONS), "tracked": True})
    if args.fixture:
        result = [fixture for fixture in result if fixture["id"] in args.fixture]
        compare.require({fixture["id"] for fixture in result} == set(args.fixture), "selected fixture is absent from the suite")
    return result


def collect(args: argparse.Namespace) -> Path:
    started_at_ns = time.time_ns()
    out, work, build = (below_target(path) for path in (args.out, args.work_root, args.build_dir))
    compare.require(not out.is_relative_to(work) and not work.is_relative_to(out)
                    and not build.is_relative_to(work) and not work.is_relative_to(build)
                    and not out.is_relative_to(build) and not build.is_relative_to(out), "report/work/build directories must be disjoint")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.mkdir()  # Never overwrite somebody else's measurements.
    logs = out / "logs"
    logs.mkdir()
    raw_cases = out / "cases"
    raw_cases.mkdir()
    run_id = uuid.uuid4().hex
    created = datetime.now(timezone.utc).isoformat()
    work.parent.mkdir(parents=True, exist_ok=True)
    work.mkdir()  # Fixed source URI across calibration and candidate processes.
    marker = work / ".sdk-session-perf-owner.json"
    compare.write_new(marker, {"run_id": run_id})
    completed = False
    try:
        chosen = fixtures(args)
        groups = args.group or ["typescript-http", "all"]
        compare.require(len(groups) == len(set(groups)), "duplicate target group")
        source = source_manifest()
        harness = manifest(list(HARNESS_FILES))
        tools = tool_identity()
        cargo_config = cargo_configuration()
        env_names = sorted(name for name in os.environ if name.startswith(("CARGO_PROFILE_", "CARGO_TARGET_")))
        env_names += ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                      "RUSTUP_TOOLCHAIN", "CC", "CFLAGS", "CXX", "CXXFLAGS", "AR", "MACOSX_DEPLOYMENT_TARGET",
                      "SDKROOT", "CARGO_BUILD_TARGET", "CARGO_BUILD_RUSTFLAGS", "CARGO_HOME", "RUSTUP_HOME",
                      "RUSTC_BOOTSTRAP", "CARGO_INCREMENTAL", "CARGO_BUILD_INCREMENTAL", "CARGO_BUILD_JOBS"]
        build_config = {"profile": args.profile, "features": [], "locked": True,
                        "cargo_config_files": cargo_config,
                        "environment": {name: os.environ.get(name) for name in sorted(set(env_names))}}
        command = ["cargo", "build", "--locked", "-p", "suspect-codegen", "--example", "sdk_session_bench", "--target-dir", str(build)]
        if args.offline:
            command.append("--offline")
        if args.profile == "release":
            command.append("--release")
        build_command = list(command)
        command_logged(command, logs, "build")
        compare.require(source_manifest() == source and manifest(list(HARNESS_FILES)) == harness
                        and cargo_configuration() == cargo_config,
                        "source/harness changed during build; the binary is not attributable to a stable source tree")
        binary_directory = out / "bin"
        binary_directory.mkdir()
        binary = binary_directory / "sdk_session_bench"
        built = build / args.profile / "examples/sdk_session_bench"
        shutil.copy2(built, binary)
        binary_pin = file_fingerprint(binary)
        identity = {"runner": runner_identity(args.runner, work), "tools": tools, "build": build_config,
                    "harness": harness, "work_root": str(work)}
        if "expected_tools_sha256" in identity["runner"]["declared"]:
            compare.require(identity["runner"]["declared"]["expected_tools_sha256"] == compare.digest(tools), "tools differ from the runner reservation pin")
        compare.write_new(out / "attribution.json", {"run_id": run_id, "identity": identity,
                                                    "source": source, "binary": binary_pin,
                                                    "build_command": build_command})
        cases = []
        commands = []
        provenance = []
        for fixture in chosen:
            for group in groups:
                case_id = f"{fixture['id']}/{group}"
                case_root = work / f"{fixture['id']}-{group}"
                targets = BACKENDS if group == "all" else (group,)
                command = [str(binary), "--spec", str(fixture["entry"]), "--out", str(case_root),
                           "--fixture", fixture["id"], "--iterations", str(args.iterations), "--warmups", str(args.warmups),
                            "--targets", ",".join(targets), "--cache-entries", str(args.cache_entries), "--cache-bytes", str(args.cache_bytes)]
                command += ["--schedule", getattr(args, "schedule", "rotating")]
                if getattr(args, "attribution", False):
                    command.append("--attribution")
                if args.functional_only:
                    command.append("--functional-only")
                for operation in fixture["operations"]:
                    command.extend(("--operation-id", operation))
                commands.append(command)
                command_logged(command, logs, case_id.replace("/", "-"))
                report = compare.load(case_root / "report.json")
                prepared_root = out / "prepared-inputs" / case_id.replace("/", "-")
                for item in report["prepared_inputs"]:
                    relative = Path(item["path"])
                    compare.require(not relative.is_absolute() and ".." not in relative.parts, "invalid prepared input path")
                    source_path = Path(report["private_root"]) / relative
                    compare.require(file_fingerprint(source_path, item["path"]) == item, "prepared input changed before archival")
                    destination = prepared_root / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with destination.open("xb") as stream:
                        stream.write(source_path.read_bytes())
                    compare.require(file_fingerprint(destination, item["path"]) == item, "prepared input archive differs")
                case = {"id": case_id, "report": report, "prepared_input_root": str(prepared_root)}
                compare.write_new(raw_cases / f"{case_id.replace('/', '-')}.json", case)
                compare.validate_case(case)
                compare.require(report["binary"]["sha256"] == binary_pin["sha256"], "case binary differs from the frozen binary")
                for item in report["inputs"]:
                    path = Path(report["input_root"]) / item["path"]
                    compare.require(file_fingerprint(path, item["path"]) == item, f"input changed during case: {path}")
                    provenance.append({"case": case_id, **git_provenance(path, fixture["tracked"])})
                cases.append(case)
                print(f"{case_id}: {len(report['samples'])} checked samples; functional gates passed", flush=True)
        compare.require(file_fingerprint(binary) == binary_pin, "frozen binary changed during samples")
        compare.require(tool_identity() == tools, "tool identity changed during samples")
        compare.require(cargo_configuration() == cargo_config, "Cargo configuration changed during samples")
        compare.require(runner_identity(args.runner, work) == identity["runner"], "runner identity/policy changed during samples")
        compare.require(manifest(list(HARNESS_FILES)) == harness, "measurement harness changed during samples")
        for case in cases:
            report = case["report"]
            for item in report["inputs"]:
                compare.require(file_fingerprint(Path(report["input_root"]) / item["path"], item["path"]) == item,
                                "source input changed between cases")
        result = {
            "format": compare.SUITE_FORMAT, "run_id": run_id, "created_at": created,
            "functional_status": "passed", "performance_status": "not-measured" if args.functional_only else "observational",
            "process_id": os.getpid(), "started_at_ns": started_at_ns, "finished_at_ns": time.time_ns(),
            "integrity": {"source_at_build": True, "binary": True, "tools": True, "inputs": True},
            "identity": identity,
            "provenance": {"source": source, "binary": binary_pin,
                           "git_head_at_report": output(["git", "rev-parse", "HEAD"]), "inputs": provenance},
            "commands": {"build": build_command, "cases": commands},
            "cases": cases,
            "notes": [
                "Original files were only read. Case edits and artifact writes were confined to the owned work tree.",
                "Source fingerprints identify actual bytes immediately before and after compilation, including dirty/new files. The frozen binary remains attributable if unrelated development continues afterwards.",
                "Numerical observations are not a calibrated gate. Use compare.py with an explicit baseline and --gate on a qualified dedicated runner.",
                "Public means the tracked public entry with five selected HTTP operations; it does not claim all four corpus documents or all OpenAPI operations are admitted.",
            ],
        }
        compare.validate_report(result)
        compare.write_new(out / "report.json", result)
        completed = True
        return out / "report.json"
    except Exception as error:
        compare.write_new(out / "failure.json", {"format": compare.SUITE_FORMAT, "run_id": run_id,
                                                  "functional_status": "failed", "performance_status": "unmeasured",
                                                  "error": str(error), "work_root": str(work)})
        raise
    finally:
        if completed and not args.keep_work:
            compare.require(compare.load(marker) == {"run_id": run_id}, "private work ownership marker changed")
            shutil.rmtree(work)
        else:
            print(f"Private work retained at {work}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, help="new report directory under target/")
    parser.add_argument("--inspect-runner", type=Path, help="write observed machine identity; never declares or qualifies a dedicated runner")
    parser.add_argument("--work-root", type=Path, default=ROOT / "target/sdk-session-perf-work", help="new private work tree; use the same path across calibration/candidate runs")
    parser.add_argument("--build-dir", type=Path, default=ROOT / "target/sdk-session-perf-build")
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--suite", choices=("compact", "public"), default="compact")
    parser.add_argument("--openrouter-root", type=Path, default=Path(os.environ["OPENROUTER_WEB_ROOT"]) if "OPENROUTER_WEB_ROOT" in os.environ else None)
    parser.add_argument("--group", choices=(*BACKENDS, "all"), action="append", help="repeatable; default: typescript-http and all five canonical profiles")
    parser.add_argument("--fixture", choices=("m2-small", "split-recursive", "openrouter-public-five"), action="append", help="optional focused subset; full policy coverage is required for qualification")
    parser.add_argument("--runner", type=Path, help="explicit versioned runner declaration; default is observational")
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--cache-entries", type=int, default=4)
    parser.add_argument("--cache-bytes", type=int, default=268_435_456)
    parser.add_argument("--offline", action="store_true", help="require Cargo dependency cache")
    parser.add_argument("--keep-work", action="store_true", help="retain private copies/generated packages after success")
    parser.add_argument("--functional-only", action="store_true", help="one untimed checked cycle; not performance evidence")
    parser.add_argument("--attribution", action="store_true", help="record process CPU/IO/fault/scheduler counters outside timed intervals")
    parser.add_argument("--schedule", choices=("rotating", "fixed"), default="rotating", help="prospective scenario order; different schedules are incompatible workloads")
    args = parser.parse_args()
    try:
        if args.inspect_runner:
            compare.require(args.out is None, "choose --inspect-runner or --out")
            runner = runner_identity(None, ROOT / "target")
            compare.write_new(below_target(args.inspect_runner), {"format": "suspect-sdk-session-runner-inventory-v2", "status": "observational",
                                                                 "actual": runner["actual"], "actual_sha256": compare.digest(runner["actual"])})
            print(f"Observed runner identity written to {args.inspect_runner}; no qualification inferred")
            return 0
        compare.require(args.out is not None, "--out is required for execution")
        if args.functional_only:
            compare.require(not args.attribution, "--functional-only cannot include timing attribution")
            args.iterations, args.warmups = 1, 0
        compare.require(1 <= args.iterations <= 10000 and 0 <= args.warmups <= 100,
                        "iterations must be 1..10000 and warmups 0..100")
        compare.require(args.cache_entries >= 2 and args.cache_bytes > 0, "finite nonempty edit/revert cache budgets required")
        report = collect(args)
        print(f"Report: {report}\nPerformance status: {'not-measured' if args.functional_only else 'observational'}")
        return 0
    except (compare.ReportError, OSError, KeyError, ValueError, subprocess.SubprocessError) as error:
        print(f"SDK Session benchmark failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
