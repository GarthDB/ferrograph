//! Run `groundtruth` for every corpus under several `--extra-args` arms and merge the JSON per arm
//! (port of `run_matrix.py`).
//!
//! Corpora are processed one at a time and their cargo target dir is deleted afterwards
//! (`--keep-targets` to keep), because 20+ workspaces need tens of GB. Output: `<out>/<arm>.json`
//! (groundtruth-data.json format, all corpora) and `<out>/<arm>.md`. Re-running skips corpus/arm
//! pairs already measured with the same binary (sha256) and corpus ref; pairs that failed or are
//! missing are listed on stderr and make the exit status 1.

use crate::corpus::{self, Corpus};
use crate::groundtruth::{self, Data, Meta};
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const ARMS: [(&str, &str); 4] = [
    (
        "default",
        "--no-restricted-vis-not-root --no-bin-pub-not-root",
    ),
    (
        "restricted",
        "--restricted-vis-not-root --no-bin-pub-not-root",
    ),
    ("binpub", "--no-restricted-vis-not-root --bin-pub-not-root"),
    ("both", "--restricted-vis-not-root --bin-pub-not-root"),
];

pub struct Args {
    pub ferrograph: String,
    pub work: PathBuf,
    pub out: PathBuf,
    pub config: PathBuf,
    pub only: String,
    pub arms: String,
    pub keep_targets: bool,
}

fn part_of(out: &Path, bin_key: &str, c: &Corpus, arm: &str) -> PathBuf {
    let r: String = c
        .git_ref
        .as_deref()
        .filter(|r| !r.is_empty())
        .unwrap_or("head")
        .chars()
        .take(8)
        .collect();
    out.join("parts")
        .join(format!("{}.{arm}.{bin_key}-{r}.json", c.name))
}

fn load(p: &Path) -> Result<Data> {
    Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
}

/// Returns the names that are missing or failed (non-empty means exit status 1).
pub fn main(a: &Args) -> Result<Vec<String>> {
    std::fs::create_dir_all(a.out.join("parts"))?;
    std::fs::create_dir_all(&a.work)?;
    let (out, work) = (
        std::fs::canonicalize(&a.out)?,
        std::fs::canonicalize(&a.work)?,
    );
    let only: Vec<&str> = a.only.split(',').filter(|s| !s.is_empty()).collect();
    let corpora: Vec<Corpus> = corpus::load(&a.config)?
        .into_iter()
        .filter(|c| c.url.is_some() && (only.is_empty() || only.contains(&c.name.as_str())))
        .collect();
    let arms: Vec<&str> = a.arms.split(',').collect();
    for arm in &arms {
        if !ARMS.iter().any(|(n, _)| n == arm) {
            bail!(
                "unknown arm {arm} (from {})",
                ARMS.map(|(n, _)| n).join(",")
            );
        }
    }
    // parts are keyed by the binary and the corpus pin, so results of another build or ref are never reused
    let digest = Sha256::digest(std::fs::read(&a.ferrograph)?);
    let bin_key = format!(
        "{:08x}",
        u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
    );

    for c in &corpora {
        for arm in &arms {
            let part = part_of(&out, &bin_key, c, arm);
            if part.exists() && load(&part).is_ok_and(|d| d.results.iter().all(|r| r.ok)) {
                continue;
            }
            let extra = ARMS.iter().find(|(n, _)| n == arm).map_or("", |(_, e)| *e);
            let ga = groundtruth::Args {
                config: a.config.clone(),
                ferrograph: a.ferrograph.clone(),
                work: Some(work.clone()),
                only: c.name.clone(),
                timeout_secs: 1800,
                extra_args: extra.to_string(),
                max_check: 25,
                render_json: None,
                out_md: part.with_extension("md"),
                out_json: part,
                print_report: false,
            };
            if let Err(e) = groundtruth::main(&ga) {
                eprintln!("{}/{arm}: {e:#}", c.name); // the part is then missing, and reported below
            }
        }
        if !a.keep_targets {
            std::fs::remove_dir_all(work.join("target").join(c.group())).ok();
        }
    }
    let mut missing = Vec::new();
    for arm in &arms {
        let (mut results, mut meta) = (Vec::new(), None::<Meta>);
        for c in &corpora {
            let p = part_of(&out, &bin_key, c, arm);
            if !p.exists() {
                missing.push(format!("{}/{arm}", c.name));
                continue;
            }
            let d = load(&p)?;
            meta = meta.or(Some(d.meta));
            for r in &d.results {
                if !r.ok {
                    let e: String = r.error.as_deref().unwrap_or("").chars().take(80).collect();
                    missing.push(format!("{}/{arm}: {e}", r.name));
                }
            }
            results.extend(d.results);
        }
        let data = Data {
            meta: meta.unwrap_or_default(),
            results,
        };
        std::fs::write(
            out.join(format!("{arm}.json")),
            serde_json::to_string_pretty(&data)?,
        )?;
        std::fs::write(
            out.join(format!("{arm}.md")),
            groundtruth::render(&data.results, &data.meta, 25),
        )?;
    }
    if !missing.is_empty() {
        // compare intersects corpora by name, so say what is absent from the merged files
        eprintln!(
            "NOT MEASURED (absent from the merged results):\n  {}",
            missing.join("\n  ")
        );
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_include_arm_binary_and_ref() {
        let c = Corpus {
            name: "clap".into(),
            url: Some("u".into()),
            git_ref: Some("ec93e71255c0b833".into()),
            path: None,
            index_path: None,
            group: None,
            heldout: false,
            cohort: None,
            cargo_args: None,
        };
        let p = part_of(Path::new("/o"), "deadbeef", &c, "both");
        assert_eq!(p, Path::new("/o/parts/clap.both.deadbeef-ec93e712.json"));
        let c = Corpus { git_ref: None, ..c };
        assert!(part_of(Path::new("/o"), "k", &c, "default").ends_with("clap.default.k-head.json"));
    }

    #[test]
    fn arms_match_the_documented_flags() {
        assert_eq!(
            ARMS[0].1,
            "--no-restricted-vis-not-root --no-bin-pub-not-root"
        );
        assert_eq!(ARMS.len(), 4);
    }
}
