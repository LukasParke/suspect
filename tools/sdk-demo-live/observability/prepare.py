#!/usr/bin/env python3
"""Compile only the two additive consumer corrections against installed SDKs."""
from __future__ import annotations
import difflib
import json
from pathlib import Path
import shutil

from support import BASE, REPO, ROOT, SOURCES, metadata, record, save, sha

def main() -> None:
    ROOT.mkdir(exist_ok=False)
    protected={}
    for name,digest in json.loads((BASE/"delivery-02/SHA256SUMS.json").read_text()).items():
        info=metadata(Path(name))
        if info["sha256"]!=digest:raise RuntimeError(f"Prior live seal differs: {name}")
        protected[name]=info
    for name,info in json.loads((BASE/"preservation-before.json").read_text()).items():
        if metadata(Path(name))!=info:raise RuntimeError(f"Prior preserved path differs: {name}")
        protected[name]=info
    for name,digest in json.loads((BASE/"package-pins.json").read_text()).items():
        path=BASE/"packages"/name
        info=metadata(path)
        if info["sha256"]!=digest:raise RuntimeError(f"Prior SDK differs: {path}")
        protected[str(path)]=info
    for path in (BASE/"delivery-02").iterdir():
        if path.is_file():protected[str(path)]=metadata(path)
    save(ROOT/"preservation-before.json",protected)
    plans={}
    for language,filename in (("typescript","main.ts"),("python","main.py")):
        prior=json.loads((BASE/f"native/{language}-01/ready.json").read_text())
        directory=ROOT/language;directory.mkdir()
        old=REPO/f"examples/sdk-demo-live/{language}/{filename}"
        new=SOURCES/language/filename
        shutil.copy2(old,directory/f"before-{filename}")
        shutil.copy2(new,directory/filename)
        (directory/"consumer.diff").write_text("".join(difflib.unified_diff(old.read_text().splitlines(True),new.read_text().splitlines(True),fromfile=str(old),tofile=str(new))))
        if language=="typescript":
            (directory/"node_modules").symlink_to(Path(prior["consumer"])/"node_modules",target_is_directory=True)
            save(directory/"package.json",{"private":True,"type":"module"})
            node=Path(prior["tools"]["node"]["path"])
            compiler=Path(prior["package"])/"node_modules/typescript/bin/tsc"
            record(directory,"strict-compile",[node,compiler,"--strict","--exactOptionalPropertyTypes","--target","ES2022","--module","NodeNext","--moduleResolution","NodeNext","--lib","ES2023,DOM","--outDir","build",filename],directory)
            argv=[str(node),str(directory/"build/main.js")]
        else:
            python=Path(prior["argv"][0]); checker=Path(prior["tools"]["python"]["path"])
            record(directory,"strict-types",[checker,"-m","mypy","--strict","--no-incremental","--python-executable",python,filename],directory)
            argv=[str(python),str(directory/filename)]
        plans[language]={"priorPrepared":str(BASE/f"native/{language}-01/ready.json"),"argv":argv,"consumer":str(directory),"source":str(new),"sourceSha256":sha(new),"executionFile":argv[-1],"executionSha256":sha(Path(argv[-1])),"packageReused":prior["packageConfig"],"liveExecuted":False}
    save(ROOT/"prepared.json",plans)
    print(f"Prepared two additive consumers; existing SDK installs reused: {ROOT}")

if __name__=="__main__":main()
