#!/usr/bin/env python3
"""Adopt exact preparations and inspect existing probe records; never run an SDK."""
from __future__ import annotations
import json
from pathlib import Path
import shutil

from support import ACCEPTANCE_SHA, CLI_SHA, LANGUAGES, MAIN, MAIN_PACKAGES_SHA, OBSERVABILITY, PROVISIONAL, REPO, REUSED, ROOT, SESSION_SHA, SOURCE_MANIFEST_SHA, check_pins, load_base, metadata, save, sha

def main() -> None:
    if (ROOT / "ready.json").exists():
        raise RuntimeError("This cohort is already assembled; retain it and use a new candidate for changes")
    check_pins({str(MAIN / "ENV-ACCEPTANCE.json"):ACCEPTANCE_SHA,
                str(ROOT / "main-receipt.json"):ACCEPTANCE_SHA,
                str(MAIN / "source-manifest.json"):SOURCE_MANIFEST_SHA,
                str(MAIN / "live-generation-01/package-pins.json"):MAIN_PACKAGES_SHA,
                str(ROOT / "session.json"):SESSION_SHA,
                str(ROOT / "bin/suspect"):CLI_SHA})
    acceptance = json.loads((ROOT / "main-receipt.json").read_text())
    check_pins(acceptance["verified"])
    assert acceptance["independentReviews"] == {"Standards":"clean", "Spec":"clean"}
    assert acceptance["hostTests"]["failed"] == 0 and acceptance["sourceBytesStable"] is True
    expected = json.loads((MAIN / "live-generation-01/package-pins.json").read_text())
    copied = {str(p.relative_to(ROOT / "packages")):sha(p) for p in (ROOT / "packages").rglob("*") if p.is_file()}
    assert copied == expected == json.loads((ROOT / "package-pins.json").read_text()) and len(copied) == 611
    check_pins({str(MAIN / "live-generation-01/packages" / name):digest for name, digest in expected.items()})
    provenance = ROOT / "provenance"
    provenance.mkdir(exist_ok=False)
    for source, name in ((MAIN / "source-manifest.json", "source-manifest.json"),
                         (MAIN / "live-generation-01/package-pins.json", "main-package-pins.json"),
                         (MAIN / "live-generation-01/completion-02/REPORT.json", "main-generation-completion.json"),
                         (MAIN / "live-generation-01/prepared-scope-parity.json", "main-prepared-scope-parity.json")):
        shutil.copy2(source, provenance / name)
    save(provenance / "acceptance-checked.json", {"acceptanceSha256":ACCEPTANCE_SHA, "verifiedMainPins":len(acceptance["verified"]), "all611CopiedFilesEqualMain":True, "sourceManifestSha256":SOURCE_MANIFEST_SHA, "sourceFiles":1161, "cliSha256":CLI_SHA, "generationReplayed":False})

    protected = json.loads((OBSERVABILITY / "preservation-before.json").read_text())
    for filename, expected_metadata in protected.items():
        assert metadata(Path(filename)) == expected_metadata, filename
    for filename, digest in json.loads((OBSERVABILITY / "delivery-01/SHA256SUMS.json").read_text()).items():
        current = metadata(Path(filename))
        assert current["sha256"] == digest, filename
        protected[filename] = current
    for path in (OBSERVABILITY / "delivery-01").iterdir():
        if path.is_file(): protected[str(path)] = metadata(path)
    for path in (REPO / "DEMO-DX-README.md", REPO / "examples/sdk-demo-branded.json",
                 REPO / "target/sdk-credential-env-integration-20260911-01/live-observability-owner-receipt-01.json"):
        protected[str(path)] = metadata(path)
    for path in (REPO / "target/sdk-demo-dx-20260911-01/baseline-01").rglob("*"):
        if path.is_file(): protected[str(path)] = metadata(path)
    save(ROOT / "preservation-before.json", protected)

    base = load_base()
    programs, reuse, outcomes = {}, {}, []
    provisional = json.loads((PROVISIONAL / "package-pins.json").read_text())
    check_pins({str(PROVISIONAL / "packages" / name):digest for name, digest in provisional.items()})
    for language in LANGUAGES:
        parent = PROVISIONAL if language in REUSED else ROOT
        attempt = parent / "native" / f"{language}-01"
        ready_path, proof_path = attempt / "ready.json", attempt / "environment-verification.json"
        info, proof = json.loads(ready_path.read_text()), json.loads(proof_path.read_text())
        generated = {name.split("/",1)[1]:digest for name, digest in expected.items() if name.startswith(language + "/")}
        source_path = attempt / "source-pins.json"
        assert json.loads(source_path.read_text()) == generated, language
        check_pins({str(Path(info["package"]) / name):digest for name, digest in generated.items()})
        check_pins({str(REPO / name):digest for name, digest in info["snippets"].items()})
        assert proof["passed"] is True and proof["liveExecuted"] is False
        assert proof["candidate"] == str(parent)
        assert proof["packageCohortPinsSha256"] == sha(parent / "package-pins.json")
        check_pins(proof["executionPins"])
        if language in REUSED:
            old = {name.split("/",1)[1]:digest for name, digest in provisional.items() if name.startswith(language + "/")}
            assert old == generated, language
            reuse[language] = {"prepared":str(ready_path), "packageFilesEqual":len(generated), "allGeneratedSourceBytesEqual":True, "installedExecutionPinsChecked":len(proof["executionPins"]), "controlledOutcomesReused":proof["outcomes"], "sdkOrConsumerRebuilt":False}
        report_path = Path(proof["report"])
        report = json.loads(report_path.read_text())
        wire = json.loads((report_path.parent / "wire.json").read_text())
        expected_consumers = (language, "javascript") if language == "typescript" else (language,)
        assert {(row["consumer"], row["case"]) for row in report["outcomes"]} == {(name, case) for name in expected_consumers for case in ("key", "credits", "denied", "missing-env")}
        assert len(report["outcomes"]) == proof["outcomes"] == 4 * len(expected_consumers)
        assert len(wire) == 3 * len(expected_consumers) and all(row["valid"] for row in wire)
        assert report["realApiContacted"] is False
        receipt_pins = {str(p):sha(p) for p in report_path.parent.iterdir() if p.is_file()}
        for row in report["outcomes"]:
            stdout = (report_path.parent / f"{row['consumer']}-{row['case']}.stdout").read_text()
            stderr = (report_path.parent / f"{row['consumer']}-{row['case']}.stderr").read_text()
            assert stderr == "" and base.validate_response(stdout, row["operation"]) == row["response"], row
            assert row["passed"] is True
            success = row["case"] in ("key", "credits")
            assert row["exitCode"] == (0 if success else 1) and row["response"]["ok"] is success
            if success:
                assert row["response"]["status"] == 200
                assert row["response"]["usage"] == ("9007199254740993.000000000000000001" if row["case"] == "key" else "25.75")
                if row["case"] == "credits": assert row["response"]["credits"] == "100.50000000000000001"
            elif row["case"] == "denied": assert row["response"]["status"] == 401
            else: assert row["response"].get("status") in (None, 0) and row["requests"] == 0
            outcomes.append({**row, "report":str(report_path), "reusedWithoutReplay":language in REUSED})
        programs[language] = {"prepared":str(ready_path), "verification":str(proof_path), "sourcePins":str(source_path), "packageFiles":generated, "reusedExactScope":language in REUSED,
            "manifestPins":{str(ready_path):sha(ready_path), str(proof_path):sha(proof_path), str(source_path):sha(source_path), **receipt_pins}}
    assert len(outcomes) == 52 and len(reuse) == 10
    differences = [name for name in expected if expected[name] != provisional.get(name)]
    assert set(differences) == {".suspect-artifacts.json", "dart/README.md", "dart/doc/CREDENTIAL-ENV.md", "dart/lib/src/client.dart"}
    save(ROOT / "prepared-scope-parity.json", {"status":"passed", "replacementCliSha256":CLI_SHA, "differencesFromProvisional":differences, "reused":reuse, "freshlyBuilt":["go", "dart"], "sdkSourcesPatched":False, "reusedControlledOutcomes":44})
    save(ROOT / "native-index.json", programs)
    save(ROOT / "verification.json", {"status":"passed", "scope":"new configured-env consumers only", "outcomes":outcomes, "checks":52, "reusedChecks":44, "freshGoDartChecks":8, "missingEnvZeroHttpConsumers":13, "realApiContacted":False})
    save(ROOT / "ready.json", {"status":"ready", "nativeTargets":12, "javascriptBonus":True, "nativeIndexSha256":sha(ROOT / "native-index.json"), "packagePinsSha256":sha(ROOT / "package-pins.json"), "mainPackagePinsSha256":MAIN_PACKAGES_SHA, "cliSha256":CLI_SHA, "acceptanceSha256":ACCEPTANCE_SHA, "verificationSha256":sha(ROOT / "verification.json"), "liveExecutedDuringPreparation":False})
    print(f"ENV preparations assembled: 12 native targets + JS, 52 controlled outcomes (44 reused by exact parity, 8 fresh); {len(protected)} earlier files protected")

if __name__ == "__main__": main()
