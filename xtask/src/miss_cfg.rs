//! How many ground-truth misses are kept alive only through code that rustc did not compile
//! (cfg-gated callers)? Port of `miss_cfg.py`.
//!
//! For each rustc-dead fn that ferrograph does not report because of a `calls` edge from non-root
//! code or from a root item, checks whether any caller on the shown reachability chain sits under a
//! `#[cfg(..)]` (same text heuristic as the precision bucket). Such misses are an oracle artifact
//! (rustc only sees one platform/feature set), not a resolution error.

use crate::graph::split_id;
use crate::groundtruth::Data;
use crate::source::cfg_gated;
use crate::util::Counter;
use anyhow::{Context, Result};
use std::path::Path;

/// One line per corpus and a total, as printed by the Python.
pub fn report(data: &Data, work: &Path, mode: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut tot = Counter(Vec::new());
    for r in data.results.iter().filter(|r| r.ok) {
        let root = work.join("corpora").join(&r.name);
        if !root.exists() {
            continue;
        }
        let misses = &r
            .modes
            .get(mode)
            .with_context(|| format!("{}: no mode {mode}", r.name))?
            .misses;
        let mut keys = Vec::new();
        for m in misses {
            if !m.bucket.contains("calls edge") && !m.bucket.contains("reference") {
                continue;
            }
            let gated = m.chain.iter().skip(1).any(|x| {
                let nid = x.rsplit_once(" (").map_or(x.as_str(), |(a, _)| a);
                let (f, line, _) = split_id(nid);
                cfg_gated(&root, &f, line)
            });
            keys.push(if gated {
                "via cfg-gated caller"
            } else {
                "via ungated caller"
            });
        }
        let c = Counter::new(keys);
        out.push(format!("{}: {}", r.name, c.dict_repr()));
        tot.add(&c);
    }
    out.push(format!("total: {}", tot.dict_repr()));
    Ok(out)
}

pub fn main(json: &Path, work: &Path, mode: &str) -> Result<()> {
    let data: Data = serde_json::from_str(&std::fs::read_to_string(json)?)?;
    for l in report(&data, work, mode)? {
        println!("{l}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groundtruth::{CorpusResult, Meta, Miss, ModeResult};

    fn miss(bucket: &str, chain: &[&str]) -> Miss {
        Miss {
            id: "x".into(),
            file: "./a.rs".into(),
            line: 1,
            name: "x".into(),
            class: "lib".into(),
            bucket: bucket.into(),
            cause: None,
            collateral: false,
            chain: chain.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn separates_cfg_gated_and_ungated_callers() {
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("corpora/c");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("a.rs"),
            "#[cfg(windows)]\nfn gated() {}\nfn plain() {}\n",
        )
        .unwrap();
        let mut r = CorpusResult {
            name: "c".into(),
            ok: true,
            ..CorpusResult::default()
        };
        let ms = vec![
            miss(
                "calls edge from root item",
                &[
                    "./a.rs#9:1 (calls)",
                    "./a.rs#2:1 (calls)",
                    "./a.rs#1:1 (root)",
                ],
            ),
            miss(
                "calls edge from non-root code",
                &["./a.rs#9:1 (calls)", "./a.rs#3:1 (root)"],
            ),
            miss("pub root: pub", &["./a.rs#9:1 (root)"]),
        ];
        r.modes.insert(
            "default".into(),
            ModeResult {
                misses: ms,
                ..ModeResult::default()
            },
        );
        let skipped = CorpusResult {
            name: "nodir".into(),
            ok: true,
            ..CorpusResult::default()
        };
        let data = Data {
            meta: Meta::default(),
            results: vec![r, skipped],
        };
        assert_eq!(
            report(&data, work.path(), "default").unwrap(),
            [
                "c: {'via cfg-gated caller': 1, 'via ungated caller': 1}",
                "total: {'via cfg-gated caller': 1, 'via ungated caller': 1}"
            ]
        );
    }
}
