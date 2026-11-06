#!/usr/bin/env python3
"""Focused supplemental native controls; never replays the sealed CLI lifecycle matrix."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import traceback

spec = importlib.util.spec_from_file_location("terraform_acceptance", Path(__file__).with_name("sdk-terraform-acceptance.py"))
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    args = parser.parse_args()
    root, evidence, fixtures = args.root.resolve(), args.evidence.resolve(), args.fixtures.resolve()
    assert root.is_relative_to(acceptance.APPROVED)
    acceptance.require_terraform()
    run = acceptance.Runner(evidence)
    report = {"format": "suspect.terraform.native-variants.v1", "native_root": str(root), "local_acceptance_only": True, "passed": False}
    try:
        for variant in ("optional-computed", "anonymous"):
            directory = root / variant
            package = directory / "generated/terraform"
            original = acceptance.tree_hashes(directory / "generated")
            manifest = json.loads((package / "source-bindings.json").read_text())
            config = manifest["configuration"]
            proxy, info = acceptance.prepare_proxy(directory, directory / "generated/go", config)
            acceptance.save_json(evidence / f"{variant}-sdk.json", info)
            acceptance.save_json(evidence / f"{variant}-source-hashes.json", original)
            if variant == "optional-computed":
                for name in ("provider_test.go", "optional_computed_test.go"):
                    shutil.copy2(fixtures / name, package / "provider" / name)
                selector = "TestOptionalComputed|TestUpdateTrigger"
            else:
                shutil.copy2(fixtures / "anonymous_test.go", package / "provider/anonymous_test.go")
                selector = "TestAnonymous"
            # Physical source identities are deliberately fresh in the anonymous
            # SDK. focused_go_env purges the unpublished module's host from the
            # shared cache for every variant; only the checksummed public
            # dependency/toolchain cache is reused.
            env = acceptance.focused_go_env(proxy, config["sdk"]["module_path"])
            floor, go = config["go_toolchain"], acceptance.go_binary()
            for tier, selection in acceptance.go_tiers(floor).items():
                selected = {**env, **selection}
                version = run.run(f"{variant}-{tier}-version", [go, "env", "GOVERSION"], package, selected, timeout=600).decode().strip()
                acceptance.claim_go_tier(run, f"{variant}-{tier}-pin", tier, floor, version)
                run.run(f"{variant}-{tier}-tests", [go, "test", "-mod=readonly", "-race", "-count=1", "-v", "-run", selector, "./..."], package, selected, timeout=900)
                modules = list(acceptance.json_stream(run.run(f"{variant}-{tier}-sdk-link", [go, "list", "-m", "-json", "all"], package, selected)))
                acceptance.save_json(evidence / f"{variant}-{tier}-modules.json", modules)
                sdk = next(m for m in modules if m["Path"] == config["sdk"]["module_path"])
                run.claim(f"{variant}-{tier}-exact-sdk", sdk["Version"] == info["version"] and sdk["Sum"] == info["sum"] and "Replace" not in sdk, sdk)
            actual = acceptance.tree_hashes(directory / "generated")
            run.claim(f"{variant}-emitted-bytes", all(actual[name] == digest for name, digest in original.items()))
        acceptance.save_json(evidence / "base-source-hashes.json", acceptance.tree_hashes(root / "base/generated"))
        report["passed"] = True
    except Exception:
        report["failure"] = traceback.format_exc()
        print(report["failure"])
    finally:
        report["commands"] = run.commands
        report["gates"] = run.claims
        acceptance.save_json(evidence / "report.json", report)
    print(json.dumps({"passed": report["passed"], "commands": len(run.commands), "gates": len(run.claims)}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
