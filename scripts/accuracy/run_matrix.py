#!/usr/bin/env python3
"""Run groundtruth.py for every corpus under several `--extra-args` arms and merge the JSON per arm (see README.md).

Corpora are processed one at a time and their cargo target dir is deleted afterwards (`--keep-targets` to keep),
because 20+ workspaces need tens of GB. Output: <out>/<arm>.json (groundtruth-data.json format, all corpora)
and <out>/<arm>.md (the usual report). Re-running skips corpus/arm pairs whose JSON already exists.
"""

import argparse
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ARMS = {
    "default": "",
    "restricted": "--restricted-vis-not-root",
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ferrograph", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--config", default=str(HERE / "corpora.toml"))
    ap.add_argument("--only", default="")
    ap.add_argument("--arms", default="default,restricted", help=f"comma-separated, from {','.join(ARMS)}")
    ap.add_argument("--keep-targets", action="store_true")
    a = ap.parse_args()
    out, work = Path(a.out).resolve(), Path(a.work).resolve()
    (out / "parts").mkdir(parents=True, exist_ok=True)
    only = {s for s in a.only.split(",") if s}
    corpora = [c for c in tomllib.loads(Path(a.config).read_text())["corpus"] if c.get("url") and (not only or c["name"] in only)]
    arms = a.arms.split(",")
    for c in corpora:
        for arm in arms:
            part = out / "parts" / f"{c['name']}.{arm}.json"
            if part.exists():
                continue
            cmd = [sys.executable, str(HERE / "groundtruth.py"), "--config", a.config, "--ferrograph", a.ferrograph,
                   "--work", str(work), "--only", c["name"], f"--extra-args={ARMS[arm]}",
                   "--out-md", str(out / "parts" / f"{c['name']}.{arm}.md"), "--out-json", str(part)]
            subprocess.run(cmd, check=False, stdout=subprocess.DEVNULL)
        if not a.keep_targets:
            shutil.rmtree(work / "target" / c.get("group", c["name"]), ignore_errors=True)
    for arm in arms:
        results, meta = [], None
        for c in corpora:
            p = out / "parts" / f"{c['name']}.{arm}.json"
            if p.exists():
                d = json.loads(p.read_text())
                meta = meta or d["meta"]
                results += d["results"]
        (out / f"{arm}.json").write_text(json.dumps({"meta": meta, "results": results}, indent=1))
        subprocess.run([sys.executable, str(HERE / "groundtruth.py"), "--ferrograph", a.ferrograph, "--render-json", str(out / f"{arm}.json"),
                        "--out-md", str(out / f"{arm}.md")], check=False, stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    sys.exit(main())
