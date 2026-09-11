#!/usr/bin/env python3
"""Actual checksummed SDK module lookup: valid layouts plus an ambiguity oracle."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import traceback

spec=importlib.util.spec_from_file_location("terraform_acceptance",Path(__file__).with_name("sdk-terraform-acceptance.py"));helpers=importlib.util.module_from_spec(spec);spec.loader.exec_module(helpers)

def main():
    p=argparse.ArgumentParser();p.add_argument("--root",type=Path,required=True);p.add_argument("--evidence",type=Path,required=True);args=p.parse_args()
    root=args.root.resolve();evidence=args.evidence.resolve();run=helpers.Runner(evidence)
    report={"format":"suspect.terraform.module-boundaries.v1","passed":False,"native_root":str(root),"scope":"native package lookup through pinned generated SDK ZIPs; no go.mod edits, SDK replacements, publishing or API requests"}
    try:
        shared=helpers.APPROVED/"sdk-terraform-dependency-cache-20260910-01"
        for label in ("separate","sibling","nested","collision"):
            directory=root/label;package=directory/"generated/terraform"
            config=json.loads((directory/"config.json").read_text()) if label=="collision" else json.loads((package/"source-bindings.json").read_text())["configuration"]
            proxy,info=helpers.prepare_proxy(directory,directory/"generated/go",config)
            # The independent negative fixture starts with the same exact SDK
            # version + native h1 inputs as the positive generated packages.
            if label=="collision":
                (package/"go.sum").write_text(f"{info['module']} {info['version']} {info['sum']}\n{info['module']} {info['version']}/go.mod {info['go_mod_sum']}\n")
            original=helpers.tree_hashes(directory/"generated");helpers.save_json(evidence/f"{label}-sources.json",original)
            helpers.save_json(evidence/f"{label}-sdk.json",info)
            cache=directory/"module-cache";cache.mkdir();download=cache/"cache/download";download.mkdir(parents=True)
            private=config["sdk"]["module_path"].split("/")[0]
            for child in shared.iterdir():
                if child.name not in ("cache",private): (cache/child.name).symlink_to(child,target_is_directory=child.is_dir())
            for child in (shared/"cache/download").iterdir():
                if child.name!=private: (download/child.name).symlink_to(child,target_is_directory=child.is_dir())
            env={k:v for k,v in os.environ.items() if not k.startswith(("GO","TF_"))}
            env.update(GOWORK="off",GOTOOLCHAIN="local",GOPROXY=f"file://{proxy},off",GOSUMDB="off",GOMODCACHE=str(cache),GOCACHE=str(helpers.APPROVED/"sdk-terraform-build-cache-20260910-01"))
            tools={"go1.23.12":str(shared/"golang.org/toolchain@v0.0.1-go1.23.12.darwin-arm64/bin/go"),"go1.27.1":"go"}
            for tier,go in tools.items():
                version=run.run(f"{label}-{tier}-version",[go,"version"],package,env);run.claim(f"{label}-{tier}-pin",tier.encode() in version)
                run.run(f"{label}-{tier}-lookup",[go,"list","-mod=readonly","./..."],package,env,expected=(1,) if label=="collision" else (0,))
                if label=="collision":
                    text=(evidence/run.commands[-1]["stderr"]).read_text();run.claim(f"{label}-{tier}-native-ambiguity","ambiguous import" in text,text)
                else:
                    modules=list(helpers.json_stream(run.run(f"{label}-{tier}-linkage",[go,"list","-m","-json","all"],package,env)))
                    sdk=next(m for m in modules if m["Path"]==config["sdk"]["module_path"])
                    run.claim(f"{label}-{tier}-exact-sdk",sdk["Version"]==info["version"] and sdk["Sum"]==info["sum"] and "Replace" not in sdk,sdk)
                    run.run(f"{label}-{tier}-compile",[go,"test","-mod=readonly","./..."],package,env)
            run.claim(f"{label}-source-bytes",helpers.tree_hashes(directory/"generated")==original)
        report["passed"]=True
    except Exception:
        report["failure"]=traceback.format_exc();print(report["failure"])
    finally:
        report["commands"]=run.commands;report["gates"]=run.claims;helpers.save_json(evidence/"report.json",report)
    return 0 if report["passed"] else 1

if __name__=="__main__":raise SystemExit(main())
