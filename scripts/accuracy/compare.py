#!/usr/bin/env python3
"""Paired comparison of two groundtruth.py runs on the same corpora (e.g. default vs --restricted-vis-not-root, see README.md).

Usage: compare.py base.json variant.json [--mode default] [--out-md compare.md]
Per cohort: recall (all R) of each arm with Wilson intervals, items found by only one arm (exact McNemar p),
extra reports of the variant split by rustc's verdict, and the sample size needed to detect a 5-point effect.
"""

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

import fgacc as A


def load(path):
    return {r["name"]: r for r in json.loads(Path(path).read_text())["results"] if r["ok"]}


def verdict(r, mode, nid):
    fp = r["modes"][mode]["fp"]
    for k in ("not_judgeable_file", "not_judgeable_cfg", "rustc_live_some_unit", "disagree"):
        if any(x["id"] == nid for x in fp[k]):
            return k
    return "confirmed" if nid in r["classes"] else "other"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("base")
    ap.add_argument("variant")
    ap.add_argument("--mode", default="default")
    ap.add_argument("--out-md", default="compare.md")
    a = ap.parse_args()
    base, var = load(a.base), load(a.variant)
    cohorts = defaultdict(list)
    for name in base:
        if name in var:
            r = base[name]
            cohorts[r.get("cohort", "held-out" if r.get("heldout") else "original")].append(name)
    L = []
    w = L.append
    w(f"# paired comparison, mode `{a.mode}`\n")
    w("| cohort | R | recall base | recall variant | only base | only variant | McNemar p | extra reports | of which confirmed | cfg/unjudgeable | live in some unit | genuine disagreement |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|")
    tot = defaultdict(int)
    for co, names in list(cohorts.items()) + [("**all**", [n for v in cohorts.values() for n in v])]:
        R = hb = hv = ob = ov = 0
        extra = defaultdict(int)
        for n in names:
            rb, rv = base[n], var[n]
            Rset = set(rb["classes"])
            Fb, Fv = set(rb["modes"][a.mode]["F"]), set(rv["modes"][a.mode]["F"])
            R += len(Rset)
            hb += len(Fb & Rset)
            hv += len(Fv & Rset)
            ob += len((Fb - Fv) & Rset)
            ov += len((Fv - Fb) & Rset)
            for nid in Fv - Fb:
                if nid not in Rset:
                    extra[verdict(rv, a.mode, nid)] += 1
        ne = sum(extra.values()) + ov
        w(
            f"| {co} | {R} | {A.frac_ci(hb, R)} | {A.frac_ci(hv, R)} | {ob} | {ov} | {A.mcnemar_p(ob, ov):.3f} | {ne} | {ov} | "
            f"{extra['not_judgeable_file'] + extra['not_judgeable_cfg']} | {extra['rustc_live_some_unit']} | {extra['disagree'] + extra['other']} |"
        )
        if co == "**all**":
            tot.update(R=R, hb=hb, ob=ob, ov=ov)
    R = tot["R"]
    p = tot["hb"] / R if R else 0.0
    q = (tot["ob"] + tot["ov"]) / R if R else 0.0
    w(
        f"\nSample size for a 5-point recall effect (two-sided 5%, 80% power; base recall {100 * p:.0f}% over {R} R items): "
        f"**{A.n_unpaired(max(p, 0.05), 0.05)} R items per arm** if the arms were independent samples; "
        f"**{A.n_paired(q, 0.05) or 'n/a (discordance rate below the effect)'} R items** for this paired design at the observed discordance of {100 * q:.1f}% of items "
        f"(this is the number of ground-truth items, not corpora). "
        f"Measured: {R} R items, so the 95% interval of one arm's recall is ±{100 * (A.wilson(tot['hb'], R)[1] - A.wilson(tot['hb'], R)[0]) / 2:.0f} points."
    )
    Path(a.out_md).write_text("\n".join(L))
    print("\n".join(L))


if __name__ == "__main__":
    sys.exit(main())
