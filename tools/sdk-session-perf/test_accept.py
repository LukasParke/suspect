"""Acceptance bridge adversarial checks; synthetic records are not performance evidence."""

import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import accept
import compare
import run


def file_record(path):
    data = path.read_bytes()
    return {"kind": "file", "sha256": accept.byte_hash(data), "bytes": len(data),
            "mode": stat.S_IMODE(path.stat().st_mode) & 0o777, "link": None}


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sdk-session-accept-test-", dir=run.ROOT / "target")
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def sources(self):
        original, snapshot, output = (self.root / name for name in ("original", "snapshot", "acceptance"))
        original.mkdir()
        snapshot.mkdir()
        output.mkdir()
        subprocess.run(["git", "init", "--quiet", str(original)], check=True)
        names = {"Cargo.toml", "Cargo.lock", "crates/model/src/lib.rs", *run.HARNESS_FILES}
        for name in names:
            path = original / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"synthetic source for {name}\n")
            target = snapshot / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
        inventory = {name: file_record(original / name) for name in sorted(names)}
        compare.write_new(output / "source-manifest.json", inventory)
        (output / "bin").mkdir()
        (output / "bin/suspect").write_bytes(b"synthetic frozen CLI")
        args = argparse.Namespace(original=original, snapshot=snapshot, acceptance_root=output,
                                  source_sha256=accept.rust_inventory_digest(inventory),
                                  cli_sha256=accept.byte_hash(b"synthetic frozen CLI"))
        report = {"provenance": {"source": run.source_manifest(original)},
                  "identity": {"harness": run.manifest(list(run.HARNESS_FILES), original)}}
        return args, report

    def test_private_unborn_snapshot_binds_to_real_source_bytes_and_cli(self):
        args, report = self.sources()
        # The snapshot has no Git history, exactly the final runner's constraint.
        subprocess.run(["git", "init", "--quiet", str(args.snapshot)], check=True)
        _, verified = accept.snapshot_source(args, [report])
        self.assertTrue(verified["original_and_snapshot_match"])
        self.assertEqual(verified["acceptance_source_sha256"], args.source_sha256)
        (args.snapshot / "crates/model/src/lib.rs").write_text("changed after snapshot")
        with self.assertRaises(compare.ReportError):
            accept.snapshot_source(args, [report])

    def test_tag_only_source_fingerprint_cannot_hide_a_changed_census_or_binary(self):
        for mutation in ("extra", "binary", "tag"):
            with self.subTest(mutation=mutation):
                # A separate fixture prevents one mutation masking another.
                self.temporary.cleanup()
                self.setUp()
                args, report = self.sources()
                if mutation == "extra":
                    (args.original / "crates/model/src/new.rs").write_text("new source")
                elif mutation == "binary":
                    (args.acceptance_root / "bin/suspect").write_bytes(b"different CLI")
                else:
                    report["provenance"]["source"]["sha256"] = compare.digest("a trusted-looking tag")
                with self.assertRaises(compare.ReportError):
                    accept.snapshot_source(args, [report])

    def test_evidence_pin_checks_actual_bytes_and_duplicate_json(self):
        path = self.root / "input.json"
        path.write_text('{"complete":true}')
        pin = {"path": str(path), "sha256": accept.byte_hash(path.read_bytes())}
        self.assertEqual(accept.read_pin(pin)[1], {"complete": True})
        path.write_text('{"complete":false}')
        with self.assertRaises(compare.ReportError):
            accept.read_pin(pin)
        path.write_text('{"complete":true,"complete":false}')
        pin["sha256"] = accept.byte_hash(path.read_bytes())
        with self.assertRaises(compare.ReportError):
            accept.read_pin(pin)

    def test_tool_checks_hash_the_executable_instead_of_only_its_version_tag(self):
        tools = {}
        for name in ("rustc", "cargo", "python", "git", "cc"):
            executable = self.root / name
            executable.write_bytes(f"synthetic {name}".encode())
            flags = ["-Vv"] if name in ("rustc", "cargo") else ["-VV"] if name == "python" else ["--version"]
            tools[name] = {**run.file_fingerprint(executable), "query": [str(executable), *flags], "version": "synthetic version"}
        with patch.object(run, "output", return_value="synthetic version"):
            accept.verify_tools(tools, self.root)
            (self.root / "cc").write_bytes(b"changed but same claimed version")
            with self.assertRaises(compare.ReportError):
                accept.verify_tools(tools, self.root)

    def test_prepared_input_archive_and_private_input_copy_are_both_verified(self):
        original, snapshot, output = (self.root / name for name in ("original", "snapshot", "acceptance"))
        source_dir, copy_dir = original / accept.M2, snapshot / accept.M2
        source_dir.mkdir(parents=True)
        copy_dir.mkdir(parents=True)
        source = source_dir / "canonical.openapi.yaml"
        source.write_bytes(b"original fixture bytes")
        shutil.copy2(source, copy_dir / source.name)
        prepared = original / "target/suite/prepared-inputs/m2-small-all"
        prepared.mkdir(parents=True)
        (prepared / source.name).write_bytes(b'{"prepared":"fixture"}')
        case = {"id": "m2-small/all", "prepared_input_root": str(prepared), "report": {
            "fixture": "m2-small", "input_root": str(source_dir),
            "inputs": [run.file_fingerprint(source, source.name)],
            "prepared_inputs": [run.file_fingerprint(prepared / source.name, source.name)],
        }}
        report = {"cases": [case], "provenance": {"inputs": [], "binary": {"path": str(original / "target/suite/bin/bench")}}}
        args = argparse.Namespace(original=original, snapshot=snapshot, acceptance_root=output, corpus=self.root / "public")
        checked, copied = accept.verify_inputs(args, report)
        self.assertTrue(checked[0]["original_and_snapshot_match"])
        self.assertEqual(copied[0][1], b'{"prepared":"fixture"}')
        (prepared / source.name).write_bytes(b"not the measured private bytes")
        with self.assertRaises(compare.ReportError):
            accept.verify_inputs(args, report)

    def test_collection_claims_count_measured_configuration_and_refuse_warm_work(self):
        sample = {"delta": {"compiles": 0, "renders": 0}, "fresh_oracle_equal": True, "disk_current": True,
                  "redundant_rewrites": 0, "changed_paths": [], "generate": {"ms": 1.0}, "write": {"ms": 1.0}}
        cases = []
        for group in ("typescript-http", "all"):
            cases.append({"id": f"m2-small/{group}", "report": {"fixture": "m2-small",
                "samples": [{**copy.deepcopy(sample), "scenario": scenario} for scenario in ("cold", "warm", "schema-edit", "operation-edit", "docs-edit")],
                "configuration_probe": {"change": copy.deepcopy(sample)},
                "module_probe": {"change": copy.deepcopy(sample)} if group == "all" else None,
            }})
        result = accept.summary([{"cases": cases}], "m2-small")
        self.assertEqual(result["cold"]["samples"], 2)
        self.assertEqual(result["sourceChange"]["samples"], 6)
        self.assertEqual(result["configChange"]["samples"], 3)
        self.assertEqual(result["warm"]["writes"], 0)
        cases[0]["report"]["samples"][1]["changed_paths"] = ["stale.ts"]
        with self.assertRaises(compare.ReportError):
            accept.summary([{"cases": cases}], "m2-small")
        cases[0]["report"]["samples"][1]["changed_paths"] = []
        cases[0]["report"]["configuration_probe"]["change"]["write"]["ms"] = 0
        with self.assertRaises(compare.ReportError):
            accept.summary([{"cases": cases}], "m2-small")

    def test_imported_blobs_are_create_once_and_confined(self):
        root = self.root / "acceptance"
        root.mkdir()
        pin = accept.copy_blob(root, "candidate", b"raw evidence")
        self.assertEqual(accept.copy_blob(root, "candidate", b"raw evidence"), pin)
        destination = root / pin["path"]
        self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o444)
        destination.chmod(0o644)
        destination.write_bytes(b"tampered")
        with self.assertRaises(compare.ReportError):
            accept.copy_blob(root, "candidate", b"raw evidence")
        outside = self.root / "outside"
        outside.mkdir()
        shutil.rmtree(root / "performance/raw")
        (root / "performance/raw").symlink_to(outside)
        with self.assertRaises(compare.ReportError):
            accept.copy_blob(root, "candidate", b"other evidence")
        self.assertFalse(list(outside.iterdir()))

    def test_environment_replay_is_scoped_to_read_only_verification(self):
        original = os.environ.get("RUSTFLAGS")
        with accept.recorded_environment({"RUSTFLAGS": "synthetic flags"}):
            self.assertEqual(os.environ["RUSTFLAGS"], "synthetic flags")
        self.assertEqual(os.environ.get("RUSTFLAGS"), original)


if __name__ == "__main__":
    unittest.main()
