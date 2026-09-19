#!/usr/bin/env python3
"""Run the pinned automatic-env native consumers with the accepted live runner."""
from __future__ import annotations
import builtins
import json
from pathlib import Path
import sys

from support import ACCEPTANCE_SHA, FILENAMES, REPO, ROOT, SOURCES, check_pins, load_base

FACTORY = {
    "typescript":"const client = createClient(options);",
    "javascript":"const client = createClient(options);",
    "python":"with Client(timeout=15.0, server_url=verification) as client:",
    "go":"client, err := sdk.NewClientFromEnv(options)",
    "rust":"let Ok(mut client) = Client::from_env()",
    "swift":"let client = Client(options:",
    "java":"try (var client = Client.fromEnv())",
    "csharp":"using var client = Client.FromEnvironment(",
    "kotlin":"Client.fromEnv(options=",
    "ruby":"OpenRouter::Client.open(server_url:",
    "php":"$client = Sdk\\Client::fromEnv(",
    "dart":"final client = Client(transport:IoTransport()",
    "cpp":"auto connected=Client::from_env(options);",
}

def prepared(language: str) -> dict:
    native = "typescript" if language == "javascript" else language
    ready = json.loads((ROOT / "ready.json").read_text())
    if ready.get("status") != "ready":
        raise RuntimeError("The configured ENV preparation is incomplete")
    check_pins({str(ROOT / "main-receipt.json"):ACCEPTANCE_SHA,
                str(ROOT / "native-index.json"):ready["nativeIndexSha256"],
                str(ROOT / "package-pins.json"):ready["packagePinsSha256"]})
    index = json.loads((ROOT / "native-index.json").read_text())[native]
    check_pins(index["manifestPins"])
    info = json.loads(Path(index["prepared"]).read_text())
    proof = json.loads(Path(index["verification"]).read_text())
    if not proof.get("passed"):
        raise RuntimeError(f"Controlled ENV verification is incomplete: {language}")
    check_pins({str(REPO / name):digest for name, digest in info["snippets"].items()})
    check_pins(proof["executionPins"])
    check_pins({str(ROOT / "packages" / native / name):digest for name, digest in index["packageFiles"].items()})
    argv = info["javascript"] if language == "javascript" else info["argv"]
    if not argv or not Path(argv[0]).is_file():
        raise RuntimeError(f"Missing prepared ENV executable: {language}")
    return {**info, "runArgv":argv}

def load_runner():
    base = load_base()
    base.ROOT = ROOT
    base.SOURCES = SOURCES
    base.prepared = prepared
    base.__doc__ = "Live OpenRouter demo: configured native SDKs read OPENROUTER_API_KEY at client creation."
    core_excerpt = base.source_excerpt

    def excerpt(language):
        text = (SOURCES / language / FILENAMES[language]).read_text()
        factory = next(line.strip() for line in text.splitlines() if FACTORY[language] in line)
        return factory + "\n" + core_excerpt(language)

    def display(*values, **options):
        # The retained runner's credits source reference predates this edition.
        if len(values) == 1 and isinstance(values[0], str) and values[0].startswith("Source: examples/sdk-demo-live/"):
            values = (values[0].replace("Source: examples/sdk-demo-live/", "Source: examples/sdk-demo-live/environment/", 1),)
        builtins.print(*values, **options)

    base.source_excerpt = excerpt
    base.print = display
    return base

if __name__ == "__main__":
    try:
        raise SystemExit(load_runner().main())
    except KeyboardInterrupt:
        print("\nLive ENV demo cancelled.", file=sys.stderr)
        raise SystemExit(130)
