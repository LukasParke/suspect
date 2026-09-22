#!/usr/bin/env python3
"""Seal staged live SDK programs; this does not execute a live API request."""
from __future__ import annotations
import json
from pathlib import Path
from common import LANGUAGES, REPO, ROOT, SOURCES, fresh, save, sha
from run import prepared

def main() -> None:
    docs = sorted((ROOT / "checks").glob("readme-*/REPORT.json"))
    if not docs: raise RuntimeError("Exact live README checks must complete first")
    checked = json.loads(docs[-1].read_text())
    if checked["readmeSha256"] != sha(REPO/"LIVE-DEMO-README.md") or len(checked["snippets"]) != 13:
        raise RuntimeError("Final README changed after its native checks")
    redaction_checks = sorted((ROOT / "wrapper-redaction-fix-01").glob("checks-*/REPORT.json"))
    if not redaction_checks:
        raise RuntimeError("Focused output-cap redaction guards must pass before final sealing")
    redaction = json.loads(redaction_checks[-1].read_text())
    if not redaction["passed"] or redaction["runnerSha256"] != sha(REPO / "tools/sdk-demo-live/run.py"):
        raise RuntimeError("Runner changed after focused redaction verification")
    independent = json.loads((ROOT / "wrapper-redaction-fix-01/independent-green.json").read_text())
    if independent["status"] != "passed" or independent["runnerSha256"] != redaction["runnerSha256"]:
        raise RuntimeError("Main's independent redaction recheck does not match the runner")
    protected = json.loads((ROOT/"preservation-before.json").read_text())
    changed = []
    for filename, expected in protected.items():
        path = Path(filename); stat=path.stat()
        current={"sha256":sha(path),"bytes":stat.st_size,"mtimeNs":stat.st_mtime_ns,"inode":stat.st_ino,"mode":stat.st_mode}
        if current != expected: changed.append(filename)
    if changed: raise RuntimeError(f"Earlier protected artifacts changed: {changed}")
    package_pins=json.loads((ROOT/"package-pins.json").read_text())
    for name,digest in package_pins.items():
        if sha(ROOT/"packages"/name) != digest: raise RuntimeError(f"Generated package changed: {name}")
    files={}
    def pin(path):
        if path.is_file() and "__pycache__" not in path.parts: files[str(path)]=sha(path)
    for path in (REPO/"LIVE-DEMO-README.md",REPO/"demo-live.sh"): pin(path)
    for directory in (REPO/"tools/sdk-demo-live",SOURCES):
        for path in directory.rglob("*"): pin(path)
    for name in ("generation.json","session.json","source-pins.json","package-pins.json","preservation-before.json","cli-receipt.json","bin/suspect"):
        pin(ROOT/name)
    programs={}
    for language in (*LANGUAGES,"javascript"):
        info=prepared(language)
        proof=json.loads((Path(info["attempt"])/"verification.json").read_text())
        files.update(proof["executionPins"])
        for name in ("ready.json","verification.json","source-pins.json"): pin(Path(info["attempt"])/name)
        for path in (Path(info["attempt"])/"commands").rglob("*"): pin(path)
        programs[language]={"package":info["packageConfig"],"argv":info["runArgv"],"cwd":info["consumer"],"prepared":str(Path(info["attempt"])/"ready.json"),"controlledVerification":proof["receipt"],"ready":True,"liveExecutedDuringStaging":False}
    for directory in (ROOT/"verification",ROOT/"checks"):
        for path in directory.rglob("*"): pin(path)
    for path in (ROOT/"wrapper-redaction-fix-01").rglob("*"): pin(path)
    destination=fresh(ROOT,"delivery")
    report={"format":"suspect.sdk.live-demo.delivery.v1","status":"staged-ready-not-live-executed",
        "primaryReadme":str(REPO/"LIVE-DEMO-README.md"),"readmeSha256":sha(REPO/"LIVE-DEMO-README.md"),
        "command":"./demo-live.sh all","preparedRoot":str(ROOT),"defaultOperation":"getCurrentKey","route":"GET /key",
        "sourceServer":"https://openrouter.ai/api/v1","optionalManagementOperation":"getCredits",
        "credentialMode":"runtime environment or secure prompt supplied to actual native SDK constructors",
        "automaticSdkCredentialEnvConfigured":False,"nativeTargets":12,"javascriptBonus":True,
        "desiredArtifacts":len(package_pins)-1,"programs":programs,"controlledChecks":39,
        "realTokenUsedDuringStaging":False,"realApiContactedDuringStaging":False,"liveSuccessClaim":False,
        "redactionGuardReport":str(redaction_checks[-1]),"truncatedStreamsDiscardedBeforePersistenceAndParsing":True,
        "independentRedactionRecheck":str(ROOT/"wrapper-redaction-fix-01/independent-green.json"),
        "supersededInitialSeal":str(ROOT/"delivery-01/REPORT.json"),
        "priorProtectedFiles":len(protected),"priorBytesMtimesInodesModesPreserved":True,
        "nativeSdkMatricesReplayed":False,"numericalQualification":False,"sourceHashFiles":len(files),
        "liveRunPrerequisites":["A runtime OpenRouter API token","Network access to the source HTTPS server"],
        "implementationBlockers":[]}
    save(destination/"REPORT.json",report)
    save(destination/"SHA256SUMS.json",files)
    save(destination/"seal.json",{"reportSha256":sha(destination/"REPORT.json"),"manifestSha256":sha(destination/"SHA256SUMS.json"),"sealedFiles":len(files)})
    print(f"LIVE DEMO STAGED: {destination}; twelve native SDKs + JS; no actual live API run claimed.")

if __name__ == "__main__": main()
