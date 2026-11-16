//! Paired comparison of two `groundtruth` runs on the same corpora (port of `compare.py`).
//!
//! Per cohort: recall (all R) of each arm with Wilson intervals, items found by only one arm
//! (exact `McNemar` p), extra reports of the variant split by rustc's verdict, and the sample size
//! needed to detect a 5-point effect.

use crate::stats::{frac_ci, mcnemar_p, n_paired, n_unpaired, wilson};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Deserialize)]
struct File {
    results: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    name: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    heldout: bool,
    cohort: Option<String>,
    #[serde(default)]
    classes: HashMap<String, serde_json::Value>,
    #[serde(default)]
    modes: HashMap<String, serde_json::Value>, // parsed per requested mode only
}

#[derive(Deserialize)]
struct Mode {
    #[serde(rename = "F")]
    found: Vec<String>,
    fp: HashMap<String, Vec<Id>>,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

const VERDICTS: [&str; 4] = [
    "not_judgeable_file",
    "not_judgeable_cfg",
    "rustc_live_some_unit",
    "disagree",
];

impl Item {
    fn cohort(&self) -> &str {
        self.cohort
            .as_deref()
            .unwrap_or(if self.heldout { "held-out" } else { "original" })
    }

    fn mode(&self, mode: &str) -> Result<Mode> {
        let v = self
            .modes
            .get(mode)
            .with_context(|| format!("{}: no mode {mode}", self.name))?;
        Mode::deserialize(v).with_context(|| format!("{}: mode {mode}", self.name))
    }

    fn verdict(&self, mode: &str, nid: &str) -> Result<&'static str> {
        let fp = self.mode(mode)?.fp;
        for k in VERDICTS {
            let hit = fp
                .get(k)
                .with_context(|| format!("{}: no fp.{k}", self.name))?;
            if hit.iter().any(|x| x.id == nid) {
                return Ok(k);
            }
        }
        Ok(if self.classes.contains_key(nid) {
            "confirmed"
        } else {
            "other"
        })
    }
}

/// Successful results by name, in file order (a repeated name keeps its first position, last value).
fn load(path: &Path) -> Result<(Vec<String>, HashMap<String, Item>)> {
    let s = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let f: File = serde_json::from_str(&s).with_context(|| path.display().to_string())?;
    let (mut order, mut by) = (Vec::new(), HashMap::new());
    for r in f.results.into_iter().filter(|r| r.ok) {
        if !by.contains_key(&r.name) {
            order.push(r.name.clone());
        }
        by.insert(r.name.clone(), r);
    }
    Ok((order, by))
}

fn set(v: &[String]) -> HashSet<&str> {
    v.iter().map(String::as_str).collect()
}

pub fn render(base: &Path, variant: &Path, mode: &str) -> Result<String> {
    let ((order, base), (_, var)) = (load(base)?, load(variant)?);
    let mut cohorts: Vec<(String, Vec<&str>)> = Vec::new();
    for name in &order {
        if var.contains_key(name) {
            let co = base[name].cohort();
            match cohorts.iter_mut().find(|(c, _)| c == co) {
                Some((_, v)) => v.push(name),
                None => cohorts.push((co.to_string(), vec![name])),
            }
        }
    }
    let all: Vec<&str> = cohorts
        .iter()
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    cohorts.push(("**all**".into(), all));

    let mut l = format!("# paired comparison, mode `{mode}`\n\n");
    l += "| cohort | R | recall base | recall variant | only base | only variant | McNemar p | extra reports | of which confirmed | cfg/unjudgeable | live in some unit | genuine disagreement |\n";
    l += "|---|---|---|---|---|---|---|---|---|---|---|---|";
    let (mut tr, mut thb, mut tob, mut tov) = (0u64, 0u64, 0u64, 0u64);
    for (co, names) in &cohorts {
        let (mut r, mut hb, mut hv, mut ob, mut ov) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut extra: HashMap<&str, u64> = HashMap::new();
        for n in names {
            let (rb, rv) = (&base[*n], &var[*n]);
            let rset: HashSet<&str> = rb.classes.keys().map(String::as_str).collect();
            let (mb, mv) = (rb.mode(mode)?, rv.mode(mode)?);
            let (fb, fv) = (set(&mb.found), set(&mv.found));
            r += rset.len() as u64;
            hb += fb.intersection(&rset).count() as u64;
            hv += fv.intersection(&rset).count() as u64;
            ob += fb.difference(&fv).filter(|i| rset.contains(*i)).count() as u64;
            ov += fv.difference(&fb).filter(|i| rset.contains(*i)).count() as u64;
            for nid in fv.difference(&fb).filter(|i| !rset.contains(*i)) {
                *extra.entry(rv.verdict(mode, nid)?).or_default() += 1;
            }
        }
        let x = |k: &str| extra.get(k).copied().unwrap_or(0);
        let ne = extra.values().sum::<u64>() + ov;
        write!(
            l,
            "\n| {co} | {r} | {} | {} | {ob} | {ov} | {:.3} | {ne} | {ov} | {} | {} | {} |",
            frac_ci(hb, r),
            frac_ci(hv, r),
            mcnemar_p(ob, ov),
            x("not_judgeable_file") + x("not_judgeable_cfg"),
            x("rustc_live_some_unit"),
            x("disagree") + x("other"),
        )?;
        if co == "**all**" {
            (tr, thb, tob, tov) = (r, hb, ob, ov);
        }
    }
    let r = tr as f64;
    let p = if tr > 0 { thb as f64 / r } else { 0.0 };
    let q = if tr > 0 { (tob + tov) as f64 / r } else { 0.0 };
    let (lo, hi) = wilson(thb, tr, 1.96);
    let paired = n_paired(q, 0.05).map_or_else(
        || "n/a (discordance rate below the effect)".to_string(),
        |n| n.to_string(),
    );
    write!(
        l,
        "\n\nSample size for a 5-point recall effect (two-sided 5%, 80% power; base recall {:.0}% over {tr} R items): \
         **{} R items per arm** if the arms were independent samples; \
         **{paired} R items** for this paired design at the observed discordance of {:.1}% of items \
         (this is the number of ground-truth items, not corpora). \
         Measured: {tr} R items, so the 95% interval of one arm's recall is ±{:.0} points.",
        100.0 * p,
        n_unpaired(p.max(0.05), 0.05),
        100.0 * q,
        100.0 * (hi - lo) / 2.0,
    )?;
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(f: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(f)
    }

    #[test]
    fn matches_python_byte_for_byte() {
        let got = render(
            &golden("compare-base.json"),
            &golden("compare-variant.json"),
            "default",
        )
        .unwrap();
        assert_eq!(got, include_str!("../tests/golden/compare.md"));
    }

    #[test]
    fn unused_modes_need_not_be_well_formed() {
        let d = tempfile::tempdir().unwrap();
        let item = r#"{"results": [{"name": "a", "ok": true, "classes": {"x": 1},
            "modes": {"default": {"F": ["x"], "fp": {"not_judgeable_file": [], "not_judgeable_cfg": [],
            "rustc_live_some_unit": [], "disagree": []}}, "future": {"other": 1}}}]}"#;
        let p = d.path().join("r.json");
        std::fs::write(&p, item).unwrap();
        assert!(render(&p, &p, "default").is_ok());
        assert!(render(&p, &p, "future").is_err());
    }

    #[test]
    fn unknown_mode_is_an_error() {
        assert!(render(
            &golden("compare-base.json"),
            &golden("compare-variant.json"),
            "nope"
        )
        .is_err());
    }
}
