#!/usr/bin/env python3
"""Seal a read-only inventory of the prepared demo, without replaying native checks."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from demo import COMPARISON_DIR, REPO, fresh_directory, owned_root, sha, verify_pins, write_json
from native import LANGUAGES


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    root = owned_root(args.root)
    pins = verify_pins(root)
    package_pins = json.loads((root / "package-pins.json").read_text())
    for name, pin in package_pins.items():
        if sha(root / "packages" / name) != pin["sha256"]:
            raise SystemExit(f"Generated package changed: {name}")
    index = json.loads((root / "native-index.json").read_text())
    if set(index) != set(LANGUAGES):
        raise SystemExit("Twelve successful native preparations are required.")
    files: dict[str, str] = {}

    def retain(path: Path) -> None:
        if path.is_file():
            files[str(path)] = sha(path)

    for path in [REPO / "DEMO-README.md", REPO / "examples/sdk-demo-all.json"]:
        retain(path)
    for directory in (REPO / "tools/sdk-demo", REPO / "examples/sdk-demo-all"):
        for path in directory.rglob("*"):
            if "__pycache__" not in path.parts:
                retain(path)
    for name in ("pins.json", "session.json", "package-pins.json", "native-index.json", "zero-rewrite.json",
                 f"{COMPARISON_DIR}/cli.json", f"{COMPARISON_DIR}/origin-receipt.json", f"{COMPARISON_DIR}/package-parity.json",
                 "preservation-fix-01/result.json", "standard-stream/prepared.json", "terraform-demo/prepared.json",
                 "terraform-demo/native-artifact-parity.json"):
        retain(root / name)

    native = {}
    for language, info in index.items():
        attempt = Path(info["attempt"])
        source_pins = json.loads((attempt / "generated-source-pins.json").read_text())
        for name, digest in source_pins.items():
            if sha(Path(info["package"]) / name) != digest:
                raise SystemExit(f"Native package copy changed: {language}/{name}")
        for source, digest in info["snippets"].items():
            if sha(REPO / source) != digest:
                raise SystemExit(f"Checked consumer changed: {source}")
        retain(attempt / "prepared.json")
        retain(attempt / "generated-source-pins.json")
        for log in (attempt / "commands").rglob("*"):
            retain(log)
        for pattern in ("*.tgz", "*.gem", "dist/*.whl", "feed/*.nupkg"):
            for archive in attempt.glob(pattern):
                retain(archive)
        argv_files = set()
        for argument in info["argv"]:
            for text in argument.split(os.pathsep):
                path = Path(text)
                if path.is_absolute() and path.is_file():
                    retain(path)
                    argv_files.add(str(path))
                elif path.is_absolute() and path.is_dir() and path.is_relative_to(attempt):
                    for child in path.rglob("*"):
                        retain(child)
        if language == "csharp":
            for file in (Path(info["consumer"]) / "bin/Release/net8.0").glob("*"):
                retain(file)
        native[language] = {"prepared": str(attempt / "prepared.json"), "argv": info["argv"],
                            "cwd": info["cwd"], "executionFiles": sorted(argv_files)}

    runs = {}
    for language in (*LANGUAGES, "javascript"):
        runs[language] = {}
        for phase, count in (("offline", 4), ("typed-error", 1)):
            candidates = sorted((root / "runs").glob(f"{language}-{phase}-*"),
                                key=lambda path: int(path.name.rsplit("-", 1)[1]), reverse=True)
            accepted = None
            for candidate in candidates:
                command_path = candidate / "commands/run-01.json"
                wire_path = candidate / "wire.json"
                if not command_path.is_file() or not wire_path.is_file():
                    continue
                command = json.loads(command_path.read_text())
                wire = json.loads(wire_path.read_text())
                if command["exitCode"] == 0 and not wire["failures"] and len(wire["requests"]) == count:
                    accepted = candidate
                    break
            if accepted is None:
                raise SystemExit(f"Missing completed {language} {phase} receipt")
            for file in accepted.rglob("*"):
                retain(file)
            runs[language][phase] = {"receipt": str(accepted), "requests": count}

    checked = sorted((root / "readme-check").glob("check-*/report.json"))[-1]
    check = json.loads(checked.read_text())
    if check["failures"] or check["readmeSha256"] != sha(REPO / "DEMO-README.md"):
        raise SystemExit("Run check_readme.py against the final README before sealing.")
    for file in checked.parent.rglob("*"):
        retain(file)
    for directory in (root / "iteration-rehearsal-01" / COMPARISON_DIR / "run-01", root / "iteration-rehearsal-01/watch-01",
                      root / "terraform-demo/runs/rehearsal-01", root / "standard-stream/commands"):
        for file in directory.rglob("*"):
            retain(file)
    destination = fresh_directory(root, "delivery")
    write_json(destination / "SHA256SUMS.json", files)
    write_json(destination / "REPORT.json", {
        "format": "suspect.sdk.demo.readme.delivery.v1", "status": "ready",
        "readme": str(REPO / "DEMO-README.md"), "readmeSha256": sha(REPO / "DEMO-README.md"),
        "config": str(REPO / "examples/sdk-demo-all.json"), "root": str(root),
        "generationCliSha256": pins["binarySha256"],
        "comparisonCli": json.loads((root / COMPARISON_DIR / "cli.json").read_text()),
        "desiredSdkArtifacts": len(package_pins) - 1, "nativeTargets": 12, "consumerExecutions": 13,
        "latestSuccessfulLoopbackRequests": 65, "native": native, "runs": runs,
        "readmeCheck": str(checked), "sealedFiles": len(files),
        "morningDemoBlockers": [],
        "scope": "local ordinary five-operation demo; no production requests or full acceptance/performance claim",
        "nonBlockingFollowups": ["Terraform independent-review repairs/rechecks", "Main full-plan quality/acceptance and qualified numerical gates"],
    })
    print(f"DEMO SEALED: {destination}; {len(files)} file hashes; twelve targets plus JavaScript; no morning demo blocker.")


if __name__ == "__main__":
    main()
