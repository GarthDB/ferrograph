#!/usr/bin/env python3
"""Run groundtruth.py for every corpus under several `--extra-args` arms and merge the JSON per arm (see README.md).

Corpora are processed one at a time and their cargo target dir is deleted afterwards (`--keep-targets` to keep),
because 20+ workspaces need tens of GB. Output: <out>/<arm>.json (groundtruth-data.json format, all corpora)
and <out>/<arm>.md (the usual report). Re-running skips corpus/arm pairs already measured with the same binary (sha256) and corpus ref; pairs that failed or
are missing are listed on stderr and make the exit status 1.
"""

import argparse
import hashlib
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
    # parts are keyed by the binary and the corpus pin, so results of another build or ref are never reused
    bin_key = hashlib.sha256(Path(a.ferrograph).read_bytes()).hexdigest()[:8]

    def part_of(c, arm):
        key = f"{bin_key}-{(c.get('ref') or 'head')[:8]}"
        return out / "parts" / f"{c['name']}.{arm}.{key}.json"

    for c in corpora:
        for arm in arms:
            part = part_of(c, arm)
            if part.exists() and all(r["ok"] for r in json.loads(part.read_text())["results"]):
                continue
            cmd = [sys.executable, str(HERE / "groundtruth.py"), "--config", a.config, "--ferrograph", a.ferrograph,
                   "--work", str(work), "--only", c["name"], f"--extra-args={ARMS[arm]}",
                   "--out-md", str(part.with_suffix(".md")), "--out-json", str(part)]
            subprocess.run(cmd, check=False, stdout=subprocess.DEVNULL)
        if not a.keep_targets:
            shutil.rmtree(work / "target" / c.get("group", c["name"]), ignore_errors=True)
    missing = []
    for arm in arms:
        results, meta = [], None
        for c in corpora:
            p = part_of(c, arm)
            if not p.exists():
                missing.append(f"{c['name']}/{arm}")
                continue
            d = json.loads(p.read_text())
            meta = meta or d["meta"]
            results += d["results"]
            missing += [f"{r['name']}/{arm}: {r.get('error', '')[:80]}" for r in d["results"] if not r["ok"]]
        (out / f"{arm}.json").write_text(json.dumps({"meta": meta, "results": results}, indent=1))
        subprocess.run([sys.executable, str(HERE / "groundtruth.py"), "--ferrograph", a.ferrograph, "--render-json", str(out / f"{arm}.json"),
                        "--out-md", str(out / f"{arm}.md")], check=False, stdout=subprocess.DEVNULL)
    if missing:  # compare.py intersects corpora by name, so say what is absent from the merged files
        print("NOT MEASURED (absent from the merged results):\n  " + "\n  ".join(missing), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
