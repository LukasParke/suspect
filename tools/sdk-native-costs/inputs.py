"""Immutable emitted-package/installed-root linkage and input integrity."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import tomllib
from typing import Any
import xml.etree.ElementTree as ET
import zipfile

from bindings import one
from evidence import file_record, inventory, read_json, require, sha, sha_bytes, verify_file, write_json


class Inputs:
    def __init__(self, packages: Path, native: Path, output: Path):
        self.packages = packages.resolve(strict=True)
        self.native = native.resolve(strict=True)
        self.output = output
        self.trees: dict[str, dict[str, dict[str, Any]]] = {}
        self.files: dict[str, dict[str, Any]] = {}

    def tree(self, root: Path) -> dict[str, dict[str, Any]]:
        key = str(root.absolute())
        if key not in self.trees:
            self.trees[key] = inventory(root)
        return self.trees[key]

    def remember(self, path: Path) -> None:
        value = file_record(path)
        previous = self.files.setdefault(value["path"], value)
        require(previous == value, f"input changed during use: {path}")

    def generated(self) -> None:
        actual = self.tree(self.packages)
        runner_manifest = self.packages.parent / "generated-manifest.json"
        if runner_manifest.is_file():
            self.remember(runner_manifest)
            expected = read_json(runner_manifest)
            require(set(expected) == set(actual), "CLI emitted package census differs from runner manifest")
            for name, value in actual.items():
                require(expected[name]["kind"] == "file" and expected[name]["sha256"] == value["sha256"]
                        and expected[name]["bytes"] == value["bytes"], f"CLI artifact differs from runner manifest: {name}")

    def verify(self) -> None:
        for root, before in self.trees.items():
            require(inventory(Path(root)) == before, f"immutable input tree changed: {root}")
        for record in self.files.values():
            verify_file(record)

    def save(self) -> None:
        write_json(self.output / "input-inventory.json", {"trees": self.trees, "files": list(self.files.values())})

    def copy_package(self, target: dict[str, Any], destination: Path) -> Path:
        source = self.packages / target["language"]
        before = inventory(source)
        # Copy bytes, not readonly permissions or links to the original package.
        shutil.copytree(source, destination, copy_function=shutil.copyfile)
        require(inventory(destination) == before, "fresh private package copy differs")
        return destination

    def package_identity(self, target: dict[str, Any]) -> None:
        language, name = target["language"], target["package_name"]
        root = self.packages / language
        manifest = root / target["manifest"]
        self.remember(manifest)
        if language in ("typescript", "php"):
            value = read_json(manifest)
            require(value["name"] == name and value["version"] == "0.0.0", "native package identity differs")
        elif language in ("python", "rust"):
            value = tomllib.loads(manifest.read_text())["project" if language == "python" else "package"]
            require(value["name"] == name and value["version"] == "0.0.0", "native TOML identity differs")
        elif language in ("java", "kotlin"):
            xml = ET.fromstring(manifest.read_text())
            text = lambda key: xml.findtext("{*}" + key)
            require(text("groupId") + ":" + text("artifactId") == name and text("version") == "0.0.0", "Maven identity differs")
        elif language == "csharp":
            xml = ET.fromstring(manifest.read_text())
            require(xml.findtext(".//PackageId") == name and xml.findtext(".//Version") == "0.0.0", "NuGet identity differs")
        elif language == "go":
            modules = [line.split()[1] for line in manifest.read_text().splitlines() if line.startswith("module ")]
            require(modules == [name], "Go module identity differs")
        elif language in ("swift", "cpp", "dart"):
            metadata = read_json(root / "sdk-manifest.json")
            package = metadata["package"]
            if isinstance(package, dict):
                require(package["name"] == name and package["version"] == "0.0.0", "native package metadata identity differs")
            else:
                require(package == name and metadata["version"] == "0.0.0", "native package metadata identity differs")
        elif language == "ruby":
            require(read_json(root / "source-map.json")["package"]["name"] == name, "Ruby gem identity differs")

    def sources_equal(self, generated: Path, installed: Path, *, prefix: str = "", strict: bool = False) -> int:
        compared = 0
        source_files = inventory(generated)
        for name, item in source_files.items():
            if prefix and not name.startswith(prefix):
                continue
            relative = name.removeprefix(prefix)
            other = installed / relative
            if other.is_file():
                # cargo package normalizes only its package manifest. Its actual
                # source and Cargo.toml.orig carry separate byte linkage.
                if generated.name == "rust" and relative == "Cargo.toml":
                    other = installed / "Cargo.toml.orig"
                require(other.is_file() and sha(other) == item["sha256"], f"installed source differs: {other}")
                compared += 1
            elif strict or relative.startswith(("src/", "lib/", "include/", "Sources/")):
                require(False, f"installed package is missing emitted source: {other}")
            elif generated.name == "typescript" and relative.endswith(".ts") and not relative.startswith("examples/"):
                require(False, f"installed npm package is missing emitted source: {other}")
        require(compared >= 3, f"insufficient installed source linkage: {installed}")
        return compared

    def source_archive(self, source: Path, archive: Path, prefix: str) -> int:
        self.remember(archive)
        count = 0
        with zipfile.ZipFile(archive) as zipped:
            require(len(zipped.namelist()) == len(set(zipped.namelist())), f"duplicate archive members: {archive}")
            for name, item in inventory(source).items():
                if name.startswith(prefix):
                    member = name.removeprefix(prefix)
                    require(member in zipped.namelist() and sha_bytes(zipped.read(member)) == item["sha256"],
                            f"installed source archive differs: {archive}!{member}")
                    count += 1
        require(count >= 3, "source archive has no substantial generated linkage")
        return count

    def installed(self, target: dict[str, Any], slot: str) -> dict[str, Any]:
        language = target["language"]
        generated = self.packages / language
        root = self.native / language / slot
        require(root.is_dir() and root.resolve().is_relative_to(self.native), f"missing/non-confined native consumer root: {root}")
        result: dict[str, Any] = {"root": str(root), "slot": slot}
        if language == "python":
            venv = self.native.parent / ("python-" + slot)
            require((venv / "pyvenv.cfg").is_file(), f"missing runner-installed Python wheel environment: {venv}")
            site = one(list((venv / "lib").glob("python*/site-packages")), "installed Python site")
            installed = site / target["import_name"]
            result["python"] = str(venv / "bin/python")
            self.remember(venv / "pyvenv.cfg")
            result["comparedSources"] = self.sources_equal(generated, installed, prefix=f"src/{target['import_name']}/", strict=True)
            info = one(list(site.glob("sdk_full-0.0.0.dist-info")), "installed Python distribution identity")
            self.tree(info)
        elif language == "typescript":
            installed = root / "consumer/node_modules" / target["package_name"]
            result["comparedSources"] = self.sources_equal(generated, installed)
            package = read_json(installed / "package.json")
            entry = package["exports"]["."]["import"]
            require(isinstance(entry, str) and entry.startswith("./") and ".." not in Path(entry).parts, "invalid npm entrypoint")
            require((installed / entry).is_file(), "missing compiled npm entry")
            result["entry"] = str(installed / entry)
            built_dist = self.native.parent / "packages/typescript/dist"
            if built_dist.is_dir():
                for name, record in inventory(installed / "dist").items():
                    require(sha(built_dist / name) == record["sha256"], f"installed npm runtime differs from build: {name}")
        elif language == "rust":
            installed = root / "installed/sdk-full-0.0.0"
            result["comparedSources"] = self.sources_equal(generated, installed)
        elif language == "ruby":
            installed = root / "installed/gems/sdk-full-0.0.0"
            result["comparedSources"] = self.sources_equal(generated, installed)
        elif language == "php":
            installed = root / "consumer/vendor" / target["package_name"]
            result["autoload"] = str(root / "consumer/vendor/autoload.php")
            self.remember(Path(result["autoload"]))
            self.tree(root / "consumer/vendor/composer")
            result["comparedSources"] = self.sources_equal(generated, installed, strict=True)
        elif language in ("swift", "dart", "go"):
            installed = root / "installed"
            result["comparedSources"] = self.sources_equal(generated, installed, strict=True)
        elif language in ("java", "kotlin"):
            installed = root / "installed"
            jar = installed / "sdk-full-0.0.0.jar"
            source_jar = installed / "sdk-full-0.0.0-sources.jar"
            result["jar"] = str(jar)
            prefix = "src/main/" + ("java/" if language == "java" else "kotlin/")
            result["comparedSources"] = self.source_archive(generated, source_jar, prefix)
            built = root / "sdk/target" / jar.name
            require(sha(jar) == sha(built), "installed Maven jar differs from native build")
            self.remember(built)
            self.tree(root / "sdk/src")
            self.sources_equal(generated / "src", root / "sdk/src", strict=True)
            with zipfile.ZipFile(jar) as zipped:
                require(any(n.endswith(".class") for n in zipped.namelist()), "installed jar contains no compiled classes")
                for path in (generated / "src/main/resources").rglob("*"):
                    if path.is_file():
                        name = path.relative_to(generated / "src/main/resources").as_posix()
                        require(sha_bytes(zipped.read(name)) == sha(path), "installed JVM resource differs from emitted bytes")
        elif language == "csharp":
            installed = root / "installed" / target["package_name"].lower() / "0.0.0"
            dll = installed / "lib/net8.0" / (target["package_name"] + ".dll")
            result["dll"] = str(dll)
            archive = root / "feed" / (target["package_name"] + ".0.0.0.nupkg")
            self.remember(archive)
            with zipfile.ZipFile(archive) as zipped:
                for name in (f"lib/net8.0/{target['package_name']}.dll", "http-manifest.json", "docs/reference.json"):
                    require(sha_bytes(zipped.read(name)) == sha(installed / name), "NuGet installed/archive bytes differ")
            require(sha(installed / "http-manifest.json") == sha(generated / "http-manifest.json"), "NuGet manifest is not CLI emitted")
            built = root / "sdk/bin/Release/net8.0" / dll.name
            require(sha(built) == sha(dll), "NuGet installed assembly differs from native build")
            self.remember(built)
            self.tree(root / "sdk/src")
            result["comparedSources"] = self.sources_equal(generated / "src", root / "sdk/src", strict=True)
        elif language == "cpp":
            installed = root / "installed"
            result["comparedSources"] = self.sources_equal(generated / "include", installed / "include", strict=True)
            library = one(list((installed / "lib").glob("libsdk_full.*")), "installed C++ library")
            result["library"] = str(library)
            built = root / "build" / library.name
            require(sha(built) == sha(library), "installed CMake library differs from native build")
            self.remember(built)
            self.tree(root / "sdk/src")
            self.sources_equal(generated / "src", root / "sdk/src", strict=True)
        else:
            raise AssertionError(language)
        require(installed.resolve().is_relative_to(self.native.parent), "installed package escaped runner scratch")
        result["package"] = str(installed)
        result["inventory"] = self.tree(installed)
        write_json(self.output / f"installed/{language}/{slot}.json", result)
        return result


def artifact_files(roots: list[Path]) -> list[dict[str, Any]]:
    """Payload byte census. Deduplicate aliases without counting toolchain caches."""
    files: dict[str, dict[str, Any]] = {}
    for root in roots:
        require(root.exists(), f"native build did not produce declared payload: {root}")
        for path in sorted(root.rglob("*")) if root.is_dir() else [root]:
            if path.is_file():
                # SwiftPM creates convenience symlinks to its own build folders.
                # Follow only actual file endpoints and count each payload once.
                resolved = path.resolve(strict=True)
                files[str(resolved)] = file_record(resolved)
    result = list(files.values())
    require(result and sum(r["bytes"] for r in result) > 0, "native build/import artifact has zero bytes")
    return result
