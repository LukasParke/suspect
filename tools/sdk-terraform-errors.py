#!/usr/bin/env python3
"""Focused SDK API-error ownership proof using exact pinned emitted SDK ZIP bytes."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import traceback

spec=importlib.util.spec_from_file_location("terraform_acceptance",Path(__file__).with_name("sdk-terraform-acceptance.py"))
helpers=importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)

def main():
    parser=argparse.ArgumentParser();parser.add_argument("--root",type=Path,required=True);parser.add_argument("--evidence",type=Path,required=True)
    args=parser.parse_args();root=args.root.resolve();evidence=args.evidence.resolve();run=helpers.Runner(evidence)
    report={"format":"suspect.terraform.error-ownership.v1","passed":False,"native_root":str(root),"scope":"real emitted SDK/Framework; exact SDK ZIP/h1, no replacement; no publishing or full-matrix replay"}
    try:
        original=helpers.tree_hashes(root/"generated")
        helpers.save_json(evidence/"source-hashes.json",original)
        config=json.loads((root/"generated/terraform/source-bindings.json").read_text())["configuration"]
        proxy,sdk_info=helpers.prepare_proxy(root,root/"generated/go",config);helpers.save_json(evidence/"sdk-proxy.json",sdk_info)
        shared=helpers.APPROVED/"sdk-terraform-dependency-cache-20260910-01"
        cache=root/"module-cache";cache.mkdir();download=cache/"cache/download";download.mkdir(parents=True)
        private=config["sdk"]["module_path"].split("/")[0]
        for child in shared.iterdir():
            if child.name not in ("cache",private):(cache/child.name).symlink_to(child,target_is_directory=child.is_dir())
        for child in (shared/"cache/download").iterdir():
            if child.name!=private:(download/child.name).symlink_to(child,target_is_directory=child.is_dir())
        tools={"go1.23.12":str(shared/"golang.org/toolchain@v0.0.1-go1.23.12.darwin-arm64/bin/go"),"go1.27.1":"go"}
        env={k:v for k,v in os.environ.items() if not k.startswith(("GO","TF_"))}
        env.update(GOTOOLCHAIN="local",GOWORK="off",GOPROXY=f"file://{proxy},off",GOSUMDB="off",GOMODCACHE=str(cache),GOCACHE=str(helpers.APPROVED/"sdk-terraform-build-cache-20260910-01"))
        for tier,go in tools.items():
            info=json.loads(run.run(f"{tier}-tools",[go,"env","-json","GOVERSION","GOROOT"],root/"generated/terraform",env))
            run.claim(f"{tier}-version",info["GOVERSION"]==tier,info)
            helpers.save_json(evidence/f"{tier}-tool.json",{"environment":info,"go_sha256":helpers.sha(Path(info["GOROOT"])/"bin/go")})
            run.run(f"{tier}-errors",[go,"test","-mod=readonly","-race","-count=1","-v","-run","TestAllDeferred|TestBufferedMissing|TestTransportCause|TestCancellationAlso","./provider"],root/"generated/terraform",env)
            modules=list(helpers.json_stream(run.run(f"{tier}-sdk-linkage",[go,"list","-m","-json","all"],root/"generated/terraform",env)))
            sdk=next(module for module in modules if module["Path"]==sdk_info["module"])
            run.claim(f"{tier}-exact-sdk",sdk["Version"]==sdk_info["version"] and sdk["Sum"]==sdk_info["sum"] and "Replace" not in sdk,sdk)
        run.claim("emitted-source-bytes-unchanged",helpers.tree_hashes(root/"generated")==original)
        report["passed"]=True
    except Exception:
        report["failure"]=traceback.format_exc();print(report["failure"])
    finally:
        report["commands"]=run.commands;report["gates"]=run.claims;helpers.save_json(evidence/"report.json",report)
    return 0 if report["passed"] else 1

if __name__=="__main__":raise SystemExit(main())
