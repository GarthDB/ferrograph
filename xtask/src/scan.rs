//! Scan candidate repos for rustc dead-code ground truth (used to pick corpora; see
//! `scripts/accuracy/README.md`). Port of `scan.py`.
//!
//! For each `owner/repo` (or url) shallow-clone HEAD under `--work`, run the same rustc oracle as
//! `groundtruth` and print the pinned SHA, the number of fns rustc reports dead in every unit, and
//! how many of those sit in bin-only files. TSV on stdout, one line per repo, appended to `--out`.

use crate::corpus::{self, Corpus};
use crate::groundtruth::DEFAULT_CARGO_ARGS;
use crate::rustc::{all_unit_dead, collect_ground_truth, run_rustc};
use crate::util::{head, tail};
use anyhow::Result;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

fn scan(repo: &str, work: &Path, timeout: Duration) -> Result<Vec<String>> {
    let url = if repo.contains("://") {
        repo.to_string()
    } else {
        format!("https://github.com/{repo}")
    };
    let name = repo
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(repo)
        .to_string();
    let c = Corpus {
        name: name.clone(),
        url: Some(url),
        git_ref: Some("HEAD".into()),
        path: None,
        index_path: None,
        group: None,
        heldout: false,
        cohort: None,
        cargo_args: None,
    };
    let prep = corpus::prepare(&c, work)?;
    let sha = prep.sha.unwrap_or_else(|| "None".into());
    let tgt = work.join("target").join(&name);
    let args: Vec<String> = DEFAULT_CARGO_ARGS.map(String::from).to_vec();
    let rc = run_rustc(&prep.root, &args, &tgt, timeout);
    // targets are GBs; the disk fills up otherwise
    std::fs::remove_dir_all(std::fs::canonicalize(&tgt).unwrap_or(tgt)).ok();
    let rc = rc?;
    let Some(msgs) = rc.msgs else {
        let last = rc.err.trim().lines().last().unwrap_or("");
        return Ok(vec![name, sha, "BUILD-FAIL".into(), head(last, 80)]);
    };
    let gt = collect_ground_truth(&msgs, &prep.root, &prep.root)?;
    let dead = all_unit_dead(&gt);
    let nbin = dead
        .iter()
        .filter(|k| gt.file_class.get(&k.0) == Some(&"bin"))
        .count();
    let some = gt.items.len() - dead.len();
    let note = if rc.note.is_empty() {
        "ok".to_string()
    } else {
        rc.note
    };
    Ok(vec![
        name,
        sha,
        format!("dead={}", dead.len()),
        format!("bin={nbin}"),
        format!("some-unit-only={some}"),
        note,
    ])
}

pub fn main(repos: &[String], work: &Path, timeout_secs: u64, out: &Path) -> Result<()> {
    // a repo's rust-toolchain file would make rustup download that toolchain (GBs); scan with the active one
    if std::env::var_os("RUSTUP_TOOLCHAIN").is_none() {
        std::env::set_var("RUSTUP_TOOLCHAIN", "stable");
    }
    std::fs::create_dir_all(work)?;
    for r in repos {
        let row = scan(r, work, Duration::from_secs(timeout_secs)).unwrap_or_else(|e| {
            vec![
                r.clone(),
                "?".into(),
                "ERROR".into(),
                tail(&format!("{e:#}"), 100).replace('\n', " "),
            ]
        });
        let line = row.join("\t");
        println!("{line}");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(out)?;
        writeln!(f, "{line}")?;
    }
    Ok(())
}
