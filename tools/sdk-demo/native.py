#!/usr/bin/env python3
"""Prepare once, then run installed SDK quickstarts against independent loopback bytes."""
from __future__ import annotations

import argparse
import fcntl
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading

from demo import REPO, owned_root, record, sha, verify_pins, write_json

LANGUAGES = ("typescript", "python", "go", "rust", "swift", "java", "csharp",
             "kotlin", "ruby", "php", "dart", "cpp")
MISE = Path.home() / ".local/share/mise/installs"
SNIPPETS = REPO / "examples/sdk-demo-all"


def tools() -> dict[str, Path]:
    rust = Path(subprocess.check_output([str(Path.home() / ".cargo/bin/rustup"),
                "which", "--toolchain", "stable", "cargo"], text=True).strip())
    return {
        "node": MISE / "node/22.23.1/bin/node",
        "npm": MISE / "node/22.23.1/bin/npm",
        "python": REPO / "target/sdk-native-python-tools/bin/python",
        "uv": MISE / "uv/0.11.27/uv-aarch64-apple-darwin/uv",
        "go": Path.home() / "go/pkg/mod/golang.org/toolchain@v0.0.1-go1.23.12.darwin-arm64/bin/go",
        "cargo": rust, "rustc": rust.with_name("rustc"), "swift": Path("/usr/bin/swift"),
        "java_home": MISE / "java/temurin-21.0.12+101.0.LTS",
        "maven": MISE / "maven/3.9.16/apache-maven-3.9.16/bin/mvn",
        "dotnet": Path.home() / ".local/share/mise/dotnet-root/dotnet",
        "ruby": MISE / "ruby/3.3.12/bin/ruby",
        "php": REPO / "target/sdk-php-tools/php-8.3.32/php",
        "composer": REPO / "target/sdk-php-tools/composer-2.10.3.phar",
        "phpstan": REPO / "target/sdk-php-tools/phpstan-2.2.13.phar",
        "dart": REPO / "target/sdk-dart-tools/dart-sdk/bin/dart",
        "cmake": MISE / "cmake/3.31.6/cmake-3.31.6-macos-universal/CMake.app/Contents/bin/cmake",
        "cxx": Path("/usr/bin/clang++"),
    }


def put(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def copy_snippet(language: str, name: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(SNIPPETS / language / name, dest)


def build(root: Path, language: str) -> None:
    verify_pins(root)
    t = tools()
    parent = root / "native"
    parent.mkdir(exist_ok=True)
    number = 1
    while (parent / f"{language}-{number:02}").exists():
        number += 1
    attempt = parent / f"{language}-{number:02}"
    attempt.mkdir()
    package, consumer = attempt / "package", attempt / "consumer"
    shutil.copytree(root / "packages" / language, package)
    consumer.mkdir()
    source_pins = {str(p.relative_to(package)): sha(p)
                   for p in package.rglob("*") if p.is_file()}
    write_json(attempt / "generated-source-pins.json", source_pins)
    env: dict[str, str] = {}
    selected_tools: dict[str, dict] = {}

    def run(label: str, argv: list[str | Path], cwd: Path = package,
            timeout: int = 900) -> str:
        return record(attempt, label, [str(a) for a in argv], cwd=cwd,
                      env=env, timeout=timeout).stdout

    def tool(name: str, *version: str) -> Path:
        path = t[name]
        if not path.is_file():
            raise SystemExit(f"Missing prepared {name}: {path}")
        out = run(f"tool-{name}", [path, *(version or ("--version",))], cwd=attempt)
        selected_tools[name] = {"path": str(path), "resolvedPath": str(path.resolve()),
                                "sha256": sha(path), "version": out.strip()}
        return path

    extra_runs = []
    if language == "typescript":
        env["PATH"] = str(t["node"].parent) + os.pathsep + os.environ["PATH"]
        node, npm = tool("node"), tool("npm")
        run("npm-ci", [npm, "ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"])
        run("sdk-build", [npm, "run", "build"])
        run("npm-pack", [npm, "pack", "--ignore-scripts", "--pack-destination", attempt])
        archive = next(attempt.glob("*.tgz"))
        write_json(consumer / "package.json", {"private": True, "type": "module", "dependencies": {
            "@demo/openrouter-all-sdk": f"file:{archive}"}})
        run("consumer-install", [npm, "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], consumer)
        copy_snippet(language, "main.ts", consumer / "main.ts")
        copy_snippet("javascript", "main.mjs", consumer / "main.mjs")
        run("strict-types", [node, package / "node_modules/typescript/bin/tsc", "--strict",
            "--exactOptionalPropertyTypes", "--target", "ES2022", "--module", "NodeNext",
            "--moduleResolution", "NodeNext", "--lib", "ES2023,DOM", "--outDir", "build", "main.ts"], consumer)
        argv = [str(node), str(consumer / "build/main.js")]
        extra_runs = [{"language": "javascript", "argv": [str(node), str(consumer / "main.mjs")]}]
        run("packaged-examples", [node, "dist/examples/validated.js"])
    elif language == "python":
        python, uv = tool("python"), tool("uv")
        run("wheel-build", [python, "-m", "build", "--wheel", "--no-isolation", "--outdir", attempt / "dist"])
        run("venv", [uv, "venv", "--offline", "--python", python, attempt / "venv"], attempt)
        installed_python = attempt / "venv/bin/python"
        wheel = next((attempt / "dist").glob("*.whl"))
        run("wheel-install", [uv, "pip", "install", "--offline", "--python", installed_python, wheel], attempt)
        copy_snippet(language, "main.py", consumer / "main.py")
        run("mypy-version", [python, "-m", "mypy", "--version"], consumer)
        run("strict-types", [python, "-m", "mypy", "--python-executable", installed_python,
            "--strict", "--no-incremental", "main.py"], consumer)
        run("packaged-examples", [installed_python, "examples/validated.py"])
        argv = [str(installed_python), str(consumer / "main.py")]
    elif language == "go":
        go = tool("go", "version")
        previous_cache = parent / f"go-{number - 1:02}" / "go-cache"
        if number > 1 and previous_cache.is_dir():
            run("clone-previous-build-cache", ["/bin/cp", "-cR", previous_cache, attempt / "go-cache"], attempt)
        env.update({"GOTOOLCHAIN": "local", "GOWORK": "off", "GOPROXY": "off",
                    "GOSUMDB": "off", "GOCACHE": str(attempt / "go-cache")})
        put(consumer / "go.mod", "module demo.local/sdk-quickstart\n\ngo 1.23\n\n"
            "require example.com/openrouter-all-sdk v0.1.0\n"
            "replace example.com/openrouter-all-sdk => ../package\n")
        copy_snippet(language, "main.go", consumer / "main.go")
        run("consumer-build", [go, "build", "-o", consumer / "demo", "."], consumer)
        run("example-build", [go, "build", "-o", attempt / "validated", "./examples/validated"])
        run("packaged-examples", [attempt / "validated"])
        argv = [str(consumer / "demo")]
    elif language == "rust":
        cargo = tool("cargo")
        tool("rustc")
        previous_build = parent / f"rust-{number - 1:02}" / "build"
        if number > 1 and previous_build.is_dir():
            run("clone-previous-build-cache", ["/bin/cp", "-cR", previous_build, attempt / "build"], attempt)
        env.update({"RUSTC": str(t["rustc"]), "CARGO_TARGET_DIR": str(attempt / "build")})
        put(consumer / "Cargo.toml", "[package]\nname = \"demo-consumer\"\nversion = \"0.1.0\"\n"
            "edition = \"2024\"\n[workspace]\n[dependencies]\n"
            "openrouter-all-sdk = { path = \"../package\", features = [\"reqwest-rustls\"] }\n"
            "tokio = { version = \"=1.53.1\", features = [\"macros\", \"rt\", \"time\"] }\n")
        copy_snippet(language, "main.rs", consumer / "src/main.rs")
        run("consumer-build", [cargo, "build", "--offline", "--manifest-path", consumer / "Cargo.toml"], consumer)
        run("packaged-examples", [cargo, "run", "--offline", "--features", "http", "--example", "validated"])
        argv = [str(attempt / "build/debug/demo-consumer")]
    elif language == "swift":
        swift = tool("swift")
        put(consumer / "Package.swift", "// swift-tools-version: 6.0\nimport PackageDescription\n"
            "let package = Package(name: \"Demo\", platforms: [.macOS(.v13)], "
            "dependencies: [.package(path: \"../package\")], targets: ["
            ".executableTarget(name: \"Demo\", dependencies: [.product(name: \"OpenRouterAllSDK\", package: \"package\")])])\n")
        copy_snippet(language, "Main.swift", consumer / "Sources/Demo/Main.swift")
        run("consumer-build", [swift, "build", "--disable-sandbox", "--scratch-path", attempt / "build", "-Xswiftc", "-warnings-as-errors"], consumer)
        argv = [str(attempt / "build/debug/Demo")]
    elif language in ("java", "kotlin"):
        env.update({"JAVA_HOME": str(t["java_home"]),
                    "PATH": str(t["java_home"] / "bin") + os.pathsep + os.environ["PATH"]})
        t["java"] = t["java_home"] / "bin/java"
        t["javac"] = t["java_home"] / "bin/javac"
        java, maven = tool("java"), tool("maven")
        cache = attempt / "maven-repository"
        origin = REPO / ("target/sdk-java-maven-cache/java/repository" if language == "java" else "target/sdk-kotlin-maven")
        run("clone-public-dependency-cache", ["/bin/cp", "-cR", origin, cache], attempt)
        maven_cmd = [maven, "-B", "-o", f"-Dmaven.repo.local={cache}"]
        run("maven-install", [*maven_cmd, "install"])
        if language == "java":
            javac = tool("javac")
            jar = cache / "demo/openrouter/openrouter-all-sdk/0.1.0/openrouter-all-sdk-0.1.0.jar"
            copy_snippet(language, "Main.java", consumer / "Main.java")
            run("strict-types", [javac, "--release", "21", "-Xlint:all", "-Werror", "-cp", jar,
                                 "-d", consumer / "classes", consumer / "Main.java"], consumer)
            run("packaged-examples", [java, "-ea", "-cp", jar, "demo.openrouter.sdk.SdkExamples"])
            argv = [str(java), "-ea", "-cp", f"{jar}:{consumer / 'classes'}", "Main"]
        else:
            copy_snippet(language, "Smoke.kt", consumer / "src/main/kotlin/demo/Smoke.kt")
            put(consumer / "pom.xml", '<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>'
                '<groupId>demo.local</groupId><artifactId>consumer</artifactId><version>0.1.0</version>'
                '<properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>'
                '<dependencies><dependency><groupId>demo.openrouter</groupId><artifactId>openrouter-all-kotlin-sdk</artifactId>'
                '<version>0.1.0</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins>'
                '<plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.4.20</version>'
                '<configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration>'
                '<executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin>'
                '<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.5.0</version></plugin>'
                '<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.0</version></plugin>'
                '</plugins></build></project>\n')
            run("strict-types", [*maven_cmd, "compile"], consumer)
            jars = [cache / "demo/openrouter/openrouter-all-kotlin-sdk/0.1.0/openrouter-all-kotlin-sdk-0.1.0.jar",
                    cache / "org/jetbrains/kotlin/kotlin-stdlib/2.4.20/kotlin-stdlib-2.4.20.jar",
                    cache / "org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.11.0/kotlinx-coroutines-core-jvm-1.11.0.jar"]
            argv = [str(java), "-ea", "-cp", os.pathsep.join(map(str, [consumer / "target/classes", *jars])), "demo.SmokeKt"]
    elif language == "csharp":
        write_json(attempt / "global.json", {"sdk": {"version": "8.0.424", "rollForward": "disable"}})
        env.update({"DOTNET_ROOT": str(t["dotnet"].parent), "DOTNET_CLI_TELEMETRY_OPTOUT": "1",
                    "DOTNET_SKIP_FIRST_TIME_EXPERIENCE": "1", "NUGET_PACKAGES": str(attempt / "nuget")})
        dotnet = tool("dotnet")
        put(attempt / "NuGet.Config", '<configuration><packageSources><clear/></packageSources></configuration>\n')
        run("nuget-pack", [dotnet, "pack", "Suspect.csproj", "-c", "Release", "-o", attempt / "feed", "-m:1"])
        copy_snippet(language, "Program.cs", consumer / "Program.cs")
        put(consumer / "Demo.csproj", '<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType>'
            '<TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings>'
            '<TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup>'
            '<PackageReference Include="Demo.OpenRouter.AllSdk" Version="0.1.0" /></ItemGroup></Project>\n')
        run("consumer-restore", [dotnet, "restore", "Demo.csproj", "--source", attempt / "feed"], consumer)
        run("strict-types", [dotnet, "build", "Demo.csproj", "--no-restore", "-c", "Release", "-m:1"], consumer)
        argv = [str(dotnet), str(consumer / "bin/Release/net8.0/Demo.dll")]
    elif language == "ruby":
        ruby = tool("ruby")
        env.update({"PATH": str(ruby.parent) + os.pathsep + os.environ["PATH"],
                    "GEM_HOME": str(attempt / "gems"), "GEM_PATH": os.pathsep.join([
                        str(attempt / "gems"), str(ruby.parent.parent / "lib/ruby/gems/3.3.0")])})
        archive = attempt / "openrouter_all_sdk-0.1.0.gem"
        run("gem-build", [ruby, ruby.with_name("gem"), "build", "openrouter_all_sdk.gemspec", "--output", archive])
        run("gem-install", [ruby, ruby.with_name("gem"), "install", "--local", "--no-document", archive], attempt)
        copy_snippet(language, "main.rb", consumer / "main.rb")
        run("syntax", [ruby, "-c", consumer / "main.rb"], consumer)
        run("packaged-examples", [ruby, "examples/contract_examples.rb"])
        argv = [str(ruby), str(consumer / "main.rb")]
    elif language == "php":
        php = tool("php")
        env.update({"COMPOSER_DISABLE_NETWORK": "1", "COMPOSER_HOME": str(attempt / "composer-home"),
                    "COMPOSER_CACHE_DIR": str(attempt / "composer-cache")})
        copy_snippet(language, "main.php", consumer / "main.php")
        write_json(consumer / "composer.json", {"name": "demo/consumer", "require": {"demo/openrouter-all-sdk": "0.1.0"},
            "repositories": [{"type": "path", "url": "../package", "options": {"symlink": False}}, {"packagist.org": False}],
            "config": {"allow-plugins": False}})
        run("composer-install", [php, t["composer"], "update", "--no-dev", "--no-scripts", "--no-interaction"], consumer)
        run("strict-types", [php, t["phpstan"], "analyse", "--level=max", "--no-progress",
            "--memory-limit=1G", f"--autoload-file={consumer / 'vendor/autoload.php'}", consumer / "main.php"], consumer)
        argv = [str(php), str(consumer / "main.php")]
    elif language == "dart":
        dart = tool("dart")
        put(consumer / "pubspec.yaml", "name: demo_consumer\npublish_to: none\nenvironment:\n  sdk: '>=3.9.0 <4.0.0'\n"
            "dependencies:\n  openrouter_all_sdk:\n    path: ../package\n")
        copy_snippet(language, "main.dart", consumer / "bin/main.dart")
        run("pub-install", [dart, "pub", "get", "--offline"], consumer)
        run("strict-types", [dart, "analyze", "--fatal-infos"], consumer)
        run("consumer-build", [dart, "compile", "exe", "bin/main.dart", "-o", consumer / "demo"], consumer)
        run("package-pub-get", [dart, "pub", "get", "--offline"])
        run("packaged-examples", [dart, "run", "example/source_examples.dart"])
        argv = [str(consumer / "demo")]
    elif language == "cpp":
        cmake = tool("cmake")
        cxx = tool("cxx")
        run("cmake-configure", [cmake, "-S", package, "-B", attempt / "build", "-DCMAKE_BUILD_TYPE=Release",
            f"-DCMAKE_CXX_COMPILER={cxx}", f"-DCMAKE_INSTALL_PREFIX={attempt / 'install'}", "-DSUSPECT_SDK_BUILD_DOCS=OFF"])
        run("sdk-build", [cmake, "--build", attempt / "build", "--parallel", "2"])
        run("cmake-install", [cmake, "--install", attempt / "build"])
        copy_snippet(language, "main.cpp", consumer / "main.cpp")
        put(consumer / "CMakeLists.txt", "cmake_minimum_required(VERSION 3.24)\nproject(Demo LANGUAGES CXX)\n"
            "find_package(openrouter_all_sdk 0.1.0 CONFIG REQUIRED)\nadd_executable(demo main.cpp)\n"
            "target_link_libraries(demo PRIVATE openrouter_all_sdk::openrouter_all_sdk)\n"
            "target_compile_options(demo PRIVATE -Wall -Wextra -Wpedantic -Werror)\n")
        run("consumer-configure", [cmake, "-S", consumer, "-B", consumer / "build", f"-DCMAKE_PREFIX_PATH={attempt / 'install'}",
            f"-DCMAKE_CXX_COMPILER={cxx}", "-DCMAKE_BUILD_TYPE=Release"], consumer)
        run("strict-types", [cmake, "--build", consumer / "build", "--parallel", "2"], consumer)
        run("packaged-examples", [attempt / "build/sdk_validated_examples"])
        run("packaged-quickstart", [attempt / "build/sdk_client"])
        argv = [str(consumer / "build/demo")]
    else:
        raise SystemExit(f"Unknown language: {language}")

    changed = [p for p, digest in source_pins.items() if sha(package / p) != digest]
    if changed:
        raise SystemExit(f"Native tools changed generated source files: {changed}")
    snippet_pins = {str(p.relative_to(REPO)): sha(p) for p in (SNIPPETS / language).glob("*") if p.is_file()}
    if language == "typescript":
        snippet_pins.update({str(p.relative_to(REPO)): sha(p) for p in (SNIPPETS / "javascript").glob("*") if p.is_file()})
    info = {"language": language, "attempt": str(attempt), "package": str(package), "consumer": str(consumer),
            "argv": argv, "cwd": str(consumer), "environment": env, "extraRuns": extra_runs,
            "tools": selected_tools, "snippets": snippet_pins,
            "scope": "one installed/local-module quickstart; completed native matrices reused"}
    write_json(attempt / "prepared.json", info)
    index_path = root / "native-index.json"
    with (root / "native-index.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        index = json.loads(index_path.read_text()) if index_path.exists() else {}
        index[language] = info
        write_json(index_path, index)
    print(f"PREPARED {language}: {attempt}")


def smoke(root: Path, language: str, denied: bool = False) -> None:
    verify_pins(root)
    index = json.loads((root / "native-index.json").read_text())
    info = index["typescript" if language == "javascript" else language]
    for path, digest in info["snippets"].items():
        if sha(REPO / path) != digest:
            raise SystemExit(f"Snippet changed since preparation; build {language} again: {path}")
    fixtures = json.loads((REPO / "crates/suspect-codegen/tests/fixtures/openrouter-five-responses.json").read_text())
    wire: list[dict] = []
    failures: list[str] = []

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args: object) -> None:
            pass

        def request(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(min(length, 8192)).decode("utf-8")
            event = {"method": self.command, "path": self.path, "body": body,
                     "authorization": self.headers.get("Authorization")}
            wire.append(event)
            status, response = 400, '{"error":{"code":400,"message":"fixture mismatch"}}'
            if event["authorization"] != "Bearer fixture-token":
                failures.append("Expected the source-defined Bearer attachment")
            if self.command == "GET" and self.path == "/api/v1/credits":
                status, response = (401, '{"error":{"code":401,"message":"offline demo denied"}}') if denied else (200, fixtures["credits"])
            elif not denied and self.command == "POST" and self.path == "/api/v1/keys":
                if json.loads(body) != {"name": "Demo key", "limit": 50.25} or '50.25' not in body:
                    failures.append(f"Unexpected native create body: {body}")
                status, response = 201, fixtures["create"]
            elif not denied and self.command == "PATCH" and self.path == "/api/v1/keys/fixture-hash":
                expected = {} if sum(e["method"] == "PATCH" for e in wire) == 1 else {"limit": None}
                if json.loads(body) != expected:
                    failures.append(f"Absence/null mismatch: {body}")
                status, response = 200, fixtures["update"]
            else:
                failures.append(f"Unexpected request: {self.command} {self.path}")
            payload = response.encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(payload)
            self.close_connection = True

        do_GET = request
        do_POST = request
        do_PATCH = request

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    env = dict(info["environment"], SDK_DEMO_URL=f"http://127.0.0.1:{server.server_port}/api/v1")
    argv = info["argv"] if language != "javascript" else info["extraRuns"][0]["argv"]
    label = f"{language}-{'typed-error' if denied else 'offline'}"
    attempt_no = 1
    while (root / "runs" / f"{label}-{attempt_no:02}").exists():
        attempt_no += 1
    receipt = root / "runs" / f"{label}-{attempt_no:02}"
    try:
        result = record(receipt, "run", argv, cwd=Path(info["cwd"]), env=env, timeout=40)
        marker = "typed 401: offline demo denied" if denied else f"{language} OFFLINE OK"
        if marker not in result.stdout or (not denied and "credits 100.50000000000000001" not in result.stdout):
            failures.append("Missing precise-value/completion marker")
        expected_methods = ["GET"] if denied else ["GET", "POST", "PATCH", "PATCH"]
        if [e["method"] for e in wire] != expected_methods:
            failures.append(f"Expected {expected_methods}, observed {[e['method'] for e in wire]}")
        print(f"{language}: {result.stdout.strip()}")
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        write_json(receipt / "wire.json", {"loopbackOnly": True, "requests": wire, "failures": failures})
    if failures:
        raise SystemExit(f"Demo quickstart failed: {failures}; retained {receipt}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("action", choices=("build", "run", "error", "where"))
    parser.add_argument("language", choices=(*LANGUAGES, "javascript", "all"))
    args = parser.parse_args()
    root = owned_root(args.root)
    languages = LANGUAGES if args.language == "all" else (args.language,)
    for language in languages:
        if args.action == "build":
            try:
                build(root, "typescript" if language == "javascript" else language)
            except (SystemExit, OSError, subprocess.TimeoutExpired) as error:
                attempts = sorted((root / "native").glob(f"{language}-*"))
                if attempts:
                    write_json(attempts[-1] / "preparation-failure.json", {"error": str(error), "language": language})
                raise
        elif args.action == "where":
            index = json.loads((root / "native-index.json").read_text())
            info = index["typescript" if language == "javascript" else language]
            print(json.dumps({key: info[key] for key in ("language", "attempt", "package", "consumer", "argv", "cwd", "tools")}, indent=2))
        else:
            smoke(root, language, args.action == "error")
            if language == "typescript" and args.language == "all":
                smoke(root, "javascript", args.action == "error")


if __name__ == "__main__":
    main()
