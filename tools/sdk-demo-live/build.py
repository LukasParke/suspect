#!/usr/bin/env python3
"""Explicit preparation: native packages/consumers, using configuration identities."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil

from common import LANGUAGES, REPO, ROOT, SOURCES, fresh, record, save, sha

OLD = REPO / "target/sdk-demo-readme-20260911-01/candidate-02"

def put(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)

def build(language: str) -> None:
    prior = json.loads((OLD / "native-index.json").read_text())[language]
    config = next(x for x in json.loads((ROOT / "session.json").read_text())["targets"] if x["backend"] == language + "-http")
    name, version = config["package_name"], config["package_version"]
    module = config.get("import_name", name)
    attempt = fresh(ROOT / "native", language)
    package, consumer = attempt / "package", attempt / "consumer"
    shutil.copytree(ROOT / "packages" / language, package)
    consumer.mkdir()
    original = {str(p.relative_to(package)): sha(p) for p in package.rglob("*") if p.is_file()}
    save(attempt / "source-pins.json", original)
    tools = {key: Path(value["path"]) for key, value in prior["tools"].items()}
    for key, value in prior["tools"].items():
        if sha(tools[key]) != value["sha256"]:
            raise RuntimeError(f"Prepared tool changed: {key}")
    env = {}

    def run(label, args, cwd=package):
        return record(attempt, label, args, cwd, env)
    def copy(filename, destination):
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(SOURCES / language / filename, destination)
    def clone(origin, destination):
        run("clone-cache", ["/bin/cp", "-cR", origin, destination], attempt)

    bonus = None
    if language == "typescript":
        node, npm = tools["node"], tools["npm"]
        env["PATH"] = str(node.parent) + os.pathsep + os.environ["PATH"]
        actual = json.loads((package / "package.json").read_text())
        assert actual["name"] == name
        run("install-sdk-tools", [npm, "ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"])
        run("sdk-build", [npm, "run", "build"])
        run("pack", [npm, "pack", "--ignore-scripts", "--pack-destination", attempt])
        archive = next(attempt.glob("*.tgz"))
        save(consumer / "package.json", {"private": True, "type": "module", "dependencies": {name: f"file:{archive}"}})
        run("consumer-install", [npm, "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], consumer)
        copy("main.ts", consumer / "main.ts")
        shutil.copy2(SOURCES / "javascript/main.mjs", consumer / "main.mjs")
        run("typecheck", [node, package / "node_modules/typescript/bin/tsc", "--strict", "--exactOptionalPropertyTypes",
            "--target", "ES2022", "--module", "NodeNext", "--moduleResolution", "NodeNext", "--lib", "ES2023,DOM", "--outDir", "build", "main.ts"], consumer)
        argv = [str(node), str(consumer / "build/main.js")]
        bonus = [str(node), str(consumer / "main.mjs")]
    elif language == "python":
        python, uv = tools["python"], tools["uv"]
        run("wheel", [python, "-m", "build", "--wheel", "--no-isolation", "--outdir", attempt / "dist"])
        run("venv", [uv, "venv", "--offline", "--python", python, attempt / "venv"], attempt)
        executable = attempt / "venv/bin/python"
        run("install", [uv, "pip", "install", "--offline", "--python", executable, next((attempt / "dist").glob("*.whl"))], attempt)
        copy("main.py", consumer / "main.py")
        run("typecheck", [python, "-m", "mypy", "--strict", "--no-incremental", "--python-executable", executable, "main.py"], consumer)
        argv = [str(executable), str(consumer / "main.py")]
    elif language == "go":
        go = tools["go"]
        clone(Path(prior["attempt"]) / "go-cache", attempt / "go-cache")
        env.update(GOTOOLCHAIN="local", GOWORK="off", GOPROXY="off", GOSUMDB="off", GOCACHE=str(attempt / "go-cache"))
        put(consumer / "go.mod", f"module demo.local/live\n\ngo 1.23\n\nrequire {name} v{version}\nreplace {name} => ../package\n")
        copy("main.go", consumer / "main.go")
        run("build", [go, "build", "-o", consumer / "demo", "."], consumer)
        argv = [str(consumer / "demo")]
    elif language == "rust":
        cargo, rustc = tools["cargo"], tools["rustc"]
        clone(Path(prior["attempt"]) / "build", attempt / "build")
        env.update(RUSTC=str(rustc), CARGO_TARGET_DIR=str(attempt / "build"))
        put(consumer / "Cargo.toml", f'[package]\nname="live-consumer"\nversion="0.1.0"\nedition="2024"\n[workspace]\n[dependencies]\n{name}={{path="../package",features=["reqwest-rustls"]}}\ntokio={{version="=1.53.1",features=["macros","rt","time"]}}\n')
        copy("main.rs", consumer / "src/main.rs")
        run("build", [cargo, "build", "--offline"], consumer)
        argv = [str(attempt / "build/debug/live-consumer")]
    elif language == "swift":
        swift = tools["swift"]
        put(consumer / "Package.swift", f'// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name:"LiveDemo", platforms:[.macOS(.v13)], dependencies:[.package(path:"../package")], targets:[.executableTarget(name:"LiveDemo",dependencies:[.product(name:"{name}",package:"package")])])\n')
        copy("Main.swift", consumer / "Sources/LiveDemo/Main.swift")
        run("build", [swift, "build", "--disable-sandbox", "--scratch-path", attempt / "build", "-Xswiftc", "-warnings-as-errors"], consumer)
        argv = [str(attempt / "build/debug/LiveDemo")]
    elif language in ("java", "kotlin"):
        java, maven = tools["java"], tools["maven"]
        env.update(JAVA_HOME=str(java.parent.parent), PATH=str(java.parent) + os.pathsep + os.environ["PATH"])
        cache = attempt / "maven-repository"
        source_cache = REPO / ("target/sdk-java-maven-cache/java/repository" if language == "java" else "target/sdk-kotlin-maven")
        clone(source_cache, cache)
        mvn = [maven, "-B", "-o", f"-Dmaven.repo.local={cache}"]
        run("sdk-install", [*mvn, "install"])
        group, artifact = name.split(":")
        jar = cache / group.replace(".", "/") / artifact / version / f"{artifact}-{version}.jar"
        if language == "java":
            copy("Main.java", consumer / "Main.java")
            run("compile", [tools["javac"], "--release", "21", "-Xlint:all", "-Werror", "-cp", jar, "-d", consumer / "classes", "Main.java"], consumer)
            argv = [str(java), "-cp", f"{jar}:{consumer / 'classes'}", "Main"]
        else:
            copy("Smoke.kt", consumer / "src/main/kotlin/demo/Smoke.kt")
            put(consumer / "pom.xml", f'<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>demo.local</groupId><artifactId>live</artifactId><version>0.1.0</version><properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties><dependencies><dependency><groupId>{group}</groupId><artifactId>{artifact}</artifactId><version>{version}</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.4.20</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.5.0</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.0</version></plugin></plugins></build></project>\n')
            run("compile", [*mvn, "compile"], consumer)
            jars = [jar, cache / "org/jetbrains/kotlin/kotlin-stdlib/2.4.20/kotlin-stdlib-2.4.20.jar", cache / "org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.11.0/kotlinx-coroutines-core-jvm-1.11.0.jar"]
            argv = [str(java), "-cp", os.pathsep.join(map(str, [consumer / "target/classes", *jars])), "demo.SmokeKt"]
    elif language == "csharp":
        dotnet = tools["dotnet"]
        save(attempt / "global.json", {"sdk":{"version":"8.0.424","rollForward":"disable"}})
        env.update(DOTNET_ROOT=str(dotnet.parent), DOTNET_CLI_TELEMETRY_OPTOUT="1", DOTNET_SKIP_FIRST_TIME_EXPERIENCE="1", NUGET_PACKAGES=str(attempt / "nuget"))
        put(attempt / "NuGet.Config", '<configuration><packageSources><clear/></packageSources></configuration>\n')
        run("pack", [dotnet, "pack", "Suspect.csproj", "-c", "Release", "-o", attempt / "feed", "-m:1"])
        copy("Program.cs", consumer / "Program.cs")
        put(consumer / "Live.csproj", f'<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include="{name}" Version="{version}" /></ItemGroup></Project>\n')
        run("restore", [dotnet, "restore", "Live.csproj", "--source", attempt / "feed"], consumer)
        run("compile", [dotnet, "build", "Live.csproj", "--no-restore", "-c", "Release", "-m:1"], consumer)
        argv = [str(dotnet), str(consumer / "bin/Release/net8.0/Live.dll")]
    elif language == "ruby":
        ruby = tools["ruby"]
        env.update(PATH=str(ruby.parent) + os.pathsep + os.environ["PATH"], GEM_HOME=str(attempt / "gems"), GEM_PATH=os.pathsep.join([str(attempt / "gems"),str(ruby.parent.parent / "lib/ruby/gems/3.3.0")]))
        archive = attempt / f"{name}-{version}.gem"
        run("pack", [ruby, ruby.with_name("gem"), "build", f"{name}.gemspec", "--output", archive])
        run("install", [ruby, ruby.with_name("gem"), "install", "--local", "--no-document", archive], attempt)
        copy("main.rb", consumer / "main.rb")
        run("syntax", [ruby, "-c", "main.rb"], consumer)
        argv = [str(ruby), str(consumer / "main.rb")]
    elif language == "php":
        php = tools["php"]
        tool_root = REPO / "target/sdk-php-tools"
        env.update(COMPOSER_DISABLE_NETWORK="1", COMPOSER_HOME=str(attempt / "composer-home"), COMPOSER_CACHE_DIR=str(attempt / "composer-cache"))
        save(consumer / "composer.json", {"name":"demo/live", "require":{name:version}, "repositories":[{"type":"path","url":"../package","options":{"symlink":False}},{"packagist.org":False}],"config":{"allow-plugins":False}})
        copy("main.php", consumer / "main.php")
        run("install", [php, tool_root / "composer-2.10.3.phar", "update", "--no-dev", "--no-scripts", "--no-interaction"], consumer)
        run("typecheck", [php, tool_root / "phpstan-2.2.13.phar", "analyse", "--level=max", "--no-progress", "--memory-limit=1G", f"--autoload-file={consumer / 'vendor/autoload.php'}", "main.php"], consumer)
        argv = [str(php), str(consumer / "main.php")]
    elif language == "dart":
        dart = tools["dart"]
        put(consumer / "pubspec.yaml", f"name: live_consumer\npublish_to: none\nenvironment:\n  sdk: '>=3.9.0 <4.0.0'\ndependencies:\n  {name}:\n    path: ../package\n")
        copy("main.dart", consumer / "bin/main.dart")
        run("install", [dart, "pub", "get", "--offline"], consumer)
        run("analyze", [dart, "analyze", "--fatal-infos"], consumer)
        run("compile", [dart, "compile", "exe", "bin/main.dart", "-o", consumer / "demo"], consumer)
        argv = [str(consumer / "demo")]
    elif language == "cpp":
        cmake, cxx = tools["cmake"], tools["cxx"]
        run("configure", [cmake, "-S", package, "-B", attempt / "build", "-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_CXX_COMPILER={cxx}", f"-DCMAKE_INSTALL_PREFIX={attempt / 'install'}", "-DSUSPECT_SDK_BUILD_EXAMPLES=OFF", "-DSUSPECT_SDK_BUILD_DOCS=OFF"])
        run("sdk-build", [cmake, "--build", attempt / "build", "--parallel", "2"])
        run("install", [cmake, "--install", attempt / "build"])
        copy("main.cpp", consumer / "main.cpp")
        put(consumer / "CMakeLists.txt", f'cmake_minimum_required(VERSION 3.24)\nproject(Live LANGUAGES CXX)\nfind_package({name} {version} CONFIG REQUIRED)\nadd_executable(demo main.cpp)\ntarget_link_libraries(demo PRIVATE {name}::{name})\ntarget_compile_options(demo PRIVATE -Wall -Wextra -Wpedantic -Werror)\n')
        run("consumer-configure", [cmake, "-S", consumer, "-B", consumer / "build", f"-DCMAKE_PREFIX_PATH={attempt / 'install'}", f"-DCMAKE_CXX_COMPILER={cxx}", "-DCMAKE_BUILD_TYPE=Release"], consumer)
        run("compile", [cmake, "--build", consumer / "build", "--parallel", "2"], consumer)
        argv = [str(consumer / "build/demo")]
    else:
        raise RuntimeError(language)

    assert all(sha(package / path) == digest for path, digest in original.items())
    files = list((SOURCES / language).glob("*"))
    if bonus: files += list((SOURCES / "javascript").glob("*"))
    result = {"language":language,"packageConfig":config,"attempt":str(attempt),"package":str(package),"consumer":str(consumer),"argv":argv,"environment":env,"tools":prior["tools"],"snippets":{str(p.relative_to(REPO)):sha(p) for p in files if p.is_file()},"javascript":bonus,"liveExecuted":False}
    save(attempt / "ready.json", result)
    print(f"READY {language}: {attempt}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", choices=(*LANGUAGES,"all"))
    args = parser.parse_args()
    for language in LANGUAGES if args.language == "all" else (args.language,):
        build(language)
