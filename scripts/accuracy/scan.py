#!/usr/bin/env python3
"""Scan candidate repos for rustc dead-code ground truth (used to pick corpora; see README.md).

For each `owner/repo` (or url) shallow-clone HEAD under --work, run the same rustc oracle as groundtruth.py and print
the pinned SHA, the number of fns rustc reports dead in every unit, and how many of those sit in bin-only files.
Output is TSV on stdout, one line per repo, appended to --out as it goes.
"""

import argparse
import os
import shutil
import sys
from pathlib import Path

import fgacc as A
from groundtruth import DEFAULT_CARGO_ARGS


def scan(repo, work, timeout):
    url = repo if "://" in repo else f"https://github.com/{repo}"
    name = repo.rstrip("/").split("/")[-1]
    c = {"name": name, "url": url, "ref": "HEAD"}
    root, sha, _ = A.prepare_corpus(c, work)
    tgt = Path(work) / "target" / name
    try:
        msgs, cmd, note, err = A.run_rustc(root, DEFAULT_CARGO_ARGS, tgt, timeout=timeout)
        if msgs is not None:
            items, units, fclass = A.collect_ground_truth(msgs, root, root)
    finally:
        shutil.rmtree(Path(tgt).resolve(), ignore_errors=True)  # targets are GBs; the disk fills up otherwise
    if msgs is None:
        return [name, sha, "BUILD-FAIL", err.strip().splitlines()[-1][:80] if err.strip() else ""]
    dead = A.all_unit_dead(items, units)
    nbin = sum(1 for k in dead if fclass.get(k[0]) == "bin")
    some = len(items) - len(dead)
    return [name, sha, f"dead={len(dead)}", f"bin={nbin}", f"some-unit-only={some}", note or "ok"]


def main():
    # a repo's rust-toolchain file would make rustup download that toolchain (GBs); scan with the active one
    os.environ.setdefault("RUSTUP_TOOLCHAIN", "stable")
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("repos", nargs="+")
    ap.add_argument("--work", required=True)
    ap.add_argument("--timeout", type=int, default=1500)
    ap.add_argument("--out", default="scan.tsv")
    a = ap.parse_args()
    for r in a.repos:
        try:
            row = scan(r, a.work, a.timeout)
        except Exception as e:  # keep scanning
            row = [r, "?", "ERROR", str(e)[-100:].replace("\n", " ")]
        line = "\t".join(map(str, row))
        print(line, flush=True)
        with open(a.out, "a") as f:
            f.write(line + "\n")


if __name__ == "__main__":
    sys.exit(main())
