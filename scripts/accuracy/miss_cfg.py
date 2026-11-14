#!/usr/bin/env python3
"""How many ground-truth misses are kept alive only through code that rustc did not compile (cfg-gated callers)?

Usage: miss_cfg.py groundtruth.json --work /tmp/fg-acc [--arm-mode default]
For each rustc-dead fn that ferrograph does not report because of a `calls` edge from non-root code or from a
root item, checks whether any caller on the shown reachability chain sits under a `#[cfg(..)]` (same text heuristic as the
precision bucket). Such misses are an oracle artifact (rustc only sees one platform/feature set), not a resolution error.
"""

import argparse
import json
from collections import Counter
from pathlib import Path

import fgacc as A


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("json")
    ap.add_argument("--work", required=True)
    ap.add_argument("--mode", default="default")
    a = ap.parse_args()
    d = json.loads(Path(a.json).read_text())
    tot = Counter()
    for r in d["results"]:
        if not r["ok"]:
            continue
        root = Path(a.work) / "corpora" / r["name"]
        if not root.exists():
            continue
        c = Counter()
        for m in r["modes"][a.mode]["misses"]:
            if "calls edge" not in m["bucket"] and "reference" not in m["bucket"]:
                continue
            callers = [x.rsplit(" (", 1)[0] for x in m.get("chain", [])[1:]]
            gated = False
            for nid in callers:
                f, line, _ = A.split_id(nid)
                if A.cfg_gated(root, f, line):
                    gated = True
                    break
            c["via cfg-gated caller" if gated else "via ungated caller"] += 1
        print(f"{r['name']}: {dict(c)}")
        tot.update(c)
    print(f"total: {dict(tot)}")


if __name__ == "__main__":
    main()
