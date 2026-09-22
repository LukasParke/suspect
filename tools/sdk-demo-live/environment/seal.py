#!/usr/bin/env python3
"""Seal the completed additive ENV edition and its exact evidence lineage."""
from __future__ import annotations
import ast
import json
from pathlib import Path

from support import ACCEPTANCE_SHA, CLI_SHA, LANGUAGES, MAIN_PACKAGES_SHA, OBSERVABILITY, REPO, REUSED, ROOT, RUNNER_SHA, SESSION_SHA, SOURCE_MANIFEST_SHA, SOURCES, check_pins, fresh, metadata, preservation, save, sha
from run import prepared

def main() -> None:
    checks = ROOT / "checks"
    docs_path = sorted(checks.glob("readme-*/REPORT.json"))[-1]
    entry_path = sorted(checks.glob("entry-*/REPORT.json"))[-1]
    docs, entry = json.loads(docs_path.read_text()), json.loads(entry_path.read_text())
    assert docs["status"] == entry["status"] == "passed"
    assert docs["readmeSha256"] == sha(REPO / "LIVE-ENV-DEMO-README.md") and len(docs["snippets"]) == 13
    check_pins(entry["sourcePins"])
    inherited = json.loads(Path(entry["inheritedContract"]).read_text())
    check_pins(inherited["receiptPins"])
    check_pins({str(ROOT / "bin/suspect"):CLI_SHA, str(ROOT / "main-receipt.json"):ACCEPTANCE_SHA,
                str(ROOT / "session.json"):SESSION_SHA, str(ROOT / "provenance/source-manifest.json"):SOURCE_MANIFEST_SHA,
                str(ROOT / "provenance/main-package-pins.json"):MAIN_PACKAGES_SHA,
                str(REPO / "tools/sdk-demo-live/run.py"):RUNNER_SHA})
    ready = json.loads((ROOT / "ready.json").read_text())
    check_pins({str(ROOT / "verification.json"):ready["verificationSha256"]})
    control = json.loads((ROOT / "verification.json").read_text())
    assert control["status"] == "passed" and control["checks"] == 52 and control["reusedChecks"] == 44
    assert control["freshGoDartChecks"] == 8 and control["missingEnvZeroHttpConsumers"] == 13
    assert all(row["passed"] for row in control["outcomes"])
    packages = json.loads((ROOT / "package-pins.json").read_text())
    assert packages == json.loads((ROOT / "provenance/main-package-pins.json").read_text()) and len(packages) == 611
    actual_packages = {str(p.relative_to(ROOT / "packages")):sha(p) for p in (ROOT / "packages").rglob("*") if p.is_file()}
    assert actual_packages == packages
    preserved = preservation()
    owner_path = ROOT / "provenance/main-provisional-native-owner-receipt.json"
    assert sha(owner_path) == entry["mainProvisionalReceiptSha256"]
    owner = json.loads(owner_path.read_text())
    for filename, expected in owner["verified"].items():
        actual = metadata(Path(filename))
        assert all(actual[key] == value for key, value in expected.items()), filename
    assert not (ROOT / "live-runs").exists(), "Account execution must not be inferred from preparation receipts"
    assert (REPO / "demo-live-env.sh").stat().st_mode & 0o111
    for path in (REPO / "tools/sdk-demo-live/environment").glob("*.py"):
        ast.parse(path.read_text(), filename=str(path))

    files = {}
    def pin(path):
        if path.is_file() and "__pycache__" not in path.parts:
            files[str(path)] = sha(path)
    def tree(directory):
        for path in directory.rglob("*"):
            pin(path)
    for path in (REPO / "LIVE-ENV-DEMO-README.md", REPO / "demo-live-env.sh"):
        pin(path)
    for directory in (REPO / "tools/sdk-demo-live/environment", SOURCES, ROOT / "packages", ROOT / "provenance", ROOT / "checks", ROOT / "verification"):
        tree(directory)
    for path in ROOT.iterdir():
        pin(path)
    pin(ROOT / "bin/suspect")
    programs = {}
    index = json.loads((ROOT / "native-index.json").read_text())
    for language in (*LANGUAGES, "javascript"):
        native = "typescript" if language == "javascript" else language
        info = prepared(language)
        plan = index[native]
        proof = json.loads(Path(plan["verification"]).read_text())
        files.update(plan["manifestPins"])
        files.update(proof["executionPins"])
        check_pins({str(Path(info["package"]) / name):digest for name, digest in plan["packageFiles"].items()})
        for name in plan["packageFiles"]:
            pin(Path(info["package"]) / name)
        attempt, consumer = Path(info["attempt"]), Path(info["consumer"])
        tree(attempt / "commands")
        for pattern in ("*.tgz", "*.gem", "dist/*.whl", "feed/*.nupkg"):
            for path in attempt.glob(pattern): pin(path)
        for filename in ("go.mod", "Cargo.toml", "Cargo.lock", "Package.swift", "pom.xml", "package.json", "package-lock.json", "Live.csproj", "composer.json", "composer.lock", "pubspec.yaml", "pubspec.lock", "CMakeLists.txt"):
            pin(consumer / filename)
        if native == "cpp": tree(attempt / "install")
        for tool in info["tools"].values():
            check_pins({tool["path"]:tool["sha256"]})
            files[tool["path"]] = tool["sha256"]
        programs[language] = {"package":info["packageConfig"], "argv":info["runArgv"], "cwd":info["consumer"], "prepared":plan["prepared"], "controlledVerification":proof["report"], "ready":True, "reusedExactPreparedScope":native in REUSED, "liveExecutedDuringStaging":False}
    # Retain the independently accepted partial receipt's complete historical pins.
    for filename, expected in owner["verified"].items():
        files[filename] = expected["sha256"]
    lineage = {}
    for path in (REPO / "target/sdk-demo-readme-20260911-01/candidate-02/delivery-01/SHA256SUMS.json",
                 REPO / "target/sdk-demo-readme-20260911-01/candidate-02/delivery-02/seal.json",
                 REPO / "target/sdk-demo-live-20260911-01/candidate-02/delivery-02/seal.json",
                 OBSERVABILITY / "delivery-01/seal.json"):
        pin(path)
        lineage[str(path)] = sha(path)
    check_pins(files)
    destination = fresh(ROOT, "delivery")
    report = {"format":"suspect.sdk.live-env-demo.delivery.v1", "status":"staged-ready-not-live-executed",
        "readme":str(REPO / "LIVE-ENV-DEMO-README.md"), "readmeSha256":sha(REPO / "LIVE-ENV-DEMO-README.md"), "entrypoint":"./demo-live-env.sh all", "entryExecutable":True,
        "preparedRoot":str(ROOT), "cliSha256":CLI_SHA, "sourceManifestSha256":SOURCE_MANIFEST_SHA, "sourceFiles":1161, "mainAcceptanceSha256":ACCEPTANCE_SHA,
        "mainPackagePinsSha256":MAIN_PACKAGES_SHA, "localPackagePinsSha256":sha(ROOT / "package-pins.json"), "configuredArtifacts":610, "packageFilesIncludingOwnership":611,
        "automaticSdkCredentialEnvConfigured":True, "policy":{"version":"v1", "schemes":{"apiKey":"OPENROUTER_API_KEY"}}, "credentialTransport":"runtime environment or secure prompt; generated client-creation snapshot",
        "sourceServer":"https://openrouter.ai/api/v1", "defaultOperation":"getCurrentKey", "defaultRoute":"GET /key", "managementKeyOptIn":"getCredits / GET /credits",
        "nativeTargets":12, "javascriptBonus":True, "programs":programs, "controlledOutcomes":52, "reusedControlledOutcomes":44, "freshGoDartOutcomes":8, "controlledLoopbackRequests":39, "missingEnvZeroHttpConsumers":13,
        "exactReadmeSnippetsChecked":13, "readmeLinksChecked":docs["linksChecked"], "docsCheck":str(docs_path), "adapterCheck":str(entry_path), "newNativeEnvPreflight":"12/12",
        "inheritedRunnerSha256":RUNNER_SHA, "inheritedContract":entry["inheritedContract"], "inheritedF10Outcomes":16, "f10OutcomesReplayed":False, "historicalInvalidCanarySetup":inherited["discardedSetupAttempt"],
        "priorSealPins":lineage, "preservation":preserved, "independentProvisionalEntriesPreserved":owner["entriesVerified"], "sdkSourcesPatched":False,
        "realTokenUsedDuringEnvPreparation":False, "realApiContactedDuringEnvPreparation":False, "liveSuccessClaim":False, "originalLiveEntryChanged":False,
        "nativeSdkMatricesReplayed":False, "numericalQualification":False, "implementationBlockers":[], "filesSealed":len(files)}
    save(destination / "REPORT.json", report)
    save(destination / "SHA256SUMS.json", files)
    save(destination / "seal.json", {"reportSha256":sha(destination / "REPORT.json"), "manifestSha256":sha(destination / "SHA256SUMS.json"), "sealedFiles":len(files)})
    print(f"ENV delivery sealed: {destination}; {len(files)} hashes; {preserved['filesChecked']} earlier files and {owner['entriesVerified']} accepted provisional entries preserved; 52 controlled outcomes; no live claim")

if __name__ == "__main__": main()
