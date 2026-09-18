#!/usr/bin/env python3
"""Copy an immutable Main-generated ENV cohort without patching generated source."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

REPO = Path(__file__).resolve().parents[3]
PARENT = REPO / "target/sdk-demo-live-20260911-01/environment-01"

def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1048576), b""):
            digest.update(block)
    return digest.hexdigest()

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--cli-sha256", required=True)
    parser.add_argument("--packages", type=Path, required=True)
    parser.add_argument("--session", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--review-status", choices=("pending", "clean"), required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9-]+", args.candidate): raise SystemExit("Use a safe fresh candidate label")
    root = PARENT / args.candidate
    if sha(args.cli) != args.cli_sha256: raise SystemExit("CLI pin mismatch")
    config = json.loads(args.session.read_text())
    expected = json.loads((REPO / "examples/sdk-demo-live/environment/session.json").read_text())
    for key in ("operation_ids", "targets", "credential_env", "compatibility_profiles"):
        if config[key] != expected[key]: raise SystemExit(f"Configured identity differs: {key}")
    root.mkdir(parents=True, exist_ok=False)
    (root / "bin").mkdir()
    shutil.copy2(args.cli, root / "bin/suspect")
    shutil.copy2(args.session, root / "session.json")
    shutil.copy2(args.receipt, root / "main-receipt.json")
    shutil.copytree(args.packages, root / "packages")
    files = {str(p.relative_to(root / "packages")): sha(p) for p in (root / "packages").rglob("*") if p.is_file()}
    source = {str(p.relative_to(args.packages)): sha(p) for p in args.packages.rglob("*") if p.is_file()}
    if files != source: raise SystemExit("Copied packages differ from Main's cohort")
    (root / "package-pins.json").write_text(json.dumps(files, indent=2) + "\n")
    (root / "staging.json").write_text(json.dumps({"cliOrigin":str(args.cli.absolute()),"cliSha256":args.cli_sha256,"packagesOrigin":str(args.packages.absolute()),"packageFiles":len(files),"sessionSha256":sha(args.session),"receiptSha256":sha(args.receipt),"reviewStatus":args.review_status,"sdkSourcesPatched":False,"nativeConsumersBuilt":False,"realApiExecuted":False}, indent=2) + "\n")
    print(f"ENV cohort copied byte-for-byte: {root}; {len(files)} files; review {args.review_status}")

if __name__ == "__main__": main()
