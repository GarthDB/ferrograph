#!/usr/bin/env python3
"""Bucket the `pr` impacted set by depth and confidence, and score hand labels.

  impact_sample.py bucket DIR PR [PR..]   # DIR holds pr<N>-d{1,2,3}[-x].json -> CSV on stdout
  impact_sample.py score LABELS.csv       # precision by depth, default vs --exclude-low-confidence

Depth = first --impact-depth run the node appears in. A node missing from the
`-x` run at its depth is reachable only through low-confidence edges.
"""
import csv, json, sys
from collections import defaultdict
import fgacc


def ids(path):
    return {n["id"]: n for n in json.load(open(path))["impacted"]["nodes"]}


def bucket(d, prs):
    w = csv.writer(sys.stdout)
    w.writerow(["pr", "id", "name", "group", "depth", "confident", "label", "note"])
    for pr in prs:
        seen = set()
        for depth in (1, 2, 3):
            cur = ids(f"{d}/pr{pr}-d{depth}.json")
            conf = ids(f"{d}/pr{pr}-d{depth}-x.json")
            for i, n in sorted(cur.items()):
                if i not in seen:
                    w.writerow([pr, i, n["name"], n["group"], depth, int(i in conf), "", ""])
            seen |= set(cur)


def score(path):
    rows = list(csv.DictReader(open(path)))
    bad = [r["id"] for r in rows if r["label"] not in ("TP", "FP")]
    assert not bad, f"unlabelled: {bad[:5]}"
    for title, keep in (("default", lambda r: True), ("--exclude-low-confidence", lambda r: r["confident"] == "1")):
        print(title)
        for lim in (1, 2, 3):
            sel = [r for r in rows if int(r["depth"]) <= lim and keep(r)]
            tp = sum(r["label"] == "TP" for r in sel)
            print(f"  depth<={lim}: precision {fgacc.frac_ci(tp, len(sel))}")
        for dep in (2, 3):
            sel = [r for r in rows if int(r["depth"]) == dep and keep(r)]
            tp = sum(r["label"] == "TP" for r in sel)
            print(f"  depth=={dep} only: precision {fgacc.frac_ci(tp, len(sel))}")


if __name__ == "__main__":
    if sys.argv[1] == "bucket":
        bucket(sys.argv[2], sys.argv[3:])
    else:
        score(sys.argv[2])
