#!/usr/bin/env python3
"""Focused supplemental native controls; never replays the sealed CLI lifecycle matrix."""
import argparse
import importlib.util
import json
import os
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
            # SDK. Give its unpublished module a fresh namespace cache; reuse
            # only the already checksummed public dependency/toolchain cache.
            shared_cache = acceptance.APPROVED / "sdk-terraform-dependency-cache-20260910-01"
            cache = directory / "module-cache"
            cache.mkdir()
            private_host = config["sdk"]["module_path"].split("/")[0]
            for child in shared_cache.iterdir():
                if child.name not in ("cache", private_host):
                    (cache / child.name).symlink_to(child, target_is_directory=child.is_dir())
            download = cache / "cache/download"
            download.mkdir(parents=True)
            for child in (shared_cache / "cache/download").iterdir():
                if child.name != private_host:
                    (download / child.name).symlink_to(child, target_is_directory=child.is_dir())
            env = {k: v for k, v in os.environ.items() if not k.startswith(("GO", "TF_"))}
            env.update(GOWORK="off", GOMODCACHE=str(cache), GOCACHE=str(acceptance.APPROVED / "sdk-terraform-build-cache-20260910-01"), GOPROXY=f"file://{proxy},https://proxy.golang.org", GONOSUMDB=config["sdk"]["module_path"])
            for tier in ("go1.23.12", "go1.27.1"):
                selected = {**env, "GOTOOLCHAIN": tier}
                version = run.run(f"{variant}-{tier}-version", ["go", "version"], package, selected)
                run.claim(f"{variant}-{tier}-pin", tier.encode() in version)
                run.run(f"{variant}-{tier}-tests", ["go", "test", "-mod=readonly", "-race", "-count=1", "-v", "-run", selector, "./..."], package, selected, timeout=180)
                modules = list(acceptance.json_stream(run.run(f"{variant}-{tier}-sdk-link", ["go", "list", "-m", "-json", "all"], package, selected)))
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
