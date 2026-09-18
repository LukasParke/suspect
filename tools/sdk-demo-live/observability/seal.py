#!/usr/bin/env python3
"""Seal F10's additive consumer patch and unchanged base-runner linkage."""
from __future__ import annotations
import ast
import json
from pathlib import Path
import re

from support import BASE, REPO, ROOT, RUNNER_SHA, SOURCES, fresh, save, sha, verify_preservation

def main() -> None:
    proof=json.loads((ROOT/"verification.json").read_text())
    if proof["status"]!="passed" or proof["checks"]!=16:
        raise RuntimeError("The two-program focused proof is incomplete")
    if sha(REPO/"tools/sdk-demo-live/run.py")!=RUNNER_SHA:
        raise RuntimeError("Accepted redaction-safe runner changed")
    readme=SOURCES/"README.md"
    links=re.findall(r"\[[^\]]+\]\(([^)]+)\)",readme.read_text())
    for link in links:
        if not (readme.parent/link.split("#",1)[0]).exists():raise RuntimeError(link)
    for path in (REPO/"tools/sdk-demo-live/observability").glob("*.py"):
        ast.parse(path.read_text(),filename=str(path))
    preserved=verify_preservation()
    destination=fresh(ROOT,"delivery")
    files={}
    for directory in (SOURCES,REPO/"tools/sdk-demo-live/observability",ROOT):
        for path in directory.rglob("*"):
            if path.is_file() and "__pycache__" not in path.parts and "node_modules" not in path.parts and not path.is_relative_to(destination):files[str(path)]=sha(path)
    files[str(REPO/"demo-live-v2.sh")]=sha(REPO/"demo-live-v2.sh")
    save(destination/"REPORT.json",{"status":"ready-additive-not-live-executed","finding":"F10","readme":str(readme),"readmeSha256":sha(readme),"entrypoint":"./demo-live-v2.sh","onlyReplacedConsumers":["typescript","python"],"installedSdkBytesReused":True,"baseRunnerSha256":RUNNER_SHA,"baseSeal":str(BASE/"delivery-02/SHA256SUMS.json"),"baseSealSha256":sha(BASE/"delivery-02/SHA256SUMS.json"),"acceptedControlledOutcomes":16,"controlProof":str(ROOT/"verification.json"),"discardedSetupAttempt":proof["discardedSetupAttempt"],"preservation":preserved,"linksChecked":len(links),"sdkNativeImplementationChanged":False,"fullMatrixReplayed":False,"realTokenUsed":False,"activeOriginalEntryChanged":False})
    save(destination/"SHA256SUMS.json",files)
    save(destination/"seal.json",{"reportSha256":sha(destination/"REPORT.json"),"manifestSha256":sha(destination/"SHA256SUMS.json"),"files":len(files)})
    print(f"F10 additive seal: {destination}; {len(files)} new file hashes; {preserved['filesChecked']} protected files unchanged")

if __name__=="__main__":main()
