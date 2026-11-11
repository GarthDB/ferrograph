"""Shared helpers for the ferrograph accuracy harness (Python 3.11+, standard library only)."""

import json
import math
import os
import re
import shutil
import subprocess
import sys
import time
from collections import defaultdict, deque
from pathlib import Path

PREFIXES = ("pub::", "test::", "bench::")
ENTRY_ATTRS = {
    "attr::trait_impl",
    "attr::no_mangle",
    "attr::export_name",
    "attr::used",
    "attr::pub",
}
LIB_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def run(cmd, cwd=None, env=None, timeout=900, check=True, capture=True):
    """Run a command with a hard timeout; the whole process group is killed on expiry."""
    e = dict(os.environ)
    e.update(env or {})
    p = subprocess.Popen(
        cmd,
        cwd=cwd,
        env=e,
        text=True,
        start_new_session=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, 9)
        p.communicate()
        raise RuntimeError(f"timeout after {timeout}s: {' '.join(map(str, cmd))}")
    if check and p.returncode != 0:
        raise RuntimeError(
            f"exit {p.returncode}: {' '.join(map(str, cmd))}\n{(err or '')[-2000:]}"
        )
    return p.returncode, out or "", err or ""


def floor_pct(n, d):
    """Percentage floored to one decimal (never rounded up); 'n/a' for an empty denominator."""
    if d == 0:
        return "n/a"
    return f"{math.floor(1000 * n / d) / 10:.1f}%"


def frac(n, d):
    return f"{floor_pct(n, d)} ({n}/{d})"


# ---------------------------------------------------------------- corpora


def git_head(path):
    rc, out, _ = run(["git", "rev-parse", "HEAD"], cwd=path, check=False)
    return out.strip() if rc == 0 else None


def prepare_corpus(c, work):
    """Materialise a corpus copy under `work` and return its root. Originals are never written."""
    dest = Path(work) / "corpora" / c.get("group", c["name"])
    if c.get("url"):
        if not dest.exists():
            dest.parent.mkdir(parents=True, exist_ok=True)
            run(["git", "init", "-q", str(dest)])
            run(["git", "remote", "add", "origin", c["url"]], cwd=dest)
            run(
                [
                    "git",
                    "fetch",
                    "-q",
                    "--depth",
                    "1",
                    "origin",
                    c.get("ref") or "HEAD",
                ],
                cwd=dest,
                timeout=600,
            )
            run(["git", "checkout", "-q", "FETCH_HEAD"], cwd=dest)
        return dest, git_head(dest), False
    src = Path(os.path.expanduser(c["path"])).resolve()
    sha = git_head(src)
    _, st, _ = run(["git", "status", "--porcelain"], cwd=src, check=False)
    if not dest.exists():
        shutil.copytree(
            src,
            dest,
            symlinks=True,
            ignore=shutil.ignore_patterns(
                "target", ".git", "node_modules", ".ferrograph*"
            ),
        )
    return dest, sha, bool(st.strip())


# ---------------------------------------------------------------- ferrograph


class Fg:
    def __init__(self, binary):
        self.bin = str(binary)

    def version(self):
        _, out, _ = run([self.bin, "--version"])
        return out.strip().splitlines()[-1].strip()

    def index(self, root, db, timeout=900):
        if Path(db).exists():
            Path(db).unlink()
        run([self.bin, "index", str(root), "--db", str(db)], timeout=timeout)

    def query(self, db, script, order=None):
        """Run a query; with `order`, fetch all rows in pages (the CLI caps one result at 10000 rows)."""
        if order is None:
            _, out, _ = run(
                [self.bin, "query", "--db", str(db), script, "--json"], timeout=600
            )
            return json.loads(out)["rows"]
        rows, off = [], 0
        while True:
            chunk = self.query(db, f"{script} :order {order} :limit 9000 :offset {off}")
            rows += chunk
            if len(chunk) < 9000:
                return rows
            off += 9000

    def dead(self, db, strict=False):
        args = [self.bin, "dead", "--db", str(db), "--format", "json"] + (
            ["--strict"] if strict else []
        )
        _, out, _ = run(args, check=False)
        return {n["id"] for n in json.loads(out)["dead_nodes"]}

    def load_graph(self, db):
        g = {}
        g["nodes"] = {
            r[0]: (r[1], r[2]) for r in self.query(db, "?[i,t,p] := *nodes[i,t,p]", "i")
        }
        g["edges"] = [
            tuple(r) for r in self.query(db, "?[f,t,e] := *edges[f,t,e]", "f,t,e")
        ]
        g["low"] = {
            (r[0], r[1])
            for r in self.query(db, "?[f,t,e,c] := *edge_confidence[f,t,e,c]", "f,t,e")
        }
        for key, q in (
            ("nodes", "?[count(i)] := *nodes[i,_,_]"),
            ("edges", "?[count(f)] := *edges[f,_,_]"),
        ):
            n = self.query(db, q)[0][0]
            if n != len(g[key]):
                raise RuntimeError(
                    f"query result truncated for {key}: {len(g[key])} of {n}"
                )
        return g


def canon(payload):
    for p in PREFIXES:
        if payload and payload.startswith(p):
            return payload[len(p) :]
    return payload


def split_id(i):
    m = re.match(r"^(.*)#(\d+):(\d+)$", i)
    return (m.group(1), int(m.group(2)), int(m.group(3))) if m else (i, 0, 0)


# ---------------------------------------------------------------- reachability model


def root_kind(g, nid, implemented_trait_fns):
    t, payload = g["nodes"][nid]
    payload = payload or ""
    if t != "function":
        # entry attributes (e.g. `attr::pub` on a `pub const`) are roots for any node type
        attrs = g["attrs"].get(nid, set())
        return next((a[6:] for a in sorted(ENTRY_ATTRS) if a in attrs), None)
    if payload == "main":
        return "main"
    if payload.startswith("pub::"):
        return "pub"
    if payload.startswith("test::"):
        return "test"
    if payload.startswith("bench::"):
        return "bench"
    attrs = g["attrs"].get(nid, set())
    if "attr::trait_impl" in attrs:
        return "trait_impl"
    for a in ("attr::no_mangle", "attr::export_name", "attr::used", "attr::pub"):
        if a in attrs:
            return a[6:]
    if nid in implemented_trait_fns:
        return "trait_def_method"
    return None


def prepare_model(g):
    attrs = defaultdict(set)
    adj = defaultdict(list)
    contains = defaultdict(list)
    implemented = set()
    for f, t, e in g["edges"]:
        if e == "has_attribute":
            attrs[f].add(t)
        elif e == "implements_trait":
            implemented.add(t)
        elif e == "contains":
            contains[f].append(t)
        elif e in ("calls", "references", "expands_to"):
            adj[f].append((t, e))
    g["attrs"], g["adj"] = attrs, adj
    g["trait_fns"] = {
        f
        for t in implemented
        if g["nodes"].get(t, (None,))[0] == "trait"
        for f in contains[t]
        if g["nodes"].get(f, (None,))[0] == "function"
    }
    g["roots"] = {}
    for nid, (t, _) in g["nodes"].items():
        if t in ("file", "module"):
            g["roots"][nid] = t
        else:
            k = root_kind(g, nid, g["trait_fns"])
            if k:
                g["roots"][nid] = k
    return g


def reach(g, strict, dead_rustc=frozenset()):
    """Multi-source BFS replicating `dead`'s reachability: strict edges first, low-confidence call
    edges only afterwards (and never with `strict`). Returns {node: (parent, edge, is_low, cause)};
    `cause` explains why nodes in `dead_rustc` (rustc-dead) were kept alive: a root kind, the kind
    of the edge from a node outside that set, or (collateral) the cause of the rustc-dead parent."""
    info = {}
    dq = deque()
    deferred = deque()

    def visit(t, p, e, low):
        cause = None
        if t in dead_rustc:
            pk = g["roots"].get(p) or "private"
            cause = info[p][3] or f"{e}{'(low-confidence)' if low else ''}@{pk}"
        info[t] = (p, e, low, cause)
        dq.append(t)

    for r, k in g["roots"].items():
        info[r] = (None, "root", False, ("root:" + k) if r in dead_rustc else None)
        dq.append(r)
    while True:
        while dq:
            p = dq.popleft()
            for t, e in g["adj"].get(p, ()):
                if t in info:
                    continue
                if e == "calls" and (p, t) in g["low"]:
                    if not strict:
                        deferred.append((p, t, e))
                    continue
                visit(t, p, e, False)
        while deferred and not dq:
            p, t, e = deferred.popleft()
            if t not in info:
                visit(t, p, e, True)
        if not dq:
            return info


def model_dead(g, strict):
    info = reach(g, strict)
    return {n for n, (t, _) in g["nodes"].items() if t == "function" and n not in info}


def chain(info, nid, limit=6):
    out = []
    while nid is not None and len(out) < limit:
        p, e, low, _ = info[nid]
        out.append((nid, e + ("(low)" if low else "")))
        nid = p
    return out


# ---------------------------------------------------------------- rustc ground truth


def run_rustc(root, cargo_args, target_dir, timeout=1800, extra_env=None):
    """Run `cargo check` with force-warned dead_code. Returns (messages, command, fallback_note, err_tail); falls back step by step if the build fails."""
    env = {"RUSTFLAGS": "--force-warn dead_code", "CARGO_TARGET_DIR": str(target_dir)}
    env.update(extra_env or {})
    nofeat = [a for a in cargo_args if a != "--all-features"]
    nobench = ["--lib", "--bins", "--tests", "--examples"]
    variants = [(cargo_args, "")]
    if nofeat != cargo_args:
        variants.append((nofeat, "default features (--all-features failed)"))
    if "--all-targets" in cargo_args:
        for base, note in (
            (cargo_args, "no benches (--all-targets failed)"),
            (nofeat, "default features, no benches"),
        ):
            variants.append(([a for a in base if a != "--all-targets"] + nobench, note))
    err = ""
    for args, note in variants:
        cmd = ["cargo", "check", *args, "--message-format=json"]
        rc, out, err = run(cmd, cwd=root, env=env, timeout=timeout, check=False)
        if rc == 0:
            msgs = []
            for line in out.splitlines():
                try:
                    msgs.append(json.loads(line))
                except ValueError:
                    pass
            return msgs, 'RUSTFLAGS="--force-warn dead_code" ' + " ".join(cmd), note, ""
    return None, " ".join(cmd), "all variants failed", err[-1500:]


def parse_dep_files(artifact):
    fn = artifact["filenames"][0]
    d = Path(fn).parent
    stem = Path(fn).stem
    cands = [d / f"{c}.d" for c in (stem, stem[3:] if stem.startswith("lib") else None) if c]
    if "custom-build" in artifact["target"]["kind"]:
        # build scripts are emitted as `build-script-build`; their dep-info is `build_script_build-<hash>.d`
        cands += sorted(d.glob("build_script_*.d"))
    for path in cands:
        if path.exists():
            txt = path.read_text(errors="replace")
            files = set()
            for line in txt.splitlines():
                if ":" not in line:
                    continue
                deps = line.split(": ", 1)[1] if ": " in line else ""
                for tok in re.split(r"(?<!\\) ", deps):
                    tok = tok.replace("\\ ", " ").strip()
                    if tok.endswith(".rs"):
                        files.add(tok)
            return files
    return None


def module_names(root):
    """(declared `mod` names, `#[path]` basenames) in the tree. Dep-info also lists files pulled in by
    include_str!/include_bytes!, which are data, not compiled modules; this tells them apart."""
    names, paths = set(), set()
    for p in Path(root).rglob("*.rs"):
        if "target" in p.parts or ".git" in p.parts:
            continue
        txt = p.read_text(errors="replace")
        names.update(re.findall(r"\bmod\s+(\w+)\s*[;{]", txt))
        paths.update(Path(x).name for x in re.findall(r'#\[path\s*=\s*"([^"]+)"', txt))
    return names, paths


def declared_module(root, nf, mods):
    names, paths = mods
    p = Path(nf)
    stem = p.parent.name if p.stem == "mod" else p.stem
    return stem in names or p.name in paths


def collect_ground_truth(msgs, cargo_root, index_root):
    """Return (items, files_units, file_class).
    items: {(file, line, name): {"reports": n, "msg": ..., "macro": bool}} with file relative to
    index_root; files_units: {file: n_units compiling it}; file_class: {file: 'lib'|'bin'}."""
    cargo_root = Path(cargo_root).resolve()
    index_root = Path(index_root).resolve()

    def norm(f):
        p = Path(f)
        if not p.is_absolute():
            p = cargo_root / p
        p = Path(os.path.normpath(p))
        try:
            return "./" + str(p.relative_to(index_root))
        except ValueError:
            return None

    units = defaultdict(int)
    lib_units = defaultdict(int)
    seen_units = set()
    mods = module_names(cargo_root)
    for m in msgs:
        if m.get("reason") != "compiler-artifact":
            continue
        mp = Path(m.get("manifest_path", ""))
        try:
            mp.resolve().relative_to(cargo_root)
        except ValueError:
            continue
        files = parse_dep_files(m)
        if not files:
            continue
        key = (tuple(m["filenames"]), m["package_id"])
        if key in seen_units:
            continue
        seen_units.add(key)
        is_lib = bool(set(m["target"]["kind"]) & LIB_KINDS) and not m.get(
            "profile", {}
        ).get("test")
        root_src = norm(m["target"].get("src_path", ""))
        root_dir = str(Path(root_src).parent) if root_src else None
        for f in files:
            nf = norm(f)
            # files beside a crate root count as modules too: macros such as `automod::dir!` declare them
            if nf and (
                nf == root_src
                or str(Path(nf).parent) == root_dir
                or declared_module(cargo_root, nf, mods)
            ):
                units[nf] += 1
                if is_lib:
                    lib_units[nf] += 1
    items = {}
    for m in msgs:
        if m.get("reason") != "compiler-message":
            continue
        d = m["message"]
        if (d.get("code") or {}).get("code") != "dead_code":
            continue
        for s in d.get("spans", []):
            if not s.get("is_primary"):
                continue
            f = norm(s["file_name"])
            if f is None:
                continue
            txt = s["text"][0] if s.get("text") else None
            if not txt:
                continue
            name = txt["text"][txt["highlight_start"] - 1 : txt["highlight_end"] - 1]
            if not re.search(r"\bfn\s+$", txt["text"][: txt["highlight_start"] - 1]):
                continue
            k = (f, s["line_start"], name)
            it = items.setdefault(
                k,
                {
                    "reports": 0,
                    "msg": d["message"].split(" `")[0],
                    "macro": bool(s.get("expansion")),
                },
            )
            it["reports"] += 1
    fclass = {f: ("lib" if lib_units.get(f) else "bin") for f in units}
    return items, dict(units), fclass


# ---------------------------------------------------------------- cfg heuristics (policy bucket)

_HDR = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(unsafe\s+)?(mod|impl|trait|extern)\b")


def _attrs_above(lines, j):
    """Attribute text directly above line j, including multi-line attributes and interleaved comments."""
    out = []
    k = j - 1
    while k >= 0:
        t = lines[k].strip()
        if t.startswith("#[") or t.startswith("#!["):
            out.append(lines[k])
        elif t.startswith("//"):
            pass
        elif t.endswith("]"):
            # tail of a wrapped attribute: walk up to the line that opens it
            m = k
            while m >= 0 and k - m < 40 and not lines[m].lstrip().startswith("#["):
                m -= 1
            if m < 0 or k - m >= 40:
                break
            out.append(" ".join(l.strip() for l in lines[m : k + 1]))
            k = m
        else:
            break
        k -= 1
    return out


def cfg_gated(root, f, line):
    """Heuristic: is the fn at (file, line) under a `#[cfg(...)]` (own attrs, enclosing mod/impl/trait,
    or its `mod x;` declaration)? Text based, so a cfg_attr-free approximation."""
    p = Path(root) / f
    try:
        lines = p.read_text(errors="replace").splitlines()
    except OSError:
        return False
    i = line - 1
    if any("cfg(" in a for a in _attrs_above(lines, i)):
        return True
    cur = len(lines[i]) - len(lines[i].lstrip())
    for j in range(i - 1, -1, -1):
        s = lines[j]
        if not s.strip():
            continue
        ind = len(s) - len(s.lstrip())
        if ind < cur and _HDR.match(s):
            if any("cfg(" in a for a in _attrs_above(lines, j)):
                return True
            cur = ind
    stem = p.stem if p.stem != "mod" else p.parent.name
    pat = re.compile(rf"^\s*(pub(\([^)]*\))?\s+)?mod\s+{re.escape(stem)}\s*;")
    for parent in {p.parent, p.parent.parent}:
        for cand in list(parent.glob("*.rs")):
            try:
                cl = cand.read_text(errors="replace").splitlines()
            except OSError:
                continue
            for j, s in enumerate(cl):
                if pat.match(s) and any("cfg(" in a for a in _attrs_above(cl, j)):
                    return True
    return False


# ---------------------------------------------------------------- mapping


def build_index(g):
    by_file = defaultdict(list)
    for nid, (t, payload) in g["nodes"].items():
        if t == "function":
            f, ln, col = split_id(nid)
            by_file[f].append((ln, col, canon(payload or ""), nid))
    return by_file


def map_item(by_file, f, line, name, window=10):
    best = None
    for ln, col, cn, nid in by_file.get(f, ()):
        if (
            cn == name
            and line - window <= ln <= line
            and (best is None or ln > best[0])
        ):
            best = (ln, nid)
    return best[1] if best else None


def text_refs(root, name, defn_file, defn_line, limit=6):
    """Source lines mentioning `name` other than its definition (for hand-checking)."""
    pat = re.compile(rf"\b{re.escape(name)}\b")
    out = []
    for p in Path(root).rglob("*.rs"):
        if "target" in p.parts or ".git" in p.parts:
            continue
        rel = "./" + str(p.relative_to(root))
        try:
            for n, s in enumerate(p.read_text(errors="replace").splitlines(), 1):
                if pat.search(s) and not (rel == defn_file and n == defn_line):
                    out.append(f"{rel}:{n}: {s.strip()[:110]}")
                    if len(out) >= limit:
                        return out
        except OSError:
            pass
    return out


def all_unit_dead(items, units):
    """Keys of items rustc reports dead in every unit that compiles their file."""
    return {k for k, v in items.items() if units.get(k[0], 0) and v["reports"] >= units[k[0]]}


def visibility(root, nid):
    """Source-level visibility of the fn node: `pub(crate)/pub(super)` (what #91 is about) or `pub`."""
    f, line, _ = split_id(nid)
    try:
        lines = (Path(root) / f).read_text(errors="replace").splitlines()[line - 1:line + 6]
    except OSError:
        return "unknown"
    for ln in lines:
        if re.search(r"\bpub\s*\(\s*(crate|super|self|in\b)", ln):
            return "pub(crate)/pub(super)"
        if re.search(r"\bpub\s+(const\s+|async\s+|unsafe\s+)*fn\b", ln):
            return "pub"
        if re.search(r"\bfn\b", ln):
            break
    return "not pub in source (inherited, e.g. impl of a pub trait or enclosing item)"
