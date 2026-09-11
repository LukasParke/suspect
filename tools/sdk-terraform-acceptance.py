#!/usr/bin/env python3
"""Strict local Terraform acceptance over freshly emitted provider + SDK bytes.

Called by the maintained Rust native selector. Every command, expected failure,
wire exchange, source/dependency/package hash, and attempt is retained. There is
no production API, registry publishing, or mutable global Terraform installation.
"""

from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import http.server
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import tarfile
import threading
import time
import traceback
import urllib.parse
import zipfile


APPROVED = Path("/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode")
TERRAFORM = Path("/Users/luke/.local/share/mise/installs/terraform/latest/terraform")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n")


def tree_hashes(path: Path) -> dict[str, str]:
    return {p.relative_to(path).as_posix(): sha(p) for p in sorted(path.rglob("*")) if p.is_file()}


def module_hash(files: dict[str, bytes]) -> str:
    summary = "".join(f"{hashlib.sha256(body).hexdigest()}  {name}\n" for name, body in sorted(files.items()))
    return "h1:" + base64.b64encode(hashlib.sha256(summary.encode()).digest()).decode()


class Runner:
    def __init__(self, evidence: Path):
        self.evidence = evidence
        self.logs = evidence / "commands"
        self.logs.mkdir()
        self.commands = []
        self.claims = []

    def record(self, label, args, cwd, env, code, out, err, expected, duration):
        index = len(self.commands) + 1
        prefix = f"{index:03d}-{label}"
        (self.logs / f"{prefix}.stdout").write_bytes(out)
        (self.logs / f"{prefix}.stderr").write_bytes(err)
        record = {
            "label": label, "command": [str(a) for a in args], "cwd": str(cwd),
            "environment": {k: v for k, v in env.items() if k.startswith(("GO", "TF_"))},
            "exit_code": code, "expected_exit_codes": sorted(expected), "duration_seconds": duration,
            "stdout": f"commands/{prefix}.stdout", "stderr": f"commands/{prefix}.stderr",
        }
        save_json(self.logs / f"{prefix}.json", record)
        self.commands.append(record)
        if code not in expected:
            raise AssertionError(f"{label}: exit {code}, expected {expected}; retained {prefix}\n{out.decode(errors='replace')[-5000:]}\n{err.decode(errors='replace')[-5000:]}")
        return out

    def run(self, label, args, cwd, env, expected=(0,), timeout=180):
        start = time.monotonic()
        try:
            result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, capture_output=True, timeout=timeout)
            return self.record(label, args, cwd, env, result.returncode, result.stdout, result.stderr, expected, time.monotonic() - start)
        except subprocess.TimeoutExpired as error:
            self.record(label, args, cwd, env, -999, error.stdout or b"", error.stderr or b"", {-999}, time.monotonic() - start)
            raise AssertionError(f"{label}: required command timed out") from error

    def claim(self, label, condition, detail=None):
        self.claims.append({"label": label, "passed": bool(condition), "detail": detail})
        if not condition:
            raise AssertionError(f"strict gate {label} failed: {detail}")


class Fixture:
    def __init__(self):
        self.lock = threading.RLock()
        self.records = {}
        self.requests = []
        self.failures = []
        self.sequence = 0
        self.action = "setup"
        self.failed_deletes = set()
        self.cancel_started = threading.Event()
        self.cancel_closed = threading.Event()
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def do_GET(self):
                outer.handle(self)

            do_POST = do_GET
            do_PUT = do_GET
            do_DELETE = do_GET

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}/v1"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    @staticmethod
    def response(handler, status, payload):
        data = b"" if payload is None else json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
        handler.send_response(status)
        if payload is not None:
            handler.send_header("Content-Type", "application/json")
        handler.send_header("Content-Length", str(len(data)))
        handler.end_headers()
        if data:
            handler.wfile.write(data)
        handler.wfile.flush()

    def seed(self, identity, name="imported"):
        with self.lock:
            self.records[identity] = {"id": identity, "SetExtra": name, "region": "east", "enabled": True, "description": None, "credential-fingerprint": "none"}

    def handle(self, handler):
        body = handler.rfile.read(int(handler.headers.get("Content-Length", "0")))
        row = {"action": self.action, "method": handler.command, "uri": handler.path, "authorization": handler.headers.get("Authorization"), "content_type": handler.headers.get("Content-Type"), "body": body.decode(), "status": None}
        with self.lock:
            self.requests.append(row)
        try:
            assert row["authorization"] == "Bearer fixture-token", "SDK bearer authentication changed"
            assert handler.path == "/v1/records" or handler.path.startswith("/v1/records/"), "unexpected API route"
            identity = urllib.parse.unquote(handler.path[len("/v1/records/"):]) if handler.path.startswith("/v1/records/") else None
            if identity is not None:
                assert handler.path == "/v1/records/" + urllib.parse.quote(identity, safe=""), "SDK did not encode opaque ID exactly"
            value = json.loads(body) if body else None
            if handler.command in ("POST", "PUT"):
                assert row["content_type"] == "application/json", "SDK request content type changed"
                assert body == json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(), "SDK request bytes changed"
                assert isinstance(value["SetExtra"], str) and value["SetExtra"], "invented/invalid name"
                assert type(value["enabled"]) is bool, "false/unknown handling changed"
                assert "description" in value, "explicit null must not become omission"
                assert value["description"] is None or isinstance(value["description"], str)
                assert "memo" not in value or isinstance(value["memo"], str), "omitted optional became API null"
                assert "secret" not in value or isinstance(value["secret"], str) and len(value["secret"]) >= 4
            else:
                assert body == b"", "read/delete invented a body"
            if handler.command == "POST" and value["SetExtra"] == "cancel-create":
                self.cancel_started.set()
                ready, _, _ = select.select([handler.connection], [], [], 25)
                assert ready and handler.connection.recv(1) == b"", "Terraform SDK request was not cancelled"
                row["status"] = "cancelled"
                self.cancel_closed.set()
                return
            with self.lock:
                if handler.command == "POST":
                    assert identity is None
                    assert set(value) <= {"SetExtra", "region", "enabled", "description", "memo", "secret"}
                    assert value["region"] in ("east", "west")
                    if value["SetExtra"] == "fail-create":
                        status, result = 409, {"message": "response-secret-never-in-diagnostics"}
                    else:
                        self.sequence += 1
                        identity = f"r{self.sequence}/a 雪"
                        result = {k: v for k, v in value.items() if k != "secret"}
                        result.update(id=identity, **{"credential-fingerprint": hashlib.sha256(value["secret"].encode()).hexdigest() if "secret" in value else "none"})
                        self.records[identity] = result
                        status = 503 if value["SetExtra"] == "partial-create" else 201
                elif handler.command == "GET":
                    status, result = (200, self.records[identity]) if identity in self.records else (404, {"message": "missing"})
                elif handler.command == "PUT":
                    assert identity in self.records
                    assert set(value) <= {"SetExtra", "enabled", "description", "memo", "secret"}
                    if value["SetExtra"] == "fail-update":
                        status, result = 409, {"message": "response-secret-never-in-diagnostics"}
                    elif value["SetExtra"] == "partial-update":
                        self.records[identity]["SetExtra"] = value["SetExtra"]
                        status, result = 503, self.records[identity]
                    else:
                        result = self.records[identity]
                        result.update({k: v for k, v in value.items() if k != "secret"})
                        if "memo" not in value:
                            result.pop("memo", None)
                        if "secret" in value:
                            result["credential-fingerprint"] = hashlib.sha256(value["secret"].encode()).hexdigest()
                        status = 200
                elif handler.command == "DELETE":
                    if identity in self.failed_deletes:
                        status, result = 409, {"message": "delete rejected"}
                    elif identity not in self.records:
                        status, result = 404, {"message": "missing"}
                    else:
                        self.records.pop(identity)
                        status, result = 204, None
                else:
                    raise AssertionError("unexpected method")
                row["status"] = status
                row["response"] = copy.deepcopy(result)
                self.response(handler, status, result)
        except Exception:
            with self.lock:
                self.failures.append(traceback.format_exc())
            try:
                self.response(handler, 500, {"message": "fixture assertion failed"})
            except (BrokenPipeError, ConnectionResetError):
                pass


def prepare_proxy(root: Path, sdk: Path, config):
    module = config["sdk"]["module_path"]
    version = "v" + config["sdk"]["version"]
    proxy = root / "proxy"
    directory = proxy / module / "@v"
    directory.mkdir(parents=True)
    prefix = f"{module}@{version}/"
    files = {prefix + p.relative_to(sdk).as_posix(): p.read_bytes() for p in sorted(sdk.rglob("*")) if p.is_file()}
    with zipfile.ZipFile(directory / f"{version}.zip", "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            archive.writestr(info, content)
    (directory / f"{version}.mod").write_bytes((sdk / "go.mod").read_bytes())
    save_json(directory / f"{version}.info", {"Version": version, "Time": "2026-09-10T00:00:00Z"})
    (directory / "list").write_text(version + "\n")
    return proxy, {"module": module, "version": version, "sum": module_hash(files), "go_mod_sum": module_hash({"go.mod": (sdk / "go.mod").read_bytes()}), "zip_sha256": sha(directory / f"{version}.zip"), "files": {name: hashlib.sha256(content).hexdigest() for name, content in files.items()}}


def json_stream(data):
    text = data.decode()
    decoder = json.JSONDecoder()
    while text.strip():
        text = text.lstrip()
        item, end = decoder.raw_decode(text)
        yield item
        text = text[end:]


def native_packages(run, root, evidence, fixtures, config, env):
    package = root / "generated/terraform"
    emitted_mod = (package / "go.mod").read_bytes()
    emitted_sum = (package / "go.sum").read_bytes() if (package / "go.sum").exists() else None
    shutil.copy2(fixtures / "provider_test.go", package / "provider/terraform_native_test.go")
    run.run("dependencies-tidy", ["go", "mod", "tidy"], package, {**env, "GOTOOLCHAIN": config["go_toolchain"]}, timeout=300)
    shutil.copy2(package / "go.mod", evidence / "resolved.go.mod")
    shutil.copy2(package / "go.sum", evidence / "resolved.go.sum")
    # A dependency bootstrap is retained as a failed attempt until the generator
    # emits the exact package lock itself. Native gates never repair Go sources.
    run.claim("emitted-module-is-completely-pinned", emitted_mod == (package / "go.mod").read_bytes(), "go mod tidy must not change the emitted go.mod")
    run.claim("emitted-module-checksums-are-complete", emitted_sum == (package / "go.sum").read_bytes(), "go mod tidy must not change the emitted go.sum")
    binaries = {}
    for tier in ("go1.23.12", "go1.27.1"):
        selected = {**env, "GOTOOLCHAIN": tier}
        run.run(f"{tier}-version", ["go", "version"], package, selected)
        tool_env = run.run(f"{tier}-tool-env", ["go", "env", "-json", "GOVERSION", "GOROOT", "GOOS", "GOARCH", "GOTOOLCHAIN", "GOMODCACHE", "GOCACHE"], package, selected)
        info = json.loads(tool_env)
        run.claim(f"{tier}-exact-toolchain", info["GOVERSION"] == tier, info)
        run.claim(f"{tier}-native-platform", info["GOOS"] == "darwin" and info["GOARCH"] == "arm64")
        native = evidence / tier
        native.mkdir()
        save_json(native / "tool-digests.json", {"go": sha(Path(info["GOROOT"]) / "bin/go"), "compile": sha(Path(info["GOROOT"]) / "pkg/tool/darwin_arm64/compile")})
        run.run(f"{tier}-provider-tests", ["go", "test", "-mod=readonly", "-race", "-count=1", "-v", "./..."], package, selected, timeout=300)
        run.run(f"{tier}-provider-vet", ["go", "vet", "-mod=readonly", "./..."], package, selected)
        doc = run.run(f"{tier}-native-docs", ["go", "doc", "-all", "./provider"], package, selected)
        (native / "provider.godoc.txt").write_bytes(doc)
        for symbol in ["func New(", "func NewWithTransport(", "func NewResource0(", "func NewDataSource0(", "ImportState(", "Create(", "Update(", "Delete("]:
            run.claim(f"{tier}-native-doc-symbol-{symbol}", symbol.encode() in doc)
        modules = list(json_stream(run.run(f"{tier}-module-linkage", ["go", "list", "-m", "-json", "all"], package, selected)))
        save_json(native / "modules.json", modules)
        sdk = next(item for item in modules if item["Path"] == config["sdk"]["module_path"])
        expected = json.loads((evidence / "sdk-proxy.json").read_text())
        run.claim(f"{tier}-pinned-sdk-linkage", sdk["Version"] == expected["version"] and "Replace" not in sdk and sdk["Sum"] == expected["sum"] and sdk["GoModSum"] == expected["go_mod_sum"], sdk)
        binary = native / f"terraform-provider-{config['provider_name']}_v{config['version']}"
        run.run(f"{tier}-provider-build", ["go", "build", "-mod=readonly", "-trimpath", "-o", binary, "."], package, selected, timeout=300)
        build_info = run.run(f"{tier}-binary-linkage", ["go", "version", "-m", binary], package, selected)
        run.claim(f"{tier}-binary-sdk-linkage", f"{expected['module']}\t{expected['version']}\t{expected['sum']}".encode() in build_info)
        save_json(native / "binary.json", {"path": str(binary), "sha256": sha(binary), "size": binary.stat().st_size})
        binaries[tier] = binary
        # The negative consumer is in a separate module. Its local provider
        # replacement is test-only and never replaces the provider's SDK module.
        negative = root / f"negative-{tier}"
        negative.mkdir()
        (negative / "go.mod").write_text(f"module example.com/negative\n\ngo {config['go_version']}\nrequire {config['module_path']} v{config['version']}\nreplace {config['module_path']} => {package}\n")
        shutil.copy2(fixtures / "negative.go", negative / "negative.go")
        run.run(f"{tier}-negative-dependencies", ["go", "mod", "tidy"], negative, selected)
        output = run.run(f"{tier}-negative-native-consumer", ["go", "test", "./..."], negative, selected, expected=(1,))
        run.claim(f"{tier}-negative-fails-compilation", b"build failed" in output)
    return binaries


def schema_gate(run, schema, plan, tier):
    provider = schema["provider_schemas"][plan["configuration"]["provider_address"]]
    mapping = plan["mapping"]
    for kind, key in (("resources", "resource_schemas"), ("data_sources", "data_source_schemas")):
        for name, model in mapping[kind].items():
            native = provider[key][f"{plan['configuration']['provider_name']}_{name}"]["block"]
            attrs = native["attributes"]
            run.claim(f"{tier}-{kind}-{name}-exact-schema-attributes", set(attrs) == set(model["attributes"]))
            for name, attr in model["attributes"].items():
                actual = attrs[name]
                run.claim(f"{tier}-{kind}-{name}-schema", actual["type"] == attr["type"] and bool(actual.get("required")) == (attr["mode"] == "required") and bool(actual.get("optional")) == (attr["mode"] in ("optional", "optional_computed")) and bool(actual.get("computed")) == (attr["mode"] in ("computed", "optional_computed")) and bool(actual.get("sensitive")) == attr["sensitive"] and bool(actual.get("write_only")) == attr["write_only"], actual)
                run.claim(f"{tier}-{kind}-{name}-native-doc-description", actual["description"] == attr["description"])


def lifecycle(run, root, evidence, config, binary, tier, base_env):
    fixture = Fixture()
    attempt = evidence / tier
    consumer_root = root / f"terraform-{tier}"
    consumer_root.mkdir()
    mirror = consumer_root / "mirror" / config["provider_address"] / config["version"] / "darwin_arm64"
    mirror.mkdir(parents=True)
    installed = mirror / binary.name
    shutil.copy2(binary, installed)
    run.claim(f"{tier}-mirror-byte-identity", sha(binary) == sha(installed))
    cli = consumer_root / "terraform.rc"
    cli.write_text(f'provider_installation {{\n  filesystem_mirror {{\n    path = "{consumer_root / "mirror"}"\n    include = ["{config["provider_address"]}"]\n  }}\n}}\n')
    environment = {k: v for k, v in base_env.items() if not k.startswith("TF_")}
    environment.update(TF_CLI_CONFIG_FILE=str(cli), TF_IN_AUTOMATION="1", CHECKPOINT_DISABLE="1", TF_VAR_endpoint=fixture.endpoint, TF_VAR_token="fixture-token")
    initial = {"name": "alpha", "region": "east", "enabled": True, "secret": "one-secret", "secret_version": "1"}
    plan_manifest = json.loads((root / "generated/terraform/source-bindings.json").read_text())

    def env(values):
        return {**environment, **{f"TF_VAR_input_{key}": json.dumps(value) if isinstance(value, bool) else str(value) for key, value in values.items() if value is not None}}

    def tf(label, directory, values, *args, expected=(0,)):
        fixture.action = f"{tier}-{label}"
        return run.run(f"{tier}-{label}", [TERRAFORM, *args], directory, env(values), expected=expected, timeout=120)

    def consumer(name, values, data=False, unknown=False):
        directory = consumer_root / name
        directory.mkdir()
        source = root / f"generated/terraform/examples/{'data-sources' if data else 'resources'}/record/main.tf"
        hcl = source.read_text()
        if unknown:
            hcl = hcl.replace("name = var.input_name", "name = terraform_data.seed.output")
            hcl += '\nresource "terraform_data" "seed" {\n  input = var.input_name\n}\n'
        (directory / "main.tf").write_text(hcl)
        tf(f"{name}-init", directory, values, "init", "-input=false", "-no-color")
        valid = json.loads(tf(f"{name}-validate", directory, values, "validate", "-json"))
        run.claim(f"{tier}-{name}-hcl-valid", valid["valid"], valid)
        return directory

    def show(label, directory, values, file=None):
        return json.loads(tf(label, directory, values, "show", "-json", *([file] if file else [])))

    def planned(label, directory, values, actions=None):
        filename = label + ".tfplan"
        tf(label, directory, values, "plan", "-input=false", "-no-color", f"-out={filename}")
        result = show(label + "-json", directory, values, filename)
        if actions is not None:
            changes = [v for v in result.get("resource_changes", []) if v["address"] == "fixture_record.example"]
            run.claim(f"{tier}-{label}-actions", len(changes) == 1 and changes[0]["change"]["actions"] == actions, changes)
        # Write-only/ephemeral credentials cannot appear in saved plan payloads.
        with zipfile.ZipFile(directory / filename) as archive:
            bodies = [archive.read(name) for name in archive.namelist()]
        secrets = [v.encode() for k, v in values.items() if k == "secret" and isinstance(v, str)] + [b"fixture-token"]
        run.claim(f"{tier}-{label}-saved-plan-secrets", all(secret not in body for secret in secrets for body in bodies))
        return filename, result

    def apply(label, directory, values, filename, expected=(0,)):
        output = tf(label, directory, values, "apply", "-input=false", "-no-color", filename, expected=expected)
        run.claim(f"{tier}-{label}-diagnostic-redaction", b"response-secret-never-in-diagnostics" not in output)

    def state(label, directory, values, data=False):
        result = show(label, directory, values)
        resources = result.get("values", {}).get("root_module", {}).get("resources", [])
        address = "data.fixture_record.example" if data else "fixture_record.example"
        resource = next((r for r in resources if r["address"] == address), None)
        if resource and not data:
            run.claim(f"{tier}-{label}-write-only-state", resource["values"]["secret"] is None)
            run.claim(f"{tier}-{label}-sensitive-state", resource["sensitive_values"]["fingerprint"] is True)
        if (directory / "terraform.tfstate").exists():
            shutil.copy2(directory / "terraform.tfstate", attempt / f"{label}.tfstate")
            raw = (directory / "terraform.tfstate").read_bytes()
            run.claim(f"{tier}-{label}-raw-state-secrets", all(secret not in raw for secret in [b"one-secret", b"second-secret", b"fixture-token"]))
        return resource

    def destroy(label, directory, values, expected=(0,)):
        tf(label, directory, values, "destroy", "-auto-approve", "-input=false", "-no-color", expected=expected)

    def last_write(method):
        return next(r for r in reversed(fixture.requests) if r["method"] == method)

    try:
        main = consumer("main", initial)
        schema = json.loads(tf("native-schema", main, initial, "providers", "schema", "-json"))
        save_json(attempt / "terraform-schema.json", schema)
        schema_gate(run, schema, plan_manifest, tier)
        filename, _ = planned("initial-plan", main, initial, ["create"])
        run.claim(f"{tier}-plan-has-no-api-side-effects", not fixture.requests)
        apply("initial-apply", main, initial, filename)
        created = state("created", main, initial)
        identity = created["values"]["id"]
        fingerprint = created["values"]["fingerprint"]
        run.claim(f"{tier}-create-state", created["values"]["name"] == "alpha" and created["values"]["description"] is None and created["values"]["memo"] is None and created["values"]["enabled"] is True)
        tf("noop-plan", main, initial, "plan", "-input=false", "-no-color", "-detailed-exitcode")
        values = {**initial, "name": "beta", "description": "configured", "memo": "memo", "enabled": False, "secret": "second-secret"}
        filename, _ = planned("update-plan", main, values, ["update"])
        apply("update-apply", main, values, filename)
        changed = state("updated", main, values)
        run.claim(f"{tier}-unchanged-trigger-omits-secret", "secret" not in json.loads(last_write("PUT")["body"]) and changed["values"]["fingerprint"] == fingerprint)
        values["secret_version"] = "2"
        filename, _ = planned("rotation-plan", main, values, ["update"])
        apply("rotation-apply", main, values, filename)
        rotated = state("rotated", main, values)
        run.claim(f"{tier}-changed-trigger-sends-secret", json.loads(last_write("PUT")["body"])["secret"] == "second-secret" and rotated["values"]["fingerprint"] != fingerprint)
        with fixture.lock:
            fixture.records[identity].update(SetExtra="drift", description="changed-remotely")
        tf("drift-refresh", main, values, "refresh", "-input=false", "-no-color")
        drift = state("drift", main, values)
        run.claim(f"{tier}-authoritative-drift-refresh", drift["values"]["name"] == "drift" and drift["values"]["description"] == "changed-remotely")
        filename, _ = planned("drift-repair-plan", main, values, ["update"])
        apply("drift-repair-apply", main, values, filename)
        values.pop("description"); values.pop("memo")
        filename, _ = planned("null-plan", main, values, ["update"])
        apply("null-apply", main, values, filename)
        nullable = state("null", main, values)
        wire = json.loads(last_write("PUT")["body"])
        run.claim(f"{tier}-null-distinct-from-omission", wire["description"] is None and "memo" not in wire and nullable["values"]["description"] is None and nullable["values"]["memo"] is None)
        values["region"] = "west"
        filename, result = planned("replacement-plan", main, values, ["delete", "create"])
        change = next(r["change"] for r in result["resource_changes"] if r["address"] == "fixture_record.example")
        run.claim(f"{tier}-replacement-path", ["region"] in change["replace_paths"])
        apply("replacement-apply", main, values, filename)
        replaced = state("replaced", main, values)
        run.claim(f"{tier}-replacement-new-identity", replaced["values"]["id"] != identity and identity not in fixture.records)
        identity = replaced["values"]["id"]
        data_values = {"id": identity}
        data = consumer("data", data_values, data=True)
        filename, _ = planned("data-plan", data, data_values)
        apply("data-apply", data, data_values, filename)
        observed = state("data-read", data, data_values, data=True)
        run.claim(f"{tier}-data-source-native-read", observed["values"]["id"] == identity and observed["values"]["name"] == values["name"] and observed["sensitive_values"]["fingerprint"])
        imported_id = "import/a 雪"
        fixture.seed(imported_id)
        import_values = {"name": "imported", "region": "east", "enabled": True}
        imported = consumer("import", import_values)
        tf("import", imported, import_values, "import", "-input=false", "-no-color", "fixture_record.example", imported_id)
        imported_state = state("imported", imported, import_values)
        run.claim(f"{tier}-opaque-import-read", imported_state["values"]["id"] == imported_id and imported_state["values"]["name"] == "imported" and imported_state["values"]["secret_version"] is None)
        tf("import-noop-plan", imported, import_values, "plan", "-input=false", "-no-color", "-detailed-exitcode")
        destroy("import-destroy", imported, import_values)
        with fixture.lock:
            fixture.records.pop(identity)
        tf("missing-refresh", main, values, "refresh", "-input=false", "-no-color")
        run.claim(f"{tier}-missing-removes-state", state("missing", main, values) is None)
        tf("missing-data-source", data, data_values, "plan", "-input=false", "-no-color", expected=(1,))
        filename, _ = planned("missing-recreate-plan", main, values, ["create"])
        apply("missing-recreate-apply", main, values, filename)
        destroy("main-destroy", main, values)
        run.claim(f"{tier}-destroy-removes-state", state("destroyed", main, values) is None)

        unknown_values = {**initial, "name": "unknown-resolved"}
        unknown = consumer("unknown", unknown_values, unknown=True)
        before = len(fixture.requests)
        filename, result = planned("unknown-plan", unknown, unknown_values, ["create"])
        change = next(r["change"] for r in result["resource_changes"] if r["address"] == "fixture_record.example")
        run.claim(f"{tier}-terraform-unknown-is-preserved", change["after_unknown"]["name"] is True and len(fixture.requests) == before)
        apply("unknown-apply", unknown, unknown_values, filename)
        run.claim(f"{tier}-resolved-unknown-reaches-sdk", json.loads(last_write("POST")["body"])["SetExtra"] == "unknown-resolved")
        destroy("unknown-destroy", unknown, unknown_values)

        failed_values = {**initial, "name": "fail-create"}
        failed = consumer("failed-create", failed_values)
        filename, _ = planned("failed-create-plan", failed, failed_values, ["create"])
        before = len([r for r in fixture.requests if r["method"] == "POST"])
        apply("failed-create-apply", failed, failed_values, filename, expected=(1,))
        run.claim(f"{tier}-failed-create-no-state-or-retry", state("failed-create", failed, failed_values) is None and len([r for r in fixture.requests if r["method"] == "POST"]) == before + 1)
        partial_values = {**initial, "name": "partial-create"}
        partial = consumer("partial-create", partial_values)
        filename, _ = planned("partial-create-plan", partial, partial_values, ["create"])
        apply("partial-create-apply", partial, partial_values, filename, expected=(1,))
        partial_state = state("partial-create", partial, partial_values)
        raw = json.loads((partial / "terraform.tfstate").read_text())
        run.claim(f"{tier}-partial-create-keeps-tainted-identity", partial_state["values"]["id"] in fixture.records and raw["resources"][0]["instances"][0]["status"] == "tainted")
        destroy("partial-create-destroy", partial, partial_values)

        update_values = dict(initial)
        errors_root = consumer("errors", update_values)
        filename, _ = planned("errors-create-plan", errors_root, update_values, ["create"])
        apply("errors-create-apply", errors_root, update_values, filename)
        error_identity = state("errors-initial", errors_root, update_values)["values"]["id"]
        update_values.update(name="fail-update", secret_version="2", secret="second-secret")
        filename, _ = planned("failed-update-plan", errors_root, update_values, ["update"])
        apply("failed-update-apply", errors_root, update_values, filename, expected=(1,))
        unchanged = state("failed-update", errors_root, update_values)
        run.claim(f"{tier}-failed-update-keeps-prior", unchanged["values"]["name"] == "alpha" and unchanged["values"]["secret_version"] == "1")
        update_values["name"] = "partial-update"
        filename, _ = planned("partial-update-plan", errors_root, update_values, ["update"])
        apply("partial-update-apply", errors_root, update_values, filename, expected=(1,))
        partially_changed = state("partial-update", errors_root, update_values)
        run.claim(f"{tier}-partial-update-saves-actual-fields", partially_changed["values"]["name"] == "partial-update" and partially_changed["values"]["secret_version"] == "1" and fixture.records[error_identity]["SetExtra"] == "partial-update")
        fixture.failed_deletes.add(error_identity)
        destroy("failed-delete", errors_root, update_values, expected=(1,))
        run.claim(f"{tier}-failed-delete-keeps-state", state("failed-delete", errors_root, update_values)["values"]["id"] == error_identity)
        fixture.failed_deletes.remove(error_identity)
        destroy("recovered-delete", errors_root, update_values)

        cancel_values = {**initial, "name": "cancel-create"}
        cancel = consumer("cancel", cancel_values)
        filename, _ = planned("cancel-plan", cancel, cancel_values, ["create"])
        fixture.action = f"{tier}-cancel-apply"
        args = [TERRAFORM, "apply", "-input=false", "-no-color", filename]
        start = time.monotonic()
        process = subprocess.Popen([str(a) for a in args], cwd=cancel, env=env(cancel_values), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            started = fixture.cancel_started.wait(15)
            process.send_signal(signal.SIGINT)
            out, err = process.communicate(timeout=30)
        except Exception:
            process.kill()
            out, err = process.communicate()
            run.record(f"{tier}-cancel-apply", args, cancel, env(cancel_values), process.returncode, out, err, {process.returncode}, time.monotonic() - start)
            raise
        run.record(f"{tier}-cancel-apply", args, cancel, env(cancel_values), process.returncode, out, err, {1}, time.monotonic() - start)
        run.claim(f"{tier}-real-terraform-cancellation", started and fixture.cancel_closed.wait(5), "SIGINT -> Terraform plugin context -> generated SDK body/request cancellation")
        run.claim(f"{tier}-cancel-no-state", state("cancelled", cancel, cancel_values) is None)
        run.claim(f"{tier}-fixture-wire-assertions", not fixture.failures, fixture.failures)
        run.claim(f"{tier}-all-remote-records-destroyed", not fixture.records, fixture.records)
        # Keep consumer configurations, saved plans, lock files and raw state in
        # the isolated root, and retain their actual file digests in the report.
        save_json(attempt / "consumer-hashes.json", tree_hashes(consumer_root))
    finally:
        fixture.close()
        save_json(attempt / "wire.json", fixture.requests)
        save_json(attempt / "fixture-failures.json", fixture.failures)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    args = parser.parse_args()
    root, evidence, fixtures = args.root.resolve(), args.evidence.resolve(), args.fixtures.resolve()
    assert root.is_relative_to(APPROVED.resolve()), "native roots must be isolated under the approved temporary directory"
    run = Runner(evidence)
    report = {"format": "suspect.terraform.acceptance.v1", "native_root": str(root), "local_acceptance_only": True, "publishing": False, "passed": False}
    try:
        config = json.loads((fixtures / "target.json").read_text())
        generated = root / "generated"
        source_hashes = tree_hashes(generated)
        save_json(evidence / "generated-source-hashes.json", source_hashes)
        save_json(evidence / "fixture-hashes.json", tree_hashes(fixtures))
        repo = Path(__file__).resolve().parent.parent
        assets = [repo / "crates/suspect-codegen/src/terraform.rs", *sorted((repo / "crates/suspect-codegen/src/terraform").rglob("*")), repo / "crates/suspect-codegen/tests/terraform.rs", Path(__file__).resolve()]
        save_json(evidence / "generator-source-hashes.json", {str(p.relative_to(repo)): sha(p) for p in assets if p.is_file()})
        with tarfile.open(evidence / "generator-and-fixture-sources.tar.gz", "w:gz") as archive:
            for p in assets:
                if p.is_file():
                    archive.add(p, arcname=p.relative_to(repo), recursive=False)
            archive.add(fixtures, arcname=fixtures.relative_to(repo))
        proxy, proxy_info = prepare_proxy(root, generated / "go", config)
        save_json(evidence / "sdk-proxy.json", proxy_info)
        shutil.copy2(root / "proxy" / config["sdk"]["module_path"] / "@v" / f"v{config['sdk']['version']}.zip", evidence / "generated-sdk.zip")
        module_cache = APPROVED / "sdk-terraform-dependency-cache-20260910-01"
        go_cache = APPROVED / "sdk-terraform-build-cache-20260910-01"
        # The SDK's private local proxy checksum is also emitted in go.sum;
        # skip public sumdb discovery only for this unpublished fixture module.
        env = {k: v for k, v in os.environ.items() if not k.startswith(("TF_", "GOTOOLCHAIN", "GOPROXY", "GONOSUMDB", "GOPRIVATE", "GOWORK", "GOFLAGS"))}
        env.update(GOWORK="off", GOMODCACHE=str(module_cache), GOCACHE=str(go_cache), GOPROXY=f"file://{proxy},https://proxy.golang.org", GONOSUMDB=config["sdk"]["module_path"], GOFLAGS="")
        version = json.loads(run.run("terraform-version", [TERRAFORM, "version", "-json"], root, {**env, "CHECKPOINT_DISABLE": "1"}))
        run.claim("exact-terraform-version-platform", version["terraform_version"] == config["terraform_version"] and version["platform"] == "darwin_arm64", version)
        save_json(evidence / "terraform-tool.json", {"path": str(TERRAFORM.resolve()), "sha256": sha(TERRAFORM), "version": version})
        binaries = native_packages(run, root, evidence, fixtures, config, env)
        for tier, binary in binaries.items():
            lifecycle(run, root, evidence, config, binary, tier, env)
        actual = tree_hashes(generated)
        run.claim("all-emitted-source-bytes-remain-exact", all(actual[name] == digest for name, digest in source_hashes.items()))
        report["passed"] = True
    except Exception:
        report["failure"] = traceback.format_exc()
        print(report["failure"], flush=True)
    finally:
        report["commands"] = run.commands
        report["gates"] = run.claims
        save_json(evidence / "report.json", report)
    print(json.dumps({"passed": report["passed"], "commands": len(run.commands), "gates": len(run.claims), "evidence": str(evidence)}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
