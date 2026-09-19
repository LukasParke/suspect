#!/usr/bin/env python3
"""Private-copy source edits, readable watch output and source-aware comparison."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import queue
import shutil
import subprocess
import threading

from demo import COMPARISON_DIR, REPO, TRACKED, fresh_directory, owned_root, record, sha, verify_pins, write_json

BEFORE = ("                name:\n"
          "                  description: 'Name for the new API key'\n"
          "                  example: 'My New API Key'\n"
          "                  minLength: 1\n")
AFTER = BEFORE.replace("minLength: 1", "minLength: 8")


def prepare(root: Path, label: str) -> Path:
    verify_pins(root)
    work = root / f"iteration-{label}"
    work.mkdir(exist_ok=False)
    source = (root / "sources" / TRACKED[0]).read_text()
    if source.count(BEFORE) != 1:
        raise SystemExit("Pinned source no longer has the expected unique createKeys name constraint.")
    session = json.loads((root / "session.json").read_text())
    for stage in ("before", "after", "live"):
        directory = work / stage
        directory.mkdir()
        (directory / "openapi.yaml").write_text(source.replace(BEFORE, AFTER) if stage == "after" else source)
        config = dict(session, spec="openapi.yaml", owner=f"suspect-sdk:demo-iteration-{label}")
        write_json(directory / "session.json", config)
    result = record(work, "baseline-generate", [str(root / "bin/suspect"), "codegen-session",
        "--config", str(work / "live/session.json"), "--out", str(work / "packages"), "--format", "json"])
    data = json.loads(result.stdout)
    if data["status"] != "written":
        raise SystemExit(f"Unexpected first generation: {data['status']}")
    write_json(work / "change.json", {
        "operationId": "createKeys", "before": 1, "after": 8,
        "pointer": "/paths/~1keys/post/requestBody/content/application~1json/schema/properties/name/minLength",
        "sourceBeforeSha256": sha(work / "before/openapi.yaml"),
        "sourceAfterSha256": sha(work / "after/openapi.yaml"),
        "claim": "private demonstration change; upstream source is unchanged",
    })
    print(f"PRIVATE COPY READY: {work}\nModel change: createKeys request name.minLength 1 → 8")
    return work


def change(work: Path, stage: str) -> None:
    current = (work / "live/openapi.yaml").read_bytes()
    allowed = [(work / f"{name}/openapi.yaml").read_bytes() for name in ("before", "after")]
    if current not in allowed:
        raise SystemExit("Private source has an additional edit; preserve it and prepare a new iteration label.")
    shutil.copyfile(work / stage / "openapi.yaml", work / "live/openapi.yaml")
    print(f"Private name.minLength = {8 if stage == 'after' else 1}")


def compare(root: Path, work: Path) -> None:
    binary = root / COMPARISON_DIR / "bin/suspect"
    pin = json.loads((root / COMPARISON_DIR / "cli.json").read_text())
    if sha(binary) != pin["binarySha256"]:
        raise SystemExit("Pinned comparison CLI changed.")
    output = fresh_directory(work / COMPARISON_DIR, "run")
    argv = [str(binary), "codegen-compare", "--before", str(work / "before/session.json"),
            "--after", str(work / "after/session.json")]
    result = record(output, "compare-json", [*argv, "--format", "json"], expected=(1,))
    report = json.loads(result.stdout)
    write_json(output / "compatibility.json", report)
    result = record(output, "compare-markdown", argv, expected=(1,))
    (output / "compatibility.md").write_text(result.stdout)
    print(json.dumps({"expectedExitCode": 1, "summary": report.get("summary"),
                      "nativeTargets": len(report.get("native", []))}, indent=2))
    print(f"Migration notes: {output / 'compatibility.md'}")


def watch(root: Path, work: Path, check: bool) -> None:
    number = 1
    while (work / f"watch-{number:02}").exists():
        number += 1
    receipt = work / f"watch-{number:02}"
    receipt.mkdir()
    argv = [str(root / "bin/suspect"), "codegen-session", "--config", str(work / "live/session.json"),
            "--out", str(work / "packages"), "--watch", "--preview", "--interval-ms", "150", "--format", "json"]
    before = {str(p.relative_to(work / "packages")): (sha(p), p.stat().st_mtime_ns, p.stat().st_ino)
              for p in (work / "packages").rglob("*") if p.is_file()}
    messages: queue.Queue[str | None] = queue.Queue()
    events = []
    with (receipt / "stderr.log").open("w") as stderr:
        process = subprocess.Popen(argv, cwd=REPO, stdout=subprocess.PIPE, stderr=stderr, text=True)
        assert process.stdout is not None

        def read() -> None:
            assert process.stdout is not None
            for line in process.stdout:
                messages.put(line)
            messages.put(None)

        thread = threading.Thread(target=read, daemon=True)
        thread.start()
        try:
            while True:
                line = messages.get(timeout=60 if check else None)
                if line is None:
                    raise SystemExit("Watch exited before completion; see retained stderr.")
                data = json.loads(line)
                events.append(data)
                write_json(receipt / f"event-{len(events):02}.json", data)
                print(json.dumps({"generation": data["generation"], "status": data["status"],
                    "changedArtifacts": len(data["changedArtifacts"]), "delta": data["delta"],
                    "samplePaths": data["changedArtifacts"][:4]}), flush=True)
                if check:
                    if len(events) == 1:
                        if data["status"] != "current":
                            raise SystemExit("Start watch-check with the private baseline restored.")
                        change(work, "after")
                    elif len(events) == 2:
                        if data["status"] != "drift" or not data["changedArtifacts"]:
                            raise SystemExit("Expected changed model/codec preview.")
                        change(work, "before")
                    else:
                        if data["status"] != "current" or data["delta"]["compiles"] != 0 or data["delta"]["renders"] != 0:
                            raise SystemExit("Expected cached A→B→A restoration with zero recompile/render.")
                        break
        except KeyboardInterrupt:
            print("Watch stopped.")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            thread.join(timeout=5)
            process.stdout.close()
            write_json(receipt / "command.json", {"argv": argv, "cwd": str(REPO),
                       "exitCodeAfterStop": process.returncode, "records": len(events)})
    after = {str(p.relative_to(work / "packages")): (sha(p), p.stat().st_mtime_ns, p.stat().st_ino)
             for p in (work / "packages").rglob("*") if p.is_file()}
    if before != after:
        raise SystemExit("Read-only preview unexpectedly changed package files.")
    write_json(receipt / "result.json", {"readOnly": True, "filesChecked": len(before),
               "cachedRevertChecked": check, "records": len(events)})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("--label", required=True, help="Safe fresh label, e.g. morning-01")
    parser.add_argument("action", choices=("prepare", "change", "restore", "compare", "watch", "watch-check"))
    args = parser.parse_args()
    if not args.label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in args.label):
        raise SystemExit("Use a lowercase alphanumeric/hyphen label.")
    root = owned_root(args.root)
    verify_pins(root)
    work = root / f"iteration-{args.label}"
    if args.action == "prepare":
        prepare(root, args.label)
    elif args.action in ("change", "restore"):
        change(work, "after" if args.action == "change" else "before")
    elif args.action == "compare":
        compare(root, work)
    else:
        watch(root, work, args.action == "watch-check")


if __name__ == "__main__":
    main()
