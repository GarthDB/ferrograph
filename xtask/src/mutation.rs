//! Part B: mutation test of `ferrograph pr` (newly dead functions) against rustc (port of `mutation.py`).
//!
//! For one corpus: pick single-line call statements whose callee has exactly one call site, replace
//! the statement with `();` in a throwaway git repo (one commit per mutation, line numbers
//! unchanged), and compare ferrograph's `pr --base HEAD~1 --index-head` newly-dead list with the
//! `dead_code` diagnostics rustc gains. More families (`ref`, `impl`, `caller`) delete other kinds
//! of use sites.

use crate::corpus::{self, copy_tree, Corpus};
use crate::graph::{build_index, canon, chain, map_item, reach, split_id, Fg, Graph};
use crate::groundtruth::{miss_bucket, DEFAULT_CARGO_ARGS};
use crate::run::run;
use crate::rustc::{all_unit_dead, collect_ground_truth, run_rustc, ItemKey};
use crate::source::{cfg_gated, read_lossy};
use crate::stats::frac_ci;
use crate::util::{head, platform, pyf, round1, rustc_version, tail, today, Counter};
use anyhow::{bail, Context, Result};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const FAMILIES: [&str; 4] = ["call", "ref", "impl", "caller"];
const GIT: [&str; 5] = [
    "git",
    "-c",
    "user.email=harness@example.invalid",
    "-c",
    "user.name=harness",
];
const T: Duration = Duration::from_secs(900);

/// (file, first line, last line, replacement): lines `a..=b` of `file` are blanked, line `a` gets `rep`.
pub type Edit = (String, usize, usize, String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cand {
    pub family: String,
    pub callee: String,
    pub name: String,
    pub caller: String,
    pub file: String,
    pub line: usize,
    pub text: String,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePos {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalseNeg {
    pub id: String,
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutRun {
    #[serde(flatten)]
    pub cand: Cand,
    pub pr_seconds: f64,
    pub oracle: Vec<String>,
    pub reported: Vec<String>,
    pub revived: Vec<String>,
    pub tp: Vec<String>,
    pub fp: Vec<FalsePos>,
    #[serde(rename = "fn")]
    pub fns: Vec<FalseNeg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skip {
    #[serde(flatten)]
    pub cand: Cand,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Data {
    pub runs: Vec<MutRun>,
    pub skipped: Vec<Skip>,
    pub base: String,
}

// ---------------------------------------------------------------- candidate finders

fn read_lines(repo: &Path, f: &str) -> Option<Vec<String>> {
    read_lossy(&repo.join(f)).map(|t| t.lines().map(String::from).collect())
}

fn incoming_map(g: &Graph, kinds: &[&str]) -> BTreeMap<String, Vec<(String, String)>> {
    let mut inc: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (f, t, e) in &g.edges {
        if kinds.contains(&e.as_str()) && f != t {
            inc.entry(t.clone())
                .or_default()
                .push((f.clone(), e.clone()));
        }
    }
    inc
}

fn unique_names(g: &Graph) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for n in g.nodes.values().filter(|n| n.kind == "function") {
        *m.entry(canon(&n.payload).to_string()).or_default() += 1;
    }
    m
}

fn functions(g: &Graph) -> Vec<String> {
    g.nodes
        .iter()
        .filter(|(_, n)| n.kind == "function")
        .map(|(i, _)| i.clone())
        .collect()
}

fn name_of(g: &Graph, id: &str) -> String {
    g.nodes
        .get(id)
        .map(|n| canon(&n.payload).to_string())
        .unwrap_or_default()
}

fn indent(s: &str) -> usize {
    s.chars().count() - s.trim_start().chars().count()
}

/// Call statements `[recv.]callee(args);` whose callee has exactly one (high-confidence) call site.
fn find_candidates(g: &Graph, repo: &Path, rng: &mut StdRng, limit: usize) -> Vec<Cand> {
    let mut incoming: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (f, t, e) in &g.edges {
        if e == "calls" && !g.low.contains(&(f.clone(), t.clone())) {
            incoming.entry(t).or_default().push(f);
        }
    }
    let names = unique_names(g);
    let mut starts: HashMap<String, Vec<usize>> = HashMap::new();
    for id in functions(g) {
        let (f, ln, _) = split_id(&id);
        starts.entry(f).or_default().push(ln);
    }
    let mut callees: Vec<&str> = incoming
        .iter()
        .filter(|(n, callers)| {
            callers.iter().collect::<HashSet<_>>().len() == 1 && g.nodes[**n].kind == "function"
        })
        .map(|(n, _)| *n)
        .collect();
    callees.shuffle(rng);
    let mut out = Vec::new();
    for callee in callees {
        let name = name_of(g, callee);
        let caller = incoming[callee][0];
        if caller == callee
            || names[&name] != 1
            || g.nodes.get(caller).is_none_or(|n| n.kind != "function")
        {
            continue;
        }
        let (cf, cl, _) = split_id(caller);
        let end = starts[&cf]
            .iter()
            .filter(|s| **s > cl)
            .min()
            .map_or(cl + 400, |s| *s);
        let Some(lines) = read_lines(repo, &cf) else {
            continue;
        };
        let pat = Regex::new(&format!(
            r"^\s*(?:[A-Za-z_][\w:]*::|[A-Za-z_][\w.()]*\.)?{}(?:::<[^>;{{}}]*>)?\([^;{{}}]*\)\??;\s*$",
            regex::escape(&name)
        ))
        .unwrap();
        for i in cl..end.min(lines.len() + 1) {
            let Some(l) = i.checked_sub(1).and_then(|k| lines.get(k)) else {
                continue;
            };
            if pat.is_match(l)
                && !l.contains('|')
                && !["let ", "return", "//"]
                    .iter()
                    .any(|p| l.trim_start().starts_with(p))
            {
                out.push(Cand {
                    family: "call".into(),
                    callee: callee.to_string(),
                    name: name.clone(),
                    caller: caller.to_string(),
                    file: cf.clone(),
                    line: i,
                    text: l.trim().to_string(),
                    edits: vec![(cf.clone(), i, i, "();".into())],
                });
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Delete a whole use site that is a reference, not a call statement: a `.method(f)` adapter line or
/// a table entry `f,` on a line of its own (the line is blanked, so numbering is stable). The fn has
/// one using fn, uniquely named.
fn find_ref_candidates(g: &Graph, repo: &Path, rng: &mut StdRng, limit: usize) -> Vec<Cand> {
    let (inc, names) = (incoming_map(g, &["calls", "references"]), unique_names(g));
    let mut fns = functions(g);
    fns.shuffle(rng);
    let mut out = Vec::new();
    for fun in fns {
        let name = name_of(g, &fun);
        let srcs = inc.get(&fun).map_or(&[][..], Vec::as_slice);
        if srcs.iter().map(|s| &s.0).collect::<HashSet<_>>().len() != 1 || names[&name] != 1 {
            continue;
        }
        let holder = &srcs[0].0;
        if *holder == fun || !g.nodes.contains_key(holder) {
            continue;
        }
        let (hf, hl, _) = split_id(holder);
        let Some(lines) = read_lines(repo, &hf) else {
            continue;
        };
        let n = regex::escape(&name);
        let pat = Regex::new(&format!(
            r"^\s*(?:\.\w+\((?:[A-Za-z_][\w]*::)*{n}\)|(?:[A-Za-z_][\w]*::)*{n},?)\s*$"
        ))
        .unwrap();
        for i in hl..(hl + 400).min(lines.len() + 1) {
            let Some(l) = i.checked_sub(1).and_then(|k| lines.get(k)) else {
                continue;
            };
            if pat.is_match(l) {
                out.push(Cand {
                    family: "ref".into(),
                    callee: fun.clone(),
                    name: name.clone(),
                    caller: holder.clone(),
                    file: hf.clone(),
                    line: i,
                    text: l.trim().to_string(),
                    edits: vec![(hf.clone(), i, i, String::new())],
                });
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// 1-based line of the `}` closing the first `{` at or after `start` (naive brace count; cargo check validates).
pub fn block_end(lines: &[String], start: usize) -> Option<usize> {
    let strip = Regex::new(r#""(?:[^"\\]|\\.)*"|//.*"#).unwrap();
    let (mut depth, mut seen) = (0i64, false);
    for (i, line) in lines.iter().enumerate().skip(start - 1) {
        let code = strip.replace_all(line, "");
        for ch in code.chars() {
            if ch == '{' {
                depth += 1;
                seen = true;
            } else if ch == '}' {
                depth -= 1;
            }
        }
        if seen && depth <= 0 {
            return Some(i + 1);
        }
        if !seen && code.trim_end().ends_with(';') {
            return None; // bodiless item
        }
    }
    None
}

fn is_attr_line(s: &str) -> bool {
    s.starts_with("#[") || s.starts_with("#![")
}

/// First line of the attributes/doc comments directly above the item at `start` (1-based),
/// including attributes wrapped over several lines (found by walking up from their closing `]`).
pub fn attr_start(lines: &[String], start: usize) -> usize {
    let mut i = start;
    while i > 1 {
        let prev = lines[i - 2].trim();
        if is_attr_line(prev) || prev.starts_with("///") || prev.starts_with("//!") {
            i -= 1;
        } else if prev.ends_with(']') && !prev.ends_with("];") {
            let mut j = i as i64 - 2;
            while j >= 0 && (i as i64 - 2 - j) < 40 && !is_attr_line(lines[j as usize].trim_start())
            {
                j -= 1;
            }
            if j < 0 || !is_attr_line(lines[j as usize].trim_start()) {
                break;
            }
            i = j as usize + 1;
        } else {
            break;
        }
    }
    i
}

/// Delete a whole `impl Trait for T { .. }` block (blanked) whose methods are the only callers of some helper fn.
fn find_impl_candidates(g: &Graph, repo: &Path, rng: &mut StdRng, limit: usize) -> Vec<Cand> {
    let (inc, names) = (incoming_map(g, &["calls"]), unique_names(g));
    let mut helpers = functions(g);
    helpers.shuffle(rng);
    let impl_re = Regex::new(r"^\s*(unsafe\s+)?impl\b").unwrap();
    let mut out = Vec::new();
    for h in helpers {
        let callers: HashSet<&String> = inc.get(&h).into_iter().flatten().map(|c| &c.0).collect();
        let name = name_of(g, &h);
        if callers.len() != 1 || names[&name] != 1 {
            continue;
        }
        let c = *callers.iter().next().expect("one caller");
        if !g
            .attrs
            .get(c)
            .is_some_and(|a| a.contains("attr::trait_impl"))
            || *c == h
        {
            continue;
        }
        let (cf, cl, _) = split_id(c);
        let Some(lines) = read_lines(repo, &cf) else {
            continue;
        };
        let Some(cur) = cl.checked_sub(1).and_then(|k| lines.get(k)) else {
            continue;
        };
        let ind = indent(cur);
        let start = (1..cl)
            .rev()
            .find(|j| impl_re.is_match(&lines[j - 1]) && indent(&lines[j - 1]) < ind);
        let end = start.and_then(|s| block_end(&lines, s));
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        if end < cl {
            continue;
        }
        let a = attr_start(&lines, start);
        out.push(Cand {
            family: "impl".into(),
            callee: h.clone(),
            name,
            caller: c.clone(),
            file: cf.clone(),
            line: start,
            text: format!(
                "{} .. (blanked {a}-{end})",
                head(lines[start - 1].trim(), 80)
            ),
            edits: vec![(cf, a, end, String::new())],
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Delete a whole fn (blanked, with its attributes) that nothing in the repo calls or references and
/// that is the only caller of some helper: the helper's last caller disappears.
fn find_caller_candidates(g: &Graph, repo: &Path, rng: &mut StdRng, limit: usize) -> Vec<Cand> {
    let inc = incoming_map(g, &["calls", "references"]);
    let calls_in = incoming_map(g, &["calls"]);
    let mut out_calls: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (f, t, e) in &g.edges {
        if e == "calls" && f != t {
            out_calls.entry(f).or_default().insert(t);
        }
    }
    let names = unique_names(g);
    let mut fns = functions(g);
    fns.shuffle(rng);
    let mut out = Vec::new();
    for c in fns {
        let entry_root = g
            .roots
            .get(&c)
            .is_some_and(|k| ["main", "test", "bench", "trait_impl"].contains(&k.as_str()));
        if inc.get(&c).is_some_and(|v| !v.is_empty())
            || entry_root
            || g.attrs.get(&c).is_some_and(|a| !a.is_empty())
        {
            continue;
        }
        let only_c: HashSet<&String> = HashSet::from([&c]);
        let sole: Vec<&str> = out_calls
            .get(c.as_str())
            .into_iter()
            .flatten()
            .copied()
            .filter(|h| {
                calls_in
                    .get(*h)
                    .into_iter()
                    .flatten()
                    .map(|x| &x.0)
                    .collect::<HashSet<_>>()
                    == only_c
                    && g.nodes[*h].kind == "function"
                    && names[&name_of(g, h)] == 1
            })
            .collect();
        let Some(helper) = sole.first() else { continue };
        let (cf, cl, _) = split_id(&c);
        let lines = read_lines(repo, &cf);
        let end = lines.as_deref().and_then(|l| block_end(l, cl));
        let (Some(lines), Some(end)) = (lines, end) else {
            continue;
        };
        if end - cl > 80 {
            continue;
        }
        let a = attr_start(&lines, cl);
        // a nested or enclosing-item mismatch would not compile; cargo check filters those
        out.push(Cand {
            family: "caller".into(),
            callee: (*helper).to_string(),
            name: name_of(g, helper),
            caller: c.clone(),
            file: cf.clone(),
            line: cl,
            text: format!("{} .. (blanked {a}-{end})", head(lines[cl - 1].trim(), 80)),
            edits: vec![(cf, a, end, String::new())],
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn find(family: &str, g: &Graph, repo: &Path, rng: &mut StdRng, limit: usize) -> Result<Vec<Cand>> {
    Ok(match family {
        "call" => find_candidates(g, repo, rng, limit),
        "ref" => find_ref_candidates(g, repo, rng, limit),
        "impl" => find_impl_candidates(g, repo, rng, limit),
        "caller" => find_caller_candidates(g, repo, rng, limit),
        f => bail!("unknown family {f} (from {})", FAMILIES.join(", ")),
    })
}

/// Replace line ranges (1-based, inclusive) by one replacement line then blank lines: line count is unchanged.
pub fn apply_edits(root: &Path, edits: &[Edit]) -> Result<()> {
    for (f, a, b, rep) in edits {
        let path = root.join(f);
        let text = std::fs::read_to_string(&path)?;
        let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
        let nl = if lines[a - 1].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let ind: String = if rep.is_empty() {
            String::new()
        } else {
            lines[a - 1]
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect()
        };
        for l in &mut lines[a - 1..*b] {
            *l = nl.to_string();
        }
        if !rep.is_empty() {
            lines[a - 1] = format!("{ind}{rep}{nl}");
        }
        std::fs::write(&path, lines.concat())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- the run

pub struct Opts {
    pub ferrograph: String,
    pub work: PathBuf,
    pub n: usize,
    pub max_attempts: usize,
    pub family: String,
    pub seed: u64,
    pub timeout: Duration,
    pub extra_args: Vec<String>,
}

fn git(repo: &Path, args: &[&str]) -> Result<crate::run::Output> {
    let mut cmd = GIT.to_vec();
    cmd.extend(args);
    run(&cmd, Some(repo), &[], T, true)
}

type Dead = BTreeSet<ItemKey>;

struct Oracle<'a> {
    repo: &'a Path,
    index_root: &'a Path,
    cargo_args: &'a [String],
    target: PathBuf,
    timeout: Duration,
}

impl Oracle<'_> {
    /// Dead-in-all-units fns now, the command that was run and which fallback it needed.
    fn dead(&self) -> Result<(Option<Dead>, String, String)> {
        let rc = run_rustc(self.repo, self.cargo_args, &self.target, self.timeout)?;
        let dead = match rc.msgs {
            None => None,
            Some(m) => Some(all_unit_dead(&collect_ground_truth(
                &m,
                self.repo,
                self.index_root,
            )?)),
        };
        Ok((dead, rc.command, rc.note))
    }
}

pub struct Report {
    pub data: Data,
    pub md: String,
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn run_mutation(c: &Corpus, o: &Opts) -> Result<Report> {
    let work = &o.work;
    std::fs::create_dir_all(work)?;
    let fg = Fg::new(&o.ferrograph, &o.extra_args);
    let prep = corpus::prepare(c, work)?;
    let repo = work.join("mutrepo").join(&c.name);
    if repo.exists() {
        std::fs::remove_dir_all(&repo)?;
    }
    copy_tree(&prep.root, &repo)?;
    git(&repo, &["init", "-q"])?;
    git(&repo, &["add", "-A"])?;
    git(&repo, &["commit", "-q", "-m", "base"])?;
    let base = git(&repo, &["rev-parse", "HEAD"])?
        .stdout
        .trim()
        .to_string();
    let index_root = std::fs::canonicalize(repo.join(c.index_path.as_deref().unwrap_or(".")))?;
    let cargo_args: Vec<String> = c
        .cargo_args
        .clone()
        .unwrap_or_else(|| DEFAULT_CARGO_ARGS.map(String::from).to_vec());
    let oracle = Oracle {
        repo: &repo,
        index_root: &index_root,
        cargo_args: &cargo_args,
        target: work.join("target").join(c.group()).join("mut"),
        timeout: o.timeout,
    };
    let (before, cmd, note) = oracle.dead()?;
    let Some(before) = before else {
        bail!("base does not build: {note}")
    };
    let db = work.join("base.db");
    fg.index(&index_root, &db, T)?;
    let mut g = fg.load_graph(&db)?;
    g.prepare_model(fg.restricted, fg.bin_pub);
    let by_file = build_index(&g);
    let fams: Vec<String> = if o.family == "all" {
        FAMILIES.map(String::from).to_vec()
    } else {
        o.family.split(',').map(String::from).collect()
    };
    let mut cands = Vec::new();
    for fam in &fams {
        // a fresh generator per family, as the Python did
        let found = find(
            fam,
            &g,
            &index_root,
            &mut StdRng::seed_from_u64(o.seed),
            o.max_attempts,
        )?;
        eprintln!("family {fam}: {} candidates", found.len());
        cands.extend(found);
    }
    let n_cands = cands.len();
    eprintln!(
        "{n_cands} candidates; base rustc-dead fns: {}",
        before.len()
    );

    let before_ids: HashSet<Option<String>> = before
        .iter()
        .map(|(f, line, name)| map_item(&by_file, f, *line as usize, name, 10))
        .collect();
    let (mut runs, mut skipped) = (Vec::<MutRun>::new(), Vec::<Skip>::new());
    let mut per_family: HashMap<String, usize> = HashMap::new();
    for cand in cands {
        if per_family.get(&cand.family).copied().unwrap_or(0) >= o.n {
            continue;
        }
        // start every mutation from the pristine base tree (a previous mutation's commit may be checked out)
        git(&repo, &["checkout", "-q", "--detach", &base])?;
        apply_edits(&index_root, &cand.edits)?;
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-am",
                &format!("mutate {}:{}", cand.file, cand.line),
            ],
        )?;
        let (Some(after), _, _) = oracle.dead()? else {
            skipped.push(Skip {
                cand,
                reason: "cargo check failed".into(),
            });
            continue;
        };
        let t0 = Instant::now();
        let mut args = vec![
            fg.bin.as_str(),
            "pr",
            "--base",
            "HEAD~1",
            "--index-head",
            "--format",
            "json",
        ];
        args.extend(fg.extra.iter().map(String::as_str));
        let out = run(&args, Some(&index_root), &[], T, false)?;
        let pr_s = round1(t0.elapsed().as_secs_f64());
        if out.code != 0 {
            skipped.push(Skip {
                cand,
                reason: format!("pr failed: {}", tail(&out.stderr, 300)),
            });
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&out.stdout)?;
        let reported: BTreeSet<String> = v["newly_dead"]
            .as_array()
            .context("no newly_dead")?
            .iter()
            .filter_map(|x| x["id"].as_str().map(String::from))
            .collect();
        let mut oracle_ids: BTreeSet<String> = BTreeSet::new();
        for (f, line, name) in after.difference(&before) {
            oracle_ids.insert(
                map_item(&by_file, f, *line as usize, name, 10)
                    .unwrap_or_else(|| format!("UNMAPPED {f}:{line}:{name}")),
            );
        }
        let tp: Vec<String> = oracle_ids.intersection(&reported).cloned().collect();
        let fn_ids: Vec<String> = oracle_ids.difference(&reported).cloned().collect();
        let mut fp = Vec::new();
        for nid in reported.difference(&oracle_ids) {
            let (f, line, _) = split_id(nid);
            let kind = if before_ids.contains(&Some(nid.clone())) {
                "already dead in rustc before the change"
            } else if cfg_gated(&index_root, &f, line)
                || g.attrs.get(nid).is_some_and(|a| a.contains("attr::cfg"))
            {
                "not judgeable (cfg-gated)"
            } else {
                "rustc says still live"
            };
            fp.push(FalsePos {
                id: nid.clone(),
                name: name_of(&g, nid),
                kind: kind.into(),
            });
        }
        let mut fns = Vec::new();
        if !fn_ids.is_empty() {
            let hdb = work.join("head.db");
            fg.index(&index_root, &hdb, T)?;
            let mut hg = fg.load_graph(&hdb)?;
            hg.prepare_model(fg.restricted, fg.bin_pub);
            let (head_dead, base_dead) = (fg.dead(&hdb, false)?, fg.dead(&db, false)?);
            let info = reach(&hg, false, &fn_ids.iter().cloned().collect());
            for nid in &fn_ids {
                let (bucket, ch) = if nid.starts_with("UNMAPPED") {
                    (
                        "rustc item not mapped to a node (indexing miss)".to_string(),
                        None,
                    )
                } else if head_dead.contains(nid) && base_dead.contains(nid) {
                    ("ferrograph already considered it dead at base".into(), None)
                } else if head_dead.contains(nid) {
                    ("dead in head graph but not reported by pr".into(), None)
                } else if !info.contains_key(nid) {
                    ("other".into(), None)
                } else {
                    let ch = chain(&info, nid, 6)
                        .into_iter()
                        .take(4)
                        .map(|(n, e)| format!("{n} ({e})"))
                        .collect();
                    (miss_bucket(&hg, &info, nid, &index_root)?, Some(ch))
                };
                fns.push(FalseNeg {
                    id: nid.clone(),
                    bucket,
                    chain: ch,
                });
            }
        }
        let n_done = per_family.entry(cand.family.clone()).or_default();
        *n_done += 1;
        eprintln!(
            "[{} {n_done}/{}] {}:{} {}: oracle={} reported={} tp={} {pr_s}s",
            cand.family,
            o.n,
            cand.file,
            cand.line,
            cand.name,
            oracle_ids.len(),
            reported.len(),
            tp.len()
        );
        runs.push(MutRun {
            cand,
            pr_seconds: pr_s,
            oracle: oracle_ids.into_iter().collect(),
            reported: reported.into_iter().collect(),
            revived: {
                let mut r: Vec<String> = before
                    .difference(&after)
                    .map(|k| format!("{}:{}:{}", k.0, k.1, k.2))
                    .collect();
                r.sort();
                r
            },
            tp,
            fp,
            fns,
        });
    }
    git(&repo, &["checkout", "-q", "--detach", &base])?;
    let wt = git(&repo, &["worktree", "list"])?.stdout;
    let meta = ReportMeta {
        version: fg.version()?,
        rustc: rustc_version()?,
        corpus: c.name.clone(),
        sha: prep.sha,
        dirty: prep.dirty,
        seed: o.seed,
        cmd,
        note,
        extra: fg.extra.clone(),
        n: o.n,
        fams,
        n_cands,
    };
    let data = Data {
        runs,
        skipped,
        base,
    };
    let md = render(&data, &meta, &wt);
    Ok(Report { data, md })
}

struct ReportMeta {
    version: String,
    rustc: String,
    corpus: String,
    sha: Option<String>,
    dirty: bool,
    seed: u64,
    cmd: String,
    note: String,
    extra: Vec<String>,
    n: usize,
    fams: Vec<String>,
    n_cands: usize,
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn render(d: &Data, m: &ReportMeta, wt: &str) -> String {
    let runs = &d.runs;
    let tp: usize = runs.iter().map(|r| r.tp.len()).sum();
    let n_or: usize = runs.iter().map(|r| r.oracle.len()).sum();
    let n_rep: usize = runs.iter().map(|r| r.reported.len()).sum();
    let fpc = Counter::new(runs.iter().flat_map(|r| &r.fp).map(|x| x.kind.as_str()));
    let fnc = Counter::new(runs.iter().flat_map(|r| &r.fns).map(|x| x.bucket.as_str()));
    let mut l: Vec<String> = Vec::new();
    let mut w = |s: String| l.push(s);
    w("# ferrograph `pr` mutation test\n".into());
    w(format!(
        "- ferrograph `{}`; {}; {}; {}",
        m.version,
        m.rustc,
        platform(),
        today().unwrap_or_default()
    ));
    w(format!(
        "- corpus `{}` @ `{}`{}; seed {}",
        m.corpus,
        head(m.sha.as_deref().unwrap_or("?"), 12),
        if m.dirty { " (dirty tree)" } else { "" },
        m.seed
    ));
    let note = if m.note.is_empty() {
        "no fallback"
    } else {
        &m.note
    };
    w(format!(
        "- oracle command: `{}` ({note}); newly dead = dead-in-all-units after minus before (function items only)",
        m.cmd
    ));
    let extra = if m.extra.is_empty() {
        "none".to_string()
    } else {
        m.extra.join(" ")
    };
    w(format!(
        "- extra args: `{extra}`; brackets are Wilson 95% intervals"
    ));
    w("- system under test: `ferrograph pr --base HEAD~1 --index-head --format json` on a throwaway git repo (one commit per mutation)".into());
    w(format!(
        "- families: {} (see README); up to {} compiling mutations per family",
        m.fams.join(", "),
        m.n
    ));
    w(format!(
        "- {} candidates found, {} mutations compiled and measured, {} skipped (did not compile / pr failed)\n",
        m.n_cands,
        runs.len(),
        d.skipped.len()
    ));
    w("Scope: rustc cannot judge trait-impl methods or `pub` items of library crates; the comparison covers only items rustc reports \
       (in binaries and tests, `pub` items in private modules are also reported).\n"
        .into());
    let fr = |a: usize, b: usize| frac_ci(a as u64, b as u64);
    if m.fams.len() > 1 {
        w("## Per family\n".into());
        w("| family | mutations | rustc newly dead | reported | TP | recall | precision |".into());
        w("|---|---|---|---|---|---|---|".into());
        for fam in &m.fams {
            let rs: Vec<&MutRun> = runs.iter().filter(|r| &r.cand.family == fam).collect();
            let (t, o, rp) = (
                rs.iter().map(|r| r.tp.len()).sum::<usize>(),
                rs.iter().map(|r| r.oracle.len()).sum::<usize>(),
                rs.iter().map(|r| r.reported.len()).sum::<usize>(),
            );
            w(format!(
                "| {fam} | {} | {o} | {rp} | {t} | {} | {} |",
                rs.len(),
                fr(t, o),
                fr(t, rp)
            ));
        }
        w(String::new());
    }
    w("## Pooled\n".into());
    w(format!(
        "- mutations where rustc reports ≥1 newly dead fn: {}/{}; where ferrograph reports ≥1: {}/{}",
        runs.iter().filter(|r| !r.oracle.is_empty()).count(),
        runs.len(),
        runs.iter().filter(|r| !r.reported.is_empty()).count(),
        runs.len()
    ));
    w(format!(
        "- oracle newly dead: {n_or}; ferrograph reported: {n_rep}; true positives: {tp}"
    ));
    w(format!(
        "- recall: {}; precision (all reports): {}",
        fr(tp, n_or),
        fr(tp, n_rep)
    ));
    let judge = n_rep as u64
        - fpc.get("not judgeable (cfg-gated)")
        - fpc.get("already dead in rustc before the change");
    w(format!(
        "- precision over judgeable reports (excluding cfg-gated and already-dead-before): {}",
        frac_ci(tp as u64, judge)
    ));
    let mut ts: Vec<f64> = runs.iter().map(|r| r.pr_seconds).collect();
    ts.sort_by(f64::total_cmp);
    if let (Some(min), Some(max)) = (ts.first(), ts.last()) {
        w(format!(
            "- `pr` wall time per invocation: min {}s, median {}s, max {}s (includes indexing base in a temp worktree and HEAD in memory)",
            pyf(*min),
            pyf(ts[ts.len() / 2]),
            pyf(*max)
        ));
    }
    w(format!("\nFalse positives by kind: {}", fpc.joined()));
    w(format!("\nFalse negatives by bucket: {}\n", fnc.joined()));
    w("## Per mutation\n".into());
    w(
        "| # | family | mutated statement | callee | oracle | reported | TP | FN | FP | pr s |"
            .into(),
    );
    w("|---|---|---|---|---|---|---|---|---|---|".into());
    for (i, r) in runs.iter().enumerate() {
        let c = &r.cand;
        w(format!(
            "| {} | {} | `{}:{}` `{}` | `{}` | {} | {} | {} | {} | {} | {} |",
            i + 1,
            c.family,
            c.file,
            c.line,
            head(&c.text, 50),
            c.name,
            r.oracle.len(),
            r.reported.len(),
            r.tp.len(),
            r.fns.len(),
            r.fp.len(),
            pyf(r.pr_seconds)
        ));
    }
    w("\n## False negatives (examples)\n".into());
    for r in runs {
        for x in &r.fns {
            let ch = x
                .chain
                .as_ref()
                .filter(|c| !c.is_empty())
                .map_or(String::new(), |c| format!(" ({})", c.join(" ← ")));
            w(format!(
                "- `{}:{}` → `{}`: **{}**{ch}",
                r.cand.file, r.cand.line, x.id, x.bucket
            ));
        }
    }
    w("\n## False positives\n".into());
    for r in runs {
        for x in &r.fp {
            w(format!(
                "- `{}:{}` → `{}` `{}`: {}",
                r.cand.file, r.cand.line, x.id, x.name, x.kind
            ));
        }
    }
    w("\n## Skipped\n".into());
    for s in &d.skipped {
        w(format!(
            "- `{}:{}` `{}`: {}",
            s.cand.file,
            s.cand.line,
            s.cand.name,
            head(&s.reason, 100)
        ));
    }
    w(format!(
        "\n`git worktree list` after the run:\n```\n{}\n```",
        wt.trim()
    ));
    l.join("\n")
}

#[allow(clippy::struct_field_names)]
pub struct Args {
    pub config: PathBuf,
    pub corpus: String,
    pub ferrograph: String,
    pub work: Option<PathBuf>,
    pub n: usize,
    pub max_attempts: usize,
    pub family: String,
    pub seed: u64,
    pub timeout_secs: u64,
    pub extra_args: String,
    pub out_md: PathBuf,
    pub out_json: PathBuf,
}

pub fn main(a: &Args) -> Result<()> {
    let work = match &a.work {
        Some(w) => w.clone(),
        None => tempfile::Builder::new().prefix("fg-mut-").tempdir()?.keep(),
    };
    let c = corpus::load(&a.config)?
        .into_iter()
        .find(|x| x.name == a.corpus)
        .with_context(|| format!("no corpus {} in {}", a.corpus, a.config.display()))?;
    let o = Opts {
        ferrograph: a.ferrograph.clone(),
        work,
        n: a.n,
        max_attempts: a.max_attempts,
        family: a.family.clone(),
        seed: a.seed,
        timeout: Duration::from_secs(a.timeout_secs),
        extra_args: shlex::split(&a.extra_args).context("bad --extra-args quoting")?,
    };
    let r = run_mutation(&c, &o)?;
    std::fs::write(&a.out_md, &r.md)?;
    std::fs::write(&a.out_json, serde_json::to_string_pretty(&r.data)?)?;
    println!("{}", r.md);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{binary, synthetic};

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(String::from).collect()
    }

    #[test]
    fn edits_keep_line_count() {
        let src = "#[inline]\nfn a() {\n    b();\n}\n\nfn c() {}\n";
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), src).unwrap();
        let l = lines(src);
        assert_eq!(attr_start(&l, 2), 1);
        assert_eq!(block_end(&l, 2), Some(4));
        apply_edits(d.path(), &[("a.rs".into(), 1, 4, String::new())]).unwrap();
        let out = std::fs::read_to_string(d.path().join("a.rs")).unwrap();
        assert_eq!(out.matches('\n').count(), src.matches('\n').count());
        assert!(out.contains("fn c() {}") && !out.contains("fn a"));
    }

    #[test]
    fn replacement_keeps_indent_and_blanks_the_rest() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "fn a() {\n    b();\n    c();\n}\n").unwrap();
        apply_edits(d.path(), &[("a.rs".into(), 2, 3, "();".into())]).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.path().join("a.rs")).unwrap(),
            "fn a() {\n    ();\n\n}\n"
        );
    }

    #[test]
    fn attr_start_wrapped_attribute_and_array_tail() {
        let l = lines("const T: [u8; 1] = [\n    1,\n];\n#[derive(\n    A,\n)]\nfn f() {}");
        assert_eq!(attr_start(&l, 7), 4); // stops at the `#[derive(`, keeps the array
        assert_eq!(attr_start(&lines("];\nfn g() {}"), 2), 2);
    }

    #[test]
    fn block_end_ignores_braces_in_strings_and_comments() {
        let l = lines("fn a() {\n    let s = \"}\"; // }\n}\nfn b();");
        assert_eq!(block_end(&l, 1), Some(3));
        assert_eq!(block_end(&l, 4), None); // bodiless
    }

    fn opts(bin: &str, work: &Path, n: usize, family: &str) -> Opts {
        Opts {
            ferrograph: bin.into(),
            work: work.into(),
            n,
            max_attempts: 90,
            family: family.into(),
            seed: 1,
            timeout: Duration::from_secs(600),
            extra_args: vec![],
        }
    }

    #[test]
    fn pr_mutation_finds_orphaned_fn() {
        let Some(bin) = binary() else { return };
        let work = tempfile::tempdir().unwrap();
        let r = run_mutation(&synthetic(), &opts(&bin, work.path(), 10, "call")).unwrap();
        let by: BTreeMap<_, _> = r
            .data
            .runs
            .iter()
            .map(|x| (x.cand.name.as_str(), x))
            .collect();
        // other candidates may appear; only these two are asserted
        for name in ["cleanup", "teardown"] {
            let x = by
                .get(name)
                .unwrap_or_else(|| panic!("{name} missing in {:?}", by.keys()));
            // each mutation starts from the base tree: exactly one newly dead fn each, even in the same file
            assert_eq!(x.oracle.len(), 1, "{name}");
            assert_eq!(x.tp, x.oracle, "{name}");
        }
        assert!(
            r.md.contains("# ferrograph `pr` mutation test") && r.md.contains("git worktree list")
        );
    }

    #[test]
    fn pr_mutation_families() {
        let Some(bin) = binary() else { return };
        let work = tempfile::tempdir().unwrap();
        let r = run_mutation(&synthetic(), &opts(&bin, work.path(), 5, "ref,impl,caller")).unwrap();
        let by: BTreeMap<_, _> = r
            .data
            .runs
            .iter()
            .map(|x| ((x.cand.family.as_str(), x.cand.name.as_str()), x))
            .collect();
        for key in [
            ("ref", "table_entry"),
            ("impl", "greeting_helper"),
            ("caller", "only_by_entry"),
        ] {
            let x = by
                .get(&key)
                .unwrap_or_else(|| panic!("{key:?} missing in {:?}", by.keys()));
            assert_eq!(x.oracle, x.tp, "{key:?}");
            assert_eq!(x.oracle.len(), 1, "{key:?}");
        }
        assert!(r.md.contains("## Per family"));
    }

    #[test]
    fn same_seed_same_candidates() {
        use rand::Rng;
        let a: u32 = StdRng::seed_from_u64(1).gen();
        let b: u32 = StdRng::seed_from_u64(1).gen();
        assert_eq!(a, b);
    }
}
