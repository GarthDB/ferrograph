//! Part A: measure `ferrograph dead` against rustc's `dead_code` lint (port of `groundtruth.py`).

use crate::corpus::{self, Corpus};
use crate::graph::{
    build_index, canon, chain, map_item, model_dead, reach, split_id, Fg, Graph, Info,
};
use crate::rustc::{collect_ground_truth, run_rustc};
use crate::source::{cfg_gated, text_refs, visibility};
use crate::stats::{frac, frac_ci};
use crate::util::{head, platform, pyf, round1, rustc_version, tail, today, Counter};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_CARGO_ARGS: [&str; 3] = ["--workspace", "--all-targets", "--all-features"];
pub const MODES: [&str; 2] = ["default", "strict"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub binary: String,
    #[serde(default)]
    pub rustc: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unmapped {
    pub file: String,
    pub line: u64,
    pub name: String,
    pub reason: String,
    #[serde(rename = "macro")]
    pub is_macro: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RItem {
    pub file: String,
    pub line: u64,
    pub name: String,
    pub class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Miss {
    pub id: String,
    pub file: String,
    pub line: u64,
    pub name: String,
    pub class: String,
    pub bucket: String,
    pub cause: Option<String>,
    pub collateral: bool,
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FpRec {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: u64,
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_refs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Fp {
    pub not_judgeable_file: Vec<FpRec>,
    pub not_judgeable_cfg: Vec<FpRec>,
    pub rustc_live_some_unit: Vec<FpRec>,
    pub disagree: Vec<FpRec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeResult {
    #[serde(rename = "F")]
    pub found: Vec<String>,
    pub misses: Vec<Miss>,
    pub fp: Fp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfCheck {
    pub cli: u64,
    pub model: u64,
    pub diff: u64,
}

/// One corpus' result; the JSON field names are the ones `groundtruth.py` wrote.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorpusResult {
    pub name: String,
    pub ok: bool,
    #[serde(default)]
    pub heldout: bool,
    #[serde(default)]
    pub cohort: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub fallback: String,
    #[serde(default)]
    pub cargo_seconds: f64,
    #[serde(default)]
    pub index_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub fn_nodes: u64,
    #[serde(default)]
    pub rustc_items: u64,
    #[serde(default)]
    pub r_mapped: u64,
    #[serde(default)]
    pub r_unmapped: Vec<Unmapped>,
    #[serde(default)]
    pub rustc_partial: u64,
    #[serde(default)]
    pub modes: BTreeMap<String, ModeResult>,
    #[serde(default)]
    pub selfcheck: BTreeMap<String, SelfCheck>,
    #[serde(default, rename = "R")]
    pub r: BTreeMap<String, RItem>,
    #[serde(default)]
    pub classes: BTreeMap<String, String>,
    #[serde(default, rename = "roots_of_R")]
    pub roots_of_r: BTreeMap<String, Option<String>>,
    #[serde(default, rename = "F_class")]
    pub f_class: BTreeMap<String, String>,
}

impl CorpusResult {
    /// `cohort` if recorded, else held-out / original.
    fn cohort_or_default(&self) -> &str {
        self.cohort
            .as_deref()
            .unwrap_or(if self.heldout { "held-out" } else { "original" })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Data {
    pub meta: Meta,
    pub results: Vec<CorpusResult>,
}

pub struct Opts {
    pub timeout: Duration,
    pub max_check: usize,
}

/// Map a `reach` cause to a report bucket.
fn bucket(cause: Option<&str>) -> String {
    let Some(cause) = cause else {
        return "other".into();
    };
    if let Some(k) = cause.strip_prefix("root:") {
        return match k {
            "pub" => "pub root",
            "trait_impl" | "trait_def_method" => "trait-impl root",
            _ => "test/entry/export attribute root",
        }
        .into();
    }
    let (edge, src) = cause.split_once('@').unwrap_or((cause, ""));
    if edge == "calls(low-confidence)" {
        return "low-confidence call edge".into();
    }
    let place = if src == "private" {
        "non-root code"
    } else {
        "root item"
    };
    if edge == "expands_to" {
        return "macro/attribute edge (expands_to)".into();
    }
    format!("{edge} edge from {place}")
}

/// `bucket` plus, for pub roots, the source-level visibility of the root that kept `nid` alive.
pub fn miss_bucket(
    g: &Graph,
    info: &HashMap<String, Info>,
    nid: &str,
    index_root: &Path,
) -> Result<String> {
    let mut bkt = bucket(
        info.get(nid)
            .with_context(|| format!("{nid} not reachable in the model"))?
            .cause
            .as_deref(),
    );
    if bkt == "pub root" {
        let mut origin = nid;
        while let Some(p) = info[origin].parent.as_deref() {
            if g.roots.contains_key(origin) {
                break;
            }
            origin = p;
        }
        bkt = format!("{bkt}: {}", visibility(index_root, origin));
    }
    Ok(bkt)
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn analyse(c: &Corpus, fg: &Fg, work: &Path, opts: &Opts) -> Result<CorpusResult> {
    let mut res = CorpusResult {
        name: c.name.clone(),
        heldout: c.heldout,
        cohort: Some(c.cohort().to_string()),
        url: c.url.clone(),
        path: c.path.clone(),
        git_ref: c.git_ref.clone(),
        ..CorpusResult::default()
    };
    let prep = corpus::prepare(c, work)?;
    res.sha = prep.sha;
    res.dirty = prep.dirty;
    let root = prep.root;
    let index_root = std::fs::canonicalize(root.join(c.index_path.as_deref().unwrap_or(".")))?;
    let cargo_args: Vec<String> = c
        .cargo_args
        .clone()
        .unwrap_or_else(|| DEFAULT_CARGO_ARGS.map(String::from).to_vec());
    let target = work.join("target").join(c.group());
    let t0 = Instant::now();
    let rc = run_rustc(&root, &cargo_args, &target, opts.timeout)?;
    res.command = rc.command;
    res.fallback = rc.note;
    res.cargo_seconds = round1(t0.elapsed().as_secs_f64());
    let Some(msgs) = rc.msgs else {
        res.error = Some(format!("cargo check failed: {}", rc.err));
        return Ok(res);
    };
    let gt = collect_ground_truth(&msgs, &root, &index_root)?;
    let db = work.join(format!("{}.db", c.name));
    let t0 = Instant::now();
    fg.index(&index_root, &db, Duration::from_secs(900))?;
    res.index_seconds = round1(t0.elapsed().as_secs_f64());
    let mut g = fg.load_graph(&db)?;
    g.prepare_model(fg.model_flags(false));
    let by_file = build_index(&g);
    res.fn_nodes = g.nodes.values().filter(|n| n.kind == "function").count() as u64;

    // ground truth: dead in every unit that compiles the file
    let (mut r_items, mut partial) = (BTreeMap::<String, RItem>::new(), Vec::new());
    for ((f, line, name), it) in &gt.items {
        let units = gt.units.get(f).copied().unwrap_or(0);
        if units == 0 || it.reports < units {
            partial.push((f, *line, name));
            continue;
        }
        match map_item(&by_file, f, *line as usize, name, 10) {
            None => {
                let reason = if by_file.contains_key(f) {
                    "no fn node with that name within 10 lines"
                } else {
                    "file has no indexed fn nodes"
                };
                res.r_unmapped.push(Unmapped {
                    file: f.clone(),
                    line: *line,
                    name: name.clone(),
                    reason: reason.into(),
                    is_macro: it.is_macro,
                });
            }
            Some(nid) => {
                let class = gt.file_class.get(f).copied().unwrap_or("bin").to_string();
                r_items.insert(
                    nid,
                    RItem {
                        file: f.clone(),
                        line: *line,
                        name: name.clone(),
                        class,
                    },
                );
            }
        }
    }
    let partial_nodes: HashSet<String> = partial
        .iter()
        .filter_map(|(f, line, name)| map_item(&by_file, f, *line as usize, name, 10))
        .collect();
    res.rustc_items = gt.items.len() as u64;
    res.r_mapped = r_items.len() as u64;
    res.rustc_partial = partial.len() as u64;
    let rset: HashSet<String> = r_items.keys().cloned().collect();
    let fclass =
        |f: &str, default: &str| gt.file_class.get(f).copied().unwrap_or(default).to_string();

    for mode in MODES {
        let strict = mode == "strict";
        let found = fg.dead(&db, strict)?;
        // `--strict` turns both root-demoting options off unless asked for, so the model's roots differ per mode.
        g.prepare_model(fg.model_flags(strict));
        let mdead = model_dead(&g, strict);
        res.selfcheck.insert(
            mode.into(),
            SelfCheck {
                cli: found.len() as u64,
                model: mdead.len() as u64,
                diff: found.symmetric_difference(&mdead).count() as u64,
            },
        );
        let info = reach(&g, strict, &rset);
        let mut misses = Vec::new();
        let mut absent: Vec<&String> = rset.difference(&found).collect();
        absent.sort();
        for nid in absent {
            let i = info
                .get(nid)
                .with_context(|| format!("{nid} not reachable in the model"))?;
            let it = &r_items[nid];
            misses.push(Miss {
                id: nid.clone(),
                file: it.file.clone(),
                line: it.line,
                name: it.name.clone(),
                class: it.class.clone(),
                bucket: miss_bucket(&g, &info, nid, &index_root)?,
                cause: i.cause.clone(),
                collateral: i.parent.as_ref().is_some_and(|p| rset.contains(p)),
                chain: chain(&info, nid, 6)
                    .into_iter()
                    .map(|(n, ek)| format!("{n} ({ek})"))
                    .collect(),
            });
        }
        let mut fp = Fp::default();
        let mut extra: Vec<&String> = found.difference(&rset).collect();
        extra.sort();
        for nid in extra {
            let (f, line, _) = split_id(nid);
            let rec = FpRec {
                id: nid.clone(),
                name: canon(&g.nodes[nid].payload).to_string(),
                file: f.clone(),
                line: line as u64,
                class: fclass(&f, "none"),
                text_refs: None,
            };
            if !gt.units.contains_key(&f) {
                fp.not_judgeable_file.push(rec);
            } else if g.attrs.get(nid).is_some_and(|a| a.contains("attr::cfg"))
                || cfg_gated(&index_root, &f, line)
            {
                fp.not_judgeable_cfg.push(rec);
            } else if partial_nodes.contains(nid) {
                fp.rustc_live_some_unit.push(rec);
            } else {
                fp.disagree.push(rec);
            }
        }
        for rec in fp.disagree.iter_mut().take(opts.max_check) {
            rec.text_refs = Some(text_refs(
                &index_root,
                &rec.name,
                &rec.file,
                rec.line as usize,
                6,
            ));
        }
        let mut sorted: Vec<String> = found.into_iter().collect();
        sorted.sort();
        res.modes.insert(
            mode.into(),
            ModeResult {
                found: sorted,
                misses,
                fp,
            },
        );
    }
    res.classes = r_items
        .iter()
        .map(|(n, r)| (n.clone(), r.class.clone()))
        .collect();
    // `g.roots` here are the ones of the last (strict) preparation, as in the Python.
    res.roots_of_r = r_items
        .keys()
        .map(|n| (n.clone(), g.roots.get(n).cloned()))
        .collect();
    res.f_class = MODES
        .iter()
        .flat_map(|m| res.modes[*m].found.iter())
        .map(|n| (n.clone(), fclass(&split_id(n).0, "none")))
        .collect();
    res.r = r_items;
    res.ok = true;
    Ok(res)
}

// ---------------------------------------------------------------- report

#[derive(Default)]
struct Metrics {
    r: u64,
    r_eligible: u64,
    f: u64,
    hit: u64,
    f_not_judgeable: u64,
}

fn metrics(results: &[&CorpusResult], cls: &str, mode: &str) -> Metrics {
    let mut m = Metrics::default();
    for r in results.iter().filter(|r| r.ok) {
        let found: HashSet<&str> = r.modes[mode].found.iter().map(String::as_str).collect();
        let rs: HashSet<&str> = r
            .classes
            .iter()
            .filter(|(_, c)| cls == "all" || *c == cls)
            .map(|(n, _)| n.as_str())
            .collect();
        let fc: HashSet<&str> = found
            .iter()
            .copied()
            .filter(|n| cls == "all" || r.f_class.get(*n).is_some_and(|c| c == cls))
            .collect();
        let fp = &r.modes[mode].fp;
        let nj: HashSet<&str> = fp
            .not_judgeable_file
            .iter()
            .chain(&fp.not_judgeable_cfg)
            .map(|x| x.id.as_str())
            .collect();
        m.r += rs.len() as u64;
        m.r_eligible += rs
            .iter()
            .filter(|n| r.roots_of_r.get(**n).is_some_and(Option::is_none))
            .count() as u64;
        m.f += fc.len() as u64;
        m.hit += fc.intersection(&rs).count() as u64;
        m.f_not_judgeable += fc.intersection(&nj).count() as u64;
    }
    m
}

#[allow(clippy::too_many_lines)]
pub fn render(results: &[CorpusResult], meta: &Meta, max_check: usize) -> String {
    let mut l: Vec<String> = Vec::new();
    let mut w = |s: String| l.push(s);
    w("# ferrograph `dead` vs rustc `dead_code` ground truth\n".into());
    w(format!(
        "- ferrograph: `{}` (`{}`)",
        meta.version, meta.binary
    ));
    if !meta.extra_args.is_empty() {
        w(format!(
            "- extra `dead` args for every row: `{}`",
            meta.extra_args.join(" ")
        ));
    }
    w(format!(
        "- rustc: `{}`; platform: `{}`; date: {}",
        meta.rustc, meta.platform, meta.date
    ));
    w("- ground truth: functions/methods/associated functions that rustc reports as `dead_code` in **every** compilation unit \
       that includes their file (lib, bin, test, example, bench units of `--all-targets`), with `--force-warn dead_code`; \
       dependencies dropped.\n"
        .into());
    w("## Corpora\n".into());
    w("| corpus | source | commit | fn nodes | rustc fn items (all units) | R mapped | R unmapped | dead in some unit only | cargo s | index s |".into());
    w("|---|---|---|---|---|---|---|---|---|---|".into());
    for r in results {
        let src = r
            .url
            .clone()
            .or_else(|| r.path.clone())
            .unwrap_or_else(|| "None".into());
        let sha = format!(
            "`{}`{}",
            head(r.sha.as_deref().unwrap_or("?"), 12),
            if r.dirty { " (dirty tree)" } else { "" }
        );
        if !r.ok {
            w(format!(
                "| {} | {src} | {sha} | — | — | — | — | — | — | — |",
                r.name
            ));
            continue;
        }
        w(format!(
            "| {} | {src} | {sha} | {} | {} | {} | {} | {} | {} | {} |",
            r.name,
            r.fn_nodes,
            r.rustc_items,
            r.r_mapped,
            r.r_unmapped.len(),
            r.rustc_partial,
            pyf(r.cargo_seconds),
            pyf(r.index_seconds)
        ));
    }
    w(String::new());
    for r in results {
        w(format!("### {}\n", r.name));
        let fb = if r.fallback.is_empty() {
            String::new()
        } else {
            format!("  \n(**fallback used**: {})", r.fallback)
        };
        w(format!("`{}`{fb}", r.command));
        if !r.ok {
            w(format!(
                "\n**Not measured**: {}\n",
                r.error.as_deref().unwrap_or("")
            ));
            continue;
        }
        let sc: Vec<String> = MODES
            .iter()
            .map(|m| {
                let s = &r.selfcheck[*m];
                format!("{m}: cli={} model={} diff={}", s.cli, s.model, s.diff)
            })
            .collect();
        w(format!(
            "\nSelf-check (model of `dead` vs CLI): {}\n",
            sc.join(", ")
        ));
        if !r.r_unmapped.is_empty() {
            let cnt = Counter::new(r.r_unmapped.iter().map(|u| u.reason.as_str()));
            let s: Vec<String> = cnt.0.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            w(format!(
                "Unmapped rustc items (indexing misses): {}\n",
                s.join("; ")
            ));
        }
    }
    let ok: Vec<&CorpusResult> = results.iter().filter(|r| r.ok).collect();
    w("## Recall and precision\n".into());
    w("R = rustc-dead fns mapped to nodes; F = ferrograph dead set. \
       **Recall (all R)** = |F∩R|/|R|. **Recall (eligible)** = |F∩R| / |R minus items ferrograph treats as roots by policy \
       (`pub`, trait impls, tests, attributes)|. **Precision raw** = |F∩R|/|F|. **Precision judgeable** = |F∩R| / |F minus items \
       rustc cannot judge (file never compiled, or under `#[cfg]`)|. Percentages are floored; counts given; brackets are Wilson 95% intervals. Cohorts (`original`, `held-out`, and any `cohort =` set in corpora.toml) are pooled separately; held-out ones were not used while tuning.\n"
        .into());
    w("| corpus | class | mode | R | R eligible | F | F∩R | recall (all R) | recall (eligible) | precision raw | F not judgeable | F judgeable | precision judgeable |".into());
    w("|---|---|---|---|---|---|---|---|---|---|---|---|---|".into());
    let mut cohorts: Vec<&str> = Vec::new();
    for r in &ok {
        if !cohorts.contains(&r.cohort_or_default()) {
            cohorts.push(r.cohort_or_default());
        }
    }
    let mut groups: Vec<(String, Vec<&CorpusResult>)> = ok
        .iter()
        .map(|r| {
            let suffix = match r.cohort.as_deref() {
                Some(c) if c != "original" => format!(" ({c})"),
                _ => String::new(),
            };
            (format!("{}{suffix}", r.name), vec![*r])
        })
        .collect();
    groups.extend(cohorts.iter().map(|co| {
        (
            format!("**pooled ({co})**"),
            ok.iter()
                .filter(|r| r.cohort_or_default() == *co)
                .copied()
                .collect(),
        )
    }));
    for (name, rs) in &groups {
        for cls in ["lib", "bin", "all"] {
            for mode in MODES {
                let m = metrics(rs, cls, mode);
                if m.r == 0 && m.f == 0 {
                    continue;
                }
                let fj = m.f - m.f_not_judgeable;
                w(format!(
                    "| {name} | {cls} | {mode} | {} | {} | {} | {} | {} | {} | {} | {} | {fj} | {} |",
                    m.r, m.r_eligible, m.f, m.hit, frac_ci(m.hit, m.r), frac(m.hit, m.r_eligible), frac_ci(m.hit, m.f),
                    m.f_not_judgeable, frac_ci(m.hit, fj)
                ));
            }
        }
    }
    w(String::new());
    w("## False negatives (rustc-dead, ferrograph-live): why they are kept alive\n".into());
    w("Cause = how the item first became reachable in a BFS from ferrograph's roots (strict edges first). \
       *collateral* = reached through another rustc-dead node, so it inherits that node's cause.\n"
        .into());
    for mode in MODES {
        for cls in ["lib", "bin"] {
            let ms: Vec<&Miss> = ok
                .iter()
                .flat_map(|r| r.modes[mode].misses.iter())
                .filter(|x| x.class == cls)
                .collect();
            if ms.is_empty() {
                continue;
            }
            w(format!("### {mode}, {cls} crates ({} missed)\n", ms.len()));
            let cnt = Counter::new(ms.iter().map(|x| x.bucket.as_str()));
            let col = Counter::new(
                ms.iter()
                    .filter(|x| x.collateral)
                    .map(|x| x.bucket.as_str()),
            );
            w("| bucket | count | of which collateral |".into());
            w("|---|---|---|".into());
            let common = cnt.most_common();
            for (b, n) in &common {
                w(format!("| {b} | {n} | {} |", col.get(b)));
            }
            w(String::new());
            for (b, _) in &common {
                let mut in_bucket: Vec<&&Miss> = ms.iter().filter(|x| &x.bucket == b).collect();
                in_bucket.sort_by_key(|x| x.collateral);
                for x in in_bucket.into_iter().take(3) {
                    let ch: Vec<&str> = x.chain.iter().take(4).map(String::as_str).collect();
                    w(format!(
                        "- **{b}**: `{}:{}` `{}`: {}",
                        x.file,
                        x.line,
                        x.name,
                        ch.join(" ← ")
                    ));
                }
            }
            w(String::new());
        }
    }
    w("## False positives: ferrograph-dead, rustc-not-dead\n".into());
    w("| mode | corpus | F | policy: file not compiled | policy: cfg-gated | rustc-live in some unit only | genuine disagreement | of which only dead under `--strict` |".into());
    w("|---|---|---|---|---|---|---|---|".into());
    for mode in MODES {
        for r in &ok {
            let fp = &r.modes[mode].fp;
            let dflt: HashSet<&str> = r.modes["default"]
                .found
                .iter()
                .map(String::as_str)
                .collect();
            let only_strict = if mode == "strict" {
                fp.disagree
                    .iter()
                    .filter(|x| !dflt.contains(x.id.as_str()))
                    .count()
                    .to_string()
            } else {
                "-".into()
            };
            w(format!(
                "| {mode} | {} | {} | {} | {} | {} | {} | {only_strict} |",
                r.name,
                r.modes[mode].found.len(),
                fp.not_judgeable_file.len(),
                fp.not_judgeable_cfg.len(),
                fp.rustc_live_some_unit.len(),
                fp.disagree.len()
            ));
        }
    }
    w(format!(
        "\nGenuine disagreements (default mode; text references for hand-checking are in the JSON, first {max_check} per corpus):\n"
    ));
    for r in &ok {
        for x in &r.modes["default"].fp.disagree {
            w(format!("- {} `{}:{}` `{}`", r.name, x.file, x.line, x.name));
        }
    }
    w(String::new());
    l.join("\n")
}

// ---------------------------------------------------------------- command

#[allow(clippy::struct_field_names)]
pub struct Args {
    pub config: PathBuf,
    pub ferrograph: String,
    pub work: Option<PathBuf>,
    pub only: String,
    pub timeout_secs: u64,
    pub extra_args: String,
    pub max_check: usize,
    pub render_json: Option<PathBuf>,
    pub out_md: PathBuf,
    pub out_json: PathBuf,
    /// Print the Markdown report to stdout.
    pub print_report: bool,
}

/// Measure every selected corpus; one that cannot be measured is reported, not fatal.
pub fn measure(
    corpora: &[Corpus],
    fg: &Fg,
    work: &Path,
    opts: &Opts,
    only: &BTreeSet<&str>,
) -> Vec<CorpusResult> {
    let mut results = Vec::new();
    for c in corpora {
        if !only.is_empty() && !only.contains(c.name.as_str()) {
            continue;
        }
        eprintln!("== {}", c.name);
        results.push(analyse(c, fg, work, opts).unwrap_or_else(|e| CorpusResult {
            name: c.name.clone(),
            ok: false,
            error: Some(tail(&format!("{e:#}"), 1500)),
            command: "-".into(),
            url: c.url.clone(),
            path: c.path.clone(),
            ..CorpusResult::default()
        }));
    }
    results
}

pub fn main(a: &Args) -> Result<()> {
    if let Some(p) = &a.render_json {
        let d: Data = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        std::fs::write(&a.out_md, render(&d.results, &d.meta, a.max_check))?;
        return Ok(());
    }
    let work = match &a.work {
        Some(w) => w.clone(),
        None => tempfile::Builder::new().prefix("fg-acc-").tempdir()?.keep(),
    };
    std::fs::create_dir_all(&work)?;
    eprintln!("work dir: {}", work.display());
    let corpora = corpus::load(&a.config)?;
    let only: BTreeSet<&str> = a.only.split(',').filter(|s| !s.is_empty()).collect();
    let extra = shlex::split(&a.extra_args).context("bad --extra-args quoting")?;
    let fg = Fg::new(&a.ferrograph, &extra);
    let meta = Meta {
        version: fg.version()?,
        binary: a.ferrograph.clone(),
        rustc: rustc_version()?,
        platform: platform(),
        date: today()?,
        extra_args: fg.extra.clone(),
    };
    let opts = Opts {
        timeout: Duration::from_secs(a.timeout_secs),
        max_check: a.max_check,
    };
    let results = measure(&corpora, &fg, &work, &opts, &only);
    let md = render(&results, &meta, a.max_check);
    std::fs::write(&a.out_md, &md)?;
    std::fs::write(
        &a.out_json,
        serde_json::to_string_pretty(&Data { meta, results })?,
    )?;
    if a.print_report {
        println!("{md}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{binary, synthetic};
    use std::sync::OnceLock;

    #[test]
    fn render_matches_golden_report() {
        let d: Data =
            serde_json::from_str(include_str!("../tests/golden/groundtruth-input.json")).unwrap();
        assert_eq!(
            render(&d.results, &d.meta, 25),
            include_str!("../tests/golden/groundtruth-report.md")
        );
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket(None), "other");
        assert_eq!(bucket(Some("root:pub")), "pub root");
        assert_eq!(bucket(Some("root:trait_def_method")), "trait-impl root");
        assert_eq!(
            bucket(Some("root:main")),
            "test/entry/export attribute root"
        );
        assert_eq!(
            bucket(Some("calls(low-confidence)@main")),
            "low-confidence call edge"
        );
        assert_eq!(
            bucket(Some("expands_to@private")),
            "macro/attribute edge (expands_to)"
        );
        assert_eq!(
            bucket(Some("calls@private")),
            "calls edge from non-root code"
        );
        assert_eq!(
            bucket(Some("references@pub")),
            "references edge from root item"
        );
    }

    /// Measure the synthetic corpus with the given extra `dead` args.
    fn measure_synthetic(bin: &str, extra: &str) -> (CorpusResult, String) {
        let work = tempfile::tempdir().unwrap();
        let args: Vec<String> = shlex::split(extra).unwrap();
        let fg = Fg::new(bin, &args);
        let c = synthetic();
        let opts = Opts {
            timeout: Duration::from_secs(600),
            max_check: 25,
        };
        let r = measure(&[c], &fg, work.path(), &opts, &BTreeSet::new()).remove(0);
        assert!(r.ok, "{:?}", r.error);
        let meta = Meta {
            extra_args: args,
            ..Meta::default()
        };
        let md = render(std::slice::from_ref(&r), &meta, 25);
        (r, md)
    }

    const NO_OPTS: &str = "--no-restricted-vis-not-root --no-bin-pub-not-root";

    fn base() -> Option<&'static (CorpusResult, String)> {
        static R: OnceLock<(CorpusResult, String)> = OnceLock::new();
        let bin = binary()?;
        Some(R.get_or_init(|| measure_synthetic(&bin, NO_OPTS)))
    }

    fn names(r: &CorpusResult, ids: &[String]) -> BTreeSet<String> {
        ids.iter()
            .filter_map(|n| r.r.get(n))
            .map(|x| x.name.clone())
            .collect()
    }

    #[test]
    fn ground_truth_is_exactly_the_known_dead_fns() {
        let Some((r, _)) = base() else { return };
        let got: BTreeSet<_> = r.r.values().map(|x| x.name.clone()).collect();
        let want = [
            "dead_c",
            "dead_d",
            "dead_m",
            "run",
            "unused_helper",
            "pub_dead",
            "crate_dead",
            "lib_dead",
        ];
        assert_eq!(got, want.iter().map(ToString::to_string).collect());
    }

    #[test]
    fn known_dead_chain_is_found() {
        let Some((r, _)) = base() else { return };
        let found = names(r, &r.modes["default"].found);
        for n in ["dead_c", "dead_d", "dead_m", "unused_helper", "lib_dead"] {
            assert!(found.contains(n), "{n} in {found:?}");
        }
    }

    #[test]
    fn model_matches_cli() {
        let Some((r, _)) = base() else { return };
        for (mode, sc) in &r.selfcheck {
            assert_eq!(sc.diff, 0, "{mode}");
        }
    }

    #[test]
    fn misses_are_explained() {
        let Some((r, _)) = base() else { return };
        let b: BTreeMap<_, _> = r.modes["default"]
            .misses
            .iter()
            .map(|m| (m.name.as_str(), m.bucket.as_str()))
            .collect();
        assert_eq!(b.get("pub_dead"), Some(&"pub root: pub"));
        assert_eq!(
            b.get("crate_dead"),
            Some(&"pub root: pub(crate)/pub(super)")
        );
    }

    #[test]
    fn lib_and_bin_split() {
        let Some((r, _)) = base() else { return };
        let classes: BTreeSet<_> = r.classes.values().map(String::as_str).collect();
        assert_eq!(classes, BTreeSet::from(["bin", "lib"]));
    }

    #[test]
    fn report_renders() {
        let Some((_, md)) = base() else { return };
        assert!(md.contains("pooled"));
    }

    #[test]
    fn extra_args_restricted_vis_not_root() {
        let Some(bin) = binary() else { return };
        let (r, _) = measure_synthetic(&bin, "--restricted-vis-not-root --no-bin-pub-not-root");
        for (mode, sc) in &r.selfcheck {
            assert_eq!(sc.diff, 0, "{mode}"); // the model follows the option
        }
        let found = names(&r, &r.modes["default"].found);
        assert!(found.contains("crate_dead"));
        assert!(!found.contains("pub_dead")); // bare pub stays a root
    }

    #[test]
    fn extra_args_bin_pub_not_root() {
        let Some(bin) = binary() else { return };
        let (r, _) = measure_synthetic(&bin, "--no-restricted-vis-not-root --bin-pub-not-root");
        for (mode, sc) in &r.selfcheck {
            assert_eq!(sc.diff, 0, "{mode}");
        }
        assert!(names(&r, &r.modes["default"].found).contains("pub_dead")); // bare pub in a bin-only package
    }

    #[test]
    fn unmeasurable_corpus_is_reported_not_fatal() {
        let Some(bin) = binary() else { return };
        let work = tempfile::tempdir().unwrap();
        let mut c = synthetic();
        c.path = Some("/nonexistent/corpus".into());
        let opts = Opts {
            timeout: Duration::from_secs(60),
            max_check: 25,
        };
        let r = measure(
            &[c],
            &Fg::new(&bin, &[]),
            work.path(),
            &opts,
            &BTreeSet::new(),
        );
        assert!(!r[0].ok && r[0].error.is_some());
        assert!(render(&r, &Meta::default(), 25).contains("**Not measured**"));
    }
}
