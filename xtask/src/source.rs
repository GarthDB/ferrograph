//! Text heuristics over the audited sources (port of the `cfg`/mapping helpers in `fgacc.py`).

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `.rs` files under `root`, skipping `target` and `.git` directories, in path order.
pub fn rs_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut es: Vec<_> = rd.flatten().collect();
        es.sort_by_key(std::fs::DirEntry::file_name);
        for e in es {
            let p = e.path();
            let name = e.file_name();
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                if name != "target" && name != ".git" {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

pub fn read_lossy(p: &Path) -> Option<String> {
    std::fs::read(p)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn indent(s: &str) -> usize {
    s.chars().count() - s.trim_start().chars().count()
}

fn hdr() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^\s*(pub(\([^)]*\))?\s+)?(unsafe\s+)?(mod|impl|trait|extern)\b").unwrap()
    })
}

/// Attribute text directly above line `j`, including multi-line attributes and interleaved comments.
fn attrs_above(lines: &[&str], j: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut k = j as i64 - 1;
    while k >= 0 {
        let line = lines[k as usize];
        let t = line.trim();
        if t.starts_with("#[") || t.starts_with("#![") {
            out.push(line.to_string());
        } else if t.starts_with("//") {
        } else if t.ends_with(']') {
            // tail of a wrapped attribute: walk up to the line that opens it
            let mut m = k;
            while m >= 0 && k - m < 40 && !lines[m as usize].trim_start().starts_with("#[") {
                m -= 1;
            }
            if m < 0 || k - m >= 40 {
                break;
            }
            let joined: Vec<&str> = lines[m as usize..=k as usize]
                .iter()
                .map(|l| l.trim())
                .collect();
            out.push(joined.join(" "));
            k = m;
        } else {
            break;
        }
        k -= 1;
    }
    out
}

fn gated(lines: &[&str], j: usize) -> bool {
    attrs_above(lines, j).iter().any(|a| a.contains("cfg("))
}

/// Heuristic: is the fn at (file, line) under a `#[cfg(...)]` (own attrs, enclosing mod/impl/trait,
/// or its `mod x;` declaration)? Text based, so a `cfg_attr`-free approximation.
pub fn cfg_gated(root: &Path, f: &str, line: usize) -> bool {
    let p = root.join(f);
    let Some(text) = read_lossy(&p) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let Some(i) = line.checked_sub(1).filter(|i| *i < lines.len()) else {
        return false;
    };
    if gated(&lines, i) {
        return true;
    }
    let mut cur = indent(lines[i]);
    for j in (0..i).rev() {
        let s = lines[j];
        if s.trim().is_empty() {
            continue;
        }
        let ind = indent(s);
        if ind < cur && hdr().is_match(s) {
            if gated(&lines, j) {
                return true;
            }
            cur = ind;
        }
    }
    let stem = match p.file_stem().and_then(|s| s.to_str()) {
        Some("mod") => p
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str()),
        s => s,
    }
    .unwrap_or("");
    let pat = Regex::new(&format!(
        r"^\s*(pub(\([^)]*\))?\s+)?mod\s+{}\s*;",
        regex::escape(stem)
    ))
    .unwrap();
    let mut parents: Vec<&Path> = Vec::new();
    for d in [p.parent(), p.parent().and_then(Path::parent)]
        .into_iter()
        .flatten()
    {
        if !parents.contains(&d) {
            parents.push(d);
        }
    }
    for parent in parents {
        let Ok(rd) = std::fs::read_dir(parent) else {
            continue;
        };
        for cand in rd.flatten().map(|e| e.path()) {
            if cand.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let Some(ct) = read_lossy(&cand) else {
                continue;
            };
            let cl: Vec<&str> = ct.lines().collect();
            if (0..cl.len()).any(|j| pat.is_match(cl[j]) && gated(&cl, j)) {
                return true;
            }
        }
    }
    false
}

/// Source lines mentioning `name` other than its definition (for hand-checking).
pub fn text_refs(
    root: &Path,
    name: &str,
    defn_file: &str,
    defn_line: usize,
    limit: usize,
) -> Vec<String> {
    let pat = Regex::new(&format!(r"\b{}\b", regex::escape(name))).unwrap();
    let mut out = Vec::new();
    for p in rs_files(root) {
        let rel = format!("./{}", p.strip_prefix(root).unwrap_or(&p).display());
        let Some(text) = read_lossy(&p) else { continue };
        for (n, s) in text.lines().enumerate() {
            let n = n + 1;
            if pat.is_match(s) && !(rel == defn_file && n == defn_line) {
                let t: String = s.trim().chars().take(110).collect();
                out.push(format!("{rel}:{n}: {t}"));
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Source-level visibility of the fn node: `pub(crate)/pub(super)` (what #91 is about) or `pub`.
pub fn visibility(root: &Path, nid: &str) -> &'static str {
    let (f, line, _) = crate::graph::split_id(nid);
    let Some(text) = read_lossy(&root.join(&f)) else {
        return "unknown";
    };
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = (
        line.saturating_sub(1).min(lines.len()),
        (line + 6).min(lines.len()),
    );
    let restricted = Regex::new(r"\bpub\s*\(\s*(crate|super|self|in\b)").unwrap();
    let public = Regex::new(r"\bpub\s+(const\s+|async\s+|unsafe\s+)*fn\b").unwrap();
    let is_fn = Regex::new(r"\bfn\b").unwrap();
    for ln in &lines[start..end] {
        if restricted.is_match(ln) {
            return "pub(crate)/pub(super)";
        }
        if public.is_match(ln) {
            return "pub";
        }
        if is_fn.is_match(ln) {
            break;
        }
    }
    "not pub in source (inherited, e.g. impl of a pub trait or enclosing item)"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (n, c) in files {
            let p = d.path().join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        d
    }

    #[test]
    fn multiline_cfg_attribute() {
        let d = tmp(&[(
            "a.rs",
            "#[cfg(any(\n    feature = \"x\",\n))]\nfn gated() {}\n\n// note\n#[inline]\nfn plain() {}\n",
        )]);
        assert!(cfg_gated(d.path(), "./a.rs", 4));
        assert!(!cfg_gated(d.path(), "./a.rs", 8));
    }

    #[test]
    fn cfg_via_enclosing_impl_and_mod_declaration() {
        let d = tmp(&[
            (
                "a.rs",
                "#[cfg(unix)]\nimpl X {\n    fn f() {}\n}\nimpl Y {\n    fn g() {}\n}\n",
            ),
            ("lib.rs", "#[cfg(windows)]\nmod w;\nmod plain;\n"),
            ("w.rs", "fn h() {}\n"),
            ("plain.rs", "fn p() {}\n"),
        ]);
        assert!(cfg_gated(d.path(), "./a.rs", 3));
        assert!(!cfg_gated(d.path(), "./a.rs", 6));
        assert!(cfg_gated(d.path(), "./w.rs", 1));
        assert!(!cfg_gated(d.path(), "./plain.rs", 1));
        assert!(!cfg_gated(d.path(), "./missing.rs", 1));
    }

    #[test]
    fn text_refs_skip_definition_and_target() {
        let d = tmp(&[
            ("a.rs", "fn foo() {}\nfn bar() { foo(); }\n"),
            ("target/x.rs", "foo();\n"),
            ("b.rs", "// foo here\nfoobar();\n"),
        ]);
        let r = text_refs(d.path(), "foo", "./a.rs", 1, 6);
        assert_eq!(
            r,
            ["./a.rs:2: fn bar() { foo(); }", "./b.rs:1: // foo here"]
        );
        assert_eq!(text_refs(d.path(), "foo", "./a.rs", 1, 1).len(), 1);
    }

    #[test]
    fn visibility_kinds() {
        let d = tmp(&[(
            "a.rs",
            "pub fn a() {}\npub(crate) fn b() {}\nfn c() {}\npub async fn d() {}\n",
        )]);
        let v = |l: usize| visibility(d.path(), &format!("./a.rs#{l}:1"));
        assert_eq!(v(1), "pub");
        assert_eq!(v(2), "pub(crate)/pub(super)");
        assert!(v(3).starts_with("not pub in source"));
        assert_eq!(v(4), "pub");
        assert_eq!(visibility(d.path(), "./nope.rs#1:1"), "unknown");
    }
}
