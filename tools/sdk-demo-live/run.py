#!/usr/bin/env python3
"""Run prebuilt native OpenRouter SDK consumers against the live source server."""
from __future__ import annotations
import argparse
import getpass
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading

from common import LANGUAGES, REPO, ROOT, SOURCES, fresh, save, sha

DISPLAY = {"typescript":"TypeScript", "javascript":"JavaScript", "python":"Python", "go":"Go", "rust":"Rust", "swift":"Swift", "java":"Java", "csharp":"C#", "kotlin":"Kotlin", "ruby":"Ruby", "php":"PHP", "dart":"Dart", "cpp":"C++"}
ALIASES = {"ts":"typescript", "js":"javascript", "c#":"csharp", "cs":"csharp", "c++":"cpp", "py":"python", "kt":"kotlin"}
NUMBER = re.compile(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?\Z")
TRUNCATED_OUTPUT = "[output omitted: capture limit exceeded]\n"

def prepared(language: str) -> dict:
    native = "typescript" if language == "javascript" else language
    paths = sorted((ROOT / "native").glob(f"{native}-*/ready.json"), key=lambda path: int(path.parent.name.rsplit("-",1)[1]), reverse=True)
    if not paths:
        raise RuntimeError(f"{DISPLAY[language]} is not prepared; run python3 tools/sdk-demo-live/build.py {native} tonight")
    info = json.loads(paths[0].read_text())
    for name, digest in info["snippets"].items():
        if sha(REPO / name) != digest:
            raise RuntimeError(f"Source changed since compilation: {name}")
    info["runArgv"] = info["javascript"] if language == "javascript" else info["argv"]
    if not info["runArgv"] or not Path(info["runArgv"][0]).is_file():
        raise RuntimeError(f"Missing prepared executable for {language}")
    receipt = paths[0].parent / "verification.json"
    if not receipt.is_file():
        raise RuntimeError(f"{DISPLAY[language]} is built but its controlled verification is not complete")
    proof = json.loads(receipt.read_text())
    if not proof.get("passed"):
        raise RuntimeError(f"{DISPLAY[language]} controlled verification failed")
    for path, digest in proof["executionPins"].items():
        if not Path(path).is_file() or sha(Path(path)) != digest:
            raise RuntimeError(f"Prepared execution artifact changed: {path}")
    return info

def redact(text: str, token: str) -> str:
    for value in sorted({token, json.dumps(token)[1:-1]}, key=len, reverse=True):
        if value: text = text.replace(value, "[REDACTED]")
    return text

def sanitized_streams(raw: dict, token: str) -> tuple[str, str]:
    # A truncated prefix can end inside a credential. Never retain any of it.
    return tuple(TRUNCATED_OUTPUT if raw["truncated"][name] else redact(raw[name], token)
                 for name in ("stdout", "stderr"))

def capture(argv: list[str], cwd: Path, env: dict[str,str], timeout: float = 22) -> dict:
    process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    chunks = {"stdout": bytearray(), "stderr": bytearray()}
    truncated = {"stdout":False,"stderr":False}
    def read(name, pipe):
        while True:
            block = pipe.read(8192)
            if not block: break
            available = max(0, 65536-len(chunks[name]))
            chunks[name].extend(block[:available])
            truncated[name] |= len(block) > available
        pipe.close()
    readers = [threading.Thread(target=read, args=(name,pipe), daemon=True) for name,pipe in (("stdout",process.stdout),("stderr",process.stderr))]
    for reader in readers: reader.start()
    reason = None
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        reason = "deadline-exceeded"
    except KeyboardInterrupt:
        reason = "cancelled"
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try: process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        for reader in readers: reader.join(timeout=2)
    return {"exitCode":process.returncode,"stdout":chunks["stdout"].decode(errors="replace"),"stderr":chunks["stderr"].decode(errors="replace"),"truncated":truncated,"reason":reason}

def source_excerpt(language: str) -> str:
    suffix = {"typescript":"main.ts","javascript":"main.mjs","python":"main.py","go":"main.go","rust":"main.rs","swift":"Main.swift","java":"Main.java","csharp":"Program.cs","kotlin":"Smoke.kt","ruby":"main.rb","php":"main.php","dart":"main.dart","cpp":"main.cpp"}[language]
    text = (SOURCES / language / suffix).read_text()
    body = text.split("DEMO START",1)[1].split("DEMO END",1)[0]
    lines = body.splitlines()[1:]
    if lines and lines[-1].strip() in ("//", "#"): lines.pop()
    return "\n".join(line.strip() for line in lines)

def validate_response(output: str, operation: str) -> dict:
    lines = [line for line in output.splitlines() if line.strip()]
    if len(lines) != 1: raise ValueError("Expected one native SDK confirmation record")
    result = json.loads(lines[0])
    if not isinstance(result, dict): raise ValueError("Invalid native confirmation")
    if result.get("ok") is True:
        if result.get("status") != 200: raise ValueError("Unexpected success status")
        if not isinstance(result.get("usage"), str) or not NUMBER.fullmatch(result["usage"]): raise ValueError("Invalid exact usage token")
        if operation == "key":
            if type(result.get("freeTier")) is not bool or type(result.get("management")) is not bool: raise ValueError("Missing typed key confirmation")
        elif not isinstance(result.get("credits"), str) or not NUMBER.fullmatch(result["credits"]): raise ValueError("Missing exact credits token")
    return result

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", nargs="?", default="all")
    parser.add_argument("--operation", choices=("key","credits"), default="key")
    parser.add_argument("--preflight", action="store_true", help="Check preparation without credentials or network")
    parser.add_argument("--no-prompt", action="store_true", help="Require OPENROUTER_API_KEY instead of prompting")
    parser.add_argument("--no-code", action="store_true", help="Hide the source excerpt if it was just shown in the README")
    args = parser.parse_args()
    selected = ALIASES.get(args.language.lower(), args.language.lower())
    if selected not in (*LANGUAGES,"javascript","all"):
        parser.error("Choose all, typescript, python, go, rust, swift, java, csharp, kotlin, ruby, php, dart, cpp, or javascript")
    languages = list(LANGUAGES) if selected == "all" else [selected]
    ready, errors = {}, {}
    for language in languages:
        try: ready[language] = prepared(language)
        except (RuntimeError,OSError,ValueError) as error: errors[language] = str(error)
    if args.preflight:
        for language in languages:
            print(f"[{DISPLAY[language]}] {'READY — built/verified; live API not exercised by preflight' if language in ready else 'NOT READY — '+errors[language]}")
        print(f"Preparation: {len(ready)}/{len(languages)} ready. Default live operation: GET /key at the source HTTPS server.")
        return int(bool(errors))
    if not ready:
        for language in languages:
            print(f"[{DISPLAY[language]}] NOT READY — {errors[language]}", file=sys.stderr)
        return 1
    token = os.environ.get("OPENROUTER_API_KEY")
    credential_source = "environment"
    if not token:
        if args.no_prompt or not sys.stdin.isatty():
            print("Set OPENROUTER_API_KEY in the runtime environment, or run interactively for a hidden token prompt.", file=sys.stderr)
            return 2
        credential_source = "secure-prompt"
        try: token = getpass.getpass("OpenRouter API token (hidden): ")
        except (EOFError,KeyboardInterrupt): return 130
        if not token: return 2
    run = fresh(ROOT / "live-runs", "run")
    results = []
    route = "/key" if args.operation == "key" else "/credits"
    print(f"LIVE OpenRouter SDK demo — GET {route} — https://openrouter.ai/api/v1")
    if args.operation == "credits": print("Credits mode requires a management key, as documented by the source.")
    cancelled = False
    for language in languages:
        label = DISPLAY[language]
        if language not in ready:
            print(f"[{label}] NOT READY — {errors[language]}")
            results.append({"language":language,"ok":False,"kind":"not-prepared"})
            continue
        info = ready[language]
        print(f"\n[{label}] {info['packageConfig']['package_name']} — actual native SDK", flush=True)
        if not args.no_code:
            if args.operation == "key": print(source_excerpt(language), flush=True)
            else: print(f"Source: examples/sdk-demo-live/{language}/ — explicit getCredits branch", flush=True)
        env = {key:value for key,value in os.environ.items() if not key.startswith("SDK_DEMO_") and key != "OPENROUTER_API_KEY"}
        env.update(info["environment"])
        env.update(OPENROUTER_API_KEY=token, SDK_DEMO_OPERATION=args.operation)
        try:
            raw = capture(info["runArgv"], Path(info["consumer"]), env)
        except OSError as error:
            raw = {"exitCode":127,"stdout":"","stderr":str(error),"reason":"launch-failed","truncated":{"stdout":False,"stderr":False}}
        stdout, stderr = sanitized_streams(raw, token)
        (run / f"{language}.stdout").write_text(stdout)
        (run / f"{language}.stderr").write_text(stderr)
        if any(raw["truncated"].values()):
            response = {"ok":False,"kind":"output-truncated"}
        else:
            try: response = validate_response(stdout, args.operation)
            except (ValueError,KeyError): response = {"ok":False,"kind":"invalid-or-missing-native-confirmation"}
        ok = raw["exitCode"] == 0 and response.get("ok") is True and raw["reason"] is None and not any(raw["truncated"].values())
        item = {"language":language,"ok":ok,"exitCode":raw["exitCode"],"reason":raw["reason"],"confirmation":response,"argv":info["runArgv"],"cwd":info["consumer"],"outputTruncated":raw["truncated"]}
        results.append(item)
        save(run / f"{language}.json", item)
        if ok:
            fields = f"authenticated=true freeTier={str(response['freeTier']).lower()} management={str(response['management']).lower()}" if args.operation == "key" else f"credits={response['credits']}"
            print(f"[{label}] PASS GET {route} HTTP {response['status']} {fields} usage={response['usage']}", flush=True)
        else:
            print(f"[{label}] FAIL GET {route} HTTP {response.get('status') or 'unavailable'} {response.get('kind',raw['reason'] or 'failed')}", flush=True)
        if raw["reason"] == "cancelled":
            cancelled = True
            break
    passed = sum(item["ok"] for item in results)
    report = {"mode":"live", "operation":args.operation,"route":route,"server":"https://openrouter.ai/api/v1","credentialSource":credential_source,
              "tokenRecorded":False,"requestedLanguages":languages,"results":results,"passed":passed,"requested":len(languages),"cancelled":cancelled}
    save(run / "REPORT.json", report)
    print(f"\nLive result: {passed}/{len(languages)} SDKs confirmed success. Receipt: {run}")
    return 130 if cancelled else int(passed != len(languages))

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\nLive demo cancelled.", file=sys.stderr)
        raise SystemExit(130)
