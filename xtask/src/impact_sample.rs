//! Bucket the `pr` impacted set by depth and confidence, and score hand labels
//! (port of `impact_sample.py`).
//!
//! Depth = first `--impact-depth` run the node appears in. A node missing from the `-x` run at its
//! depth is reachable only through low-confidence edges.

use crate::stats::frac_ci;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Deserialize)]
struct Report {
    impacted: Impacted,
}

#[derive(Deserialize)]
struct Impacted {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    name: Option<String>,
    group: Option<String>,
}

fn ids(path: &Path) -> Result<BTreeMap<String, Node>> {
    let s = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let r: Report = serde_json::from_str(&s).with_context(|| path.display().to_string())?;
    Ok(r.impacted
        .nodes
        .into_iter()
        .map(|n| (n.id.clone(), n))
        .collect())
}

/// CSV (CRLF line endings, like Python's `csv.writer`) with an empty `label` column to fill in.
pub fn bucket(dir: &Path, prs: &[String]) -> Result<String> {
    let mut w = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    w.write_record([
        "pr",
        "id",
        "name",
        "group",
        "depth",
        "confident",
        "label",
        "note",
    ])?;
    for pr in prs {
        let mut seen = BTreeSet::new();
        for depth in 1..=3 {
            let cur = ids(&dir.join(format!("pr{pr}-d{depth}.json")))?;
            let conf = ids(&dir.join(format!("pr{pr}-d{depth}-x.json")))?;
            for (id, n) in &cur {
                if !seen.contains(id) {
                    let confident = if conf.contains_key(id) { "1" } else { "0" };
                    w.write_record([
                        pr.as_str(),
                        id,
                        n.name.as_deref().unwrap_or(""),
                        n.group.as_deref().unwrap_or(""),
                        &depth.to_string(),
                        confident,
                        "",
                        "",
                    ])?;
                }
            }
            seen.extend(cur.into_keys());
        }
    }
    Ok(String::from_utf8(w.into_inner()?)?)
}

struct Row {
    depth: u32,
    confident: bool,
    tp: bool,
}

fn read_labels(path: &Path) -> Result<Vec<Row>> {
    let mut rd = csv::ReaderBuilder::new()
        .flexible(true) // Python's DictReader tolerates ragged rows
        .from_path(path)
        .with_context(|| path.display().to_string())?;
    let h = rd.headers()?.clone();
    let col = |n: &str| {
        h.iter()
            .position(|x| x == n)
            .with_context(|| format!("no column {n}"))
    };
    let (i_id, i_depth, i_conf, i_label) =
        (col("id")?, col("depth")?, col("confident")?, col("label")?);
    let mut rows = Vec::new();
    let mut bad = Vec::new();
    for rec in rd.records() {
        let rec = rec?;
        let label = rec.get(i_label).unwrap_or("");
        if label != "TP" && label != "FP" {
            bad.push(rec.get(i_id).unwrap_or("").to_string());
        }
        rows.push(Row {
            depth: rec.get(i_depth).unwrap_or("").parse()?,
            confident: rec.get(i_conf) == Some("1"),
            tp: label == "TP",
        });
    }
    if !bad.is_empty() {
        bad.truncate(5);
        bail!("unlabelled: {bad:?}");
    }
    Ok(rows)
}

/// Precision by depth, default vs `--exclude-low-confidence`.
pub fn score(path: &Path) -> Result<String> {
    let rows = read_labels(path)?;
    let mut out = String::new();
    for (title, only_confident) in [("default", false), ("--exclude-low-confidence", true)] {
        writeln!(out, "{title}")?;
        let pick = |pred: &dyn Fn(&Row) -> bool| {
            let sel: Vec<&Row> = rows
                .iter()
                .filter(|r| pred(r) && (!only_confident || r.confident))
                .collect();
            frac_ci(sel.iter().filter(|r| r.tp).count() as u64, sel.len() as u64)
        };
        for lim in 1..=3 {
            writeln!(
                out,
                "  depth<={lim}: precision {}",
                pick(&|r| r.depth <= lim)
            )?;
        }
        for dep in 2..=3 {
            writeln!(
                out,
                "  depth=={dep} only: precision {}",
                pick(&|r| r.depth == dep)
            )?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accuracy/impact")
    }

    #[test]
    fn bucket_matches_python_byte_for_byte() {
        let prs = ["47", "49", "50"].map(String::from);
        assert_eq!(
            bucket(&root(), &prs).unwrap(),
            include_str!("../tests/golden/impact-bucket.csv")
        );
    }

    #[test]
    fn score_matches_python_byte_for_byte() {
        let got = score(&root().join("labels.csv")).unwrap();
        assert_eq!(got, include_str!("../tests/golden/impact-score.txt"));
    }

    #[test]
    fn score_tolerates_ragged_rows() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l.csv");
        std::fs::write(
            &p,
            "pr,id,name,group,depth,confident,label,note\n47,x,,g,1,1,TP,,stray\n47,y,,g,1,1,FP\n",
        )
        .unwrap();
        assert!(score(&p)
            .unwrap()
            .contains("depth<=1: precision 50.0% (1/2)"));
    }

    #[test]
    fn score_rejects_unlabelled_rows() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l.csv");
        std::fs::write(
            &p,
            "pr,id,name,group,depth,confident,label,note\n47,x,,g,1,1,,\n",
        )
        .unwrap();
        assert!(score(&p).unwrap_err().to_string().contains("unlabelled"));
    }
}
