#!/usr/bin/env python3
"""Bounded, local SDK demo preparation; no native acceptance or remote API calls."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO.parent / "openrouter-web"
CONFIG = REPO / "examples/sdk-demo-all.json"
COMPARISON_DIR = "comparison-final-02"
TRACKED = (
    "projects/docs/openapi/openapi.yaml",
    "openrouter-management.openapi.yaml",
    "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
    "packages/temporal/benchmarks.openapi.json",
)


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def fresh_directory(parent: Path, label: str) -> Path:
    parent.mkdir(parents=True, exist_ok=True)
    number = 1
    while True:
        path = parent / f"{label}-{number:02}"
        try:
            path.mkdir()
            return path
        except FileExistsError:
            number += 1


def owned_root(value: str) -> Path:
    root = Path(value).absolute()
    target = REPO / "target"
    if not root.is_relative_to(target) or not root.relative_to(target).parts[0].startswith(
        "sdk-demo-readme-20260911-"
    ):
        raise SystemExit("Choose a fresh target/sdk-demo-readme-20260911-* root.")
    if root.resolve() != root:
        raise SystemExit("Demo evidence roots must not contain symlinks.")
    return root


def record(root: Path, label: str, argv: list[str], *, cwd: Path = REPO,
           env: dict[str, str] | None = None, expected: tuple[int, ...] = (0,),
           timeout: int = 300, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    logs = root / "commands"
    logs.mkdir(parents=True, exist_ok=True)
    count = 1
    while (logs / f"{label}-{count:02}.json").exists():
        count += 1
    prefix = logs / f"{label}-{count:02}"
    merged = dict(os.environ)
    merged.pop("OPENROUTER_API_KEY", None)
    merged.update(env or {})
    timed_out = False
    try:
        result = subprocess.run(argv, cwd=cwd, env=merged, capture_output=True,
                                text=True, timeout=timeout, input=input_text)
    except subprocess.TimeoutExpired as error:
        timed_out = True
        def decoded(value: str | bytes | None) -> str:
            return value.decode(errors="replace") if isinstance(value, bytes) else value or ""
        result = subprocess.CompletedProcess(argv, 124, decoded(error.stdout), decoded(error.stderr))
    if input_text is not None:
        prefix.with_suffix(".stdin").write_text(input_text)
    prefix.with_suffix(".stdout").write_text(result.stdout)
    prefix.with_suffix(".stderr").write_text(result.stderr)
    write_json(prefix.with_suffix(".json"), {
        "argv": argv, "cwd": str(cwd), "environmentOverrides": env or {},
        "exitCode": result.returncode, "expectedExitCodes": list(expected),
        "timedOut": timed_out,
        "stdout": str(prefix.with_suffix(".stdout")),
        "stderr": str(prefix.with_suffix(".stderr")),
        "stdoutSha256": sha(prefix.with_suffix(".stdout")),
        "stderrSha256": sha(prefix.with_suffix(".stderr")),
        "stdinSha256": sha(prefix.with_suffix(".stdin")) if input_text is not None else None,
    })
    if result.returncode not in expected:
        sys.stderr.write(result.stderr[-6000:])
        sys.stderr.write(result.stdout[-6000:])
        raise SystemExit(f"{label} exited {result.returncode}; retained {prefix}.json")
    return result


def freeze(root: Path, binary: Path, receipt: Path | None) -> None:
    root.mkdir(parents=True, exist_ok=False)
    (root / "bin").mkdir()
    shutil.copy2(binary, root / "bin/suspect")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=SOURCE,
                                       text=True).strip()
    sources = []
    for relative in TRACKED:
        path = SOURCE / relative
        head = subprocess.check_output(["git", "show", f"HEAD:{relative}"], cwd=SOURCE)
        if path.read_bytes() != head:
            raise SystemExit(f"Tracked source differs from HEAD: {path}; attempt retained.")
        snapshot = root / "sources" / relative
        snapshot.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, snapshot)
        sources.append({"path": str(path), "snapshot": str(snapshot),
                        "classification": "tracked-head", "sha256": sha(path),
                        "sourceRevision": revision, "bytes": path.stat().st_size})
    config = json.loads(CONFIG.read_text())
    config["spec"] = str(SOURCE / TRACKED[0])
    write_json(root / "session.json", config)
    if receipt:
        shutil.copy2(receipt, root / "main-cli-receipt.json")
    write_json(root / "pins.json", {
        "scope": "bounded SDK demo; not full acceptance or performance qualification",
        "binaryOrigin": str(binary), "binarySha256": sha(root / "bin/suspect"),
        "mainReceipt": str(receipt) if receipt else None,
        "configOrigin": str(CONFIG), "configSha256": sha(CONFIG),
        "sessionSha256": sha(root / "session.json"), "sources": sources,
        "responseFixture": str(REPO / "crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json"),
        "responseFixtureSha256": sha(REPO / "crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json"),
    })
    result = record(root, "profiles", [str(root / "bin/suspect"), "codegen-profiles",
                                       "--format", "json"])
    write_json(root / "profiles.json", json.loads(result.stdout))
    print(f"Frozen demo candidate: {root}\nCLI SHA-256: {sha(root / 'bin/suspect')}")


def verify_pins(root: Path) -> dict:
    pins = json.loads((root / "pins.json").read_text())
    if sha(root / "bin/suspect") != pins["binarySha256"]:
        raise SystemExit("Frozen CLI hash changed.")
    if sha(root / "session.json") != pins["sessionSha256"]:
        raise SystemExit("Frozen session config hash changed.")
    for source in pins["sources"]:
        if sha(Path(source["path"])) != source["sha256"]:
            raise SystemExit(f"Source hash changed: {source['path']}")
    return pins


def freeze_comparison(root: Path, binary: Path, receipt: Path) -> None:
    base = verify_pins(root)
    origin = json.loads(receipt.read_text())
    if sha(binary) != origin["binarySha256"]:
        raise SystemExit("Comparison binary differs from its owner receipt.")
    work = root / COMPARISON_DIR
    work.mkdir(exist_ok=False)
    (work / "bin").mkdir()
    shutil.copy2(binary, work / "bin/suspect")
    shutil.copy2(receipt, work / "origin-receipt.json")
    result = record(work, "generation-parity", [str(work / "bin/suspect"), "codegen-session",
        "--config", str(root / "session.json"), "--out", str(work / "packages"), "--format", "json"])
    data = json.loads(result.stdout)
    expected = {name: item["sha256"] for name, item in json.loads((root / "package-pins.json").read_text()).items()}
    actual = {str(path.relative_to(work / "packages")): sha(path)
              for path in (work / "packages").rglob("*") if path.is_file()}
    changed = [name for name in sorted(set(expected) | set(actual)) if expected.get(name) != actual.get(name)]
    write_json(work / "package-parity.json", {"desiredArtifacts": len(data["changedArtifacts"]),
        "filesIncludingOwnership": len(actual), "changed": changed, "allBytesEqual": not changed})
    if changed:
        raise SystemExit("Comparison-only candidate changed package bytes; keep the native demo pinned.")
    write_json(work / "cli.json", {"binaryOrigin": str(binary), "binarySha256": sha(binary),
        "originReceiptSha256": sha(receipt), "baseBinarySha256": base["binarySha256"],
        "scope": "additive compatibility capture repair; generated SDK bytes verified identical"})
    print(f"COMPARISON CLI PINNED: {sha(binary)}; {len(actual)} package/ownership files byte-identical.")


def inventory(root: Path) -> dict[str, dict]:
    output = root / "packages"
    return {str(p.relative_to(output)): {
        "sha256": sha(p), "bytes": p.stat().st_size,
        "mtimeNs": p.stat().st_mtime_ns, "inode": p.stat().st_ino,
    } for p in sorted(output.rglob("*")) if p.is_file()}


def generation(root: Path, action: str) -> None:
    verify_pins(root)
    argv = [str(root / "bin/suspect"), "codegen-session", "--config",
            str(root / "session.json"), "--out", str(root / "packages"),
            "--format", "json"]
    if action in ("check", "preview"):
        argv += [f"--{action}"]
    before = inventory(root) if action != "generate" else None
    result = record(root, action, argv)
    data = json.loads(result.stdout)
    if before is not None and inventory(root) != before:
        raise SystemExit("Read-only demo action changed package bytes or metadata.")
    if action == "generate":
        current = inventory(root)
        receipt = fresh_directory(root / "generation-receipts", "generation")
        write_json(receipt / "package-pins.json", current)
        if not (root / "package-pins.json").exists():
            write_json(root / "package-pins.json", current)
    print(json.dumps({**{key: data.get(key) for key in (
        "status", "success", "generation", "delta", "stats"
    )}, "changedArtifactCount": len(data.get("changedArtifacts", []))}, indent=2))
    if action == "preview":
        print(f"Preview retained: {len(data.get('artifacts', []))} artifacts; no writes.")


def no_rewrite(root: Path) -> None:
    verify_pins(root)
    before = inventory(root)
    generation(root, "generate")
    after = inventory(root)
    changed = [p for p in set(before) | set(after) if before.get(p) != after.get(p)]
    result = {"changedFiles": changed, "filesChecked": len(before), "bytesMtimesInodesPreserved": not changed}
    receipt = fresh_directory(root / "generation-receipts", "regeneration")
    write_json(receipt / "zero-rewrite.json", result)
    if not (root / "zero-rewrite.json").exists():
        write_json(root / "zero-rewrite.json", result)
    if changed:
        raise SystemExit(f"Regeneration changed files: {changed[:10]}")
    print(f"ZERO REWRITE: {len(before)} files; bytes, mtimes and inodes preserved.")


def provenance(root: Path) -> None:
    verify_pins(root)
    manifest = root / "packages/typescript/http-manifest.json"
    data = json.loads(manifest.read_text())
    operation = next(op for op in data["operations"] if op["operationId"] == "getCredits")
    view = {key: operation[key] for key in ("operationId", "sourceOperationId", "export", "source")}
    view["provenance"] = {key: operation["provenance"][key] for key in ("use_site", "terminal", "references")}
    print(json.dumps(view, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="New or prepared demo candidate root")
    sub = parser.add_subparsers(dest="action", required=True)
    create = sub.add_parser("freeze", help="Copy a Main-coordinated CLI and pin the four tracked sources")
    create.add_argument("--bin", required=True, type=Path)
    create.add_argument("--receipt", type=Path)
    comparison = sub.add_parser("freeze-comparison", help="Pin an additive comparison CLI and verify unchanged SDK bytes")
    comparison.add_argument("--bin", required=True, type=Path)
    comparison.add_argument("--receipt", required=True, type=Path)
    for action in ("generate", "check", "preview", "regenerate", "verify-pins", "provenance"):
        sub.add_parser(action)
    args = parser.parse_args()
    root = owned_root(args.root)
    if args.action == "freeze":
        freeze(root, args.bin.absolute(), args.receipt.absolute() if args.receipt else None)
    elif args.action == "freeze-comparison":
        freeze_comparison(root, args.bin.absolute(), args.receipt.absolute())
    elif args.action == "regenerate":
        no_rewrite(root)
    elif args.action == "verify-pins":
        verify_pins(root)
        print("CLI, session config and all four tracked source pins match.")
    elif args.action == "provenance":
        provenance(root)
    else:
        generation(root, args.action)


if __name__ == "__main__":
    main()
