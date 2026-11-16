//! Replay mode: false-alarm rate of `ferrograph pr` on ordinary commits (port of `replay.py`).
//!
//! For the last N first-parent commits of a corpus, run `ferrograph pr --base <parent> --index-head`
//! at the commit and compare its newly-dead functions with rustc's (dead-in-all-units after minus
//! before). Unlike `mutation` this uses real changes, so it measures how often `pr` cries wolf on
//! normal PRs.

use crate::corpus::{self, Corpus};
use crate::graph::{split_id, Fg};
use crate::groundtruth::DEFAULT_CARGO_ARGS;
use crate::run::run;
use crate::rustc::{all_unit_dead, collect_ground_truth, run_rustc, ItemKey};
use crate::source::cfg_gated;
use crate::stats::frac_ci;
use crate::util::{head, platform, round1, rustc_version, tail, today, Counter};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CFG: &str = "not judgeable (cfg-gated)";
const COMMON: [&str; 21] = [
    "new", "default", "from", "into", "fmt", "eq", "hash", "clone", "drop", "next", "len", "get",
    "set", "run", "main", "build", "write", "read", "parse", "name", "init",
];
const T: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub rustc: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub n: usize,
    #[serde(default)]
    pub depth: usize,
    #[serde(default)]
    pub select: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalseAlarm {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub sha: String,
    pub rs_files: usize,
    pub removed_calls: Vec<String>,
    pub oracle: Vec<ItemKey>,
    pub reported: Vec<(String, String)>,
    pub tp: Vec<String>,
    pub fp: Vec<FalseAlarm>,
    #[serde(rename = "fn")]
    pub missed: Vec<ItemKey>,
    pub pr_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    pub sha: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Outcome {
    pub name: String,
    #[serde(default)]
    pub heldout: bool,
    #[serde(default)]
    pub cohort: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub skipped: Vec<Skipped>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Data {
    pub meta: Meta,
    pub results: Vec<Outcome>,
}

fn git(repo: &Path, args: &[&str], check: bool) -> Result<crate::run::Output> {
    let mut cmd = vec!["git"];
    cmd.extend(args);
    run(&cmd, Some(repo), &[], T, check)
}

/// Own clone with at least `n` commits of history (the shared corpus copy is shallow).
fn clone(c: &Corpus, work: &Path, n: usize) -> Result<PathBuf> {
    let dest = work.join("replay").join(&c.name);
    let url = c.url.as_deref().context("replay needs a url corpus")?;
    let depth = (n + 2).to_string();
    // a directory without the `replay-head` tag is a leftover of an interrupted run
    if dest.exists()
        && git(
            &dest,
            &["rev-parse", "--verify", "-q", "replay-head"],
            false,
        )?
        .code
            != 0
    {
        std::fs::remove_dir_all(&dest)?;
    }
    if dest.exists() {
        let cnt: usize = git(&dest, &["rev-list", "--count", "replay-head"], true)?
            .stdout
            .trim()
            .parse()?;
        let shallow = git(&dest, &["rev-parse", "--is-shallow-repository"], true)?.stdout;
        // an earlier run fetched less history (e.g. default mode, now `--select calls`); a complete history cannot grow
        if cnt < n + 2 && shallow.trim() == "true" {
            let head = git(&dest, &["rev-parse", "replay-head"], true)?.stdout;
            git(
                &dest,
                &["fetch", "-q", "--depth", &depth, "origin", head.trim()],
                true,
            )?;
        }
        return Ok(dest);
    }
    // Clone beside `dest` and rename on success (as `corpus::prepare` does), so a failure or kill
    // mid-clone never leaves a half-made `dest` for a later run to reuse.
    let tmp = dest.with_file_name(format!(".{}.partial", c.name));
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::create_dir_all(&tmp)?;
    let built = (|| -> Result<()> {
        git(&tmp, &["init", "-q"], true)?;
        git(&tmp, &["remote", "add", "origin", url], true)?;
        git(
            &tmp,
            &[
                "fetch",
                "-q",
                "--depth",
                &depth,
                "origin",
                c.git_ref.as_deref().unwrap_or("HEAD"),
            ],
            true,
        )?;
        git(&tmp, &["tag", "replay-head", "FETCH_HEAD"], true)?;
        git(&tmp, &["checkout", "-q", "--detach", "replay-head"], true)?;
        Ok(())
    })();
    if let Err(e) = built {
        std::fs::remove_dir_all(&tmp).ok();
        return Err(e);
    }
    std::fs::rename(&tmp, &dest)?;
    Ok(dest)
}

fn fn_def() -> Regex {
    Regex::new(r"\bfn\s+([A-Za-z_]\w*)").unwrap()
}

/// Names of fns defined in the `.rs` files of `rev` (via `git grep`; common trait-method names dropped).
fn defined_fns(repo: &Path, rev: &str) -> Result<BTreeSet<String>> {
    let o = git(
        repo,
        &[
            "grep",
            "-hoE",
            "fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*",
            rev,
            "--",
            "*.rs",
        ],
        false,
    )?;
    let re = fn_def();
    Ok(o.stdout
        .lines()
        .filter_map(|l| re.captures(l).map(|m| m[1].to_string()))
        .filter(|n| n.chars().count() >= 4 && !COMMON.contains(&n.as_str()))
        .collect())
}

/// Does the commit delete a line that calls or references a fn defined in the repo? Returns the names hit.
fn removes_call(
    repo: &Path,
    parent: &str,
    sha: &str,
    names: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let diff = git(repo, &["diff", "-U0", parent, sha, "--", "*.rs"], false)?.stdout;
    let (def, word) = (fn_def(), Regex::new(r"[A-Za-z_]\w*").unwrap());
    let mut hit = BTreeSet::new();
    for l in diff
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .map(|l| &l[1..])
    {
        if l.trim_start().starts_with("//") || def.is_match(l) {
            continue;
        }
        for m in word.find_iter(l) {
            if names.contains(m.as_str()) {
                let rest = l[m.end()..].trim_start();
                let before = &l[..m.start()];
                if rest.starts_with(['('])
                    || rest.starts_with("::<")
                    || [".", "::", "(", ", ", "& ", "&"]
                        .iter()
                        .any(|s| before.ends_with(s))
                {
                    hit.insert(m.as_str().to_string());
                }
            }
        }
    }
    Ok(hit.into_iter().collect())
}

/// rustc item (file, line, name) vs a ferrograph id `./file#line:col`: same file, the node starts
/// at or up to 10 lines before rustc's line (same rule as `map_item`).
fn matches(item: &ItemKey, node_id: &str) -> bool {
    let (f, line, _) = split_id(node_id);
    item.0 == f && item.1 >= line as u64 && item.1 - (line as u64) <= 10
}

/// rustc's answer depends only on the commit, so it is kept on disk and reused across `--extra-args` arms.
struct Oracle<'a> {
    repo: &'a Path,
    index_root: &'a Path,
    cargo_args: &'a [String],
    target: PathBuf,
    timeout: Duration,
    cache_file: PathBuf,
    cache: BTreeMap<String, Option<BTreeSet<ItemKey>>>,
}

impl Oracle<'_> {
    fn dead_at(&mut self, sha: &str) -> Result<Option<BTreeSet<ItemKey>>> {
        if !self.cache.contains_key(sha) {
            git(self.repo, &["checkout", "-q", "-f", "--detach", sha], true)?;
            let rc = run_rustc(self.repo, self.cargo_args, &self.target, self.timeout)?;
            let dead = match rc.msgs {
                None => None,
                Some(msgs) => Some(all_unit_dead(&collect_ground_truth(
                    &msgs,
                    self.repo,
                    self.index_root,
                )?)),
            };
            self.cache.insert(sha.to_string(), dead);
            std::fs::write(&self.cache_file, serde_json::to_string(&self.cache)?)?;
        }
        Ok(self.cache[sha].clone())
    }
}

pub struct Opts {
    pub n: usize,
    pub depth: usize,
    pub calls: bool,
    pub timeout: Duration,
}

#[allow(clippy::too_many_lines)]
fn replay(c: &Corpus, fg: &Fg, work: &Path, o: &Opts) -> Result<Outcome> {
    let mut outcome = Outcome {
        name: c.name.clone(),
        heldout: c.heldout,
        cohort: c.cohort().into(),
        ..Outcome::default()
    };
    let repo = clone(c, work, o.depth)?;
    let index_root = std::fs::canonicalize(repo.join(c.index_path.as_deref().unwrap_or(".")))?;
    let cargo_args: Vec<String> = c
        .cargo_args
        .clone()
        .unwrap_or_else(|| DEFAULT_CARGO_ARGS.map(String::from).to_vec());
    outcome.head = Some(
        git(&repo, &["rev-parse", "replay-head"], true)?
            .stdout
            .trim()
            .to_string(),
    );
    let revs = git(
        &repo,
        &[
            "rev-list",
            "--first-parent",
            "-n",
            &o.depth.to_string(),
            "replay-head",
        ],
        true,
    )?
    .stdout;
    let fns = if o.calls {
        defined_fns(&repo, "replay-head")?
    } else {
        BTreeSet::new()
    };
    let cache_file = work.join("replay").join(format!("{}.dead.json", c.name));
    let raw: BTreeMap<String, Option<BTreeSet<ItemKey>>> = if cache_file.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cache_file)?)?
    } else {
        BTreeMap::new()
    };
    let mut oracle = Oracle {
        repo: &repo,
        index_root: &index_root,
        cargo_args: &cargo_args,
        target: work.join("target").join(c.group()).join("replay"),
        timeout: o.timeout,
        cache_file,
        cache: raw,
    };
    let (mut runs, mut skipped, mut selected) = (Vec::new(), Vec::new(), 0);
    let skip = |sha: &str, reason: String| Skipped {
        sha: sha.to_string(),
        reason,
    };
    for sha in revs.split_whitespace() {
        if selected >= o.n {
            break;
        }
        let parents = git(&repo, &["rev-list", "--parents", "-n", "1", sha], true)?.stdout;
        let Some(parent) = parents.split_whitespace().nth(1) else {
            continue;
        }; // root of the fetched history
        let changed = git(&repo, &["diff", "--name-only", parent, sha], true)?.stdout;
        let rs = changed.lines().filter(|f| f.ends_with(".rs")).count();
        if rs == 0 {
            skipped.push(skip(sha, "no .rs change".into()));
            continue;
        }
        let removed = if o.calls {
            removes_call(&repo, parent, sha, &fns)?
        } else {
            Vec::new()
        };
        if o.calls && removed.is_empty() {
            continue; // not a candidate: not counted as skipped, not built
        }
        let (Some(before), Some(after)) = (oracle.dead_at(parent)?, oracle.dead_at(sha)?) else {
            skipped.push(skip(sha, "cargo check failed".into()));
            continue;
        };
        git(&repo, &["checkout", "-q", "-f", "--detach", sha], true)?;
        let t0 = Instant::now();
        let mut args = vec![
            fg.bin.as_str(),
            "pr",
            "--base",
            parent,
            "--index-head",
            "--format",
            "json",
        ];
        args.extend(fg.extra.iter().map(String::as_str));
        let out = run(&args, Some(&index_root), &[], T, false)?;
        let pr_s = round1(t0.elapsed().as_secs_f64());
        if out.code != 0 {
            skipped.push(skip(sha, format!("pr failed: {}", tail(&out.stderr, 200))));
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&out.stdout)?;
        let reported: Vec<(String, String)> = v["newly_dead"]
            .as_array()
            .context("no newly_dead")?
            .iter()
            .map(|x| {
                (
                    x["id"].as_str().unwrap_or("").to_string(),
                    x["name"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let found = |set: &BTreeSet<ItemKey>, nid: &str, name: &str| {
            set.iter().any(|o| o.2 == name && matches(o, nid))
        };
        let (mut tp, mut fp) = (Vec::new(), Vec::new());
        for (nid, name) in &reported {
            if after
                .difference(&before)
                .any(|o| o.2 == *name && matches(o, nid))
            {
                tp.push(nid.clone());
                continue;
            }
            let (f, line, _) = split_id(nid);
            let kind = if found(&before, nid, name) {
                "rustc already dead before the commit"
            } else if cfg_gated(&index_root, &f, line) {
                CFG
            } else {
                "rustc says still live"
            };
            fp.push(FalseAlarm {
                id: nid.clone(),
                name: name.clone(),
                kind: kind.into(),
            });
        }
        let oracle_items: Vec<ItemKey> = after.difference(&before).cloned().collect();
        let missed: Vec<ItemKey> = oracle_items
            .iter()
            .filter(|o| !reported.iter().any(|(i, nm)| o.2 == *nm && matches(o, i)))
            .cloned()
            .collect();
        selected += 1; // only measured commits count toward --n; unbuildable ones are skipped above
        eprintln!(
            "[{} {}] {} oracle={} reported={} fp={} {pr_s}s",
            c.name,
            runs.len() + 1,
            head(sha, 8),
            oracle_items.len(),
            reported.len(),
            fp.len()
        );
        runs.push(Run {
            sha: sha.to_string(),
            rs_files: rs,
            removed_calls: removed,
            oracle: oracle_items,
            reported,
            tp,
            fp,
            missed,
            pr_seconds: pr_s,
        });
    }
    outcome.ok = true;
    outcome.runs = runs;
    outcome.skipped = skipped;
    Ok(outcome)
}

// ---------------------------------------------------------------- report

#[derive(Default)]
struct Pool {
    commits: u64,
    oracle_commits: u64,
    alarm_commits: u64,
    commits_with_report: u64,
    reported: u64,
    tp: u64,
    fp: u64,
    oracle: u64,
}

fn pool(rs: &[&Outcome]) -> Pool {
    let runs: Vec<&Run> = rs
        .iter()
        .filter(|r| r.ok)
        .flat_map(|r| r.runs.iter())
        .collect();
    let judged = |x: &Run| x.fp.iter().filter(|f| f.kind != CFG).count() as u64;
    Pool {
        commits: runs.len() as u64,
        oracle_commits: runs.iter().filter(|x| !x.oracle.is_empty()).count() as u64,
        alarm_commits: runs.iter().filter(|x| judged(x) > 0).count() as u64,
        commits_with_report: runs.iter().filter(|x| !x.reported.is_empty()).count() as u64,
        reported: runs.iter().map(|x| x.reported.len() as u64).sum(),
        tp: runs.iter().map(|x| x.tp.len() as u64).sum(),
        fp: runs.iter().map(|x| judged(x)).sum(),
        oracle: runs.iter().map(|x| x.oracle.len() as u64).sum(),
    }
}

fn row(name: &str, p: &Pool) -> String {
    format!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
        p.commits,
        p.oracle_commits,
        p.commits_with_report,
        frac_ci(p.alarm_commits, p.commits),
        p.reported,
        frac_ci(p.tp, p.tp + p.fp),
        p.oracle,
        frac_ci(p.tp, p.oracle)
    )
}

pub fn render(results: &[Outcome], meta: &Meta) -> String {
    let mut l: Vec<String> = Vec::new();
    let mut w = |s: String| l.push(s);
    w("# ferrograph `pr` replay on real commits\n".into());
    w(format!(
        "- ferrograph `{}`; {}; {}; {}",
        meta.version, meta.rustc, meta.platform, meta.date
    ));
    let sel = if meta.select == "calls" {
        format!("selection `calls`: the first {} of the last {} first-parent commits whose diff removes a line calling or referencing a fn defined in the repo", meta.n, meta.depth)
    } else {
        format!("selection `last`: the last {} first-parent commits", meta.n)
    };
    let extra = if meta.extra_args.is_empty() {
        "none".to_string()
    } else {
        meta.extra_args.join(" ")
    };
    w(format!("- extra args: `{extra}`; {sel}"));
    w("- per commit with a `.rs` change: `ferrograph pr --base <parent> --index-head`; oracle = rustc dead-in-all-units after minus before. \
       **False-alarm commit rate** = commits where `pr` reports a newly dead fn that rustc does not confirm (cfg-gated reports excluded) / commits with `.rs` changes. \
       **Precision** = confirmed / (confirmed + unconfirmed judgeable). Brackets are Wilson 95% intervals; \
       commits that did not build are skipped, so this is biased toward buildable history.\n"
        .into());
    w("| corpus | commits | commits with rustc newly dead | commits with a report | false-alarm commits | reported | precision | rustc newly dead | recall |".into());
    w("|---|---|---|---|---|---|---|---|---|".into());
    let ok: Vec<&Outcome> = results.iter().filter(|r| r.ok).collect();
    for r in &ok {
        let suffix = if r.cohort == "original" {
            String::new()
        } else {
            format!(" ({})", r.cohort)
        };
        w(row(&format!("{}{suffix}", r.name), &pool(&[r])));
    }
    let mut cohorts: Vec<&str> = Vec::new();
    for r in &ok {
        if !cohorts.contains(&r.cohort.as_str()) {
            cohorts.push(&r.cohort);
        }
    }
    for co in cohorts {
        let rs: Vec<&Outcome> = ok.iter().filter(|r| r.cohort == co).copied().collect();
        w(row(&format!("**pooled ({co})**"), &pool(&rs)));
    }
    let kinds = Counter::new(
        ok.iter()
            .flat_map(|r| &r.runs)
            .flat_map(|x| &x.fp)
            .map(|f| f.kind.as_str()),
    );
    w(format!("\nUnconfirmed reports by kind: {}", kinds.joined()));
    w("\n## Unconfirmed reports\n".into());
    for r in &ok {
        for x in &r.runs {
            for f in &x.fp {
                w(format!(
                    "- {} `{}` `{}` `{}`: {}",
                    r.name,
                    head(&x.sha, 10),
                    f.id,
                    f.name,
                    f.kind
                ));
            }
        }
    }
    w("\n## Skipped commits\n".into());
    for r in results {
        if !r.ok {
            w(format!(
                "- {}: not measured: {}",
                r.name,
                r.error.as_deref().unwrap_or("")
            ));
            continue;
        }
        let cnt = Counter::new(
            r.skipped
                .iter()
                .map(|s| s.reason.split(':').next().unwrap_or("")),
        );
        w(format!("- {}: {}", r.name, cnt.joined()));
    }
    l.join("\n")
}

#[allow(clippy::struct_field_names)]
pub struct Args {
    pub config: PathBuf,
    pub ferrograph: String,
    pub work: Option<PathBuf>,
    pub only: String,
    pub n: usize,
    pub calls: bool,
    pub depth: Option<usize>,
    pub extra_args: String,
    pub timeout_secs: u64,
    pub out_md: PathBuf,
    pub out_json: PathBuf,
}

pub fn main(a: &Args) -> Result<()> {
    let depth = a.depth.unwrap_or(if a.calls { a.n * 10 } else { a.n });
    let work = match &a.work {
        Some(w) => w.clone(),
        None => tempfile::Builder::new()
            .prefix("fg-replay-")
            .tempdir()?
            .keep(),
    };
    std::fs::create_dir_all(&work)?;
    let fg = Fg::new(
        &a.ferrograph,
        &shlex::split(&a.extra_args).context("bad --extra-args quoting")?,
    );
    let meta = Meta {
        version: fg.version()?,
        rustc: rustc_version()?,
        platform: platform(),
        date: today()?,
        extra_args: fg.extra.clone(),
        n: a.n,
        depth,
        select: if a.calls { "calls" } else { "last" }.into(),
    };
    let only: Vec<&str> = a.only.split(',').filter(|s| !s.is_empty()).collect();
    let opts = Opts {
        n: a.n,
        depth,
        calls: a.calls,
        timeout: Duration::from_secs(a.timeout_secs),
    };
    let mut results = Vec::new();
    for c in corpus::load(&a.config)? {
        if (!only.is_empty() && !only.contains(&c.name.as_str())) || c.url.is_none() {
            continue;
        }
        eprintln!("== {}", c.name);
        // one broken corpus must not hide the others
        results.push(replay(&c, &fg, &work, &opts).unwrap_or_else(|e| Outcome {
            name: c.name.clone(),
            heldout: c.heldout,
            cohort: c.cohort().into(),
            ok: false,
            error: Some(tail(&format!("{e:#}"), 800)),
            ..Outcome::default()
        }));
    }
    let md = render(&results, &meta);
    std::fs::write(&a.out_md, &md)?;
    std::fs::write(
        &a.out_json,
        serde_json::to_string_pretty(&Data { meta, results })?,
    )?;
    println!("{md}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (n, c) in files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        let g = |a: &[&str]| {
            let mut v = vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ];
            v.extend(a);
            git(d.path(), &v, true).unwrap();
        };
        g(&["init", "-q"]);
        g(&["add", "."]);
        g(&["commit", "-qm", "x"]);
        d
    }

    #[test]
    fn matches_within_ten_lines_above() {
        let item: ItemKey = ("./a.rs".into(), 20, "f".into());
        assert!(matches(&item, "./a.rs#20:1") && matches(&item, "./a.rs#10:1"));
        assert!(
            !matches(&item, "./a.rs#9:1")
                && !matches(&item, "./a.rs#21:1")
                && !matches(&item, "./b.rs#20:1")
        );
    }

    #[test]
    fn finds_removed_calls_of_repo_fns() {
        let d = tmp_repo(&[(
            "a.rs",
            "fn helper_one() {}\nfn helper_two() {}\nfn new() {}\nfn main() {\n    helper_one();\n    // helper_two();\n    let x = obj.helper_two(1);\n    new();\n    let y = helper_one;\n}\n",
        )]);
        let names = defined_fns(d.path(), "HEAD").unwrap();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            ["helper_one", "helper_two"]
        ); // `new`, `main` are dropped
        let names: BTreeSet<String> = ["helper_one".to_string(), "helper_two".to_string()].into();
        std::fs::write(
            d.path().join("a.rs"),
            "fn helper_one() {}\nfn helper_two() {}\nfn new() {}\nfn main() {\n}\n",
        )
        .unwrap();
        git(
            d.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qam",
                "y",
            ],
            true,
        )
        .unwrap();
        // the commented-out line is ignored; the call, the method call and the bare reference count
        assert_eq!(
            removes_call(d.path(), "HEAD~1", "HEAD", &names).unwrap(),
            ["helper_one", "helper_two"]
        );
    }

    #[test]
    fn render_summarises_pools_skips_and_failures() {
        let run = |sha: &str, oracle: usize, tp: usize, kinds: &[&str]| Run {
            sha: sha.into(),
            rs_files: 1,
            removed_calls: vec![],
            oracle: (0..oracle)
                .map(|i| ("./a.rs".to_string(), i as u64, "f".to_string()))
                .collect(),
            reported: (0..tp + kinds.len())
                .map(|i| (format!("./a.rs#{i}:1"), "f".into()))
                .collect(),
            tp: (0..tp).map(|i| format!("./a.rs#{i}:1")).collect(),
            fp: kinds
                .iter()
                .map(|k| FalseAlarm {
                    id: "./b.rs#1:1".into(),
                    name: "g".into(),
                    kind: (*k).into(),
                })
                .collect(),
            missed: vec![],
            pr_seconds: 1.5,
        };
        let a = Outcome {
            name: "a".into(),
            cohort: "original".into(),
            ok: true,
            runs: vec![
                run("1234567890abcdef", 2, 2, &[]),
                run("fedcba0987654321", 0, 0, &["rustc says still live", CFG]),
            ],
            skipped: vec![
                Skipped {
                    sha: "x".into(),
                    reason: "no .rs change".into(),
                },
                Skipped {
                    sha: "y".into(),
                    reason: "pr failed: boom".into(),
                },
            ],
            ..Outcome::default()
        };
        let b = Outcome {
            name: "b".into(),
            cohort: "held-out".into(),
            ok: false,
            error: Some("nope".into()),
            ..Outcome::default()
        };
        let meta = Meta {
            version: "v1".into(),
            rustc: "rustc 1".into(),
            platform: "p".into(),
            date: "d".into(),
            n: 5,
            depth: 50,
            select: "calls".into(),
            ..Meta::default()
        };
        let md = render(&[a, b], &meta);
        assert!(md.contains("- extra args: `none`; selection `calls`: the first 5 of the last 50"));
        assert!(md.contains("| a | 2 | 1 | 2 | 50.0% (1/2) [9.5-90.5] | 4 | 66.6% (2/3) [20.8-93.9] | 2 | 100.0% (2/2) [34.2-100.0] |"), "{md}");
        assert!(md.contains(
            "Unconfirmed reports by kind: rustc says still live: 1, not judgeable (cfg-gated): 1"
        ));
        assert!(md.contains("- a `fedcba0987` `./b.rs#1:1` `g`: rustc says still live"));
        assert!(md.contains("- a: no .rs change: 1, pr failed: 1"));
        assert!(md.contains("- b: not measured: nope"));
    }

    #[test]
    fn failed_clone_leaves_nothing_and_a_tagless_leftover_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        let mut c = crate::testutil::synthetic_history(dir.path());
        let good = c.url.clone();
        c.url = Some(format!("file://{}", dir.path().join("missing").display()));
        assert!(clone(&c, &work, 5).is_err());
        assert!(!work.join("replay/hist").exists() && !work.join("replay/.hist.partial").exists());
        // an interrupted earlier run (init done, nothing fetched)
        std::fs::create_dir_all(work.join("replay/hist")).unwrap();
        git(&work.join("replay/hist"), &["init", "-q"], true).unwrap();
        c.url = good;
        let d = clone(&c, &work, 5).unwrap();
        assert!(git(&d, &["rev-parse", "--verify", "replay-head"], true).is_ok());
    }

    #[test]
    fn replays_a_local_history() {
        let Some(bin) = crate::testutil::binary() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let c = crate::testutil::synthetic_history(dir.path());
        let work = dir.path().join("work");
        let fg = Fg::new(&bin, &[]);
        let o = Opts {
            n: 5,
            depth: 5,
            calls: false,
            timeout: Duration::from_secs(600),
        };
        let r = replay(&c, &fg, &work, &o).unwrap();
        assert!(r.ok);
        // newest first; the root commit has no parent in the fetched history
        let shas: Vec<_> = r
            .runs
            .iter()
            .map(|x| (x.oracle.len(), x.reported.len(), x.tp.len(), x.fp.len()))
            .collect();
        assert_eq!(shas, [(0, 0, 0, 0), (1, 1, 1, 0)]);
        assert_eq!(r.runs[1].oracle[0].2, "cleanup");
        assert_eq!(r.runs[1].missed, []);
        // `--select calls` keeps only the commit that deletes a call to a repo fn
        let o = Opts { calls: true, ..o };
        let r = replay(&c, &fg, &work, &o).unwrap();
        assert_eq!(r.runs.len(), 1);
        assert_eq!(r.runs[0].removed_calls, ["cleanup"]);
        // rustc's answers are cached per commit and reused
        assert!(work.join("replay/hist.dead.json").is_file());
    }
}
