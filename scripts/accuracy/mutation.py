#!/usr/bin/env python3
"""Part B: mutation test of `ferrograph pr` (newly dead functions) against rustc (see README.md).

For one corpus: pick single-line call statements whose callee has exactly one call site, replace
the statement with `();` in a throwaway git repo (one commit per mutation, line numbers unchanged),
and compare ferrograph's `pr --base HEAD~1 --index-head` newly-dead list with the dead_code
diagnostics rustc gains.
"""

import argparse
import shlex
import json
import platform
import random
import re
import shutil
import sys
import tempfile
import time
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

import fgacc as A
from groundtruth import DEFAULT_CARGO_ARGS, miss_bucket

GIT = ["git", "-c", "user.email=harness@example.invalid", "-c", "user.name=harness"]


def git(repo, *a, **kw):
    return A.run(GIT + list(a), cwd=repo, **kw)


def find_candidates(g, repo, rng, limit):
    """Call statements `[recv.]callee(args);` whose callee has exactly one (high-confidence) call site."""
    incoming = defaultdict(list)
    for f, t, e in g["edges"]:
        if e == "calls" and (f, t) not in g["low"]:
            incoming[t].append(f)
    names = Counter(
        A.canon(p or "") for _, (t, p) in g["nodes"].items() if t == "function"
    )
    starts = defaultdict(list)
    for nid, (t, _) in g["nodes"].items():
        if t == "function":
            f, ln, _ = A.split_id(nid)
            starts[f].append(ln)
    out = []
    callees = [
        n
        for n, callers in incoming.items()
        if len(set(callers)) == 1 and g["nodes"][n][0] == "function"
    ]
    rng.shuffle(callees)
    for callee in callees:
        name = A.canon(g["nodes"][callee][1] or "")
        caller = incoming[callee][0]
        if caller == callee or names[name] != 1 or g["nodes"][caller][0] != "function":
            continue
        cf, cl, _ = A.split_id(caller)
        later = [s for s in starts[cf] if s > cl]
        end = min(later) if later else cl + 400
        try:
            lines = (Path(repo) / cf).read_text().splitlines()
        except OSError:
            continue
        pat = re.compile(
            rf"^\s*(?:[A-Za-z_][\w:]*::|[A-Za-z_][\w.()]*\.)?{re.escape(name)}(?:::<[^>;{{}}]*>)?\([^;{{}}]*\)\??;\s*$"
        )
        for i in range(cl, min(end, len(lines) + 1)):
            if (
                pat.match(lines[i - 1])
                and "|" not in lines[i - 1]
                and not lines[i - 1].lstrip().startswith(("let ", "return", "//"))
            ):
                out.append(
                    {
                        "family": "call",
                        "callee": callee,
                        "name": name,
                        "caller": caller,
                        "file": cf,
                        "line": i,
                        "text": lines[i - 1].strip(),
                        "edits": [(cf, i, i, "();")],
                    }
                )
                break
        if len(out) >= limit:
            break
    return out


def read_lines(repo, f):
    try:
        return (Path(repo) / f).read_text().splitlines()
    except OSError:
        return None


def incoming_map(g, kinds=("calls", "references")):
    inc = defaultdict(list)
    for f, t, e in g["edges"]:
        if e in kinds and f != t:
            inc[t].append((f, e))
    return inc


def unique_names(g):
    return Counter(A.canon(p or "") for _, (t, p) in g["nodes"].items() if t == "function")


def find_ref_candidates(g, repo, rng, limit):
    """Delete a whole use site that is a reference, not a call statement: a `.method(f)` adapter line or a table
    entry `f,` on a line of its own (the line is blanked, so numbering is stable). The fn has one using fn, uniquely named."""
    inc, names = incoming_map(g), unique_names(g)
    fns = [n for n, (t, _) in g["nodes"].items() if t == "function"]
    rng.shuffle(fns)
    out = []
    for fn in fns:
        name = A.canon(g["nodes"][fn][1] or "")
        srcs = inc.get(fn, [])
        if len({f for f, _ in srcs}) != 1 or names[name] != 1:
            continue
        holder = srcs[0][0]
        if holder == fn:
            continue
        if holder not in g["nodes"]:
            continue
        hf, hl, _ = A.split_id(holder)
        lines = read_lines(repo, hf)
        if lines is None:
            continue
        pat = re.compile(
            rf"^\s*(?:\.\w+\((?:[A-Za-z_][\w]*::)*{re.escape(name)}\)|(?:[A-Za-z_][\w]*::)*{re.escape(name)},?)\s*$"
        )
        for i in range(hl, min(hl + 400, len(lines) + 1)):
            if pat.match(lines[i - 1]):
                out.append(
                    {
                        "family": "ref",
                        "callee": fn,
                        "name": name,
                        "caller": holder,
                        "file": hf,
                        "line": i,
                        "text": lines[i - 1].strip(),
                        "edits": [(hf, i, i, "")],
                    }
                )
                break
        if len(out) >= limit:
            break
    return out


def block_end(lines, start):
    """1-based line of the `}` closing the first `{` at or after `start` (naive brace count; cargo check validates)."""
    depth, seen = 0, False
    for i in range(start - 1, len(lines)):
        code = re.sub(r'"(?:[^"\\]|\\.)*"|//.*', "", lines[i])
        for ch in code:
            if ch == "{":
                depth, seen = depth + 1, True
            elif ch == "}":
                depth -= 1
        if seen and depth <= 0:
            return i + 1
        if not seen and code.rstrip().endswith(";"):
            return None  # bodiless item
    return None


def attr_start(lines, start):
    """First line of the attributes/doc comments directly above the item at `start` (1-based), including
    attributes wrapped over several lines (found by walking up from their closing `]`)."""
    i = start
    while i > 1:
        prev = lines[i - 2].strip()
        if prev.startswith(("#[", "#![", "///", "//!")):
            i -= 1
        elif prev.endswith("]") and not prev.endswith("];"):
            j = i - 2
            while j >= 0 and i - 2 - j < 40 and not lines[j].lstrip().startswith(("#[", "#![")):
                j -= 1
            if j < 0 or not lines[j].lstrip().startswith(("#[", "#![")):
                break
            i = j + 1
        else:
            break
    return i


def find_impl_candidates(g, repo, rng, limit):
    """Delete a whole `impl Trait for T { .. }` block (blanked) whose methods are the only callers of some helper fn."""
    inc = incoming_map(g, ("calls",))
    names = unique_names(g)
    helpers = [n for n, (t, _) in g["nodes"].items() if t == "function"]
    rng.shuffle(helpers)
    out = []
    for h in helpers:
        callers = {f for f, _ in inc.get(h, [])}
        name = A.canon(g["nodes"][h][1] or "")
        if len(callers) != 1 or names[name] != 1:
            continue
        c = next(iter(callers))
        if "attr::trait_impl" not in g["attrs"].get(c, ()) or c == h:
            continue
        cf, cl, _ = A.split_id(c)
        lines = read_lines(repo, cf)
        if lines is None:
            continue
        ind = len(lines[cl - 1]) - len(lines[cl - 1].lstrip())
        start = next(
            (
                j
                for j in range(cl - 1, 0, -1)
                if re.match(r"^\s*(unsafe\s+)?impl\b", lines[j - 1])
                and len(lines[j - 1]) - len(lines[j - 1].lstrip()) < ind
            ),
            None,
        )
        end = block_end(lines, start) if start else None
        if not end or end < cl:
            continue
        a = attr_start(lines, start)
        out.append(
            {
                "family": "impl",
                "callee": h,
                "name": name,
                "caller": c,
                "file": cf,
                "line": start,
                "text": lines[start - 1].strip()[:80] + f" .. (blanked {a}-{end})",
                "edits": [(cf, a, end, "")],
            }
        )
        if len(out) >= limit:
            break
    return out


def find_caller_candidates(g, repo, rng, limit):
    """Delete a whole fn (blanked, with its attributes) that nothing in the repo calls or references and
    that is the only caller of some helper: the helper's last caller disappears."""
    inc = incoming_map(g)
    calls_in = incoming_map(g, ("calls",))
    out_calls = defaultdict(set)
    for f, t, e in g["edges"]:
        if e == "calls" and f != t:
            out_calls[f].add(t)
    names = unique_names(g)
    fns = [n for n, (t, _) in g["nodes"].items() if t == "function"]
    rng.shuffle(fns)
    out = []
    for c in fns:
        if inc.get(c) or c in g["roots"] and g["roots"][c] in ("main", "test", "bench", "trait_impl"):
            continue
        if g["attrs"].get(c):
            continue
        sole = [
            h
            for h in out_calls.get(c, ())
            if {f for f, _ in calls_in[h]} == {c}
            and g["nodes"][h][0] == "function"
            and names[A.canon(g["nodes"][h][1] or "")] == 1
        ]
        if not sole:
            continue
        cf, cl, _ = A.split_id(c)
        lines = read_lines(repo, cf)
        end = block_end(lines, cl) if lines else None
        if not end or end - cl > 80:
            continue
        a = attr_start(lines, cl)
        # a nested or enclosing-item mismatch would not compile; cargo check filters those
        out.append(
            {
                "family": "caller",
                "callee": sole[0],
                "name": A.canon(g["nodes"][sole[0]][1] or ""),
                "caller": c,
                "file": cf,
                "line": cl,
                "text": lines[cl - 1].strip()[:80] + f" .. (blanked {a}-{end})",
                "edits": [(cf, a, end, "")],
            }
        )
        if len(out) >= limit:
            break
    return out


FAMILIES = {
    "call": find_candidates,
    "ref": find_ref_candidates,
    "impl": find_impl_candidates,
    "caller": find_caller_candidates,
}


def apply_edits(root, edits):
    """Replace line ranges (1-based, inclusive) by one replacement line then blank lines: line count is unchanged."""
    for f, a, b, rep in edits:
        path = Path(root) / f
        lines = path.read_text().splitlines(keepends=True)
        nl = "\n" if lines[a - 1].endswith("\n") else ""
        indent = re.match(r"\s*", lines[a - 1]).group(0) if rep else ""
        for k in range(a - 1, b):
            lines[k] = nl
        if rep:
            lines[a - 1] = indent + rep + nl
        path.write_text("".join(lines))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", default=str(Path(__file__).with_name("corpora.toml")))
    ap.add_argument("--corpus", default="clap")
    ap.add_argument("--ferrograph", required=True)
    ap.add_argument("--work", default=None)
    ap.add_argument(
        "--n", type=int, default=30, help="number of compiling mutations to run"
    )
    ap.add_argument("--max-attempts", type=int, default=90, help="candidates generated per family")
    ap.add_argument(
        "--family",
        default="call",
        help=f"comma-separated mutation families ({', '.join(FAMILIES)}) or 'all'; --n applies per family",
    )
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=1800)
    ap.add_argument(
        "--extra-args",
        default="",
        help="extra args for `ferrograph pr`/`dead`, e.g. '--restricted-vis-not-root'",
    )
    ap.add_argument("--out-md", default="mutation-report.md")
    ap.add_argument("--out-json", default="mutation-data.json")
    args = ap.parse_args()

    work = Path(args.work) if args.work else Path(tempfile.mkdtemp(prefix="fg-mut-"))
    work.mkdir(parents=True, exist_ok=True)
    c = next(
        x
        for x in tomllib.loads(Path(args.config).read_text())["corpus"]
        if x["name"] == args.corpus
    )
    fg = A.Fg(args.ferrograph, shlex.split(args.extra_args))
    src, sha, dirty = A.prepare_corpus(c, work)
    repo = work / "mutrepo" / args.corpus
    if repo.exists():
        shutil.rmtree(repo)
    shutil.copytree(
        src, repo, ignore=shutil.ignore_patterns(".git", "target", ".ferrograph*")
    )
    git(repo, "init", "-q")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "base")
    _, base, _ = git(repo, "rev-parse", "HEAD")
    base = base.strip()
    index_root = (repo / c.get("index_path", ".")).resolve()
    cargo_args = c.get("cargo_args", DEFAULT_CARGO_ARGS)
    tgt = work / "target" / c.get("group", c["name"]) / "mut"

    def rustc_dead():
        msgs, cmd, note, err = A.run_rustc(repo, cargo_args, tgt, timeout=args.timeout)
        if msgs is None:
            return None, cmd, note
        items, units, _ = A.collect_ground_truth(msgs, repo, index_root)
        return A.all_unit_dead(items, units), cmd, note

    before, cmd, note = rustc_dead()
    if before is None:
        sys.exit("base does not build: " + note)
    db = work / "base.db"
    fg.index(index_root, db)
    g = A.prepare_model(fg.load_graph(db), fg.restricted, fg.bin_pub)
    by_file = A.build_index(g)
    fams = list(FAMILIES) if args.family == "all" else args.family.split(",")
    cands = []
    for fam in fams:
        found = FAMILIES[fam](g, index_root, random.Random(args.seed), args.max_attempts)
        A.log(f"family {fam}: {len(found)} candidates")
        cands += found
    A.log(f"{len(cands)} candidates; base rustc-dead fns: {len(before)}")

    runs, skipped = [], []
    per_family = Counter()
    for cand in cands:
        if per_family[cand["family"]] >= args.n:
            continue
        # start every mutation from the pristine base tree (a previous mutation's commit may be checked out)
        git(repo, "checkout", "-q", "--detach", base)
        apply_edits(index_root, cand["edits"])
        git(repo, "commit", "-q", "-am", f"mutate {cand['file']}:{cand['line']}")
        after, _, _ = rustc_dead()
        if after is None:
            skipped.append({**cand, "reason": "cargo check failed"})
            continue
        t0 = time.time()
        rc, out, err = A.run(
            [
                args.ferrograph,
                "pr",
                "--base",
                "HEAD~1",
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
            skipped.append({**cand, "reason": f"pr failed: {err[-300:]}"})
            continue
        pr = json.loads(out)
        reported = {x["id"] for x in pr["newly_dead"]}
        oracle_items = after - before
        oracle = {}
        for f, line, name in oracle_items:
            nid = A.map_item(by_file, f, line, name)
            oracle[nid or f"UNMAPPED {f}:{line}:{name}"] = (f, line, name)
        run = {
            **cand,
            "pr_seconds": pr_s,
            "oracle": sorted(oracle),
            "reported": sorted(reported),
            "revived": sorted(f"{k[0]}:{k[1]}:{k[2]}" for k in before - after),
        }
        run["tp"] = sorted(set(oracle) & reported)
        fn_ids = sorted(set(oracle) - reported)
        fp_ids = sorted(reported - set(oracle))
        before_set = {A.map_item(by_file, f, line, name) for f, line, name in before}
        run["fp"] = []
        for nid in fp_ids:
            f, line, _ = A.split_id(nid)
            nm = A.canon(g["nodes"].get(nid, (None, ""))[1] or "")
            if nid in before_set:
                kind = "already dead in rustc before the change"
            elif A.cfg_gated(index_root, f, line) or "attr::cfg" in g["attrs"].get(
                nid, ()
            ):
                kind = "not judgeable (cfg-gated)"
            else:
                kind = "rustc says still live"
            run["fp"].append({"id": nid, "name": nm, "kind": kind})
        run["fn"] = []
        if fn_ids:
            hdb = work / "head.db"
            fg.index(index_root, hdb)
            hg = A.prepare_model(fg.load_graph(hdb), fg.restricted, fg.bin_pub)
            head_dead = fg.dead(hdb)
            base_dead = fg.dead(db)
            info = A.reach(hg, False, dead_rustc=set(fn_ids))
            for nid in fn_ids:
                if nid.startswith("UNMAPPED"):
                    run["fn"].append(
                        {
                            "id": nid,
                            "bucket": "rustc item not mapped to a node (indexing miss)",
                        }
                    )
                elif nid in head_dead and nid in base_dead:
                    run["fn"].append(
                        {
                            "id": nid,
                            "bucket": "ferrograph already considered it dead at base",
                        }
                    )
                elif nid in head_dead:
                    run["fn"].append(
                        {
                            "id": nid,
                            "bucket": "dead in head graph but not reported by pr",
                        }
                    )
                elif nid not in info:
                    run["fn"].append({"id": nid, "bucket": "other"})
                else:
                    run["fn"].append(
                        {
                            "id": nid,
                            "bucket": miss_bucket(hg, info, nid, index_root),
                            "chain": [f"{n} ({e})" for n, e in A.chain(info, nid)][:4],
                        }
                    )
        runs.append(run)
        per_family[cand["family"]] += 1
        A.log(
            f"[{cand['family']} {per_family[cand['family']]}/{args.n}] {cand['file']}:{cand['line']} {cand['name']}: oracle={len(oracle)} reported={len(reported)} tp={len(run['tp'])} {pr_s}s"
        )
    git(repo, "checkout", "-q", "--detach", base)
    _, wt, _ = git(repo, "worktree", "list")

    tp = sum(len(r["tp"]) for r in runs)
    n_or = sum(len(r["oracle"]) for r in runs)
    n_rep = sum(len(r["reported"]) for r in runs)
    fpc = Counter(x["kind"] for r in runs for x in r["fp"])
    fnc = Counter(x["bucket"] for r in runs for x in r["fn"])
    _, rv, _ = A.run(["rustc", "-V"])
    L = []
    w = L.append
    w("# ferrograph `pr` mutation test\n")
    w(
        f"- ferrograph `{fg.version()}`; {rv.strip()}; {platform.platform()}; {time.strftime('%Y-%m-%d')}"
    )
    w(
        f"- corpus `{args.corpus}` @ `{(sha or '?')[:12]}`{' (dirty tree)' if dirty else ''}; seed {args.seed}"
    )
    w(
        f"- oracle command: `{cmd}` ({note or 'no fallback'}); newly dead = dead-in-all-units after minus before (function items only)"
    )
    w(f"- extra args: `{' '.join(fg.extra) or 'none'}`; brackets are Wilson 95% intervals")
    w(
        f"- system under test: `ferrograph pr --base HEAD~1 --index-head --format json` on a throwaway git repo (one commit per mutation)"
    )
    w(f"- families: {', '.join(fams)} (see README); up to {args.n} compiling mutations per family")
    w(
        f"- {len(cands)} candidates found, {len(runs)} mutations compiled and measured, {len(skipped)} skipped (did not compile / pr failed)\n"
    )
    w(
        "Scope: rustc cannot judge trait-impl methods or `pub` items of library crates; the comparison covers only items rustc reports "
        "(in binaries and tests, `pub` items in private modules are also reported).\n"
    )
    if len(fams) > 1:
        w("## Per family\n")
        w("| family | mutations | rustc newly dead | reported | TP | recall | precision |")
        w("|---|---|---|---|---|---|---|")
        for fam in fams:
            fr = [r for r in runs if r["family"] == fam]
            t, o, rp = (sum(len(r[k]) for r in fr) for k in ("tp", "oracle", "reported"))
            w(f"| {fam} | {len(fr)} | {o} | {rp} | {t} | {A.frac_ci(t, o)} | {A.frac_ci(t, rp)} |")
        w("")
    w("## Pooled\n")
    w(
        f"- mutations where rustc reports ≥1 newly dead fn: {sum(1 for r in runs if r['oracle'])}/{len(runs)}; where ferrograph reports ≥1: {sum(1 for r in runs if r['reported'])}/{len(runs)}"
    )
    w(
        f"- oracle newly dead: {n_or}; ferrograph reported: {n_rep}; true positives: {tp}"
    )
    w(f"- recall: {A.frac_ci(tp, n_or)}; precision (all reports): {A.frac_ci(tp, n_rep)}")
    judge = n_rep - fpc["not judgeable (cfg-gated)"] - fpc["already dead in rustc before the change"]
    w(f"- precision over judgeable reports (excluding cfg-gated and already-dead-before): {A.frac_ci(tp, judge)}")
    ts = [r["pr_seconds"] for r in runs]
    if ts:
        w(
            f"- `pr` wall time per invocation: min {min(ts)}s, median {sorted(ts)[len(ts) // 2]}s, max {max(ts)}s (includes indexing base in a temp worktree and HEAD in memory)"
        )
    w(
        "\nFalse positives by kind: "
        + (", ".join(f"{k}: {v}" for k, v in fpc.items()) or "none")
    )
    w(
        "\nFalse negatives by bucket: "
        + (", ".join(f"{k}: {v}" for k, v in fnc.items()) or "none")
        + "\n"
    )
    w("## Per mutation\n")
    w("| # | family | mutated statement | callee | oracle | reported | TP | FN | FP | pr s |")
    w("|---|---|---|---|---|---|---|---|---|---|")
    for i, r in enumerate(runs, 1):
        w(
            f"| {i} | {r['family']} | `{r['file']}:{r['line']}` `{r['text'][:50]}` | `{r['name']}` | {len(r['oracle'])} | {len(r['reported'])} | "
            f"{len(r['tp'])} | {len(r['fn'])} | {len(r['fp'])} | {r['pr_seconds']} |"
        )
    w("\n## False negatives (examples)\n")
    for r in runs:
        for x in r["fn"]:
            w(
                f"- `{r['file']}:{r['line']}` → `{x['id']}`: **{x['bucket']}**"
                + (f" ({' ← '.join(x['chain'])})" if x.get("chain") else "")
            )
    w("\n## False positives\n")
    for r in runs:
        for x in r["fp"]:
            w(f"- `{r['file']}:{r['line']}` → `{x['id']}` `{x['name']}`: {x['kind']}")
    w("\n## Skipped\n")
    for s in skipped:
        w(f"- `{s['file']}:{s['line']}` `{s['name']}`: {s['reason'][:100]}")
    w(f"\n`git worktree list` after the run:\n```\n{wt.strip()}\n```")
    md = "\n".join(L)
    Path(args.out_md).write_text(md)
    Path(args.out_json).write_text(
        json.dumps({"runs": runs, "skipped": skipped, "base": base}, indent=1)
    )
    print(md)


if __name__ == "__main__":
    sys.exit(main())
