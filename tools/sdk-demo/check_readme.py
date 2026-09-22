#!/usr/bin/env python3
"""Check local README links/excerpts and exercise its optional live-read code with injected bytes."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

from demo import REPO, fresh_directory, owned_root, record, sha, verify_pins, write_json


def headings(text: str) -> set[str]:
    result = set(re.findall(r'<a\s+id="([^"]+)"', text))
    counts: dict[str, int] = {}
    fenced = False
    for line in text.splitlines():
        if line.startswith("```"):
            fenced = not fenced
        if fenced:
            continue
        match = re.match(r"^#{1,6}\s+(.+)$", line)
        if match:
            name = match[1].strip().lower()
            name = re.sub(r"[^\w\- ]", "", name).replace(" ", "-")
            count = counts.get(name, 0)
            counts[name] = count + 1
            result.add(name if count == 0 else f"{name}-{count}")
    return result


def normalized(text: str) -> str:
    return "\n".join(line.strip() for line in text.strip().splitlines())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    root = owned_root(args.root)
    verify_pins(root)
    readme = REPO / "DEMO-README.md"
    text = readme.read_text()
    failures: list[str] = []
    links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", text)
    for link in links:
        if re.match(r"^https?://", link):
            continue
        path, _, fragment = link.partition("#")
        file = (REPO / path) if path else readme
        if not file.exists():
            failures.append(f"Missing link: {link}")
        elif fragment and file.suffix == ".md" and fragment not in headings(file.read_text()):
            failures.append(f"Missing heading: {link}")

    suffixes = {"ts": ".ts", "python": ".py", "go": ".go", "rust": ".rs", "swift": ".swift",
                "java": ".java", "csharp": ".cs", "kotlin": ".kt", "ruby": ".rb", "php": ".php", "cpp": ".cpp", "dart": ".dart"}
    checked = []
    for language, code in re.findall(r"```(\w+)\n(.*?)\n```", text, re.S):
        if language not in suffixes:
            continue
        candidates = list((REPO / "examples/sdk-demo-all").rglob("*" + suffixes[language]))
        source = next((p for p in candidates if normalized(code) in normalized(p.read_text())), None)
        if source is None:
            failures.append(f"Native {language} block is not an exact checked consumer excerpt: {code[:70]}")
        else:
            checked.append({"language": language, "source": str(source.relative_to(REPO)), "sha256": sha(source)})

    index = json.loads((root / "native-index.json").read_text())
    for info in index.values():
        for source, digest in info["snippets"].items():
            if sha(REPO / source) != digest:
                failures.append(f"Snippet changed after native preparation: {source}")

    work = fresh_directory(root / "readme-check", "check")
    # Extract the exact heredoc, including the environment lookup shown to the human.
    live = re.search(r'"\$NODE" --input-type=module <<\'JS\'\n(.*?)\nJS', text, re.S)
    if live is None:
        failures.append("Optional-live heredoc not found")
    else:
        fixture = REPO / "crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json"
        preload = work / "injected-live-fetch.mjs"
        preload.write_text("import assert from 'node:assert/strict';\nimport fs from 'node:fs';\n"
            f"const body = JSON.parse(fs.readFileSync({json.dumps(str(fixture))}, 'utf8')).credits;\n"
            "let calls = 0;\n"
            "globalThis.fetch = async (input, init) => {\n"
            "  calls++;\n"
            "  assert.equal(String(input), 'https://openrouter.ai/api/v1/credits');\n"
            "  assert.equal(init.method, 'GET');\n"
            "  assert.equal(new Headers(init.headers).get('authorization'), 'Bearer fixture-token');\n"
            "  return new Response(body, {status: 200, headers: {'content-type': 'application/json'}});\n"
            "};\nprocess.on('beforeExit', () => assert.equal(calls, 1));\n")
        node = index["typescript"]["tools"]["node"]["path"]
        result = record(work, "optional-live-injected-fetch", [node, "--import", str(preload), "--input-type=module"],
                        cwd=Path(index["typescript"]["consumer"]), env={"OPENROUTER_API_KEY": "fixture-token"},
                        input_text=live[1], timeout=20)
        if "100.50000000000000001" not in result.stdout:
            failures.append("Optional-live code did not preserve the injected exact value")

    result = {"readmeSha256": sha(readme), "localLinksChecked": len(links), "nativeExcerpts": checked,
              "preparedTargets": len(index), "optionalLiveTest": "injected fetch only; no remote service contacted",
              "failures": failures}
    write_json(work / "report.json", result)
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"README CHECKED: {len(links)} links, {len(checked)} exact native excerpts, {len(index)} prepared targets, injected-only live read.")


if __name__ == "__main__":
    main()
