#!/usr/bin/env python3
"""Additive F10 entry: reuse the sealed redaction-safe runner with two replacements."""
from __future__ import annotations
import importlib.util
import json
from pathlib import Path
import sys

from support import BASE, REPO, ROOT, RUNNER_SHA, SOURCES, sha

def load_runner():
    path = REPO / "tools/sdk-demo-live/run.py"
    if sha(path) != RUNNER_SHA:
        raise RuntimeError("The accepted base live runner changed")
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("accepted_live_runner", path)
    assert spec is not None and spec.loader is not None
    base = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(base)
    original_prepared = base.prepared
    original_excerpt = base.source_excerpt
    original_fresh = base.fresh

    def prepared(language):
        info = original_prepared(language)
        if language not in ("typescript", "python"):
            return info
        pins = json.loads((ROOT / "ready.json").read_text())
        plan = pins["consumers"][language]
        for filename, digest in pins["files"].items():
            if sha(Path(filename)) != digest:
                raise RuntimeError(f"Additive observability file changed: {filename}")
        return {**info, "runArgv": plan["argv"], "consumer": plan["consumer"]}

    def excerpt(language):
        if language not in ("typescript", "python"):
            return original_excerpt(language)
        filename = "main.ts" if language == "typescript" else "main.py"
        text = (SOURCES / language / filename).read_text()
        body = text.split("DEMO START", 1)[1].split("DEMO END", 1)[0]
        lines = body.splitlines()[1:]
        if lines and lines[-1].strip() in ("//", "#"):
            lines.pop()
        return "\n".join(line.strip() for line in lines)

    def fresh(parent, prefix):
        return original_fresh(ROOT / "live-runs" if parent == BASE / "live-runs" else parent, prefix)

    base.prepared = prepared
    base.source_excerpt = excerpt
    base.fresh = fresh
    return base

if __name__ == "__main__":
    try:
        raise SystemExit(load_runner().main())
    except KeyboardInterrupt:
        print("\nLive demo cancelled.", file=sys.stderr)
        raise SystemExit(130)
