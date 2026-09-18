"""Maintained per-language native build and installed-consumer recipes.

Preparation commands are retained separately from the four measured phases.
Every build starts with a byte-checked new package copy and private outputs.
Under the repeated-v1 methodology each measured phase executes a run set:
cold runs, excluded warmups, then the steady-state batch. Between runs the
fresh package copy is restored to pristine bytes and each recipe's private
scratch is cleared outside the timed region; every run is retained raw.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
from typing import Any, Callable
import xml.etree.ElementTree as ET
from xml.sax.saxutils import escape

from bindings import one
from evidence import Commands, DEFAULT_RUN_POLICY, PRECISION, RUN_KINDS, TOKEN, read_json, require, sha, write_json, write_new
from inputs import Inputs


WORKSPACE = Path(__file__).resolve().parents[2]
DRIVERS = Path(__file__).resolve().parent / "drivers"
SUFFIXES = {"typescript": "ts", "rust": "rs", "python": "py", "go": "go", "swift": "swift", "java": "java",
            "kotlin": "kt", "csharp": "cs", "ruby": "rb", "php": "php", "dart": "dart", "cpp": "cpp"}


def discard(path: Path) -> None:
    """Remove a per-run scratch output so the next measured run starts cold."""
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)


def native_environment(source: dict[str, str], work: Path) -> dict[str, str]:
    keys = ("PATH", "HOME", "RUSTUP_HOME", "CARGO_HOME", "UV_CACHE_DIR", "npm_config_cache", "GOMODCACHE",
            "DEVELOPER_DIR", "MACOSX_DEPLOYMENT_TARGET")
    env = {key: source[key] for key in keys if key in source}
    env.update({
        "PATH": source.get("PATH", os.defpath), "HOME": str(work / "home"), "TMPDIR": str(work / "tmp"),
        "LANG": "C", "LC_ALL": "C", "PYTHONDONTWRITEBYTECODE": "1", "PYTHONNOUSERSITE": "1",
        "CARGO_NET_OFFLINE": "true", "CARGO_BUILD_JOBS": "2", "CARGO_TERM_COLOR": "never", "RUSTUP_AUTO_INSTALL": "0",
        "CARGO_HOME": source.get("CARGO_HOME", str(Path(source.get("HOME", "")) / ".cargo")),
        "RUSTUP_HOME": source.get("RUSTUP_HOME", str(Path(source.get("HOME", "")) / ".rustup")),
        "GOTOOLCHAIN": "local", "GOWORK": "off", "GOENV": "off", "GOPROXY": "off", "GOSUMDB": "off",
        "GOCACHE": str(work / "go-cache"), "GOPATH": str(work / "go-path"),
        "npm_config_offline": "true", "npm_config_audit": "false", "npm_config_fund": "false",
        "npm_config_userconfig": str(work / "home/.npmrc"), "UV_OFFLINE": "1", "UV_PYTHON_DOWNLOADS": "never",
        "PIP_NO_INDEX": "1", "PIP_DISABLE_PIP_VERSION_CHECK": "1", "PIP_NO_CACHE_DIR": "1",
        "COMPOSER_DISABLE_NETWORK": "1", "COMPOSER_NO_INTERACTION": "1", "COMPOSER_HOME": str(work / "composer-home"),
        "COMPOSER_CACHE_DIR": str(work / "composer-cache"), "PUB_CACHE": str(work / "pub-cache"),
        "DART_SUPPRESS_ANALYTICS": "true", "DOTNET_CLI_HOME": str(work / "dotnet-home"),
        "DOTNET_CLI_TELEMETRY_OPTOUT": "1", "DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE": "true",
        "DOTNET_NOLOGO": "1", "DOTNET_MULTILEVEL_LOOKUP": "0", "MSBUILDDISABLENODEREUSE": "1",
        "NUGET_PACKAGES": str(work / "nuget-cache"), "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
        "MAVEN_ARGS": "--offline --no-transfer-progress", "MAVEN_USER_HOME": str(work / "home/.m2"),
    })
    for name in ("home", "tmp", "home/.m2"):
        (work / name).mkdir(parents=True)
    write_new(work / "home/.npmrc", "")
    return env


class Tools:
    def __init__(self, source: dict[str, str], inputs: Inputs, commands: Commands):
        self.env = source
        self.inputs = inputs
        self.commands = commands
        self.selections: dict[str, str] = {}
        manifest = inputs.packages.parent / "full-tool-selection.json"
        if manifest.is_file():
            inputs.remember(manifest)
            self.selections = read_json(manifest)["selectors"]
        self.home = Path(self.selections.get("{caller-home}", source.get("HOME", "")))
        self.installs = self.home / ".local/share/mise/installs"

    def select(self, selector: str, token: str, default: Path | str) -> Path:
        value = self.env.get(selector) or self.selections.get(token) or str(default)
        for _ in range(10):
            expanded = value
            for key, replacement in self.selections.items():
                expanded = expanded.replace(key, replacement)
            if expanded == value:
                break
            value = expanded
        require("{" not in value, f"unresolved installed tool selection {selector}: {value}")
        path = Path(value).expanduser().absolute()
        require(path.exists(), f"missing installed tool {selector}: {path}")
        require("/shims/" not in str(path), f"select actual executable, not an activating shim: {path}")
        return path

    def on_path(self, program: str) -> Path:
        found = shutil.which(program, path=self.env.get("PATH", os.defpath))
        require(found, f"missing installed executable on runner PATH: {program}")
        path = Path(found).absolute()
        require("/shims/" not in str(path), f"activating tool shim refused: {path}")
        return path

    def probe(self, argv: list[str | Path], ctx: Context, expected: str | None = None) -> str:
        record = self.commands.run(argv, ctx.work, ctx.env, label=f"{ctx.key}: tool version/identity", timeout=30)
        for path in [Path(argv[0])]:
            self.inputs.remember(path)
        text = self.commands.stdout(record) + (self.commands.root / record["stderr"]).read_text()
        if expected:
            require(expected in text, f"{ctx.key}: installed tool does not match tier {expected!r}; inspect {record['record']}")
        return text.strip()

    def node(self, ctx: Context) -> tuple[Path, Path]:
        floor = ctx.slot == "floor"
        node = self.select("SUSPECT_DOCS_NODE" if floor else "SUSPECT_NODE24_BIN", "{node22}" if floor else "{node24}",
                           self.installs / f"node/{'22.23.1' if floor else '24.21.0'}/bin/node")
        compiler = WORKSPACE / "crates/suspect-codegen/tools" / ("typescript-floor" if floor else "typescript-docs") / "node_modules/typescript/lib/tsc.js"
        require(compiler.is_file(), f"runner's locked offline TypeScript compiler is missing: {compiler}")
        self.probe([node, "--version"], ctx, "v22.23.1" if floor else "v24.21.0")
        self.probe([node, compiler, "--version"], ctx, "5.5.4" if floor else "5.9.3")
        self.inputs.remember(compiler)
        # Since TS 5.9 tsc.js loads _tsc.js. Attribute that implementation too.
        for p in compiler.parent.glob("*tsc.js"):
            self.inputs.remember(p)
        return node, compiler

    def jdk(self, ctx: Context) -> Path:
        version = "21.0.12+101.0.LTS" if ctx.slot == "floor" else "25.0.4+101.0.LTS"
        home = self.select(f"SUSPECT_JAVA_{ctx.slot.upper()}_HOME", f"{{jdk-{ctx.slot}}}", self.installs / f"java/temurin-{version}")
        ctx.env["JAVA_HOME"] = str(home)
        self.probe([home / "bin/java", "--version"], ctx, "21.0.12" if ctx.slot == "floor" else "25.0.4")
        self.probe([home / "bin/javac", "--version"], ctx, "21.0.12" if ctx.slot == "floor" else "25.0.4")
        return home

    def swift(self, ctx: Context) -> tuple[Path, Path]:
        floor = ctx.slot == "floor"
        if floor:
            temp = Path(self.env.get("TMPDIR", "/private/var/tmp"))
            default = temp / "opencode/swift-6.0.3-toolchain/expanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"
            floor_root = Path(self.env.get("SUSPECT_SWIFT_FLOOR_ROOT", str(default)))
            swift = self.select("SUSPECT_SWIFT_FLOOR_BIN", "{swift-floor}", floor_root / "usr/bin/swift")
            sdk = self.select("SUSPECT_SWIFT_FLOOR_SDKROOT", "{swift-floor-sdk}", "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk")
            compiler = self.select("SUSPECT_SWIFTC_FLOOR_BIN", "{swiftc-floor}", swift.with_name("swiftc"))
        else:
            xcrun = Path("/usr/bin/xcrun")
            candidate = self.env.get("SUSPECT_SWIFT_BIN", self.selections.get("{swift}"))
            if candidate in (None, "/usr/bin/swift"):
                candidate = self.probe([xcrun, "--find", "swift"], ctx)
            swift = self.select("SUSPECT_SWIFT_BIN", "{swift}", candidate)
            if swift == Path("/usr/bin/swift"):
                swift = Path(candidate)
            sdk_candidate = self.env.get("SUSPECT_SWIFT_SDKROOT", self.selections.get("{swift-sdk}"))
            if not sdk_candidate:
                sdk_candidate = self.probe([xcrun, "--sdk", "macosx", "--show-sdk-path"], ctx)
            sdk = self.select("SUSPECT_SWIFT_SDKROOT", "{swift-sdk}", sdk_candidate)
            compiler = self.select("SUSPECT_SWIFTC_BIN", "{swiftc}", swift.with_name("swiftc"))
            if compiler == Path("/usr/bin/swiftc"):
                compiler = Path(self.probe([xcrun, "--find", "swiftc"], ctx))
        self.probe([swift, "--version"], ctx, "6.0.3" if floor else "6.3.3")
        self.probe([compiler, "--version"], ctx, "6.0.3" if floor else "6.3.3")
        settings = sdk / "SDKSettings.json"
        require(read_json(settings)["Version"] == ("15.4" if floor else "26.5"), "Swift SDK version differs from tier")
        self.inputs.remember(settings)
        ctx.env.update(SWIFT_EXEC=str(compiler), SDKROOT=str(sdk), CLANG_MODULE_CACHE_PATH=str(ctx.work / "clang-cache"))
        return swift, sdk


class Context:
    def __init__(self, target: dict[str, Any], tier: str, slot: str, binding: dict[str, Any], installed: dict[str, Any],
                 fixture: str, inputs: Inputs, tools: Tools, commands: Commands,
                 build_observer: Callable[[list[dict[str, Any]], list[Path], str], None],
                 policy: dict[str, int] | None = None):
        self.target, self.tier, self.slot, self.binding = target, tier, slot, binding
        self.language = target["language"]
        self.key = f"{self.language}/{tier}"
        self.work = commands.root / "work" / self.language / slot
        self.work.mkdir(parents=True)
        self.env = native_environment(tools.env, self.work)
        self.env["SUSPECT_COSTS_PACKAGE"] = installed["package"]
        self.installed = installed
        self.package = Path(installed["package"])
        self.root = Path(installed["root"])
        self.inputs, self.tools, self.commands = inputs, tools, commands
        self.fresh = inputs.copy_package(target, self.work / "sdk")
        self.fixture = fixture
        self.observe_build = build_observer
        self.policy = dict(policy) if policy else dict(DEFAULT_RUN_POLICY)
        self.command: list[str | Path] = []
        self.runtime_artifacts: list[Path] = []
        self.import_scope = "new linked consumer process startup, runtime initialization and public SDK type load"
        self.runtime_scope = "subprocess wall time, including native runtime/module startup, typed assertions and cleanup"
        self.stage = "build"

    def run(self, argv: list[str | Path], cwd: Path | None = None, *, label: str = "preparation", timeout: float = 300,
            env: dict[str, str] | None = None) -> dict[str, Any]:
        return self.commands.run(argv, cwd or self.work, env or self.env, label=f"{self.key}: {label}", timeout=timeout)

    def restore_fresh(self) -> None:
        """Restore the private emitted-package copy to pristine bytes between measured runs."""
        shutil.rmtree(self.fresh)
        self.fresh = self.inputs.copy_package(self.target, self.fresh)

    def build(self, argv: list[str | Path], artifacts: list[Path] | Callable[[], list[Path]], scope: str,
              cwd: Path | None = None, env: dict[str, str] | None = None,
              reset: Callable[[], None] | None = None) -> None:
        """Measured build run set: every run rebuilds from pristine private inputs.

        The first run needs no reset. Later runs restore the fresh package copy
        and re-establish the recipe's private scratch state (compiler caches,
        target directories, regenerated preparation outputs) strictly outside
        the timed region. Cold, warmup and steady runs are all retained raw;
        summaries are computed only over the steady batch.
        """
        self.stage = "build"
        runs: list[dict[str, Any]] = []
        for kind in RUN_KINDS:
            for index in range(self.policy[kind]):
                if runs:
                    self.restore_fresh()
                    if reset:
                        reset()
                record = self.run(argv, cwd, label=f"measured build ({kind} {index + 1}/{self.policy[kind]})",
                                  timeout=600, env=env)
                runs.append({"kind": kind, "record": record})
        self.observe_build(runs, artifacts() if callable(artifacts) else artifacts, scope)
        self.stage = "consumer-preparation"

    def driver(self, path: Path) -> None:
        source = DRIVERS / f"{self.language}.{SUFFIXES[self.language]}.in"
        self.inputs.remember(source)
        replacements = {key.upper(): str(value) for key, value in self.binding.items() if isinstance(value, str)}
        replacements.update(PACKAGE=self.target["package_name"], IMPORT=self.target.get("import_name") or "", TOKEN=TOKEN, PRECISION=PRECISION)
        literal = json.dumps(self.fixture, ensure_ascii=True)
        if self.language == "php":
            literal = "'" + self.fixture.replace("\\", "\\\\").replace("'", "\\'") + "'"
        elif self.language in ("kotlin", "dart"):
            literal = literal.replace("$", "\\$")
        replacements["FIXTURE"] = literal
        if self.language == "kotlin":
            variant = self.binding["variant"]
            native = variant if "." in variant else self.binding["result"] + "." + variant
            replacements["KOTLIN_DATA"] = "response.data" if self.binding["directData"] else f"(response as {native}).data"
        text = source.read_text()
        for key, value in replacements.items():
            text = text.replace("@@" + key + "@@", value)
        require("@@" not in text, f"unresolved {self.language} driver binding")
        write_new(path, text)


def prepare(ctx: Context) -> None:
    globals()["prepare_" + ctx.language](ctx)
    require(ctx.command and ctx.runtime_artifacts, f"{ctx.key}: native consumer was not prepared")
    write_json(ctx.work / "consumer.json", {"command": [str(a) for a in ctx.command], "binding": ctx.binding,
                                            "importScope": ctx.import_scope, "runtimeScope": ctx.runtime_scope})


def prepare_typescript(ctx: Context) -> None:
    node, compiler = ctx.tools.node(ctx)
    ctx.build([node, compiler, "--project", ctx.fresh / "tsconfig.json", "--pretty", "false"], [ctx.fresh / "dist"],
              "fresh emitted package: TypeScript compile/declarations, locked warm compiler, private dist", cwd=ctx.fresh)
    consumer = ctx.work / "consumer"
    write_json(consumer / "package.json", {"name": "native-costs-consumer", "private": True, "type": "module"})
    link = consumer / "node_modules" / ctx.target["package_name"]
    link.parent.mkdir(parents=True)
    link.symlink_to(ctx.package, target_is_directory=True)
    ctx.driver(consumer / "probe.ts")
    write_json(consumer / "tsconfig.json", {"compilerOptions": {"strict": True, "exactOptionalPropertyTypes": True,
               "noUncheckedIndexedAccess": True, "target": "ES2022", "module": "NodeNext", "moduleResolution": "NodeNext",
               "lib": ["ES2022", "DOM", "DOM.Iterable"], "outDir": "dist"}, "files": ["probe.ts"]})
    ctx.run([node, compiler, "--project", consumer / "tsconfig.json", "--pretty", "false"], consumer)
    ctx.command = [node, consumer / "dist/probe.js"]
    ctx.runtime_artifacts = [ctx.package / "dist", consumer / "dist/probe.js"]
    ctx.import_scope = "new Node process, actual installed public ESM entry and transitive SDK module evaluation"


def prepare_python(ctx: Context) -> None:
    floor = ctx.slot == "floor"
    python = ctx.tools.select(f"SUSPECT_PYTHON_{ctx.slot.upper()}_BIN", f"{{python-{ctx.slot}}}",
                             ctx.tools.home / ".local/share/uv/python/cpython-3.11-macos-aarch64-none/bin/python3.11" if floor
                             else "/opt/homebrew/opt/python@3.14/bin/python3.14")
    tools = ctx.tools.select("SUSPECT_PYTHON_TOOLS", "{python-tools}", WORKSPACE / "target/sdk-native-python-tools/bin/python")
    site = one(list((tools.parent.parent / "lib").glob("python*/site-packages")), "warm Python build-backend site")
    ctx.tools.probe([python, "--version"], ctx, "Python " + ctx.tier)
    ctx.tools.probe([Path(ctx.installed["python"]), "--version"], ctx, "Python " + ctx.tier)
    build_env = {**ctx.env, "PYTHONPATH": str(site)}

    def reset_run() -> None:
        discard(ctx.work / "wheel")

    ctx.build([python, "-m", "build", "--wheel", "--no-isolation", "--outdir", ctx.work / "wheel"],
              [ctx.work / "wheel"], "fresh emitted package: native-tier Python wheel build, warm installed hatchling/build, no isolation/downloads",
              ctx.fresh, build_env, reset=reset_run)
    ctx.driver(ctx.work / "consumer/probe.py")
    ctx.command = [Path(ctx.installed["python"]), "-B", ctx.work / "consumer/probe.py"]
    ctx.runtime_artifacts = [ctx.package, ctx.work / "consumer/probe.py"]
    ctx.import_scope = "new installed-wheel interpreter, actual package import and public model/codec module load; bytecode writes disabled"


def prepare_go(ctx: Context) -> None:
    selected = ctx.tools.selections.get(f"{{go-{ctx.slot}-bin}}")
    if selected:
        go = Path(selected) / "go"
    elif ctx.slot == "current":
        go = ctx.tools.on_path("go").resolve()
    else:
        seeds = [Path(ctx.tools.env.get("GOMODCACHE", str(ctx.tools.home / "go/pkg/mod"))), ctx.tools.home / "go/pkg/mod"]
        candidates = [p for seed in seeds for p in seed.glob("golang.org/toolchain@v0.0.1-go1.23.12.*/bin/go")]
        candidates += [ctx.tools.installs / "go/1.23.12/bin/go"]
        go = one(sorted({p.resolve() for p in candidates if p.is_file()}), "already installed Go floor implementation")
    ctx.tools.probe([go, "version"], ctx, "go" + ctx.tier)

    def reset_run() -> None:
        # A cold archive build starts from an empty private compiler cache.
        discard(ctx.work / "go-cache")
        discard(ctx.work / "sdk.a")

    ctx.build([go, "build", "-buildmode=archive", "-o", ctx.work / "sdk.a", "."], [ctx.work / "sdk.a"],
              "fresh emitted Go module: SDK archive compilation with private compiler cache/output and offline module sources", ctx.fresh,
              reset=reset_run)
    consumer = ctx.work / "consumer"
    write_new(consumer / "go.mod", f"module suspect.example/native-costs\n\ngo 1.23\n\nrequire {ctx.target['package_name']} v0.0.0\nreplace {ctx.target['package_name']} => {json.dumps(str(ctx.package))}\n")
    ctx.driver(consumer / "main.go")
    ctx.run([go, "build", "-o", consumer / "native-costs", "."], consumer)
    ctx.command = [consumer / "native-costs"]
    ctx.runtime_artifacts = [consumer / "native-costs"]


def prepare_rust(ctx: Context) -> None:
    rustup = ctx.tools.on_path("rustup")
    toolchain = ctx.tier
    cargo_record = ctx.run([rustup, "which", "--toolchain", toolchain, "cargo"], label="installed Cargo resolution", timeout=30)
    rustc_record = ctx.run([rustup, "which", "--toolchain", toolchain, "rustc"], label="installed rustc resolution", timeout=30)
    cargo = Path(ctx.commands.stdout(cargo_record).strip())
    rustc = Path(ctx.commands.stdout(rustc_record).strip())
    ctx.env.update(RUSTUP_TOOLCHAIN=toolchain, RUSTC=str(rustc), CARGO_TARGET_DIR=str(ctx.work / "package-build"))
    ctx.tools.probe([cargo, "--version"], ctx)
    ctx.tools.probe([rustc, "--version", "--verbose"], ctx, "rustc 1.88.0" if ctx.slot == "floor" else None)
    ctx.run([cargo, "generate-lockfile", "--offline", "--manifest-path", ctx.fresh / "Cargo.toml"])

    def reset_run() -> None:
        # A cold release build starts from an empty private target directory;
        # restoring the pristine package copy removes Cargo.lock, so the
        # --locked lockfile is regenerated as unmeasured preparation.
        discard(ctx.work / "package-build")
        ctx.run([cargo, "generate-lockfile", "--offline", "--manifest-path", ctx.fresh / "Cargo.toml"],
                label="per-run cold-build lockfile regeneration", timeout=120)

    ctx.build([cargo, "build", "--locked", "--offline", "--release", "--features", "reqwest-rustls", "--manifest-path", ctx.fresh / "Cargo.toml"],
              [ctx.work / "package-build/release/libsdk_full.rlib"],
              "fresh emitted Rust package: release reqwest-rustls SDK build, offline dependency sources, private empty target directory",
              reset=reset_run)
    consumer = ctx.work / "consumer"
    write_new(consumer / "Cargo.toml", f"[package]\nname='native-costs'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nsdk_full={{package={json.dumps(ctx.target['package_name'])},path={json.dumps(str(ctx.package))},features=['reqwest-rustls']}}\ntokio={{version='1',features=['rt','time','net']}}\n")
    ctx.driver(consumer / "src/main.rs")
    ctx.env["CARGO_TARGET_DIR"] = str(ctx.work / "consumer-build")
    ctx.run([cargo, "generate-lockfile", "--offline", "--manifest-path", consumer / "Cargo.toml"])
    ctx.run([cargo, "build", "--locked", "--offline", "--release", "--manifest-path", consumer / "Cargo.toml"], timeout=600)
    ctx.command = [ctx.work / "consumer-build/release/native-costs"]
    ctx.runtime_artifacts = [Path(ctx.command[0])]


def prepare_swift(ctx: Context) -> None:
    swift, sdk = ctx.tools.swift(ctx)
    build = ctx.work / "package-build"
    module = ctx.target["import_name"]

    def reset_run() -> None:
        # Cold SwiftPM builds start from empty scratch, module and Clang caches.
        discard(build)
        discard(ctx.work / "swift-cache")
        discard(ctx.work / "clang-cache")

    ctx.build([swift, "build", "--package-path", ctx.fresh, "--scratch-path", build, "--cache-path", ctx.work / "swift-cache",
               "--sdk", sdk, "-c", "release", "--jobs", "2"],
              lambda: [p for p in build.rglob("*.o") if p.parent.name == module + ".build"]
                      + [p for p in build.rglob(module + ".swiftmodule") if p.parent.name == "Modules"],
              "fresh emitted Swift package: release SwiftPM module/object compilation, explicit toolchain/SDK and private scratch",
              reset=reset_run)
    consumer = ctx.work / "consumer"
    write_new(consumer / "Package.swift", f'// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: "NativeCosts", platforms: [.macOS(.v13)], dependencies: [.package(name: "{module}", path: {json.dumps(str(ctx.package))})], targets: [.executableTarget(name: "NativeCosts", dependencies: [.product(name: "{module}", package: "{module}")])])\n')
    ctx.driver(consumer / "Sources/NativeCosts/Probe.swift")
    ctx.run([swift, "build", "--package-path", consumer, "--scratch-path", ctx.work / "consumer-build", "--cache-path", ctx.work / "consumer-cache",
             "--sdk", sdk, "-c", "release", "--jobs", "2"])
    ctx.command = [ctx.work / "consumer-build/release/NativeCosts"]
    ctx.runtime_artifacts = [Path(ctx.command[0])]


def prepare_java(ctx: Context) -> None:
    jdk = ctx.tools.jdk(ctx)
    classes = ctx.work / "package-classes"
    classes.mkdir()
    source = sorted((ctx.fresh / "src/main/java").rglob("*.java"))
    require(source, "fresh Java package contains no native sources")

    def reset_run() -> None:
        discard(classes)
        classes.mkdir()

    ctx.build([jdk / "bin/javac", "--release", "21", "-Xlint:all", "-Werror", "-d", classes, *source], [classes],
              "fresh emitted Java package: javac compilation of every main source into a private class directory (no Javadoc)",
              reset=reset_run)
    consumer = ctx.work / "consumer"
    ctx.driver(consumer / "NativeCosts.java")
    jar = Path(ctx.installed["jar"])
    ctx.run([jdk / "bin/javac", "--release", "21", "-Xlint:all", "-Werror", "-cp", jar, "-d", consumer / "classes", consumer / "NativeCosts.java"])
    ctx.command = [jdk / "bin/java", "-ea", "-cp", str(jar) + os.pathsep + str(consumer / "classes"), "NativeCosts"]
    ctx.runtime_artifacts = [jar, consumer / "classes"]


def prepare_kotlin(ctx: Context) -> None:
    jdk = ctx.tools.jdk(ctx)
    maven = ctx.tools.select("SUSPECT_MAVEN_BIN", "{maven}", ctx.tools.installs / "maven/3.9.16/apache-maven-3.9.16/bin/mvn")
    ctx.tools.probe([maven, "--version"], ctx, "3.9.16")
    seed = WORKSPACE / "target/sdk-kotlin-maven"
    if not seed.is_dir():
        seed = ctx.tools.select("SUSPECT_FULL_KOTLIN_MAVEN_CACHE", "{kotlin-maven-seed}", seed)
    repo = ctx.commands.root / "caches/kotlin-maven"
    if not repo.exists():
        shutil.copytree(seed, repo, copy_function=shutil.copyfile)
    args = [maven, "--offline", "-B", "--no-transfer-progress", f"-Dmaven.repo.local={repo}"]
    ctx.build([*args, "compile"], [ctx.fresh / "target/classes"],
              "fresh emitted Kotlin package: Maven compile lifecycle, Kotlin 2.4.20, warm copied offline dependency/plugin cache, private classes", ctx.fresh)
    pom = ET.parse(ctx.fresh / "pom.xml").getroot()
    dependencies = pom.find("{*}dependencies")
    require(dependencies is not None, "Kotlin emitted dependencies missing")
    versions = {d.findtext("{*}artifactId"): d.findtext("{*}version") for d in dependencies}
    require(versions.get("kotlin-stdlib") == "2.4.20" and versions.get("kotlinx-coroutines-core-jvm") == "1.11.0", "Kotlin dependency tier changed")
    consumer = ctx.work / "consumer"
    # System scope names the actual runner-installed jar instead of resolving a
    # coordinate that a later tier may have replaced in the shared Maven cache.
    jar = Path(ctx.installed["jar"])
    write_new(consumer / "pom.xml", f'''<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>suspect.costs</groupId><artifactId>native-costs</artifactId><version>0.0.0</version>
<properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon></properties>
<dependencies><dependency><groupId>suspect.installed</groupId><artifactId>sdk</artifactId><version>0.0.0</version><scope>system</scope><systemPath>{escape(str(jar))}</systemPath></dependency>
<dependency><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-stdlib</artifactId><version>2.4.20</version></dependency>
<dependency><groupId>org.jetbrains.kotlinx</groupId><artifactId>kotlinx-coroutines-core-jvm</artifactId><version>1.11.0</version></dependency></dependencies>
<build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins>
<plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.4.20</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.5.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.0</version></plugin>
</plugins></build></project>\n''')
    ctx.driver(consumer / "src/main/kotlin/NativeCosts.kt")
    ctx.run([*args, "compile"], consumer)
    dependencies = [repo / "org/jetbrains/kotlin/kotlin-stdlib/2.4.20/kotlin-stdlib-2.4.20.jar",
                    repo / "org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.11.0/kotlinx-coroutines-core-jvm-1.11.0.jar"]
    for dependency in dependencies:
        ctx.inputs.remember(dependency)
    classpath = [jar, consumer / "target/classes", *dependencies]
    ctx.command = [jdk / "bin/java", "-ea", "-cp", os.pathsep.join(str(p) for p in classpath), "nativecosts.NativeCosts"]
    ctx.runtime_artifacts = classpath


def prepare_csharp(ctx: Context) -> None:
    version, tfm = ctx.tier.split("-")
    dotnet = ctx.tools.select("SUSPECT_DOTNET_BIN", "{dotnet}", ctx.tools.home / ".local/share/mise/dotnet-root/dotnet")
    ctx.env["DOTNET_ROOT"] = str(dotnet.parent)
    write_json(ctx.work / "global.json", {"sdk": {"version": version, "rollForward": "disable"}})
    config = ctx.work / "NuGet.Config"
    write_new(config, "<configuration><packageSources><clear/></packageSources></configuration>\n")
    ctx.tools.probe([dotnet, "--version"], ctx, version)
    ctx.run([dotnet, "restore", ctx.fresh / "Suspect.csproj", "--configfile", config])

    def reset_run() -> None:
        # Restoring the pristine package copy removes bin/obj with the restore
        # assets, so the unmeasured restore runs again before a cold build.
        ctx.run([dotnet, "restore", ctx.fresh / "Suspect.csproj", "--configfile", config],
                label="per-run cold-build package restore")

    ctx.build([dotnet, "build", ctx.fresh / "Suspect.csproj", "-c", "Release", "--no-restore", "-m:1", "-p:UseSharedCompilation=false"],
              [ctx.fresh / "bin/Release/net8.0"], "fresh emitted C# SDK: pinned .NET SDK release build, local framework packs, private bin/obj",
              reset=reset_run)
    consumer = ctx.work / "consumer"
    dll = Path(ctx.installed["dll"])
    write_new(consumer / "NativeCosts.csproj", f'''<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>{tfm}</TargetFramework><OutputType>Exe</OutputType><AssemblyName>NativeCosts</AssemblyName><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>disable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><Reference Include="{ctx.target['package_name']}"><HintPath>{escape(str(dll))}</HintPath><Private>true</Private></Reference></ItemGroup></Project>\n''')
    ctx.driver(consumer / "Program.cs")
    ctx.run([dotnet, "restore", consumer / "NativeCosts.csproj", "--configfile", config])
    ctx.run([dotnet, "build", consumer / "NativeCosts.csproj", "-c", "Release", "--no-restore", "-m:1", "-p:UseSharedCompilation=false"])
    binary = consumer / "bin/Release" / tfm
    require(sha(binary / dll.name) == sha(dll), "consumer's runtime assembly differs from installed NuGet DLL")
    ctx.command = [dotnet, binary / "NativeCosts.dll"]
    ctx.runtime_artifacts = [binary]


def prepare_ruby(ctx: Context) -> None:
    home = ctx.tools.select(f"SUSPECT_RUBY_{ctx.slot.upper()}_HOME", f"{{ruby-{ctx.slot}}}", ctx.tools.installs / "ruby" / ctx.tier)
    ruby = home / "bin/ruby"
    defaults = one([p for p in (home / "lib/ruby/gems").iterdir() if p.is_dir()], "Ruby default gem ABI directory")
    ctx.env.update(GEM_HOME=str(ctx.root / "installed"), GEM_PATH=str(ctx.root / "installed") + os.pathsep + str(defaults))
    ctx.tools.probe([ruby, "--version"], ctx, "ruby " + ctx.tier)
    gem = home / "bin/gem"
    ctx.inputs.remember(gem)

    def reset_run() -> None:
        discard(ctx.work / "sdk.gem")

    ctx.build([ruby, gem, "build", ctx.target["manifest"], "--output", ctx.work / "sdk.gem"], [ctx.work / "sdk.gem"],
              "fresh emitted Ruby package: native gem archive build with installed RubyGems", ctx.fresh, reset=reset_run)
    ctx.driver(ctx.work / "consumer/probe.rb")
    ctx.command = [ruby, ctx.work / "consumer/probe.rb"]
    ctx.runtime_artifacts = [ctx.package / "lib", ctx.work / "consumer/probe.rb"]
    ctx.import_scope = "new Ruby VM, actual installed gem activation/require, generated public client/models/codecs load"


def prepare_php(ctx: Context) -> None:
    version = ctx.tier
    php = ctx.tools.select(f"SUSPECT_PHP_{ctx.slot.upper()}_BIN", f"{{php-{ctx.slot}}}", WORKSPACE / f"target/sdk-php-tools/php-{version}/php")
    composer = ctx.tools.select("SUSPECT_COMPOSER_PHAR", "{composer}", WORKSPACE / "target/sdk-php-tools/composer-2.10.3.phar")
    ctx.tools.probe([php, "-n", "--version"], ctx, "PHP " + version)
    ctx.tools.probe([php, "-n", composer, "--version"], ctx, "2.10.3")
    ctx.inputs.remember(composer)
    ctx.build([php, "-n", composer, "dump-autoload", "--classmap-authoritative", "--no-dev", "--no-plugins", "--no-scripts"],
              [ctx.fresh / "vendor", ctx.fresh / "src"],
              "fresh emitted PHP package: Composer authoritative classmap/autoloader generation; PHP has no separate library compilation", ctx.fresh)
    ctx.env["SUSPECT_COSTS_AUTOLOAD"] = ctx.installed["autoload"]
    ctx.driver(ctx.work / "consumer/probe.php")
    ctx.command = [php, "-n", "-d", "error_reporting=-1", ctx.work / "consumer/probe.php"]
    ctx.runtime_artifacts = [ctx.package / "src", Path(ctx.installed["autoload"]), ctx.root / "consumer/vendor/composer", ctx.work / "consumer/probe.php"]
    ctx.import_scope = "new PHP VM, installed Composer autoloader and actual public SDK Client/Codecs class loads"


def prepare_dart(ctx: Context) -> None:
    default = WORKSPACE / "target/sdk-dart-tools" / ("dart-sdk/bin/dart" if ctx.slot == "floor" else "current-3.13.3/dart-sdk/bin/dart")
    dart = ctx.tools.select(f"SUSPECT_DART_{ctx.slot.upper()}_BIN", f"{{dart-{ctx.slot}}}", default)
    ctx.tools.probe([dart, "--version"], ctx, ctx.tier)

    def consumer(root: Path, package: Path) -> Path:
        write_new(root / "pubspec.yaml", f"name: native_costs\nversion: 0.0.0\npublish_to: none\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\ndependencies:\n  {ctx.target['package_name']}:\n    path: {json.dumps(str(package))}\n")
        ctx.driver(root / "bin/probe.dart")
        ctx.run([dart, "pub", "get", "--offline"], root)
        return root / "native-costs"

    fresh_consumer = ctx.work / "build-consumer"
    built = consumer(fresh_consumer, ctx.fresh)
    ctx.build([dart, "compile", "exe", "bin/probe.dart", "-o", built], [built],
              "fresh emitted Dart package: typed public IO consumer AOT executable build after offline path dependency resolution", fresh_consumer)
    runtime = ctx.work / "consumer"
    binary = consumer(runtime, ctx.package)
    ctx.run([dart, "compile", "exe", "bin/probe.dart", "-o", binary], runtime)
    ctx.command = [binary]
    ctx.runtime_artifacts = [binary]
    ctx.import_scope = "new AOT Dart linked-consumer startup and public codec type access (dart compile exe output)"


def prepare_cpp(ctx: Context) -> None:
    cmake = ctx.tools.select("SUSPECT_CPP_CMAKE", "{cmake}", ctx.tools.installs / "cmake/3.31.6/cmake-3.31.6-macos-universal/CMake.app/Contents/bin/cmake")
    cxx = ctx.tools.select("SUSPECT_CPP_CXX", "{cxx}", "/usr/bin/clang++")
    ctx.tools.probe([cmake, "--version"], ctx, "3.31.6")
    ctx.tools.probe([cxx, "--version"], ctx, "clang version 21.0.0")
    sdk_candidate = ctx.tools.env.get("SUSPECT_SWIFT_SDKROOT", ctx.tools.selections.get("{swift-sdk}"))
    if not sdk_candidate:
        sdk_candidate = ctx.tools.probe([Path("/usr/bin/xcrun"), "--sdk", "macosx", "--show-sdk-path"], ctx)
    sdk = ctx.tools.select("SUSPECT_SWIFT_SDKROOT", "{swift-sdk}", sdk_candidate)
    ctx.env["SDKROOT"] = str(sdk)
    options = ["-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_CXX_COMPILER={cxx}", f"-DCMAKE_OSX_SYSROOT={sdk}", "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=OFF"]
    build = ctx.work / "package-build"
    configure = [cmake, "-S", ctx.fresh, "-B", build, *options, "-DSUSPECT_SDK_BUILD_DOCS=OFF", "-DSUSPECT_SDK_BUILD_EXAMPLES=OFF"]
    ctx.run(configure)

    def reset_run() -> None:
        # Cold static-library builds start from an empty private build tree;
        # re-configuration is unmeasured preparation outside the timed region.
        discard(build)
        ctx.run(configure, label="per-run cold-build CMake configuration")

    ctx.build([cmake, "--build", build, "--target", ctx.target["package_name"], "--parallel", "2"],
              [build / ("lib" + ctx.target["package_name"] + ".a")],
              "fresh emitted C++ package: release CMake static SDK/library compile, installed libcurl and explicit Apple Clang/SDK, private build",
              reset=reset_run)
    consumer = ctx.work / "consumer"
    package = ctx.target["package_name"]
    write_new(consumer / "CMakeLists.txt", f"cmake_minimum_required(VERSION 3.24)\nproject(NativeCosts LANGUAGES CXX)\nfind_package({package} 0.0.0 EXACT CONFIG REQUIRED)\nadd_executable(native_costs Probe.cpp)\ntarget_link_libraries(native_costs PRIVATE {package}::{package})\ntarget_compile_options(native_costs PRIVATE -Wall -Wextra -Wpedantic -Werror)\n")
    ctx.driver(consumer / "Probe.cpp")
    native_build = ctx.work / "consumer-build"
    ctx.run([cmake, "-S", consumer, "-B", native_build, *options, f"-DCMAKE_PREFIX_PATH={ctx.package}"])
    ctx.run([cmake, "--build", native_build, "--parallel", "2"])
    ctx.command = [native_build / "native_costs"]
    ctx.runtime_artifacts = [native_build / "native_costs"]
