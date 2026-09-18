"""Pinned paths and append-only receipts for the automatic-environment edition."""
from __future__ import annotations
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[3]
PARENT = REPO / "target/sdk-demo-live-20260911-01/environment-01"
ROOT = PARENT / "candidate-02"
PROVISIONAL = PARENT / "provisional-01"
MAIN = REPO / "target/sdk-main-env-candidate-20260911-02"
OBSERVABILITY = REPO / "target/sdk-demo-live-20260911-01/observability-01"
SOURCES = REPO / "examples/sdk-demo-live/environment"
LANGUAGES = ("typescript", "python", "go", "rust", "swift", "java", "csharp", "kotlin", "ruby", "php", "dart", "cpp")
REUSED = tuple(language for language in LANGUAGES if language not in ("go", "dart"))
CLI_SHA = "31c2fe23c760f191fdb8546cc10d2d78973935d3a728cd8875dfb66f45975261"
ACCEPTANCE_SHA = "841ff131324377c5d551dba7aa527fb6a34efd66cebc100836cefc5044574c4a"
MAIN_PACKAGES_SHA = "7d27ae8229b1d3bddcef76cab66da5f7b4ba738cf33847b514fb2e2a5ec22e27"
SOURCE_MANIFEST_SHA = "35bbf1114bee47cab2b5c72460eec878130c0a48e0ef723d3e8bb9ae8ab0d1be"
SESSION_SHA = "877a532f9b2e40bdca356937f3df134d24d021b0164435b980ee46f670c6274e"
RUNNER_SHA = "578b79a4a34bf5a55966e217c96d3322ac31955ad9bf2015de09d9bcb85d3ee7"
FILENAMES = {"typescript":"main.ts", "javascript":"main.mjs", "python":"main.py", "go":"main.go", "rust":"main.rs", "swift":"Main.swift", "java":"Main.java", "csharp":"Program.cs", "kotlin":"Smoke.kt", "ruby":"main.rb", "php":"main.php", "dart":"main.dart", "cpp":"main.cpp"}

def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1048576), b""):
            digest.update(block)
    return digest.hexdigest()

def metadata(path: Path) -> dict:
    stat = path.stat()
    return {"sha256":sha(path), "bytes":stat.st_size, "mtimeNs":stat.st_mtime_ns, "inode":stat.st_ino, "mode":stat.st_mode}

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

def check_pins(pins: dict[str, str]) -> None:
    for filename, digest in pins.items():
        if sha(Path(filename)) != digest:
            raise RuntimeError(f"Pinned file changed: {filename}")

def load_base():
    path = REPO / "tools/sdk-demo-live/run.py"
    check_pins({str(path):RUNNER_SHA})
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("accepted_env_base_runner", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def preservation() -> dict:
    before = json.loads((ROOT / "preservation-before.json").read_text())
    changed = [name for name, expected in before.items() if metadata(Path(name)) != expected]
    if changed:
        raise RuntimeError(f"Protected original/F10 files changed: {changed}")
    return {"filesChecked":len(before), "changed":[], "bytesMtimesInodesModesPreserved":True}
