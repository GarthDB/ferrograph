//! Running ferrograph, loading its graph and the Python model of `dead` (port of `fgacc.py`).

use crate::run::run;
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::Duration;

const PREFIXES: [&str; 3] = ["pub::", "test::", "bench::"];
/// Sorted, so the first match wins like `sorted(ENTRY_ATTRS)` in Python.
const ENTRY_ATTRS: [&str; 5] = [
    "attr::export_name",
    "attr::no_mangle",
    "attr::pub",
    "attr::trait_impl",
    "attr::used",
];

pub fn canon(payload: &str) -> &str {
    PREFIXES
        .iter()
        .find_map(|p| payload.strip_prefix(p))
        .unwrap_or(payload)
}

/// `./file.rs#line:col` -> (file, line, col); anything else -> (id, 0, 0).
pub fn split_id(i: &str) -> (String, usize, usize) {
    let re = Regex::new(r"^(.*)#(\d+):(\d+)$").unwrap();
    match re.captures(i) {
        Some(m) => (
            m[1].to_string(),
            m[2].parse().unwrap_or(0),
            m[3].parse().unwrap_or(0),
        ),
        None => (i.to_string(), 0, 0),
    }
}

// ---------------------------------------------------------------- ferrograph CLI

pub struct Fg {
    pub bin: String,
    /// Passed to `dead` and `pr` only (never to `index`/`query`).
    pub extra: Vec<String>,
    pub restricted: bool,
    pub bin_pub: bool,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: String,
    /// `null` in the database is stored as an empty string (Python used `payload or ""` everywhere).
    pub payload: String,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: BTreeMap<String, Node>,
    pub edges: Vec<(String, String, String)>,
    /// (from, to) of calls with a low-confidence edge.
    pub low: HashSet<(String, String)>,
    // model, filled by `prepare_model`
    pub attrs: HashMap<String, BTreeSet<String>>,
    pub adj: HashMap<String, Vec<(String, String)>>,
    pub trait_fns: HashSet<String>,
    pub roots: BTreeMap<String, String>,
}

fn s(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_string)
        .with_context(|| format!("expected string, got {v}"))
}

impl Fg {
    pub fn new(binary: &str, extra_args: &[String]) -> Self {
        let has = |f: &str| extra_args.iter().any(|a| a == f);
        let strict = has("--strict"); // strict turns both options off unless asked
        let on = |yes: &str, no: &str| has(yes) || (!has(no) && !strict); // on by default since 5.0
        Fg {
            bin: binary.to_string(),
            extra: extra_args.to_vec(),
            restricted: on("--restricted-vis-not-root", "--no-restricted-vis-not-root"),
            bin_pub: on("--bin-pub-not-root", "--no-bin-pub-not-root"),
        }
    }

    /// (`restricted`, `bin_pub`) the model must use for a `dead` run: `--strict` turns both options off
    /// unless asked for explicitly.
    pub fn model_flags(&self, strict: bool) -> (bool, bool) {
        let has = |f: &str| self.extra.iter().any(|a| a == f);
        (
            self.restricted && (!strict || has("--restricted-vis-not-root")),
            self.bin_pub && (!strict || has("--bin-pub-not-root")),
        )
    }

    pub fn version(&self) -> Result<String> {
        let o = run(
            &[&self.bin, "--version"],
            None,
            &[],
            Duration::from_secs(900),
            true,
        )?;
        Ok(o.stdout
            .trim()
            .lines()
            .last()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    pub fn index(&self, root: &Path, db: &Path, timeout: Duration) -> Result<()> {
        if db.exists() {
            std::fs::remove_file(db)?;
        }
        let (r, d) = (root.to_str().context("path")?, db.to_str().context("path")?);
        run(
            &[&self.bin, "index", r, "--db", d],
            None,
            &[],
            timeout,
            true,
        )?;
        Ok(())
    }

    /// Run a query; with `order`, fetch all rows in pages (the CLI caps one result at 10000 rows).
    pub fn query(&self, db: &Path, script: &str, order: Option<&str>) -> Result<Vec<Vec<Value>>> {
        let d = db.to_str().context("path")?;
        let one = |script: &str| -> Result<Vec<Vec<Value>>> {
            let o = run(
                &[&self.bin, "query", "--db", d, script, "--json"],
                None,
                &[],
                Duration::from_secs(600),
                true,
            )?;
            let v: Value = serde_json::from_str(&o.stdout)?;
            Ok(serde_json::from_value(v["rows"].clone())?)
        };
        let Some(order) = order else {
            return one(script);
        };
        let (mut rows, mut off) = (Vec::new(), 0);
        loop {
            let chunk = one(&format!(
                "{script} :order {order} :limit 9000 :offset {off}"
            ))?;
            let n = chunk.len();
            rows.extend(chunk);
            if n < 9000 {
                return Ok(rows);
            }
            off += 9000;
        }
    }

    pub fn dead(&self, db: &Path, strict: bool) -> Result<HashSet<String>> {
        let d = db.to_str().context("path")?;
        let mut args = vec![self.bin.as_str(), "dead", "--db", d, "--format", "json"];
        if strict {
            args.push("--strict");
        }
        args.extend(self.extra.iter().map(String::as_str));
        let o = run(&args, None, &[], Duration::from_secs(900), false)?;
        let v: Value = serde_json::from_str(&o.stdout).with_context(|| {
            format!("`dead` output is not JSON (exit {}): {}", o.code, o.stderr)
        })?;
        v["dead_nodes"]
            .as_array()
            .context("no dead_nodes")?
            .iter()
            .map(|n| s(&n["id"]))
            .collect()
    }

    pub fn load_graph(&self, db: &Path) -> Result<Graph> {
        let mut g = Graph::default();
        for r in self.query(db, "?[i,t,p] := *nodes[i,t,p]", Some("i"))? {
            let payload = r[2].as_str().unwrap_or("").to_string();
            g.nodes.insert(
                s(&r[0])?,
                Node {
                    kind: s(&r[1])?,
                    payload,
                },
            );
        }
        for r in self.query(db, "?[f,t,e] := *edges[f,t,e]", Some("f,t,e"))? {
            g.edges.push((s(&r[0])?, s(&r[1])?, s(&r[2])?));
        }
        for r in self.query(db, "?[f,t,e,c] := *edge_confidence[f,t,e,c]", Some("f,t,e"))? {
            g.low.insert((s(&r[0])?, s(&r[1])?));
        }
        for (key, have, q) in [
            ("nodes", g.nodes.len(), "?[count(i)] := *nodes[i,_,_]"),
            ("edges", g.edges.len(), "?[count(f)] := *edges[f,_,_]"),
        ] {
            let n = self.query(db, q, None)?[0][0].as_u64().context("count")?;
            if n != have as u64 {
                bail!("query result truncated for {key}: {have} of {n}");
            }
        }
        Ok(g)
    }
}

// ---------------------------------------------------------------- reachability model

impl Graph {
    fn has_attr(&self, nid: &str, a: &str) -> bool {
        self.attrs.get(nid).is_some_and(|x| x.contains(a))
    }

    fn root_kind(&self, nid: &str, restricted: bool, bin_pub: bool) -> Option<String> {
        let n = &self.nodes[nid];
        let payload = n.payload.as_str();
        if n.kind != "function" {
            // entry attributes (e.g. `attr::pub` on a `pub const`) are roots for any node type
            return ENTRY_ATTRS
                .iter()
                .find(|a| self.has_attr(nid, a))
                .map(|a| a[6..].to_string());
        }
        if payload == "main" {
            return Some("main".into());
        }
        if canon(payload).starts_with('_') {
            return Some("underscore".into()); // rustc never reports `_name` fns; roots since 5.0
        }
        let demoted = ((restricted && self.has_attr(nid, "attr::pub_restricted"))
            || (bin_pub && self.has_attr(nid, "attr::pub_bin_only")))
            && !payload.starts_with("pub::test::")
            && !payload.starts_with("pub::bench::")
            && payload != "pub::main";
        if payload.starts_with("pub::") && !demoted {
            return Some("pub".into());
        }
        if payload.starts_with("test::") {
            return Some("test".into());
        }
        if payload.starts_with("bench::") {
            return Some("bench".into());
        }
        if self.has_attr(nid, "attr::trait_impl") {
            return Some("trait_impl".into());
        }
        for a in [
            "attr::no_mangle",
            "attr::export_name",
            "attr::used",
            "attr::pub",
        ] {
            if self.has_attr(nid, a) {
                return Some(a[6..].to_string());
            }
        }
        if self.trait_fns.contains(nid) {
            return Some("trait_def_method".into());
        }
        None
    }

    /// Fill the model fields (attributes, adjacency, trait fns, roots). `restricted`/`bin_pub`
    /// mirror `--restricted-vis-not-root` / `--bin-pub-not-root`.
    pub fn prepare_model(&mut self, restricted: bool, bin_pub: bool) {
        let mut contains: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut implemented: HashSet<&str> = HashSet::new();
        self.attrs.clear();
        self.adj.clear();
        for (f, t, e) in &self.edges {
            match e.as_str() {
                "has_attribute" => {
                    self.attrs.entry(f.clone()).or_default().insert(t.clone());
                }
                "implements_trait" => {
                    implemented.insert(t);
                }
                "contains" => contains.entry(f).or_default().push(t),
                "calls" | "references" | "expands_to" => {
                    self.adj
                        .entry(f.clone())
                        .or_default()
                        .push((t.clone(), e.clone()));
                }
                _ => {}
            }
        }
        let is = |id: &str, kind: &str| self.nodes.get(id).is_some_and(|n| n.kind == kind);
        self.trait_fns = implemented
            .iter()
            .filter(|t| is(t, "trait"))
            .flat_map(|t| contains.get(t).into_iter().flatten())
            .filter(|f| is(f, "function"))
            .map(|f| (*f).to_string())
            .collect();
        let mut roots = BTreeMap::new();
        for (nid, n) in &self.nodes {
            if n.kind == "file" || n.kind == "module" {
                roots.insert(nid.clone(), n.kind.clone());
            } else if let Some(k) = self.root_kind(nid, restricted, bin_pub) {
                roots.insert(nid.clone(), k);
            }
        }
        self.roots = roots;
    }
}

#[derive(Debug, Clone)]
pub struct Info {
    pub parent: Option<String>,
    pub edge: String,
    pub low: bool,
    pub cause: Option<String>,
}

/// Multi-source BFS replicating `dead`'s reachability: strict edges first, low-confidence call
/// edges only afterwards (and never with `strict`). `cause` explains why nodes in `dead_rustc`
/// (rustc-dead) were kept alive: a root kind, the kind of the edge from a node outside that set,
/// or (collateral) the cause of the rustc-dead parent.
pub fn reach(g: &Graph, strict: bool, dead_rustc: &HashSet<String>) -> HashMap<String, Info> {
    let mut info: HashMap<String, Info> = HashMap::new();
    let mut dq: VecDeque<String> = VecDeque::new();
    let mut deferred: VecDeque<(String, String, String)> = VecDeque::new();

    let visit = |info: &mut HashMap<String, Info>,
                 dq: &mut VecDeque<String>,
                 t: &str,
                 p: &str,
                 e: &str,
                 low: bool| {
        let mut cause = None;
        if dead_rustc.contains(t) {
            let pk = g.roots.get(p).map_or("private", String::as_str);
            cause = info[p].cause.clone().or_else(|| {
                Some(format!(
                    "{e}{}@{pk}",
                    if low { "(low-confidence)" } else { "" }
                ))
            });
        }
        info.insert(
            t.to_string(),
            Info {
                parent: Some(p.to_string()),
                edge: e.to_string(),
                low,
                cause,
            },
        );
        dq.push_back(t.to_string());
    };

    for (r, k) in &g.roots {
        let cause = dead_rustc.contains(r).then(|| format!("root:{k}"));
        info.insert(
            r.clone(),
            Info {
                parent: None,
                edge: "root".into(),
                low: false,
                cause,
            },
        );
        dq.push_back(r.clone());
    }
    loop {
        while let Some(p) = dq.pop_front() {
            for (t, e) in g.adj.get(&p).into_iter().flatten() {
                if info.contains_key(t) {
                    continue;
                }
                if e == "calls" && g.low.contains(&(p.clone(), t.clone())) {
                    if !strict {
                        deferred.push_back((p.clone(), t.clone(), e.clone()));
                    }
                    continue;
                }
                visit(&mut info, &mut dq, t, &p, e, false);
            }
        }
        while dq.is_empty() {
            let Some((p, t, e)) = deferred.pop_front() else {
                break;
            };
            if !info.contains_key(&t) {
                visit(&mut info, &mut dq, &t, &p, &e, true);
            }
        }
        if dq.is_empty() {
            return info;
        }
    }
}

pub fn model_dead(g: &Graph, strict: bool) -> HashSet<String> {
    let info = reach(g, strict, &HashSet::new());
    g.nodes
        .iter()
        .filter(|(n, v)| v.kind == "function" && !info.contains_key(*n))
        .map(|(n, _)| n.clone())
        .collect()
}

/// Path from `nid` back towards a root, at most `limit` hops: (node, edge kind with `(low)` mark).
pub fn chain(info: &HashMap<String, Info>, nid: &str, limit: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = Some(nid);
    while let Some(n) = cur {
        if out.len() >= limit {
            break;
        }
        let i = &info[n];
        out.push((
            n.to_string(),
            format!("{}{}", i.edge, if i.low { "(low)" } else { "" }),
        ));
        cur = i.parent.as_deref();
    }
    out
}

// ---------------------------------------------------------------- mapping

/// Function nodes by file: (line, col, canonical name, id), in id order.
pub fn build_index(g: &Graph) -> BTreeMap<String, Vec<(usize, usize, String, String)>> {
    let mut by_file: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (nid, n) in &g.nodes {
        if n.kind == "function" {
            let (f, ln, col) = split_id(nid);
            by_file.entry(f).or_default().push((
                ln,
                col,
                canon(&n.payload).to_string(),
                nid.clone(),
            ));
        }
    }
    by_file
}

/// The fn node named `name` defined at most `window` lines above `line` (the closest one).
pub fn map_item(
    by_file: &BTreeMap<String, Vec<(usize, usize, String, String)>>,
    f: &str,
    line: usize,
    name: &str,
    window: usize,
) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for (ln, _, cn, nid) in by_file.get(f).into_iter().flatten() {
        if cn == name && *ln + window >= line && *ln <= line && best.is_none_or(|(b, _)| *ln > b) {
            best = Some((*ln, nid));
        }
    }
    best.map(|(_, n)| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(
        nodes: &[(&str, &str, &str)],
        edges: &[(&str, &str, &str)],
        low: &[(&str, &str)],
    ) -> Graph {
        let mut g = Graph::default();
        for (i, k, p) in nodes {
            g.nodes.insert(
                (*i).into(),
                Node {
                    kind: (*k).into(),
                    payload: (*p).into(),
                },
            );
        }
        for (f, t, e) in edges {
            g.edges.push(((*f).into(), (*t).into(), (*e).into()));
        }
        for (f, t) in low {
            g.low.insert(((*f).into(), (*t).into()));
        }
        g
    }

    #[test]
    fn canon_and_split_id() {
        assert_eq!(canon("pub::test::x"), "test::x");
        assert_eq!(canon("bench::b"), "b");
        assert_eq!(canon("plain"), "plain");
        assert_eq!(split_id("./a/b.rs#12:5"), ("./a/b.rs".into(), 12, 5));
        assert_eq!(split_id("./a/b.rs"), ("./a/b.rs".into(), 0, 0));
    }

    #[test]
    fn fg_flags_follow_extra_args() {
        let f = |a: &[&str]| {
            let v: Vec<String> = a.iter().map(ToString::to_string).collect();
            let fg = Fg::new("x", &v);
            (fg.restricted, fg.bin_pub)
        };
        assert_eq!(f(&[]), (true, true));
        assert_eq!(f(&["--strict"]), (false, false));
        assert_eq!(f(&["--strict", "--bin-pub-not-root"]), (false, true));
        assert_eq!(f(&["--no-restricted-vis-not-root"]), (false, true));
    }

    fn model() -> Graph {
        // file -> main -> a (strict) ; main -> b (low-confidence); b -> c ; d is unreachable;
        // pub_f is a root; res is demoted when `restricted`; tr_m implements a trait.
        let mut g = g(
            &[
                ("f.rs", "file", ""),
                ("m", "function", "main"),
                ("a", "function", "a"),
                ("b", "function", "b"),
                ("c", "function", "c"),
                ("d", "function", "d"),
                ("p", "function", "pub::pub_f"),
                ("res", "function", "pub::res"),
                ("_u", "function", "_unused"),
                ("T", "trait", "T"),
                ("tm", "function", "tm"),
            ],
            &[
                ("m", "a", "calls"),
                ("m", "b", "calls"),
                ("b", "c", "calls"),
                ("res", "attr::pub_restricted", "has_attribute"),
                ("X", "T", "implements_trait"),
                ("T", "tm", "contains"),
            ],
            &[("m", "b")],
        );
        g.prepare_model(true, true);
        g
    }

    #[test]
    fn roots_follow_visibility_options() {
        let g = model();
        let k = |n: &str| g.roots.get(n).map(String::as_str);
        assert_eq!(k("f.rs"), Some("file"));
        assert_eq!(k("m"), Some("main"));
        assert_eq!(k("p"), Some("pub"));
        assert_eq!(k("res"), None); // demoted by restricted
        assert_eq!(k("_u"), Some("underscore"));
        assert_eq!(k("tm"), Some("trait_def_method"));
        assert_eq!(k("d"), None);
        let mut h = model();
        h.prepare_model(false, true);
        assert_eq!(h.roots.get("res").map(String::as_str), Some("pub"));
    }

    #[test]
    fn dead_set_defers_low_confidence_edges() {
        let g = model();
        let dead = |strict| {
            let mut v: Vec<_> = model_dead(&g, strict).into_iter().collect();
            v.sort();
            v
        };
        assert_eq!(dead(false), ["d", "res"]); // b, c reached through the low-confidence edge
        assert_eq!(dead(true), ["b", "c", "d", "res"]);
    }

    #[test]
    fn reach_records_causes_and_chain() {
        let g = model();
        let dead_rustc: HashSet<String> = ["b", "c", "p"].map(String::from).into();
        let info = reach(&g, false, &dead_rustc);
        assert_eq!(
            info["b"].cause.as_deref(),
            Some("calls(low-confidence)@main")
        );
        assert_eq!(
            info["c"].cause.as_deref(),
            Some("calls(low-confidence)@main")
        ); // collateral
        assert_eq!(info["p"].cause.as_deref(), Some("root:pub"));
        assert!(info["a"].cause.is_none());
        assert_eq!(
            chain(&info, "c", 6),
            [
                ("c".into(), "calls".into()),
                ("b".into(), "calls(low)".into()),
                ("m".into(), "root".into())
            ]
        );
        assert_eq!(chain(&info, "c", 2).len(), 2);
    }

    #[test]
    fn map_item_picks_closest_definition_within_window() {
        let g = g(
            &[
                ("./a.rs#10:1", "function", "pub::foo"),
                ("./a.rs#14:1", "function", "foo"),
                ("./a.rs#30:1", "function", "foo"),
            ],
            &[],
            &[],
        );
        let ix = build_index(&g);
        assert_eq!(
            map_item(&ix, "./a.rs", 16, "foo", 10).as_deref(),
            Some("./a.rs#14:1")
        );
        assert_eq!(
            map_item(&ix, "./a.rs", 20, "foo", 10).as_deref(),
            Some("./a.rs#14:1")
        );
        assert_eq!(map_item(&ix, "./a.rs", 26, "foo", 10), None);
        assert_eq!(map_item(&ix, "./b.rs", 16, "foo", 10), None);
    }

    /// Python's results on the graph of the synthetic corpus (`xtask/tests/golden/model-synthetic.json`).
    #[test]
    #[allow(clippy::too_many_lines, clippy::many_single_char_names)]
    fn model_matches_python_on_synthetic_graph() {
        let gold: Value =
            serde_json::from_str(include_str!("../tests/golden/model-synthetic.json")).unwrap();
        let strs = |v: &Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let sorted = |h: HashSet<String>| {
            let mut v: Vec<String> = h.into_iter().collect();
            v.sort();
            v
        };
        let mut raw = Graph::default();
        for n in gold["graph"]["nodes"].as_array().unwrap() {
            let payload = n[2].as_str().unwrap_or("").to_string();
            raw.nodes.insert(
                n[0].as_str().unwrap().into(),
                Node {
                    kind: n[1].as_str().unwrap().into(),
                    payload,
                },
            );
        }
        for e in gold["graph"]["edges"].as_array().unwrap() {
            raw.edges.push((
                e[0].as_str().unwrap().into(),
                e[1].as_str().unwrap().into(),
                e[2].as_str().unwrap().into(),
            ));
        }
        for l in gold["graph"]["low"].as_array().unwrap() {
            raw.low
                .insert((l[0].as_str().unwrap().into(), l[1].as_str().unwrap().into()));
        }
        for case in gold["cases"].as_array().unwrap() {
            let (r, b) = (
                case["restricted"].as_bool().unwrap(),
                case["bin_pub"].as_bool().unwrap(),
            );
            let mut g = Graph {
                nodes: raw.nodes.clone(),
                edges: raw.edges.clone(),
                low: raw.low.clone(),
                ..Graph::default()
            };
            g.prepare_model(r, b);
            let roots: BTreeMap<String, String> = case["roots"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().into()))
                .collect();
            assert_eq!(g.roots, roots, "roots {r} {b}");
            assert_eq!(sorted(g.trait_fns.clone()), strs(&case["trait_fns"]));
            assert_eq!(
                sorted(model_dead(&g, false)),
                strs(&case["dead"]),
                "dead {r} {b}"
            );
            assert_eq!(
                sorted(model_dead(&g, true)),
                strs(&case["dead_strict"]),
                "strict {r} {b}"
            );
            let dead_rustc: HashSet<String> = strs(&case["dead_rustc"]).into_iter().collect();
            for (key, strict) in [("reach", false), ("reach_strict", true)] {
                let info = reach(&g, strict, &dead_rustc);
                let want = case[key].as_object().unwrap();
                assert_eq!(info.len(), want.len(), "{key}");
                for (n, w) in want {
                    let i = &info[n];
                    assert_eq!(i.parent.as_deref(), w[0].as_str(), "{key} {n}");
                    assert_eq!(
                        (i.edge.as_str(), i.low, i.cause.as_deref()),
                        (
                            w[1].as_str().unwrap(),
                            w[2].as_bool().unwrap(),
                            w[3].as_str()
                        ),
                        "{key} {n}"
                    );
                }
            }
            let info = reach(&g, false, &dead_rustc);
            for (n, w) in case["chains"].as_object().unwrap() {
                let want: Vec<(String, String)> = w
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into()))
                    .collect();
                assert_eq!(chain(&info, n, 6), want, "chain {n}");
            }
            let ix = build_index(&g);
            for (k, w) in case["map"].as_object().unwrap() {
                let (f, rest) = k.split_once(':').unwrap();
                let (l, name) = rest.split_once(':').unwrap();
                assert_eq!(
                    map_item(&ix, f, l.parse().unwrap(), name, 10).as_deref(),
                    w.as_str(),
                    "{k}"
                );
            }
        }
    }

    /// The Python model must agree with the CLI's own `dead` (the harness's `selfcheck`).
    /// Skipped without a binary (`$FERROGRAPH` or `cargo build`).
    #[test]
    fn model_agrees_with_cli_on_synthetic_corpus() {
        let Some(bin) = crate::testutil::binary() else {
            return;
        };
        let root = std::fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accuracy/synthetic"),
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("s.db");
        for extra in [
            vec![],
            vec![
                "--no-restricted-vis-not-root".to_string(),
                "--no-bin-pub-not-root".to_string(),
            ],
        ] {
            let fg = Fg::new(&bin, &extra);
            assert!(fg.version().unwrap().starts_with('v'));
            fg.index(&root, &db, Duration::from_secs(300)).unwrap();
            let mut g = fg.load_graph(&db).unwrap();
            assert!(g.nodes.len() > 20 && !g.edges.is_empty());
            for strict in [false, true] {
                let (r, b) = fg.model_flags(strict);
                g.prepare_model(r, b);
                assert_eq!(
                    model_dead(&g, strict),
                    fg.dead(&db, strict).unwrap(),
                    "{extra:?} strict={strict}"
                );
            }
        }
    }
}
