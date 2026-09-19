#!/usr/bin/env python3
"""Check the new README's exact snippets against prepared packages, without calls."""
from __future__ import annotations
import json
from pathlib import Path
import re
import shutil

from support import REPO, ROOT, fresh, load_base, save, sha
from run import prepared

def main() -> None:
    work = fresh(ROOT / "checks", "readme")
    readme = REPO / "LIVE-ENV-DEMO-README.md"
    text = readme.read_text()
    links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", text)
    for link in links:
        if not re.match(r"^[a-z]+://", link) and not (REPO / link.split("#", 1)[0]).exists():
            raise RuntimeError(f"Broken ENV README link: {link}")
    load_base()
    from common import record
    mapping = {"ts":"typescript", "python":"python", "go":"go", "rust":"rust", "swift":"swift", "java":"java", "csharp":"csharp", "kotlin":"kotlin", "ruby":"ruby", "php":"php", "dart":"dart", "cpp":"cpp", "js":"javascript"}
    outcomes = []
    for fence, snippet in re.findall(r"```(\w+)\n(.*?)\n```", text, re.S):
        if fence not in mapping: continue
        language = mapping[fence]
        assert len(snippet.splitlines()) <= 4, language
        info = prepared(language)
        tools = {key:Path(value["path"]) for key, value in info["tools"].items()}
        consumer, attempt = Path(info["consumer"]), Path(info["attempt"])
        directory = work / language
        directory.mkdir()
        (directory / "excerpt.txt").write_text(snippet + "\n")
        env = {**info["environment"], "PYTHONDONTWRITEBYTECODE":"1"}
        def cmd(label, args, cwd=directory): return record(directory, label, args, cwd, env)
        def put(path, body):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body)
        if language in ("typescript", "javascript"):
            (directory / "node_modules").symlink_to(consumer / "node_modules", target_is_directory=True)
            save(directory / "package.json", {"private":True, "type":"module"})
            body = snippet.replace("import { createClient } from '@openrouter/sdk';\n", "")
            put(directory / "excerpt.ts", "import { createClient } from '@openrouter/sdk';\nexport async function example(): Promise<void> {\n" + body + "\n}\n")
            cmd("strict-typecheck", [tools["node"], Path(info["package"]) / "node_modules/typescript/bin/tsc", "--strict", "--exactOptionalPropertyTypes", "--noEmit", "--target", "ES2022", "--module", "NodeNext", "--moduleResolution", "NodeNext", "--lib", "ES2023,DOM", "excerpt.ts"])
            if language == "javascript":
                put(directory / "excerpt.mjs", snippet + "\n")
                cmd("javascript-syntax", [tools["node"], "--check", "excerpt.mjs"])
        elif language == "python":
            body = snippet.replace("from openrouter import Client\n", "")
            put(directory / "excerpt.py", "from openrouter import Client\ndef example() -> None:\n" + "\n".join("    " + line for line in body.splitlines()) + "\n")
            cmd("strict-typecheck", [tools["python"], "-m", "mypy", "--strict", "--no-incremental", "--python-executable", attempt / "venv/bin/python", "excerpt.py"])
        elif language == "go":
            put(directory / "go.mod", (consumer / "go.mod").read_text().replace("=> ../package", f"=> {info['package']}"))
            put(directory / "excerpt.go", 'package excerpt\nimport("context";"fmt";sdk "github.com/openrouter/sdk-go")\nfunc Example(ctx context.Context) error {\n' + snippet + '\nreturn nil\n}\n')
            cmd("compile", [tools["go"], "build", "./..."])
        elif language == "rust":
            libraries = sorted((attempt / "build/debug/deps").glob("libopenrouter-*.rlib"), key=lambda path:path.stat().st_mtime_ns, reverse=True)
            put(directory / "excerpt.rs", 'pub async fn example() -> Result<(), Box<dyn std::error::Error>> {\n' + snippet + '\nOk(())\n}\n')
            cmd("typecheck", [tools["rustc"], "--edition=2024", "--crate-type=lib", "--emit=metadata", "-L", f"dependency={attempt / 'build/debug/deps'}", "--extern", f"openrouter={libraries[0]}", "excerpt.rs", "-o", "excerpt.rmeta"])
        elif language == "swift":
            body = snippet.replace("import OpenRouter\n", "")
            put(directory / "Excerpt.swift", "import OpenRouter\nfunc example() async throws {\n" + body + "\n}\n")
            cmd("typecheck", [tools["swift"].with_name("swiftc"), "-typecheck", "-swift-version", "6", "-target", "arm64-apple-macosx13.0", "-I", attempt / "build/debug/Modules", "Excerpt.swift"])
        elif language == "java":
            jar = Path(info["argv"][2].split(":")[0])
            put(directory / "Excerpt.java", "import ai.openrouter.sdk.*;\npublic class Excerpt { public static void example() {\n" + snippet + "\n}}\n")
            cmd("strict-compile", [tools["javac"], "--release", "21", "-Xlint:all", "-Werror", "-cp", jar, "-d", directory, "Excerpt.java"])
        elif language == "csharp":
            put(directory / "Excerpt.cs", "using OpenRouter;\npublic static class Excerpt { public static async Task Example(CancellationToken cancellationToken) {\n" + snippet + "\n}}\n")
            put(directory / "Excerpt.csproj", f'<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><Reference Include="OpenRouter.SDK"><HintPath>{consumer / "bin/Release/net8.0/OpenRouter.SDK.dll"}</HintPath></Reference></ItemGroup></Project>')
            save(directory / "global.json", {"sdk":{"version":"8.0.424", "rollForward":"disable"}})
            put(directory / "NuGet.Config", '<configuration><packageSources><clear/></packageSources></configuration>')
            cmd("compile", [tools["dotnet"], "build", "Excerpt.csproj", "-m:1"])
        elif language == "kotlin":
            shutil.copy2(consumer / "pom.xml", directory / "pom.xml")
            put(directory / "src/main/kotlin/Excerpt.kt", "import ai.openrouter.kotlin.*\nsuspend fun example() {\n" + snippet + "\n}\n")
            cmd("compile", [tools["maven"], "-B", "-o", f"-Dmaven.repo.local={attempt / 'maven-repository'}", "compile"])
        elif language == "ruby":
            put(directory / "excerpt.rb", "require 'openrouter'\ndef example\n" + snippet + "\nend\n")
            cmd("syntax-and-import", [tools["ruby"], "-r", directory / "excerpt.rb", "-e", "abort unless defined?(OpenRouter::Client)"])
        elif language == "php":
            body = snippet.replace("use OpenRouter as Sdk;\n", "")
            put(directory / "excerpt.php", "<?php\ndeclare(strict_types=1);\nrequire " + repr(str(consumer / "vendor/autoload.php")) + ";\nuse OpenRouter as Sdk;\nfunction example(): void {\n" + body + "\n}\n")
            cmd("typecheck-max", [tools["php"], REPO / "target/sdk-php-tools/phpstan-2.2.13.phar", "analyse", "--level=max", "--no-progress", f"--autoload-file={consumer / 'vendor/autoload.php'}", "excerpt.php"])
        elif language == "dart":
            body = snippet.replace("import 'package:openrouter/openrouter_io.dart';\n", "")
            put(directory / "excerpt.dart", "import 'package:openrouter/openrouter_io.dart';\nFuture<void> example() async {\n" + body + "\n}\n")
            package_config = json.loads((consumer / ".dart_tool/package_config.json").read_text())
            for package in package_config["packages"]:
                if not package["rootUri"].startswith("file:"):
                    package["rootUri"] = (consumer / ".dart_tool" / package["rootUri"]).resolve().as_uri() + "/"
            save(directory / ".dart_tool/package_config.json", package_config)
            shutil.copy2(consumer / "pubspec.yaml", directory / "pubspec.yaml")
            cmd("analyze", [tools["dart"], "analyze", "--fatal-infos", "excerpt.dart"])
        elif language == "cpp":
            put(directory / "excerpt.cpp", '#include <openrouter/sdk.hpp>\n#include <iostream>\nusing namespace openrouter;\ntemplate<class E> int failed(E&) { return 1; }\nint example() {\n' + snippet + '\nreturn 0;\n}\n')
            cmd("strict-typecheck", [tools["cxx"], "-std=c++20", "-Wall", "-Wextra", "-Wpedantic", "-Werror", "-Dopenrouter_HAS_CURL=1", "-I", attempt / "install/include", "-fsyntax-only", "excerpt.cpp"])
        outcome = {"language":language, "excerptSha256":sha(directory / "excerpt.txt"), "lines":len(snippet.splitlines()), "status":"passed", "prepared":info["attempt"]}
        save(directory / "REPORT.json", outcome)
        outcomes.append(outcome)
    assert len(outcomes) == 13
    save(work / "REPORT.json", {"status":"passed", "readmeSha256":sha(readme), "linksChecked":len(links), "snippets":outcomes, "nativeConsumersExecuted":False, "realApiContacted":False})
    print(f"ENV README checked: {len(links)} local links, 13 exact four-line native snippets; no consumer/API execution")

if __name__ == "__main__": main()
