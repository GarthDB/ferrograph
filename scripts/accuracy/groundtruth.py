#!/usr/bin/env python3
"""Part A: measure `ferrograph dead` against rustc's `dead_code` lint (see README.md)."""

import argparse
import json
import platform
import shlex
import sys
import tempfile
import time
import tomllib
from collections import Counter
from pathlib import Path

import fgacc as A

DEFAULT_CARGO_ARGS = ["--workspace", "--all-targets", "--all-features"]
MODES = ("default", "strict")


def bucket(cause):
    """Map a `reach` cause to a report bucket."""
    if cause is None:
        return "other"
    if cause.startswith("root:"):
        k = cause[5:]
        if k == "pub":
            return "pub root"
        return "trait-impl root" if k in ("trait_impl", "trait_def_method") else "test/entry/export attribute root"
    edge, _, src = cause.partition("@")
    if edge == "calls(low-confidence)":
        return "low-confidence call edge"
    where = "root item" if src != "private" else "non-root code"
    if edge == "expands_to":
        return "macro/attribute edge (expands_to)"
    return f"{edge} edge from {where}"


def miss_bucket(g, info, nid, index_root):
    """`bucket` plus, for pub roots, the source-level visibility of the root that kept `nid` alive."""
    bkt = bucket(info[nid][3])
    if bkt == "pub root":
        origin = nid
        while info[origin][0] is not None and origin not in g["roots"]:
            origin = info[origin][0]
        bkt += ": " + A.visibility(index_root, origin)
    return bkt


def analyse(c, fg, work, args):
    res = {"name": c["name"], "ok": False, "heldout": bool(c.get("heldout")), "cohort": A.cohort(c)}
    root, sha, dirty = A.prepare_corpus(c, work)
    res.update(
        sha=sha, dirty=dirty, url=c.get("url"), path=c.get("path"), ref=c.get("ref")
    )
    index_root = (root / c.get("index_path", ".")).resolve()
    cargo_args = c.get("cargo_args", DEFAULT_CARGO_ARGS)
    tgt = Path(work) / "target" / c.get("group", c["name"])
    t0 = time.time()
    msgs, cmd, fallback, err = A.run_rustc(root, cargo_args, tgt, timeout=args.timeout)
    res.update(
        command=cmd,
        fallback=fallback,
        cargo_seconds=round(time.time() - t0, 1),
    )
    if msgs is None:
        res["error"] = "cargo check failed: " + err
        return res
    items, units, fclass = A.collect_ground_truth(msgs, root, index_root)
    db = Path(work) / f"{c['name']}.db"
    t0 = time.time()
    fg.index(index_root, db)
    res["index_seconds"] = round(time.time() - t0, 1)
    g = A.prepare_model(fg.load_graph(db), fg.restricted, fg.bin_pub)
    by_file = A.build_index(g)
    res["fn_nodes"] = sum(1 for t, _ in g["nodes"].values() if t == "function")

    # ground truth: dead in every unit that compiles the file
    R, partial, unmapped = {}, [], []
    for (f, line, name), it in sorted(items.items()):
        if units.get(f, 0) == 0 or it["reports"] < units[f]:
            partial.append((f, line, name))
            continue
        nid = A.map_item(by_file, f, line, name)
        if nid is None:
            reason = (
                "file has no indexed fn nodes"
                if f not in by_file
                else "no fn node with that name within 10 lines"
            )
            unmapped.append(
                {
                    "file": f,
                    "line": line,
                    "name": name,
                    "reason": reason,
                    "macro": it["macro"],
                }
            )
        else:
            R[nid] = {
                "file": f,
                "line": line,
                "name": name,
                "class": fclass.get(f, "bin"),
            }
    partial_nodes = {
        A.map_item(by_file, f, line, name) for f, line, name in partial
    } - {None}
    res.update(
        rustc_items=len(items),
        r_mapped=len(R),
        r_unmapped=unmapped,
        rustc_partial=len(partial),
    )
    Rset = set(R)
    res["modes"], res["selfcheck"] = {}, {}
    for mode in MODES:
        strict = mode == "strict"
        F = fg.dead(db, strict)
        mdead = A.model_dead(g, strict)
        res["selfcheck"][mode] = {
            "cli": len(F),
            "model": len(mdead),
            "diff": len(F ^ mdead),
        }
        info = A.reach(g, strict, dead_rustc=Rset)
        misses = []
        for nid in sorted(Rset - F):
            p, _, _, cause = info[nid]
            bkt = miss_bucket(g, info, nid, index_root)
            misses.append(
                {
                    "id": nid,
                    **R[nid],
                    "bucket": bkt,
                    "cause": cause,
                    "collateral": p in Rset,
                    "chain": [f"{n} ({ek})" for n, ek in A.chain(info, nid)],
                }
            )
        fp = {
            "not_judgeable_file": [],
            "not_judgeable_cfg": [],
            "rustc_live_some_unit": [],
            "disagree": [],
        }
        for nid in sorted(F - Rset):
            f, line, _ = A.split_id(nid)
            rec = {
                "id": nid,
                "name": A.canon(g["nodes"][nid][1]),
                "file": f,
                "line": line,
                "class": fclass.get(f, "none"),
            }
            if f not in units:
                fp["not_judgeable_file"].append(rec)
            elif "attr::cfg" in g["attrs"].get(nid, ()) or A.cfg_gated(
                index_root, f, line
            ):
                fp["not_judgeable_cfg"].append(rec)
            elif nid in partial_nodes:
                fp["rustc_live_some_unit"].append(rec)
            else:
                fp["disagree"].append(rec)
        for rec in fp["disagree"][: args.max_check]:
            rec["text_refs"] = A.text_refs(
                index_root, rec["name"], rec["file"], rec["line"]
            )
        res["modes"][mode] = {"F": sorted(F), "misses": misses, "fp": fp}
    res["R"] = R
    res["classes"] = {n: R[n]["class"] for n in R}
    res["roots_of_R"] = {n: g["roots"].get(n) for n in R}
    res["F_class"] = {
        n: fclass.get(A.split_id(n)[0], "none")
        for m in MODES
        for n in res["modes"][m]["F"]
    }
    res["ok"] = True
    return res


def metrics(results, cls, mode):
    m = Counter()
    for r in results:
        if not r["ok"]:
            continue
        F = set(r["modes"][mode]["F"])
        R = {n for n in r["classes"] if cls == "all" or r["classes"][n] == cls}
        Fc = {n for n in F if cls == "all" or r["F_class"][n] == cls}
        fp = r["modes"][mode]["fp"]
        nj = {
            x["id"] for k in ("not_judgeable_file", "not_judgeable_cfg") for x in fp[k]
        }
        m["R"] += len(R)
        m["R_eligible"] += sum(1 for n in R if r["roots_of_R"][n] is None)
        m["F"] += len(Fc)
        m["hit"] += len(Fc & R)
        m["F_not_judgeable"] += len(Fc & nj)
    return m


def render(results, meta, args):
    L = []
    w = L.append
    w("# ferrograph `dead` vs rustc `dead_code` ground truth\n")
    w(f"- ferrograph: `{meta['version']}` (`{meta['binary']}`)")
    if meta.get("extra_args"):
        w(f"- extra `dead` args for every row: `{' '.join(meta['extra_args'])}`")
    w(
        f"- rustc: `{meta['rustc']}`; platform: `{meta['platform']}`; date: {meta['date']}"
    )
    w(
        "- ground truth: functions/methods/associated functions that rustc reports as `dead_code` in **every** compilation unit "
        "that includes their file (lib, bin, test, example, bench units of `--all-targets`), with `--force-warn dead_code`; "
        "dependencies dropped.\n"
    )
    w("## Corpora\n")
    w(
        "| corpus | source | commit | fn nodes | rustc fn items (all units) | R mapped | R unmapped | dead in some unit only | cargo s | index s |"
    )
    w("|---|---|---|---|---|---|---|---|---|---|")
    for r in results:
        src = r.get("url") or r.get("path")
        sha = f"`{(r.get('sha') or '?')[:12]}`" + (
            " (dirty tree)" if r.get("dirty") else ""
        )
        if not r["ok"]:
            w(f"| {r['name']} | {src} | {sha} | — | — | — | — | — | — | — |")
            continue
        w(
            f"| {r['name']} | {src} | {sha} | {r['fn_nodes']} | {r['rustc_items']} | {r['r_mapped']} | "
            f"{len(r['r_unmapped'])} | {r['rustc_partial']} | {r['cargo_seconds']} | {r['index_seconds']} |"
        )
    w("")
    for r in results:
        w(f"### {r['name']}\n")
        w(
            f"`{r['command']}`"
            + (
                f"  \n(**fallback used**: {r['fallback']})"
                if r.get("fallback")
                else ""
            )
        )
        if not r["ok"]:
            w(f"\n**Not measured**: {r['error']}\n")
            continue
        sc = r["selfcheck"]
        w(
            "\nSelf-check (python model of `dead` vs CLI): "
            + ", ".join(
                f"{m}: cli={sc[m]['cli']} model={sc[m]['model']} diff={sc[m]['diff']}"
                for m in MODES
            )
            + "\n"
        )
        if r["r_unmapped"]:
            cnt = Counter(u["reason"] for u in r["r_unmapped"])
            w(
                "Unmapped rustc items (indexing misses): "
                + "; ".join(f"{k}: {v}" for k, v in cnt.items())
                + "\n"
            )
    ok = [r for r in results if r["ok"]]
    w("## Recall and precision\n")
    w(
        "R = rustc-dead fns mapped to nodes; F = ferrograph dead set. "
        "**Recall (all R)** = |F∩R|/|R|. **Recall (eligible)** = |F∩R| / |R minus items ferrograph treats as roots by policy "
        "(`pub`, trait impls, tests, attributes)|. **Precision raw** = |F∩R|/|F|. **Precision judgeable** = |F∩R| / |F minus items "
        "rustc cannot judge (file never compiled, or under `#[cfg]`)|. Percentages are floored; counts given; brackets are Wilson 95% intervals. Cohorts (`original`, `held-out`, and any `cohort =` set in corpora.toml) are pooled separately; held-out ones were not used while tuning.\n"
    )
    w(
        "| corpus | class | mode | R | R eligible | F | F∩R | recall (all R) | recall (eligible) | precision raw | F not judgeable | F judgeable | precision judgeable |"
    )
    w("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    cohorts = list(dict.fromkeys(r.get("cohort", "held-out" if r.get("heldout") else "original") for r in ok))
    pools = [(f"**pooled ({co})**", [r for r in ok if r.get("cohort", "held-out" if r.get("heldout") else "original") == co]) for co in cohorts]
    for name, rs in [(r["name"] + (f" ({r['cohort']})" if r.get("cohort") not in (None, "original") else ""), [r]) for r in ok] + pools:
        for cls in ("lib", "bin", "all"):
            for mode in MODES:
                m = metrics(rs, cls, mode)
                if m["R"] == 0 and m["F"] == 0:
                    continue
                fj = m["F"] - m["F_not_judgeable"]
                w(
                    f"| {name} | {cls} | {mode} | {m['R']} | {m['R_eligible']} | {m['F']} | {m['hit']} | {A.frac_ci(m['hit'], m['R'])} | "
                    f"{A.frac(m['hit'], m['R_eligible'])} | {A.frac_ci(m['hit'], m['F'])} | {m['F_not_judgeable']} | {fj} | {A.frac_ci(m['hit'], fj)} |"
                )
    w("")
    w("## False negatives (rustc-dead, ferrograph-live): why they are kept alive\n")
    w(
        "Cause = how the item first became reachable in a BFS from ferrograph's roots (strict edges first). "
        "*collateral* = reached through another rustc-dead node, so it inherits that node's cause.\n"
    )
    for mode in MODES:
        for cls in ("lib", "bin"):
            ms = [
                x for r in ok for x in r["modes"][mode]["misses"] if x["class"] == cls
            ]
            if not ms:
                continue
            w(f"### {mode}, {cls} crates ({len(ms)} missed)\n")
            cnt = Counter(x["bucket"] for x in ms)
            col = Counter(x["bucket"] for x in ms if x["collateral"])
            w("| bucket | count | of which collateral |")
            w("|---|---|---|")
            for b, n in cnt.most_common():
                w(f"| {b} | {n} | {col[b]} |")
            w("")
            for b, _ in cnt.most_common():
                for x in sorted(
                    (x for x in ms if x["bucket"] == b), key=lambda x: x["collateral"]
                )[:3]:
                    w(
                        f"- **{b}**: `{x['file']}:{x['line']}` `{x['name']}`: "
                        + " ← ".join(x["chain"][:4])
                    )
            w("")
    w("## False positives: ferrograph-dead, rustc-not-dead\n")
    w(
        "| mode | corpus | F | policy: file not compiled | policy: cfg-gated | rustc-live in some unit only | genuine disagreement | of which only dead under `--strict` |"
    )
    w("|---|---|---|---|---|---|---|---|")
    for mode in MODES:
        for r in ok:
            fp = r["modes"][mode]["fp"]
            dflt = set(r["modes"]["default"]["F"])
            only_strict = sum(1 for x in fp["disagree"] if x["id"] not in dflt) if mode == "strict" else 0
            w(
                f"| {mode} | {r['name']} | {len(r['modes'][mode]['F'])} | {len(fp['not_judgeable_file'])} | "
                f"{len(fp['not_judgeable_cfg'])} | {len(fp['rustc_live_some_unit'])} | {len(fp['disagree'])} | "
                f"{only_strict if mode == 'strict' else '-'} |"
            )
    w(
        f"\nGenuine disagreements (default mode; text references for hand-checking are in the JSON, first {args.max_check} per corpus):\n"
    )
    for r in ok:
        for x in r["modes"]["default"]["fp"]["disagree"]:
            w(f"- {r['name']} `{x['file']}:{x['line']}` `{x['name']}`")
    w("")
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", default=str(Path(__file__).with_name("corpora.toml")))
    ap.add_argument("--ferrograph", required=True)
    ap.add_argument(
        "--work",
        default=None,
        help="work dir (default: a fresh temp dir; reused if given)",
    )
    ap.add_argument("--only", default="", help="comma-separated corpus names")
    ap.add_argument(
        "--timeout", type=int, default=1800, help="per cargo invocation, seconds"
    )
    ap.add_argument(
        "--extra-args",
        default="",
        help="extra args for `ferrograph dead`, e.g. '--restricted-vis-not-root' (the python model follows it)",
    )
    ap.add_argument("--max-check", type=int, default=25)
    ap.add_argument(
        "--render-json",
        default=None,
        help="only re-render this groundtruth JSON (e.g. merged by run_matrix.py) to --out-md",
    )
    ap.add_argument("--out-md", default="groundtruth-report.md")
    ap.add_argument("--out-json", default="groundtruth-data.json")
    args = ap.parse_args()
    if args.render_json:
        d = json.loads(Path(args.render_json).read_text())
        Path(args.out_md).write_text(render(d["results"], d["meta"], args))
        return

    work = Path(args.work) if args.work else Path(tempfile.mkdtemp(prefix="fg-acc-"))
    work.mkdir(parents=True, exist_ok=True)
    A.log("work dir:", work)
    cfg = tomllib.loads(Path(args.config).read_text())
    only = {s for s in args.only.split(",") if s}
    fg = A.Fg(args.ferrograph, shlex.split(args.extra_args))
    _, rv, _ = A.run(["rustc", "-V"])
    meta = {
        "version": fg.version(),
        "binary": str(args.ferrograph),
        "rustc": rv.strip(),
        "platform": platform.platform(),
        "date": time.strftime("%Y-%m-%d"),
        "extra_args": fg.extra,
    }
    results = []
    for c in cfg["corpus"]:
        if only and c["name"] not in only:
            continue
        A.log("==", c["name"])
        try:
            results.append(analyse(c, fg, work, args))
        except (
            Exception
        ) as e:  # a corpus that cannot be measured is reported, not fatal
            results.append(
                {
                    "name": c["name"],
                    "ok": False,
                    "error": str(e)[-1500:],
                    "command": "-",
                    "sha": None,
                    "url": c.get("url"),
                    "path": c.get("path"),
                }
            )
    md = render(results, meta, args)
    Path(args.out_md).write_text(md)
    Path(args.out_json).write_text(
        json.dumps({"meta": meta, "results": results}, indent=1)
    )
    print(md)


if __name__ == "__main__":
    sys.exit(main())
