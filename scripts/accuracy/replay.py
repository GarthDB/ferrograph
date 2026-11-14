#!/usr/bin/env python3
"""Replay mode: false-alarm rate of `ferrograph pr` on ordinary commits (see README.md).

For the last N first-parent commits of a corpus, run `ferrograph pr --base <parent> --index-head` at the
commit and compare its newly-dead functions with rustc's (dead-in-all-units after minus before).
Unlike mutation.py this uses real changes, so it measures how often `pr` cries wolf on normal PRs.
"""

import argparse
import json
import platform
import re
import shlex
import sys
import tempfile
import time
import tomllib
from collections import Counter
from pathlib import Path

import fgacc as A

DEFAULT_CARGO_ARGS = ["--workspace", "--all-targets", "--all-features"]
CFG = "not judgeable (cfg-gated)"


def git(repo, *a, check=True):
    return A.run(["git", *a], cwd=repo, check=check)


def clone(c, work, n):
    """Own clone with at least `n` commits of history (the shared corpus copy is shallow)."""
    dest = Path(work) / "replay" / c["name"]
    if not dest.exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        A.run(["git", "init", "-q", str(dest)])
        A.run(["git", "remote", "add", "origin", c["url"]], cwd=dest)
        A.run(
            [
                "git",
                "fetch",
                "-q",
                "--depth",
                str(n + 2),
                "origin",
                c.get("ref") or "HEAD",
            ],
            cwd=dest,
            timeout=900,
        )
        A.run(["git", "tag", "replay-head", "FETCH_HEAD"], cwd=dest)
        A.run(["git", "checkout", "-q", "--detach", "replay-head"], cwd=dest)
    else:
        _, cnt, _ = A.run(["git", "rev-list", "--count", "replay-head"], cwd=dest)
        _, shallow, _ = A.run(["git", "rev-parse", "--is-shallow-repository"], cwd=dest)
        # an earlier run fetched less history (e.g. default mode, now `--select calls`); a complete history cannot grow
        if int(cnt) < n + 2 and shallow.strip() == "true":
            _, head, _ = A.run(["git", "rev-parse", "replay-head"], cwd=dest)
            A.run(
                ["git", "fetch", "-q", "--depth", str(n + 2), "origin", head.strip()],
                cwd=dest,
                timeout=900,
            )
    return dest


FN_DEF = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
COMMON = {"new", "default", "from", "into", "fmt", "eq", "hash", "clone", "drop", "next", "len", "get", "set", "run", "main", "build", "write", "read", "parse", "name", "init"}


def defined_fns(repo, rev):
    """Names of fns defined in the `.rs` files of `rev` (via `git grep`; common trait-method names dropped)."""
    _, out, _ = A.run(["git", "grep", "-hoE", r"fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*", rev, "--", "*.rs"], cwd=repo, check=False)
    names = {m.group(1) for m in map(FN_DEF.search, out.splitlines()) if m}
    return {n for n in names if len(n) >= 4 and n not in COMMON}


def removes_call(repo, parent, sha, names):
    """Does the commit delete a line that calls or references a fn defined in the repo? Returns the names hit."""
    _, diff, _ = A.run(["git", "diff", "-U0", parent, sha, "--", "*.rs"], cwd=repo, check=False)
    removed = [l[1:] for l in diff.splitlines() if l.startswith("-") and not l.startswith("---")]
    hit = set()
    for l in removed:
        if l.lstrip().startswith("//") or FN_DEF.search(l):
            continue
        for m in re.finditer(r"[A-Za-z_]\w*", l):
            w = m.group(0)
            if w in names:
                rest = l[m.end():].lstrip()
                before = l[: m.start()]
                if rest.startswith(("(", "::<")) or before.endswith((".", "::", "(", ", ", "& ", "&")):
                    hit.add(w)
    return sorted(hit)


def match(item, node_id):
    """rustc item (file, line, name) vs a ferrograph id `./file#line:col`: same file, the node starts
    at or up to 10 lines before rustc's line (same rule as fgacc.map_item)."""
    f, line, _ = A.split_id(node_id)
    return item[0] == f and 0 <= item[1] - line <= 10


def replay(c, fg, work, args):
    res = {"name": c["name"], "heldout": bool(c.get("heldout")), "cohort": A.cohort(c), "ok": False}
    repo = clone(c, work, args.depth)
    index_root = (repo / c.get("index_path", ".")).resolve()
    cargo_args = c.get("cargo_args", DEFAULT_CARGO_ARGS)
    tgt = Path(work) / "target" / c.get("group", c["name"]) / "replay"
    _, head, _ = git(repo, "rev-parse", "replay-head")
    res["head"] = head.strip()
    _, revs, _ = git(repo, "rev-list", "--first-parent", "-n", str(args.depth), "replay-head")
    fns = defined_fns(repo, "replay-head") if args.select == "calls" else set()
    selected = 0
    # rustc's answer depends only on the commit, so it is kept on disk and reused across `--extra-args` arms.
    cache_file = Path(work) / "replay" / f"{c['name']}.dead.json"
    raw = json.loads(cache_file.read_text()) if cache_file.exists() else {}
    cache = {k: None if v is None else {tuple(i) for i in v} for k, v in raw.items()}

    def dead_at(sha):
        if sha not in cache:
            git(repo, "checkout", "-q", "-f", "--detach", sha)
            msgs, _, _, _ = A.run_rustc(repo, cargo_args, tgt, timeout=args.timeout)
            if msgs is None:
                cache[sha] = None
            else:
                items, units, _ = A.collect_ground_truth(msgs, repo, index_root)
                cache[sha] = A.all_unit_dead(items, units)
            cache_file.write_text(
                json.dumps({k: None if v is None else sorted(v) for k, v in cache.items()})
            )
        return cache[sha]

    runs, skipped = [], []
    for sha in revs.split():
        if selected >= args.n:
            break
        _, parents, _ = git(repo, "rev-list", "--parents", "-n", "1", sha)
        parts = parents.split()
        if len(parts) < 2:
            continue  # root of the fetched history
        parent = parts[1]
        _, changed, _ = git(repo, "diff", "--name-only", parent, sha)
        rs = [f for f in changed.splitlines() if f.endswith(".rs")]
        if not rs:
            skipped.append({"sha": sha, "reason": "no .rs change"})
            continue
        removed = []
        if args.select == "calls":
            removed = removes_call(repo, parent, sha, fns)
            if not removed:
                continue  # not a candidate: not counted as skipped, not built
        before, after = dead_at(parent), dead_at(sha)
        if before is None or after is None:
            skipped.append({"sha": sha, "reason": "cargo check failed"})
            continue
        git(repo, "checkout", "-q", "-f", "--detach", sha)
        t0 = time.time()
        rc, out, err = A.run(
            [
                args.ferrograph,
                "pr",
                "--base",
                parent,
                "--index-head",
                "--format",
                "json",
                *fg.extra,
            ],
            cwd=index_root,
            check=False,
            timeout=900,
        )
        pr_s = round(time.time() - t0, 1)
        if rc != 0:
            skipped.append({"sha": sha, "reason": f"pr failed: {err[-200:]}"})
            continue
        reported = [(x["id"], x["name"]) for x in json.loads(out)["newly_dead"]]
        oracle = sorted(after - before)
        tp, fp = [], []
        for nid, name in reported:
            if any(o[2] == name and match(o, nid) for o in oracle):
                tp.append(nid)
                continue
            f, line, _ = A.split_id(nid)
            if any(o[2] == name and match(o, nid) for o in before):
                kind = "rustc already dead before the commit"
            elif A.cfg_gated(index_root, f, line):
                kind = CFG
            else:
                kind = "rustc says still live"
            fp.append({"id": nid, "name": name, "kind": kind})
        missed = [
            o
            for o in oracle
            if not any(o[2] == nm and match(o, i) for i, nm in reported)
        ]
        selected += 1  # only measured commits count toward --n; unbuildable ones are skipped above
        runs.append(
            {
                "sha": sha,
                "rs_files": len(rs),
                "removed_calls": removed,
                "oracle": [list(o) for o in oracle],
                "reported": [list(r) for r in reported],
                "tp": tp,
                "fp": fp,
                "fn": [list(o) for o in missed],
                "pr_seconds": pr_s,
            }
        )
        A.log(
            f"[{c['name']} {len(runs)}] {sha[:8]} oracle={len(oracle)} reported={len(reported)} fp={len(fp)} {pr_s}s"
        )
    res.update(ok=True, runs=runs, skipped=skipped)
    return res


def pool(rs):
    runs = [x for r in rs if r["ok"] for x in r["runs"]]
    judged_fp = [f for x in runs for f in x["fp"] if f["kind"] != CFG]
    return {
        "commits": len(runs),
        "oracle_commits": sum(1 for x in runs if x["oracle"]),
        "alarm_commits": sum(1 for x in runs if any(f["kind"] != CFG for f in x["fp"])),
        "commits_with_report": sum(1 for x in runs if x["reported"]),
        "reported": sum(len(x["reported"]) for x in runs),
        "tp": sum(len(x["tp"]) for x in runs),
        "fp": len(judged_fp),
        "oracle": sum(len(x["oracle"]) for x in runs),
    }


def row(name, p):
    return (
        f"| {name} | {p['commits']} | {p['oracle_commits']} | {p['commits_with_report']} | {A.frac_ci(p['alarm_commits'], p['commits'])} | "
        f"{p['reported']} | {A.frac_ci(p['tp'], p['tp'] + p['fp'])} | {p['oracle']} | {A.frac_ci(p['tp'], p['oracle'])} |"
    )


def render(results, meta):
    L = []
    w = L.append
    w("# ferrograph `pr` replay on real commits\n")
    w(
        f"- ferrograph `{meta['version']}`; {meta['rustc']}; {meta['platform']}; {meta['date']}"
    )
    w(
        f"- extra args: `{' '.join(meta['extra_args']) or 'none'}`; "
        + (
            f"selection `calls`: the first {meta['n']} of the last {meta['depth']} first-parent commits whose diff removes a line calling or referencing a fn defined in the repo"
            if meta["select"] == "calls"
            else f"selection `last`: the last {meta['n']} first-parent commits"
        )
    )
    w(
        "- per commit with a `.rs` change: `ferrograph pr --base <parent> --index-head`; oracle = rustc dead-in-all-units after minus before. "
        "**False-alarm commit rate** = commits where `pr` reports a newly dead fn that rustc does not confirm (cfg-gated reports excluded) / commits with `.rs` changes. "
        "**Precision** = confirmed / (confirmed + unconfirmed judgeable). Brackets are Wilson 95% intervals; "
        "commits that did not build are skipped, so this is biased toward buildable history.\n"
    )
    w(
        "| corpus | commits | commits with rustc newly dead | commits with a report | false-alarm commits | reported | precision | rustc newly dead | recall |"
    )
    w("|---|---|---|---|---|---|---|---|---|")
    ok = [r for r in results if r["ok"]]
    for r in ok:
        w(row(r["name"] + (f" ({r['cohort']})" if r["cohort"] != "original" else ""), pool([r])))
    for co in dict.fromkeys(r["cohort"] for r in ok):
        w(row(f"**pooled ({co})**", pool([r for r in ok if r["cohort"] == co])))
    kinds = Counter(f["kind"] for r in ok for x in r["runs"] for f in x["fp"])
    w(
        "\nUnconfirmed reports by kind: "
        + (", ".join(f"{k}: {v}" for k, v in kinds.items()) or "none")
    )
    w("\n## Unconfirmed reports\n")
    for r in ok:
        for x in r["runs"]:
            for f in x["fp"]:
                w(
                    f"- {r['name']} `{x['sha'][:10]}` `{f['id']}` `{f['name']}`: {f['kind']}"
                )
    w("\n## Skipped commits\n")
    for r in results:
        if not r["ok"]:
            w(f"- {r['name']}: not measured: {r['error']}")
            continue
        cnt = Counter(s["reason"].split(":")[0] for s in r["skipped"])
        w(
            f"- {r['name']}: "
            + (", ".join(f"{k}: {v}" for k, v in cnt.items()) or "none")
        )
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", default=str(Path(__file__).with_name("corpora.toml")))
    ap.add_argument("--ferrograph", required=True)
    ap.add_argument("--work", default=None)
    ap.add_argument(
        "--only", default="", help="comma-separated corpus names (url corpora only)"
    )
    ap.add_argument(
        "--n", type=int, default=50, help="recent first-parent commits per corpus"
    )
    ap.add_argument(
        "--select",
        choices=["last", "calls"],
        default="last",
        help="`last`: the last N commits; `calls`: the N most recent commits (within --depth) whose diff removes a call/reference to a repo fn",
    )
    ap.add_argument(
        "--depth",
        type=int,
        default=None,
        help="first-parent commits to scan (default N for `last`, 10*N for `calls`)",
    )
    ap.add_argument(
        "--extra-args",
        default="",
        help="extra args for `ferrograph pr`, e.g. '--restricted-vis-not-root'",
    )
    ap.add_argument("--timeout", type=int, default=1800)
    ap.add_argument("--out-md", default="replay-report.md")
    ap.add_argument("--out-json", default="replay-data.json")
    args = ap.parse_args()
    args.depth = args.depth or (args.n * 10 if args.select == "calls" else args.n)
    work = Path(args.work) if args.work else Path(tempfile.mkdtemp(prefix="fg-replay-"))
    work.mkdir(parents=True, exist_ok=True)
    fg = A.Fg(args.ferrograph, shlex.split(args.extra_args))
    _, rv, _ = A.run(["rustc", "-V"])
    meta = {
        "version": fg.version(),
        "rustc": rv.strip(),
        "platform": platform.platform(),
        "date": time.strftime("%Y-%m-%d"),
        "extra_args": fg.extra,
        "n": args.n,
        "depth": args.depth,
        "select": args.select,
    }
    only = {s for s in args.only.split(",") if s}
    results = []
    for c in tomllib.loads(Path(args.config).read_text())["corpus"]:
        if (only and c["name"] not in only) or not c.get("url"):
            continue
        A.log("==", c["name"])
        try:
            results.append(replay(c, fg, work, args))
        except Exception as e:  # one broken corpus must not hide the others
            results.append(
                {
                    "name": c["name"],
                    "heldout": bool(c.get("heldout")),
                    "cohort": A.cohort(c),
                    "ok": False,
                    "error": str(e)[-800:],
                }
            )
    md = render(results, meta)
    Path(args.out_md).write_text(md)
    Path(args.out_json).write_text(
        json.dumps({"meta": meta, "results": results}, indent=1)
    )
    print(md)


if __name__ == "__main__":
    sys.exit(main())
