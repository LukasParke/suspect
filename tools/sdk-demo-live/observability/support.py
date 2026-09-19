"""Isolated support for the additive two-consumer observability correction."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[3]
BASE = REPO / "target/sdk-demo-live-20260911-01/candidate-02"
ROOT = REPO / "target/sdk-demo-live-20260911-01/observability-01"
SOURCES = REPO / "examples/sdk-demo-live/observability"
RUNNER_SHA = "578b79a4a34bf5a55966e217c96d3322ac31955ad9bf2015de09d9bcb85d3ee7"

def sha(path: Path) -> str:
    digest=hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda:file.read(1048576),b""): digest.update(block)
    return digest.hexdigest()

def metadata(path: Path) -> dict:
    stat=path.stat()
    return {"sha256":sha(path),"bytes":stat.st_size,"mtimeNs":stat.st_mtime_ns,"inode":stat.st_ino,"mode":stat.st_mode}

def save(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True,exist_ok=True)
    with path.open("x") as file: json.dump(value,file,indent=2);file.write("\n")

def fresh(parent: Path, prefix: str) -> Path:
    parent.mkdir(parents=True,exist_ok=True)
    number=1
    while True:
        path=parent/f"{prefix}-{number:02}"
        try: path.mkdir();return path
        except FileExistsError:number+=1

def record(root: Path, label: str, argv: list[str|Path], cwd: Path, env=None, expected=0) -> dict:
    receipt=fresh(root/"commands",label)
    environment={key:value for key,value in os.environ.items() if key!="OPENROUTER_API_KEY" and not key.startswith("SDK_DEMO_")}
    environment.update(env or {})
    args=list(map(str,argv))
    result=subprocess.run(args,cwd=cwd,env=environment,capture_output=True,text=True,timeout=120)
    (receipt/"stdout.log").write_text(result.stdout)
    (receipt/"stderr.log").write_text(result.stderr)
    row={"argv":args,"cwd":str(cwd),"environment":env or {},"exitCode":result.returncode,"expectedExitCode":expected,"stdoutSha256":sha(receipt/"stdout.log"),"stderrSha256":sha(receipt/"stderr.log")}
    save(receipt/"command.json",row)
    if result.returncode!=expected: raise RuntimeError(f"{label} failed: {receipt}\n{result.stdout}\n{result.stderr}")
    return {**row,"stdout":result.stdout,"stderr":result.stderr,"receipt":str(receipt)}

def verify_preservation() -> dict:
    before=json.loads((ROOT/"preservation-before.json").read_text())
    changed=[name for name,expected in before.items() if metadata(Path(name))!=expected]
    if changed:raise RuntimeError(f"Protected files changed: {changed}")
    return {"filesChecked":len(before),"changed":[],"bytesMtimesInodesModesPreserved":True}
