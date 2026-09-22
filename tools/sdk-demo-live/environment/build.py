#!/usr/bin/env python3
"""Fresh configured-env consumer preparation through the retained native builder."""
from __future__ import annotations
import argparse
import importlib.util
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[3]
PARENT = REPO / "target/sdk-demo-live-20260911-01/environment-01"

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language")
    parser.add_argument("--candidate", required=True)
    args = parser.parse_args()
    root = PARENT / args.candidate
    if root.parent != PARENT or not (root / "session.json").is_file():
        raise SystemExit("Select a staged, source-pinned fresh ENV candidate")
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(REPO / "tools/sdk-demo-live"))
    spec = importlib.util.spec_from_file_location("retained_native_builder", REPO / "tools/sdk-demo-live/build.py")
    assert spec is not None and spec.loader is not None
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    if args.language not in (*builder.LANGUAGES, "all"):
        raise SystemExit("Select one of the twelve language targets or all")
    builder.ROOT = root
    builder.SOURCES = REPO / "examples/sdk-demo-live/environment"
    for language in builder.LANGUAGES if args.language == "all" else (args.language,):
        builder.build(language)

if __name__ == "__main__": main()
