"""Append-only preparation utilities for the live SDK demo."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[2]
ROOT = REPO / "target/sdk-demo-live-20260911-01/candidate-02"
SOURCES = REPO / "examples/sdk-demo-live"
LANGUAGES = ("typescript", "python", "go", "rust", "swift", "java", "csharp", "kotlin", "ruby", "php", "dart", "cpp")

def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()

def save(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as file:
        json.dump(value, file, indent=2)
        file.write("\n")

def fresh(parent: Path, prefix: str) -> Path:
    parent.mkdir(parents=True, exist_ok=True)
    number = 1
    while True:
        path = parent / f"{prefix}-{number:02}"
        try:
            path.mkdir()
            return path
        except FileExistsError:
            number += 1

def record(root: Path, label: str, argv: list[str | Path], cwd: Path,
           env: dict[str, str] | None = None, timeout: int = 600) -> str:
    receipt = fresh(root / "commands", label)
    environment = dict(os.environ)
    environment.pop("OPENROUTER_API_KEY", None)
    environment.update(env or {})
    args = list(map(str, argv))
    try:
        result = subprocess.run(args, cwd=cwd, env=environment, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        def text(value):
            return value.decode(errors="replace") if isinstance(value, bytes) else value or ""
        result = subprocess.CompletedProcess(args, 124, text(error.stdout), text(error.stderr))
    (receipt / "stdout.log").write_text(result.stdout)
    (receipt / "stderr.log").write_text(result.stderr)
    save(receipt / "command.json", {"argv": args, "cwd": str(cwd), "environment": env or {}, "exitCode": result.returncode,
        "stdoutSha256": sha(receipt / "stdout.log"), "stderrSha256": sha(receipt / "stderr.log")})
    if result.returncode != 0:
        raise RuntimeError(f"{label} failed; {receipt}\n{result.stdout[-2000:]}\n{result.stderr[-4000:]}")
    return result.stdout
