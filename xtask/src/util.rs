//! Small helpers that reproduce Python behaviour the reports depend on.

use crate::run::run;
use anyhow::Result;
use std::time::Duration;

/// Python's `Counter`: keys in first-seen order, `most_common` stable on ties.
pub struct Counter(pub Vec<(String, u64)>);

impl Counter {
    pub fn new<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        let mut v: Vec<(String, u64)> = Vec::new();
        for k in keys {
            match v.iter_mut().find(|(x, _)| x == k) {
                Some(e) => e.1 += 1,
                None => v.push((k.to_string(), 1)),
            }
        }
        Counter(v)
    }

    pub fn most_common(&self) -> Vec<(String, u64)> {
        let mut v = self.0.clone();
        v.sort_by_key(|e| std::cmp::Reverse(e.1)); // stable, like Python's reverse sort
        v
    }

    pub fn get(&self, k: &str) -> u64 {
        self.0.iter().find(|(x, _)| x == k).map_or(0, |e| e.1)
    }

    pub fn total(&self) -> u64 {
        self.0.iter().map(|e| e.1).sum()
    }

    /// `k: v, k: v` in first-seen order, or `none`.
    pub fn joined(&self) -> String {
        if self.0.is_empty() {
            return "none".into();
        }
        self.0
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Python's `repr(dict(counter))`: `{'a': 1, 'b': 2}`.
    pub fn dict_repr(&self) -> String {
        let items: Vec<String> = self.0.iter().map(|(k, v)| format!("'{k}': {v}")).collect();
        format!("{{{}}}", items.join(", "))
    }

    pub fn add(&mut self, other: &Counter) {
        for (k, v) in &other.0 {
            match self.0.iter_mut().find(|(x, _)| x == k) {
                Some(e) => e.1 += v,
                None => self.0.push((k.clone(), *v)),
            }
        }
    }
}

/// Python's `repr` of a float (`12.0`, not `12`).
pub fn pyf(x: f64) -> String {
    format!("{x:?}")
}

/// Round to one decimal (Python's `round(x, 1)`).
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// First `n` characters.
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Last `n` characters (Python's `s[-n:]`).
pub fn tail(s: &str, n: usize) -> String {
    let skip = s.chars().count().saturating_sub(n);
    s.chars().skip(skip).collect()
}

const T: Duration = Duration::from_secs(60);

pub fn rustc_version() -> Result<String> {
    Ok(run(&["rustc", "-V"], None, &[], T, true)?
        .stdout
        .trim()
        .to_string())
}

pub fn today() -> Result<String> {
    Ok(run(&["date", "+%Y-%m-%d"], None, &[], T, true)?
        .stdout
        .trim()
        .to_string())
}

pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_orders_like_python() {
        let c = Counter::new(["b", "a", "b", "c", "a", "d"]);
        let mc: Vec<_> = c.most_common().into_iter().map(|(k, _)| k).collect();
        assert_eq!(mc, ["b", "a", "c", "d"]); // ties keep first-seen order
        assert_eq!(c.dict_repr(), "{'b': 2, 'a': 2, 'c': 1, 'd': 1}");
        assert_eq!(Counter::new([]).dict_repr(), "{}");
        assert_eq!(Counter::new([]).joined(), "none");
    }

    #[test]
    fn counter_add_keeps_first_seen_order() {
        let mut a = Counter::new(["x"]);
        a.add(&Counter::new(["y", "x"]));
        assert_eq!(a.dict_repr(), "{'x': 2, 'y': 1}");
    }

    #[test]
    fn python_float_repr_and_slices() {
        assert_eq!(
            (pyf(12.0), pyf(0.25), pyf(round1(0.34))),
            ("12.0".into(), "0.25".into(), "0.3".into())
        );
        assert_eq!(
            (head("héllo", 2), tail("héllo", 2)),
            ("hé".into(), "lo".into())
        );
    }
}
