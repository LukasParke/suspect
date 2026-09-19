#!/usr/bin/env python3
"""Optional local Terraform walkthrough using the sealed Go-SDK-backed fixture provider."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil

from demo import REPO, owned_root, record, sha, verify_pins, write_json

EVIDENCE = REPO / "target/sdk-terraform-stretch-20260910-01/native-02"
PROVIDER_SHA = "159f8433f828a609aac24da6d14182e756e06bf77d90d843fb9cd0fc27c64111"


def prepare(root: Path) -> None:
    work = root / "terraform-demo"
    work.mkdir(exist_ok=False)
    fixture = REPO / "crates/suspect-codegen/tests/fixtures/terraform-v1"
    args = [str(root / "bin/suspect"), "codegen-terraform", str(fixture / "openapi.json"),
            "--mapping", str(fixture / "mapping.json"), "--target-config", str(fixture / "target.json"),
            "--out", str(work / "generated"), "--format", "json"]
    record(work, "generate", args)
    record(work, "drift", [*args, "--check"])
    binary = EVIDENCE / "go1.23.12/terraform-provider-fixture_v0.1.0"
    if sha(binary) != PROVIDER_SHA:
        raise SystemExit("Sealed provider binary hash changed.")
    original = json.loads((EVIDENCE / "generated-source-hashes.json").read_text())
    compared = {name: digest for name, digest in original.items() if not name.startswith(".")}
    changed = [name for name, digest in compared.items() if sha(work / "generated" / name) != digest]
    write_json(work / "native-artifact-parity.json", {"checked": len(compared), "changed": changed})
    if changed:
        raise SystemExit(f"Fresh CLI output differs from the retained native fixture: {changed}")
    mirror = work / "mirror/registry.terraform.io/suspect/fixture/0.1.0/darwin_arm64"
    mirror.mkdir(parents=True)
    shutil.copy2(binary, mirror / binary.name)
    cli = work / "terraform.rc"
    cli.write_text('provider_installation {\n  filesystem_mirror {\n'
                   f'    path = "{work / "mirror"}"\n'
                   '    include = ["registry.terraform.io/suspect/fixture"]\n  }\n}\n')
    helper = work / "fixture-support.py"
    shutil.copy2(REPO / "tools/sdk-terraform-acceptance.py", helper)
    terraform = Path(json.loads((EVIDENCE / "terraform-tool.json").read_text())["path"])
    version = record(work, "terraform-version", [str(terraform), "version", "-json"])
    if json.loads(version.stdout)["terraform_version"] != "1.15.8":
        raise SystemExit("Use the retained Terraform 1.15.8 tool.")
    write_json(work / "prepared.json", {"terraform": str(terraform), "terraformSha256": sha(terraform),
        "provider": str(mirror / binary.name), "providerSha256": PROVIDER_SHA,
        "fixtureSupportSha256": sha(helper), "sdkVersion": "v0.4.2",
        "nativeSourceParity": len(compared), "reviewStatus": "independent review tracked by Main"})
    print(f"TERRAFORM PREPARED: {work}; {len(compared)} artifacts match the native witness.")


def run_demo(root: Path, label: str) -> None:
    work = root / "terraform-demo"
    info = json.loads((work / "prepared.json").read_text())
    if sha(Path(info["terraform"])) != info["terraformSha256"] or sha(Path(info["provider"])) != info["providerSha256"]:
        raise SystemExit("Prepared Terraform/provider binary changed.")
    support = work / "fixture-support.py"
    if sha(support) != info["fixtureSupportSha256"]:
        raise SystemExit("Prepared fixture helper changed.")
    module_spec = importlib.util.spec_from_file_location("terraform_fixture", support)
    assert module_spec is not None and module_spec.loader is not None
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    receipt = work / "runs" / label
    receipt.mkdir(parents=True, exist_ok=False)
    consumer = receipt / "consumer"
    consumer.mkdir()
    shutil.copy2(work / "generated/terraform/examples/resources/record/main.tf", consumer / "main.tf")
    fixture = module.Fixture()
    fixture.seed("demo-import", name="alpha")
    env = {"TF_CLI_CONFIG_FILE": str(work / "terraform.rc"), "TF_IN_AUTOMATION": "1", "CHECKPOINT_DISABLE": "1",
           "TF_VAR_endpoint": fixture.endpoint, "TF_VAR_token": "fixture-token", "TF_VAR_input_name": "alpha",
           "TF_VAR_input_region": "east", "TF_VAR_input_enabled": "true"}
    for key in os.environ:
        if key.startswith("TF_VAR_") and key not in env:
            env[key] = ""
    write_json(receipt / "environment.json", env)

    def tf(stage: str, *args: str) -> str:
        fixture.action = stage
        output = record(receipt, stage, [info["terraform"], *args], cwd=consumer, env=env, timeout=90).stdout
        print(f"PASS terraform {' '.join(args)}")
        return output

    try:
        tf("init", "init", "-input=false", "-no-color")
        valid = json.loads(tf("validate", "validate", "-json"))
        if not valid["valid"]:
            raise SystemExit("Generated HCL was not valid.")
        tf("plan", "plan", "-input=false", "-no-color", "-out=create.tfplan")
        tf("apply", "apply", "-input=false", "-no-color", "create.tfplan")
        tf("no-change", "plan", "-input=false", "-no-color", "-detailed-exitcode")
        tf("refresh", "refresh", "-input=false", "-no-color")
        tf("destroy-created", "destroy", "-auto-approve", "-input=false", "-no-color")
        tf("import", "import", "-input=false", "-no-color", "fixture_record.example", "demo-import")
        state = json.loads(tf("state", "show", "-json"))
        write_json(receipt / "imported-state.json", state)
        tf("destroy-imported", "destroy", "-auto-approve", "-input=false", "-no-color")
        if fixture.failures or fixture.records:
            raise SystemExit(f"Fixture did not complete: {fixture.failures}")
        print("TERRAFORM OFFLINE OK: create, read, refresh, opaque import, destroy through the pinned Go SDK.")
    finally:
        fixture.close()
        write_json(receipt / "wire.json", {"loopbackOnly": True, "requests": fixture.requests, "failures": fixture.failures})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("action", choices=("prepare", "run"))
    parser.add_argument("--label", default="morning-01")
    args = parser.parse_args()
    if not args.label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in args.label):
        raise SystemExit("Use a lowercase alphanumeric/hyphen run label.")
    root = owned_root(args.root)
    verify_pins(root)
    prepare(root) if args.action == "prepare" else run_demo(root, args.label)


if __name__ == "__main__":
    main()
