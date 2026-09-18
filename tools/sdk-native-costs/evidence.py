"""Create-once native observations and independently checkable raw evidence.

This module uses only the Python standard library. Timings are subprocess wall
observations, never a Rust test duration or a statistical qualification. Under
the repeated-v1 methodology each measured phase records a full run set: cold
runs, excluded warmups, then a steady-state batch. Every raw run is retained
with full attribution and every stored summary is recomputed from the retained
raw steady-state samples during validation. Historical single-sample receipts
still verify and summarize as their own one-run set.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import signal
import socket
import subprocess
import threading
import time
from typing import Any


FORMAT = "suspect.sdk.native-costs.v1"
PHASES = ("build", "import", "codec", "request")
ITERATIONS = {"build": 1, "import": 1, "codec": 16, "request": 3}
METHODOLOGY = "repeated-v1"
DEFAULT_RUN_POLICY = {"cold": 5, "warmup": 2, "steady": 20}
RUN_KINDS = ("cold", "warmup", "steady")
PRECISION = "100.50000000000000001"
TOKEN = "sdk-native-costs-loopback-only"
MARKER = "SUSPECT_NATIVE_COSTS "
REQUIRED_ENV = (
    "SUSPECT_SDK_FULL_PACKAGES",
    "SUSPECT_SDK_FULL_NATIVE_ROOT",
    "SUSPECT_SDK_FULL_TARGETS",
    "SUSPECT_SDK_FULL_SOURCE_SHA256",
    "SUSPECT_SDK_FULL_BINARY_SHA256",
    "SUSPECT_SDK_FULL_MEASUREMENTS",
)
TIERS = {
    "typescript": ["node22-ts55", "node24-ts59"],
    "rust": ["1.88.0", "stable"],
    "python": ["3.11", "3.14"],
    "go": ["1.23.12", "1.27.1"],
    "swift": ["6.0.3-sdk15.4", "6.3.3-sdk26.5"],
    "java": ["21.0.12.1", "25.0.4.1"],
    "csharp": ["8.0.424-net8.0", "10.0.400-net10.0"],
    "kotlin": ["2.4.20-jdk21", "2.4.20-jdk25"],
    "ruby": ["3.3.12", "4.0.6"],
    "php": ["8.3.32", "8.5.8"],
    "dart": ["3.9.4", "3.13.3"],
    "cpp": ["apple-clang21-cpp20-libcurl8.7.1"],
}


class EvidenceError(RuntimeError):
    """An unmet observation or evidence contract, including missing tools."""


def require(condition: Any, message: str) -> None:
    if not condition:
        raise EvidenceError(message)


def positive(value: Any) -> bool:
    return type(value) is int and 0 < value < 2**64


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def is_sha(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def read_json(path: Path) -> Any:
    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key!r} in {path}")
            result[key] = value
        return result

    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique)


def write_new(path: Path, data: bytes | str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(data.encode("utf-8") if isinstance(data, str) else data)


def write_json(path: Path, data: Any) -> None:
    write_new(path, json.dumps(data, indent=2, sort_keys=True, ensure_ascii=True) + "\n")


def relative_file(root: Path, name: Any) -> Path:
    require(isinstance(name, str) and name and "\\" not in name, "invalid evidence path")
    pure = PurePosixPath(name)
    require(not pure.is_absolute() and all(p not in ("", ".", "..") for p in name.split("/")),
            f"non-confined evidence path: {name}")
    path = root.joinpath(*pure.parts)
    require(path.resolve(strict=True).is_relative_to(root.resolve(strict=True)),
            f"evidence path escapes root: {name}")
    require(path.is_file(), f"not an evidence file: {name}")
    return path


def inventory(root: Path) -> dict[str, dict[str, Any]]:
    """Census regular files, rejecting links/devices rather than following inputs."""
    require(root.is_dir() and not root.is_symlink(), f"missing/nonliteral input directory: {root}")
    files = {}
    for parent, directories, names in os.walk(root, followlinks=False):
        for name in directories + names:
            path = Path(parent, name)
            require(not path.is_symlink(), f"symbolic link in immutable package: {path}")
        for name in sorted(names):
            path = Path(parent, name)
            require(path.is_file(), f"nonregular package file: {path}")
            files[path.relative_to(root).as_posix()] = {"sha256": sha(path), "bytes": path.stat().st_size}
    require(files, f"empty input tree: {root}")
    return dict(sorted(files.items()))


def file_record(path: Path) -> dict[str, Any]:
    path = path.absolute()
    require(path.is_file(), f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def verify_file(record: dict[str, Any]) -> None:
    path = Path(record["path"])
    require(path.is_absolute() and path.is_file(), f"missing absolute artifact: {path}")
    require(type(record["bytes"]) is int and record["bytes"] >= 0, f"invalid byte size: {path}")
    require(path.stat().st_size == record["bytes"] and sha(path) == record["sha256"],
            f"artifact/executable changed: {path}")


def targets_from(path: Path) -> list[dict[str, Any]]:
    targets = read_json(path)
    require(isinstance(targets, list) and len(targets) == len(TIERS), "exactly twelve maintained SDK targets required")
    seen = set()
    for target in targets:
        language = target.get("language")
        require(language in TIERS and language not in seen, f"unknown/duplicate SDK language: {language}")
        seen.add(language)
        require(target.get("toolchain_tiers") == TIERS[language], f"unmaintained toolchain tiers for {language}")
        name = target.get("manifest", "")
        require(isinstance(name, str) and name and Path(name).name == name and name not in (".", ".."),
                f"invalid package manifest path for {language}")
        require(isinstance(target.get("package_name"), str) and target["package_name"], "package_name required")
    return targets


def dimensions(targets: list[dict[str, Any]]) -> set[tuple[str, str, str]]:
    return {(t["language"], tier, phase) for t in targets for tier in t["toolchain_tiers"] for phase in PHASES}


def run_policy(cold: Any, warmup: Any, steady: Any) -> dict[str, int]:
    """Validate the repeated-run flag triple: N cold >= 1, W warmups >= 0, M steady >= 1."""
    for name, value, minimum in (("cold", cold, 1), ("warmup", warmup, 0), ("steady", steady, 1)):
        require(type(value) is int and value >= minimum,
                f"{name} run count must be an integer >= {minimum} (--cold-runs/--warmups/--steady-runs)")
    return {"cold": cold, "warmup": warmup, "steady": steady}


def run_statistics(values: list[int]) -> dict[str, Any]:
    """Exact aggregates over raw nanosecond durations of one steady-state batch.

    Percentiles interpolate linearly between closest ranks, the mean uses
    compensated summation and stddev is the population stddev. Validation
    recomputes receipts with this same function from the retained raw samples,
    so the definition lives in exactly one place.
    """
    require(bool(values) and all(type(value) is int and value > 0 for value in values),
            "statistics require positive raw nanosecond samples")
    ordered = sorted(values)
    count = len(ordered)

    def percentile(part: float) -> float:
        if count == 1:
            return float(ordered[0])
        rank = (count - 1) * part
        low, high = math.floor(rank), math.ceil(rank)
        if low == high:
            return float(ordered[low])
        return ordered[low] + (ordered[high] - ordered[low]) * (rank - low)

    mean = math.fsum(ordered) / count
    variance = math.fsum((value - mean) ** 2 for value in ordered) / count
    return {"runs": count, "min": ordered[0], "max": ordered[-1], "median": percentile(0.5),
            "p90": percentile(0.9), "p99": percentile(0.99), "mean": mean, "stddev": math.sqrt(variance)}


def check_summary(summary: Any) -> None:
    """Shape and self-consistency of a stored steady-state summary."""
    require(isinstance(summary, dict) and set(summary) == {"runs", "min", "max", "median", "p90", "p99", "mean", "stddev"},
            "summary must record exactly the agreed statistics fields")
    require(type(summary["runs"]) is int and summary["runs"] >= 1, "summary runs must be a positive integer")
    for key in ("min", "max", "median", "p90", "p99", "mean"):
        value = summary[key]
        require(type(value) in (int, float) and math.isfinite(value) and value > 0,
                f"summary {key} must be a finite positive number")
    require(type(summary["stddev"]) in (int, float) and math.isfinite(summary["stddev"]) and summary["stddev"] >= 0,
            "summary stddev must be a finite nonnegative number")
    require(summary["min"] <= summary["median"] <= summary["p90"] <= summary["p99"] <= summary["max"],
            "summary order statistics are inconsistent")
    require(summary["min"] <= summary["mean"] <= summary["max"], "summary mean escapes the observed range")


def verify_summary(stored: Any, recomputed: dict[str, Any], label: str) -> None:
    """Reject receipts whose stored summary disagrees with recomputed raw statistics."""
    check_summary(stored)
    for key, value in recomputed.items():
        require(type(stored[key]) is type(value) and stored[key] == value,
                f"{label}: summary {key} disagrees with the raw steady-state samples")


def verify_run_set(row: dict[str, Any]) -> None:
    """Adversarial check of a repeated-v1 run set against its retained raw runs."""
    runs = row.get("runs")
    require(isinstance(runs, dict) and set(runs) == set(RUN_KINDS),
            "repeated-run row must declare exactly cold, warmup and steady counts")
    policy = run_policy(runs["cold"], runs["warmup"], runs["steady"])
    kinds: dict[str, list[int]] = {kind: [] for kind in RUN_KINDS}
    for sample in row["samples"]:
        kind = sample.get("runKind") if isinstance(sample, dict) else None
        require(kind in kinds, "repeated-run sample lacks a valid runKind tag")
        kinds[kind].append(sample["nanoseconds"])
    retained = {kind: len(values) for kind, values in kinds.items()}
    require(retained == policy, f"retained run kinds {retained} disagree with the declared run set {policy}")
    require(isinstance(row.get("summary"), dict), "repeated-run row lacks a summary")
    verify_summary(row["summary"], run_statistics(kinds["steady"]),
                   f"{row.get('language')}/{row.get('tier')}/{row.get('phase')}")


def run_set_summary(row: dict[str, Any]) -> dict[str, Any]:
    """Read-side normalization: a historical single-sample row is its own one-run set."""
    if row.get("methodology") == METHODOLOGY:
        return dict(row["summary"])
    return run_statistics([sample["nanoseconds"] for sample in row["samples"]])


def required_environment(env: dict[str, str]) -> None:
    for key in REQUIRED_ENV:
        require(bool(env.get(key)), f"{key} is required when the native test is explicitly run")
    for key in ("SUSPECT_SDK_FULL_SOURCE_SHA256", "SUSPECT_SDK_FULL_BINARY_SHA256"):
        require(is_sha(env[key]), f"{key} must be a lowercase SHA-256")
    for key in REQUIRED_ENV[:3] + REQUIRED_ENV[-1:]:
        require(Path(env[key]).is_absolute(), f"{key} must be an absolute path")


class Commands:
    """One bounded process group per recorded command; logs survive every failure."""

    def __init__(self, root: Path):
        self.root = root
        self.count = 0
        self.records: list[dict[str, Any]] = []

    def run(self, argv: list[str | Path], cwd: Path, env: dict[str, str], *, label: str,
            timeout: float = 180, check: bool = True) -> dict[str, Any]:
        argv = [str(arg) for arg in argv]
        require(argv and Path(argv[0]).is_absolute(), f"absolute executable required: {argv}")
        self.count += 1
        stem = f"commands/{self.count:04d}"
        record: dict[str, Any] = {"label": label, "command": argv, "cwd": str(cwd), "environment": dict(env),
                                  "stdout": stem + ".stdout.log", "stderr": stem + ".stderr.log",
                                  "record": stem + ".json", "timeoutSeconds": timeout}
        start = time.perf_counter_ns()
        process = None
        stdout, stderr = b"", b""
        try:
            record["programSha256"] = sha(Path(argv[0]))
            start = time.perf_counter_ns()
            process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       start_new_session=True)
            try:
                stdout, stderr = process.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                record["timedOut"] = True
                self._terminate(process)
                stdout, stderr = process.communicate(timeout=5)
            record["exitCode"] = process.returncode
        except (OSError, subprocess.SubprocessError) as error:
            record["error"] = f"{type(error).__name__}: {error}"
            record["exitCode"] = None
        except BaseException:
            if process is not None:
                self._terminate(process)
                stdout, stderr = process.communicate(timeout=5)
                record["exitCode"] = process.returncode
            record["interrupted"] = True
            raise
        finally:
            record["nanoseconds"] = time.perf_counter_ns() - start
            write_new(self.root / record["stdout"], stdout)
            write_new(self.root / record["stderr"], stderr)
            record["stdoutSha256"] = sha_bytes(stdout)
            record["stderrSha256"] = sha_bytes(stderr)
            if "programSha256" in record and Path(argv[0]).is_file():
                record["programUnchanged"] = sha(Path(argv[0])) == record["programSha256"]
            self.records.append(record)
            write_json(self.root / record["record"], record)
        if check:
            require(record.get("exitCode") == 0 and not record.get("timedOut") and record.get("programUnchanged"),
                    f"{label}: command failed; inspect {record['record']} and raw logs")
        return record

    @staticmethod
    def _terminate(process: subprocess.Popen[bytes]) -> None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
        # Descendants can hold pipes after their parent exits. Kill the owned
        # group even when wait() already observed the leader's termination.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def stdout(self, record: dict[str, Any]) -> str:
        return (self.root / record["stdout"]).read_text(encoding="utf-8")


def witness(text: str, phase: str, iterations: int) -> dict[str, Any]:
    rows = [json.loads(line[len(MARKER):]) for line in text.splitlines() if line.startswith(MARKER)]
    require(len(rows) == 1, "native consumer must emit exactly one completed-work witness")
    value = rows[0]
    require(value.get("phase") == phase and type(value.get("iterations")) is int
            and value["iterations"] == iterations, "native work count/phase differs")
    if phase == "import":
        require(value.get("loaded") is True, "module/linked consumer did not load")
    else:
        require(value.get("precision") == PRECISION, "native typed response lost the independent decimal token")
        if phase == "codec":
            require(positive(value.get("encodedBytes")), "native codec did not encode any bytes")
    return value


class RecordingServer:
    """Finite, owned HTTP/1 loopback peer, with exact independent wire checks.

    A small socket server avoids global HTTP-server state, accepts no remote bind,
    caps request headers/bodies, and closes every connection. Repeated requests
    also exercise the SDK's no-cookie-persistence default.
    """

    def __init__(self, response: bytes, expected: int):
        self.response = response
        self.expected = expected
        self.records: list[dict[str, Any]] = []
        self.errors: list[str] = []
        self.stop = threading.Event()
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(0.1)
        self.port = self.listener.getsockname()[1]
        self.url = f"http://127.0.0.1:{self.port}/api/v1"
        self.active: socket.socket | None = None
        self.worker = threading.Thread(target=self._serve, name="sdk-native-costs-loopback", daemon=True)

    def __enter__(self) -> RecordingServer:
        self.worker.start()
        return self

    def _serve(self) -> None:
        while not self.stop.is_set():
            try:
                connection, address = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            with connection:
                self.active = connection
                connection.settimeout(2)
                try:
                    self._exchange(connection, address)
                except (OSError, ValueError, EvidenceError) as error:
                    self.errors.append(f"{type(error).__name__}: {error}")
                finally:
                    self.active = None

    def _exchange(self, connection: socket.socket, address: tuple[str, int]) -> None:
        require(address[0] == "127.0.0.1", "non-loopback peer")
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = connection.recv(4096)
            require(chunk, "incomplete HTTP headers")
            data += chunk
            require(len(data) <= 65536, "request header budget exceeded")
        head, body = data.split(b"\r\n\r\n", 1)
        lines = head.decode("ascii").split("\r\n")
        method, target, version = lines[0].split(" ")
        headers: dict[str, list[str]] = {}
        for line in lines[1:]:
            name, value = line.split(":", 1)
            headers.setdefault(name.lower(), []).append(value.strip())
        require("transfer-encoding" not in headers, "GET has transfer encoding")
        length = headers.get("content-length", ["0"])
        require(len(length) == 1 and length[0].isdigit() and int(length[0]) <= 4096, "invalid GET length")
        while len(body) < int(length[0]):
            chunk = connection.recv(int(length[0]) - len(body))
            require(chunk, "truncated request body")
            body += chunk
        record = {"method": method, "target": target, "version": version, "headers": headers,
                  "bodyHex": body.hex(), "responseSha256": sha_bytes(self.response)}
        self.records.append(record)
        require(len(self.records) <= self.expected, "SDK made extra requests/retries")
        require(method == "GET" and target == "/api/v1/credits" and version == "HTTP/1.1", "wrong wire request")
        require(headers.get("host") == [f"127.0.0.1:{self.port}"], "request escaped owned server")
        require(headers.get("authorization") == [f"Bearer {TOKEN}"], "wrong/missing/duplicate SDK auth")
        require(headers.get("accept") == ["application/json"], "SDK response media was not selected")
        require(body == b"" and int(length[0]) == 0, "GET sent a body")
        require("cookie" not in headers and "content-type" not in headers, "unexpected cookie/GET content type")
        reply = (f"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {len(self.response)}\r\n"
                 "Connection: close\r\nSet-Cookie: native_costs=must_not_persist; Path=/\r\n\r\n").encode("ascii")
        connection.sendall(reply + self.response)

    def __exit__(self, *_: Any) -> None:
        self.stop.set()
        self.listener.close()
        if self.active is not None:
            try:
                self.active.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.worker.join(timeout=3)
        require(not self.worker.is_alive(), "loopback server did not terminate")

    def verify(self) -> None:
        require(not self.errors, f"loopback wire failures: {self.errors}")
        require(len(self.records) == self.expected, f"expected {self.expected} requests, saw {len(self.records)}")


def validate_summary_view(root: Path, report: dict[str, Any], targets: list[dict[str, Any]], packages: Path,
                          source: str, cli: str, *, allow_incomplete: bool) -> dict[str, Any]:
    """A --report summary view must agree, row for row, with its fully verified raw document."""
    require(report.get("reportView") == "summary", "unknown report view selector")
    require(report.get("format") == FORMAT and report.get("methodology") == METHODOLOGY,
            "summary view format/methodology differs")
    raw_name = report.get("rawReport")
    require(isinstance(raw_name, str) and raw_name, "summary view lacks its raw report pointer")
    raw_path = relative_file(root, raw_name)
    require(sha(raw_path) == report.get("rawReportSha256"), "summary view raw-report digest differs")
    require("reportView" not in read_json(raw_path), "summary view must point at a raw report, not another view")
    raw = validate_report(root, targets, packages, source, cli, allow_incomplete=allow_incomplete, report_name=raw_name)
    for key in ("sourceFingerprint", "cliSha256", "complete", "inputInventory", "requiredDimensionCount",
                "missing", "failures", "selection"):
        require(report.get(key) == raw.get(key), f"summary view header disagrees with the raw report: {key}")
    rows = report.get("measurements")
    require(isinstance(rows, list) and len(rows) == len(raw["measurements"]),
            "summary view row count disagrees with the raw report")
    for view_row, raw_row in zip(rows, raw["measurements"]):
        require(isinstance(view_row, dict), "summary view rows must be objects")
        require("samples" not in view_row, "summary view must not duplicate raw samples")
        for key in ("language", "tier", "phase", "methodology", "runs", "summary", "artifactBytes"):
            require(view_row.get(key) == raw_row.get(key),
                    f"summary view row disagrees with the raw report: "
                    f"{view_row.get('language')}/{view_row.get('tier')}/{view_row.get('phase')}: {key}")
    return report


def validate_report(root: Path, targets: list[dict[str, Any]], packages: Path,
                    source: str, cli: str, *, allow_incomplete: bool = False,
                    report_name: str = "report.json") -> dict[str, Any]:
    """Check raw observations, dimensions, package bytes and command attribution."""
    report = read_json(relative_file(root, report_name))
    if report.get("reportView") is not None:
        return validate_summary_view(root, report, targets, packages, source, cli, allow_incomplete=allow_incomplete)
    require(report.get("format") == FORMAT, "wrong native report format")
    require(report.get("methodology", METHODOLOGY) == METHODOLOGY, "unknown native report methodology")
    require(report.get("sourceFingerprint") == source and report.get("cliSha256") == cli,
            "native report belongs to different source/CLI")
    require(type(report.get("complete")) is bool, "missing complete flag")
    require(allow_incomplete or report["complete"], "native report is incomplete")
    inputs = read_json(relative_file(root, report["inputInventory"]))
    for path, before in inputs["trees"].items():
        require(inventory(Path(path)) == before, f"immutable input changed: {path}")
    for record in inputs["files"]:
        verify_file(record)
    required = dimensions(targets)
    found = set()
    for row in report["measurements"]:
        key = (row["language"], row["tier"], row["phase"])
        require(key in required and key not in found, f"duplicate/unknown dimension: {key}")
        found.add(key)
        target = next(t for t in targets if t["language"] == row["language"])
        require(row["packageManifestSha256"] == sha(packages / row["language"] / target["manifest"]),
                f"package manifest changed: {key}")
        manifest_path = relative_file(root, row["artifactManifest"])
        require(sha(manifest_path) == row["artifactManifestSha256"], "artifact inventory digest differs")
        artifacts = read_json(manifest_path)
        require(artifacts and len({a["path"] for a in artifacts}) == len(artifacts), "empty/duplicate artifact inventory")
        for artifact in artifacts:
            verify_file(artifact)
        require(positive(row["artifactBytes"]) and row["artifactBytes"] == sum(a["bytes"] for a in artifacts),
                "native artifact byte count differs")
        samples = row["samples"]
        require(isinstance(samples, list) and samples, "native observation lacks samples")
        for sample in samples:
            require(positive(sample["nanoseconds"]) and positive(sample["iterations"])
                    and type(sample["exitCode"]) is int and sample["exitCode"] == 0, "failed/untimed native observation")
            argv = sample["command"]
            require(isinstance(argv, list) and argv and all(isinstance(a, str) for a in argv), "invalid native argv")
            executable = Path(argv[0])
            require(executable.is_absolute() and sha(executable) == sample["programSha256"], "native executable changed")
            command = read_json(relative_file(root, sample["record"]))
            for field in ("command", "exitCode", "nanoseconds", "programSha256", "stdout", "stderr", "stdoutSha256", "stderrSha256"):
                require(command[field] == sample[field], f"sample attribution differs from subprocess: {field}")
            require(command.get("programUnchanged") is True and not command.get("timedOut")
                    and not command.get("interrupted"), "failed/interrupted native subprocess")
            for stream in ("stdout", "stderr"):
                require(sha(relative_file(root, sample[stream])) == sample[stream + "Sha256"], "raw log digest differs")
            require(sample["iterations"] == ITERATIONS[row["phase"]], "unmaintained iteration count")
            if row["phase"] != "build":
                witness(relative_file(root, sample["stdout"]).read_text(), row["phase"], sample["iterations"])
            if row["phase"] == "request":
                wire_path = relative_file(root, sample["wire"])
                require(sha(wire_path) == sample["wireSha256"], "wire record digest differs")
                wire = read_json(wire_path)
                require(not wire["errors"] and wire.get("threadTerminated") is True
                        and len(wire["requests"]) == sample["iterations"], "request witness/cleanup is incomplete")
                for request in wire["requests"]:
                    require(request["method"] == "GET" and request["target"] == "/api/v1/credits"
                            and request["bodyHex"] == "", "incorrect recorded request")
                    headers = request["headers"]
                    require(headers.get("authorization") == [f"Bearer {TOKEN}"] and headers.get("accept") == ["application/json"]
                            and "cookie" not in headers and "content-type" not in headers, "incorrect recorded auth/body headers")
                    require(request["responseSha256"] == report["fixture"]["creditsSha256"], "response fixture attribution differs")
        if "methodology" in row:
            require(row["methodology"] == METHODOLOGY, f"unknown measurement methodology: {row['methodology']!r}")
            verify_run_set(row)
        else:
            # Historical single-sample receipt: each sample is fully attributed
            # above and the row summarizes as its own one-run set.
            require(all("runKind" not in sample for sample in samples),
                    "run-kind tags require the declared repeated-run methodology")
    if report["complete"]:
        require(found == required and not report["failures"], "complete report has missing rows/failures")
    else:
        require(found != required or report["failures"], "incomplete report lacks an explanation")
    return report


def summary_view(report: dict[str, Any], raw_name: str, raw_digest: str) -> dict[str, Any]:
    """Derived aggregate view of a raw evidence document.

    The complete raw document is always retained beside the view; validation
    re-verifies the raw document and rejects any view row that disagrees with
    it. Aggregates alone are never accepted as evidence.
    """
    require(report.get("methodology") == METHODOLOGY, "summary views require the repeated-run methodology")
    return {
        "format": FORMAT, "reportView": "summary", "methodology": METHODOLOGY,
        "complete": report["complete"], "sourceFingerprint": report["sourceFingerprint"], "cliSha256": report["cliSha256"],
        "inputInventory": report["inputInventory"], "rawReport": raw_name, "rawReportSha256": raw_digest,
        "requiredDimensionCount": report["requiredDimensionCount"], "missing": report["missing"],
        "failures": report["failures"], "selection": report["selection"],
        "measurements": [{key: row[key] for key in ("language", "tier", "phase", "methodology", "runs", "summary",
                                                    "artifactBytes")} for row in report["measurements"]],
    }
