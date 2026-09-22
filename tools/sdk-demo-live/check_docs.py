#!/usr/bin/env python3
"""Compile the exact published live DX snippets without making API requests."""
from __future__ import annotations
import json
from pathlib import Path
import re
import shutil

from common import REPO, ROOT, fresh, record, save, sha
from run import prepared

def main() -> None:
    work = fresh(ROOT / "checks", "readme")
    text = (REPO / "LIVE-DEMO-README.md").read_text()
    links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", text)
    for link in links:
        if not re.match(r"^[a-z]+://",link) and not (REPO / link.split("#",1)[0]).exists():
            raise RuntimeError(f"Broken live README link: {link}")
    mapping = {"ts":"typescript","python":"python","go":"go","rust":"rust","swift":"swift","java":"java","csharp":"csharp","kotlin":"kotlin","ruby":"ruby","php":"php","dart":"dart","cpp":"cpp","js":"javascript"}
    outcomes = []
    for fence, snippet in re.findall(r"```(\w+)\n(.*?)\n```",text,re.S):
        if fence not in mapping: continue
        language = mapping[fence]
        info = prepared(language)
        tools = {key:Path(value["path"]) for key,value in info["tools"].items()}
        consumer = Path(info["consumer"])
        attempt = Path(info["attempt"])
        directory = work / language; directory.mkdir()
        (directory / "excerpt.txt").write_text(snippet+"\n")
        env = dict(info["environment"])
        def cmd(label,args,cwd=directory): return record(directory,label,args,cwd,env)
        def put(path,body): path.parent.mkdir(parents=True,exist_ok=True);path.write_text(body)
        if language in ("typescript","javascript"):
            path = consumer / f"readme-{work.name}.ts"
            body = snippet.replace("import { createClient } from '@openrouter/sdk';","")
            put(path,"import { createClient } from '@openrouter/sdk';\nexport async function example(token: string): Promise<void> {\n"+body+"\n}\n")
            cmd("typecheck",[tools["node"],Path(info["package"])/"node_modules/typescript/bin/tsc","--strict","--exactOptionalPropertyTypes","--noEmit","--target","ES2022","--module","NodeNext","--moduleResolution","NodeNext","--lib","ES2023,DOM",path],consumer)
        elif language == "python":
            path = directory / "excerpt.py"
            body=snippet.replace("from openrouter import Client\n","")
            put(path,"from openrouter import Client\ndef example(token: str) -> None:\n"+"\n".join("    "+line for line in body.splitlines())+"\n")
            cmd("typecheck",[tools["python"],"-m","mypy","--strict","--no-incremental","--python-executable",attempt/"venv/bin/python",path])
        elif language == "go":
            shutil.copy2(consumer/"go.mod",directory/"go.mod")
            config=(directory/"go.mod").read_text().replace("=> ../package",f"=> {info['package']}")
            put(directory/"go.mod",config)
            put(directory/"excerpt.go",'package excerpt\nimport("context";"fmt";sdk "github.com/openrouter/sdk-go")\nfunc Example(token string, ctx context.Context) error {\n'+snippet+'\nreturn nil\n}\n')
            cmd("compile",[tools["go"],"build","./..."])
        elif language == "rust":
            libraries=sorted((attempt/"build/debug/deps").glob("libopenrouter-*.rlib"),key=lambda p:p.stat().st_mtime_ns,reverse=True)
            put(directory/"excerpt.rs",'pub async fn example(token: String) -> Result<(), Box<dyn std::error::Error>> {\n'+snippet+'\nOk(())\n}\n')
            cmd("compile",[tools["rustc"],"--edition=2024","--crate-type=lib","--emit=metadata","-L",f"dependency={attempt/'build/debug/deps'}","--extern",f"openrouter={libraries[0]}",directory/"excerpt.rs","-o",directory/"excerpt.rmeta"])
        elif language == "swift":
            body=snippet.replace("import OpenRouter\n","")
            put(directory/"Excerpt.swift","import OpenRouter\nfunc example(token: String) async throws {\n"+body+"\n}\n")
            cmd("typecheck",[tools["swift"].with_name("swiftc"),"-typecheck","-swift-version","6","-target","arm64-apple-macosx13.0","-I",attempt/"build/debug/Modules",directory/"Excerpt.swift"])
        elif language == "java":
            jar = Path(info["argv"][2].split(":")[0])
            put(directory/"Excerpt.java","import ai.openrouter.sdk.*;\npublic class Excerpt { public static void example(String token) {\n"+snippet+"\n}}\n")
            cmd("compile",[tools["javac"],"--release","21","-Xlint:all","-Werror","-cp",jar,"-d",directory,"Excerpt.java"])
        elif language == "csharp":
            body=snippet.replace("using OpenRouter;\n","")
            put(directory/"Excerpt.cs","using OpenRouter;\npublic static class Excerpt { public static async Task Example(string token, CancellationToken cancellationToken) {\n"+body+"\n}}\n")
            put(directory/"Excerpt.csproj",f'<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings></PropertyGroup><ItemGroup><Reference Include="OpenRouter.SDK"><HintPath>{consumer / "bin/Release/net8.0/OpenRouter.SDK.dll"}</HintPath></Reference></ItemGroup></Project>')
            save(directory/"global.json",{"sdk":{"version":"8.0.424","rollForward":"disable"}})
            put(directory/"NuGet.Config",'<configuration><packageSources><clear/></packageSources></configuration>')
            cmd("compile",[tools["dotnet"],"build","Excerpt.csproj","-m:1"])
        elif language == "kotlin":
            shutil.copy2(consumer/"pom.xml",directory/"pom.xml")
            put(directory/"src/main/kotlin/Excerpt.kt","import ai.openrouter.kotlin.*\nsuspend fun example(token: String) {\n"+snippet+"\n}\n")
            cmd("compile",[tools["maven"],"-B","-o",f"-Dmaven.repo.local={attempt/'maven-repository'}","compile"])
        elif language == "ruby":
            put(directory/"excerpt.rb","require 'openrouter'\ndef example(token)\n"+snippet+"\nend\n")
            cmd("syntax-and-import",[tools["ruby"],"-r",directory/"excerpt.rb","-e","abort unless defined?(OpenRouter::Client)"])
        elif language == "php":
            body=snippet.replace("use OpenRouter as Sdk;\n","")
            put(directory/"excerpt.php","<?php\ndeclare(strict_types=1);\nrequire "+repr(str(consumer/"vendor/autoload.php"))+";\nuse OpenRouter as Sdk;\nfunction example(string $token): void {\n"+body+"\n}\n")
            cmd("typecheck",[tools["php"],REPO/"target/sdk-php-tools/phpstan-2.2.13.phar","analyse","--level=max","--no-progress",f"--autoload-file={consumer/'vendor/autoload.php'}",directory/"excerpt.php"])
        elif language == "dart":
            body=snippet.replace("import 'package:openrouter/openrouter_io.dart';\n","")
            path=consumer/f"excerpt_{work.name.replace('-','_')}.dart"
            put(path,"import 'package:openrouter/openrouter_io.dart';\nFuture<void> example(String token) async {\n"+body+"\n}\n")
            cmd("analyze",[tools["dart"],"analyze","--fatal-infos",path],consumer)
        elif language == "cpp":
            put(directory/"excerpt.cpp",'#include <openrouter/sdk.hpp>\nusing namespace openrouter;\ntemplate<class E> int failed(E&) { return 1; }\nint example(Credentials credentials) {\n'+snippet+'\n(void)response; return 0;\n}\n')
            cmd("compile",[tools["cxx"],"-std=c++20","-Wall","-Wextra","-Wpedantic","-Werror","-Dopenrouter_HAS_CURL=1","-I",attempt/"install/include","-fsyntax-only",directory/"excerpt.cpp"])
        outcomes.append({"language":language,"excerptSha256":sha(directory/"excerpt.txt"),"status":"passed"})
    save(work/"REPORT.json",{"readmeSha256":sha(REPO/"LIVE-DEMO-README.md"),"linksChecked":len(links),"snippets":outcomes,"realApiContacted":False})
    print(f"Live README checked: {len(links)} local links and {len(outcomes)} native snippets; no API request.")

if __name__ == "__main__": main()
