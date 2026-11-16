//! The rustc oracle: run `cargo check` with `dead_code` force-warned and extract the dead fns
//! (port of the "rustc ground truth" section of `fgacc.py`).

use crate::run::run;
use crate::source::{read_lossy, rs_files};
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const LIB_KINDS: [&str; 6] = ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];

pub struct Rustc {
    /// `None` when every variant failed.
    pub msgs: Option<Vec<Value>>,
    pub command: String,
    /// Which fallback variant ran (empty: the requested one).
    pub note: String,
    /// Tail of stderr of the last failure.
    pub err: String,
}

/// Run `cargo check` with force-warned `dead_code`; falls back step by step if the build fails.
pub fn run_rustc(
    root: &Path,
    cargo_args: &[String],
    target_dir: &Path,
    timeout: Duration,
) -> Result<Rustc> {
    std::fs::create_dir_all(target_dir)?;
    let target = std::fs::canonicalize(target_dir)?;
    let env = [
        ("RUSTFLAGS", "--force-warn dead_code"),
        ("CARGO_TARGET_DIR", target.to_str().unwrap_or_default()),
    ];
    let without = |args: &[String], x: &str| -> Vec<String> {
        args.iter().filter(|a| *a != x).cloned().collect()
    };
    let nofeat = without(cargo_args, "--all-features");
    let nobench: Vec<String> = ["--lib", "--bins", "--tests", "--examples"]
        .map(String::from)
        .into();
    let mut variants: Vec<(Vec<String>, String)> = vec![(cargo_args.to_vec(), String::new())];
    if nofeat != cargo_args {
        variants.push((
            nofeat.clone(),
            "default features (--all-features failed)".into(),
        ));
    }
    if cargo_args.iter().any(|a| a == "--all-targets") {
        for (base, note) in [
            (cargo_args, "no benches (--all-targets failed)"),
            (nofeat.as_slice(), "default features, no benches"),
        ] {
            let base = without(base, "--all-targets");
            variants.push(([base.clone(), nobench.clone()].concat(), note.into()));
            // bin-only packages have no library target, and `--lib` then fails the whole command
            variants.push((
                [base, nobench[1..].to_vec()].concat(),
                format!("{note}, no --lib"),
            ));
        }
    }
    let (mut err, mut last) = (String::new(), String::new());
    for (args, note) in variants {
        let mut cmd: Vec<String> = vec!["cargo".into(), "check".into()];
        cmd.extend(args);
        cmd.push("--message-format=json".into());
        let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
        let o = run(&refs, Some(root), &env, timeout, false)?;
        last = cmd.join(" ");
        if o.code == 0 {
            let msgs = o
                .stdout
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            return Ok(Rustc {
                msgs: Some(msgs),
                command: format!("RUSTFLAGS=\"--force-warn dead_code\" {last}"),
                note,
                err: String::new(),
            });
        }
        err = o.stderr;
    }
    let skip = err.chars().count().saturating_sub(1500);
    Ok(Rustc {
        msgs: None,
        command: last,
        note: "all variants failed".into(),
        err: err.chars().skip(skip).collect(),
    })
}

/// Split on spaces not preceded by a backslash (Python: `re.split(r"(?<!\\) ", s)`).
fn split_unescaped(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut prev) = (Vec::new(), String::new(), None);
    for c in s.chars() {
        if c == ' ' && prev != Some('\\') {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        prev = Some(c);
    }
    out.push(cur);
    out
}

/// `.rs` files a compilation unit read, from its dep-info file; `None` when there is none.
pub fn parse_dep_files(artifact: &Value) -> Option<BTreeSet<String>> {
    let fname = Path::new(artifact["filenames"][0].as_str()?);
    let d = fname.parent()?;
    let stem = fname.file_stem()?.to_str()?;
    let mut cands: Vec<PathBuf> = vec![d.join(format!("{stem}.d"))];
    if let Some(rest) = stem.strip_prefix("lib").filter(|r| !r.is_empty()) {
        cands.push(d.join(format!("{rest}.d")));
    }
    if artifact["target"]["kind"]
        .as_array()?
        .iter()
        .any(|k| k == "custom-build")
    {
        // build scripts are emitted as `build-script-build`; their dep-info is `build_script_build-<hash>.d`
        let mut b: Vec<PathBuf> = std::fs::read_dir(d)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("build_script_") && n.ends_with(".d"))
            })
            .collect();
        b.sort();
        cands.extend(b);
    }
    let path = cands.into_iter().find(|p| p.exists())?;
    let txt = read_lossy(&path)?;
    let mut files = BTreeSet::new();
    for line in txt.lines() {
        if !line.contains(':') {
            continue;
        }
        let deps = line.split_once(": ").map_or("", |(_, d)| d);
        for tok in split_unescaped(deps) {
            let tok = tok.replace("\\ ", " ");
            let tok = tok.trim();
            if tok.ends_with(".rs") {
                files.insert(tok.to_string());
            }
        }
    }
    Some(files)
}

pub struct Mods {
    names: HashSet<String>,
    paths: HashSet<String>,
}

/// Declared `mod` names and `#[path]` basenames in the tree. Dep-info also lists files pulled in by
/// `include_str!`/`include_bytes!`, which are data, not compiled modules; this tells them apart.
pub fn module_names(root: &Path) -> Mods {
    let re_mod = Regex::new(r"\bmod\s+(\w+)\s*[;{]").unwrap();
    let re_path = Regex::new(r#"#\[path\s*=\s*"([^"]+)""#).unwrap();
    let mut m = Mods {
        names: HashSet::new(),
        paths: HashSet::new(),
    };
    for p in rs_files(root) {
        let Some(txt) = read_lossy(&p) else { continue };
        m.names
            .extend(re_mod.captures_iter(&txt).map(|c| c[1].to_string()));
        m.paths.extend(re_path.captures_iter(&txt).filter_map(|c| {
            Path::new(&c[1])
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        }));
    }
    m
}

fn declared_module(nf: &str, mods: &Mods) -> bool {
    let p = Path::new(nf);
    let stem = match p.file_stem().and_then(|s| s.to_str()) {
        Some("mod") => p.parent().and_then(Path::file_name),
        _ => p.file_stem(),
    };
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    stem.and_then(|s| s.to_str())
        .is_some_and(|s| mods.names.contains(s))
        || mods.paths.contains(name)
}

/// Lexical normalisation (Python's `os.path.normpath`): drops `.` and resolves `..` without touching the disk.
fn normpath(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// A dead fn that rustc reported: how often, the lint text, and whether it comes from a macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadItem {
    pub reports: u64,
    pub msg: String,
    pub is_macro: bool,
}

/// (file, line, name) with file relative to the index root.
pub type ItemKey = (String, u64, String);

pub struct GroundTruth {
    pub items: BTreeMap<ItemKey, DeadItem>,
    /// Number of compilation units that compile each file.
    pub units: BTreeMap<String, u64>,
    /// `lib` if any non-test library unit compiles the file, else `bin`.
    pub file_class: BTreeMap<String, &'static str>,
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
    }
}

type Norm<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Compilation units per file (all, and library non-test ones), from each artifact's dep-info.
fn count_units(
    msgs: &[Value],
    cargo_root: &Path,
    norm: &Norm,
) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    let mut units: BTreeMap<String, u64> = BTreeMap::new();
    let mut lib_units: BTreeMap<String, u64> = BTreeMap::new();
    let mut seen: HashSet<(Vec<String>, String)> = HashSet::new();
    let mods = module_names(cargo_root);
    for m in msgs {
        if m["reason"] != "compiler-artifact" {
            continue;
        }
        let mp = PathBuf::from(m["manifest_path"].as_str().unwrap_or(""));
        let mp = std::fs::canonicalize(&mp).unwrap_or(mp);
        if !mp.starts_with(cargo_root) {
            continue;
        }
        let Some(files) = parse_dep_files(m).filter(|f| !f.is_empty()) else {
            continue;
        };
        let filenames: Vec<String> = m["filenames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(String::from))
            .collect();
        if !seen.insert((
            filenames,
            m["package_id"].as_str().unwrap_or("").to_string(),
        )) {
            continue;
        }
        let kinds: Vec<&str> = m["target"]["kind"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let is_lib = kinds.iter().any(|k| LIB_KINDS.contains(k)) && !truthy(&m["profile"]["test"]);
        let root_src = norm(m["target"]["src_path"].as_str().unwrap_or(""));
        let root_dir = root_src
            .as_deref()
            .map(|r| Path::new(r).parent().map(Path::to_path_buf));
        for f in files {
            let Some(nf) = norm(&f) else { continue };
            // files beside a crate root count as modules too: macros such as `automod::dir!` declare them
            let beside = root_dir
                .as_ref()
                .is_some_and(|d| Path::new(&nf).parent().map(Path::to_path_buf) == *d);
            if root_src.as_deref() == Some(nf.as_str()) || beside || declared_module(&nf, &mods) {
                *units.entry(nf.clone()).or_default() += 1;
                if is_lib {
                    *lib_units.entry(nf).or_default() += 1;
                }
            }
        }
    }

    (units, lib_units)
}

/// Dead fns rustc reported, keyed by (file, line, name).
fn dead_items(msgs: &[Value], norm: &Norm) -> BTreeMap<ItemKey, DeadItem> {
    let mut items: BTreeMap<ItemKey, DeadItem> = BTreeMap::new();
    let fn_before = Regex::new(r"\bfn\s+$").unwrap();
    for m in msgs {
        if m["reason"] != "compiler-message" {
            continue;
        }
        let d = &m["message"];
        if d["code"]["code"] != "dead_code" {
            continue;
        }
        for sp in d["spans"].as_array().into_iter().flatten() {
            if !truthy(&sp["is_primary"]) {
                continue;
            }
            let Some(f) = sp["file_name"].as_str().and_then(norm) else {
                continue;
            };
            let Some(txt) = sp["text"].get(0) else {
                continue;
            };
            let line: Vec<char> = txt["text"].as_str().unwrap_or("").chars().collect();
            let (hs, he) = (
                txt["highlight_start"].as_u64().unwrap_or(1) as usize,
                txt["highlight_end"].as_u64().unwrap_or(1) as usize,
            );
            let at = |i: usize| i.saturating_sub(1).min(line.len());
            let name: String = line[at(hs)..at(he).max(at(hs))].iter().collect();
            let before: String = line[..at(hs)].iter().collect();
            if !fn_before.is_match(&before) {
                continue;
            }
            let key = (f, sp["line_start"].as_u64().unwrap_or(0), name);
            let it = items.entry(key).or_insert_with(|| DeadItem {
                reports: 0,
                msg: d["message"]
                    .as_str()
                    .unwrap_or("")
                    .split(" `")
                    .next()
                    .unwrap_or("")
                    .to_string(),
                is_macro: truthy(&sp["expansion"]),
            });
            it.reports += 1;
        }
    }
    items
}

pub fn collect_ground_truth(
    msgs: &[Value],
    cargo_root: &Path,
    index_root: &Path,
) -> Result<GroundTruth> {
    let cargo_root = std::fs::canonicalize(cargo_root)?;
    let index_root = std::fs::canonicalize(index_root)?;
    let norm = |f: &str| -> Option<String> {
        let mut p = PathBuf::from(f);
        if !p.is_absolute() {
            p = cargo_root.join(p);
        }
        let rel = normpath(&p).strip_prefix(&index_root).ok()?.to_path_buf();
        let rel = if rel.as_os_str().is_empty() {
            ".".to_string()
        } else {
            rel.display().to_string()
        };
        Some(format!("./{rel}"))
    };

    let (units, lib_units) = count_units(msgs, &cargo_root, &norm);
    let items = dead_items(msgs, &norm);
    let file_class = units
        .keys()
        .map(|f| {
            (
                f.clone(),
                if lib_units.get(f).copied().unwrap_or(0) > 0 {
                    "lib"
                } else {
                    "bin"
                },
            )
        })
        .collect();
    Ok(GroundTruth {
        items,
        units,
        file_class,
    })
}

/// Keys of items rustc reports dead in every unit that compiles their file.
pub fn all_unit_dead(gt: &GroundTruth) -> BTreeSet<ItemKey> {
    gt.items
        .iter()
        .filter(|(k, v)| {
            gt.units
                .get(&k.0)
                .is_some_and(|u| *u > 0 && v.reports >= *u)
        })
        .map(|(k, _)| k.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_on_unescaped_spaces_only() {
        assert_eq!(
            split_unescaped(r"a.rs b\ c.rs  d.rs"),
            ["a.rs", r"b\ c.rs", "", "d.rs"]
        );
    }

    #[test]
    fn dep_info_lists_rs_files_including_escaped_spaces() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("foo.d"),
            "/t/foo: /w/src/lib.rs /w/my\\ dir/a.rs /w/data.txt\n/w/src/lib.rs:\n",
        )
        .unwrap();
        let art = json!({"filenames": [d.path().join("libfoo.rlib")], "target": {"kind": ["lib"]}});
        let got: Vec<_> = parse_dep_files(&art).unwrap().into_iter().collect();
        assert_eq!(got, ["/w/my dir/a.rs", "/w/src/lib.rs"]);
        let none =
            json!({"filenames": [d.path().join("libbar.rlib")], "target": {"kind": ["lib"]}});
        assert!(parse_dep_files(&none).is_none());
    }

    #[test]
    fn normpath_is_lexical() {
        assert_eq!(normpath(Path::new("/a/./b/../c")), Path::new("/a/c"));
    }

    fn artifact(dir: &Path, root: &Path, kind: &str, test: bool) -> Value {
        json!({
            "reason": "compiler-artifact",
            "manifest_path": root.join("Cargo.toml"),
            "package_id": "p 0.1.0",
            "filenames": [dir.join(format!("lib{kind}.rlib"))],
            "target": {"kind": [kind], "src_path": root.join("src/lib.rs")},
            "profile": {"test": test},
        })
    }

    fn dead_msg(root: &Path, file: &str, line: u64, text: &str, hs: u64, he: u64) -> Value {
        json!({
            "reason": "compiler-message",
            "message": {
                "message": "function `x` is never used",
                "code": {"code": "dead_code"},
                "spans": [{
                    "file_name": root.join(file), "is_primary": true, "line_start": line, "expansion": null,
                    "text": [{"text": text, "highlight_start": hs, "highlight_end": he}],
                }],
            },
        })
    }

    #[test]
    fn collects_items_units_and_classes() {
        let w = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(w.path()).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod util;\n").unwrap();
        std::fs::write(root.join("src/util.rs"), "").unwrap();
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let deps = format!(
            "x: {0}/src/lib.rs {0}/src/util.rs {0}/data.txt\n",
            root.display()
        );
        std::fs::write(out.join("libfoo.d"), &deps).unwrap();
        std::fs::write(out.join("libbar.d"), &deps).unwrap();
        // same sources compiled as a lib unit and as a test unit
        let mut lib = artifact(&out, &root, "foo", false);
        lib["target"]["kind"] = json!(["lib"]);
        let mut test = artifact(&out, &root, "bar", true);
        test["target"]["kind"] = json!(["lib"]);
        let msgs = vec![
            lib,
            test,
            dead_msg(&root, "src/util.rs", 3, "    fn helper() {}", 8, 14),
            dead_msg(&root, "src/util.rs", 3, "    fn helper() {}", 8, 14),
            dead_msg(&root, "src/util.rs", 9, "    const X: u8 = 1;", 11, 12), // not a fn
            dead_msg(&root, "/elsewhere/x.rs", 1, "fn y() {}", 4, 5), // outside the index root
        ];
        let gt = collect_ground_truth(&msgs, &root, &root).unwrap();
        assert_eq!(gt.units["./src/lib.rs"], 2);
        assert_eq!(gt.units["./src/util.rs"], 2);
        assert!(!gt.units.contains_key("./data.txt"));
        assert_eq!(gt.file_class["./src/util.rs"], "lib");
        let k = ("./src/util.rs".to_string(), 3, "helper".to_string());
        assert_eq!(gt.items.len(), 1);
        assert_eq!(
            gt.items[&k],
            DeadItem {
                reports: 2,
                msg: "function".into(),
                is_macro: false
            }
        );
        assert_eq!(all_unit_dead(&gt), BTreeSet::from([k]));
    }

    #[test]
    fn runs_cargo_on_the_synthetic_corpus() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accuracy/synthetic");
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("corpus");
        let c = crate::corpus::Corpus {
            name: "synthetic".into(),
            url: None,
            git_ref: None,
            path: Some(src.display().to_string()),
            index_path: None,
            group: None,
            heldout: false,
            cohort: None,
            cargo_args: None,
        };
        let p = crate::corpus::prepare(&c, work.path()).unwrap();
        assert_ne!(p.root, root);
        let args: Vec<String> = ["--workspace", "--all-targets", "--all-features"]
            .map(String::from)
            .into();
        let r = run_rustc(
            &p.root,
            &args,
            &work.path().join("target"),
            Duration::from_secs(600),
        )
        .unwrap();
        let msgs = r.msgs.expect(&r.err);
        let gt = collect_ground_truth(&msgs, &p.root, &p.root).unwrap();
        let mut names: Vec<_> = all_unit_dead(&gt).into_iter().map(|k| k.2).collect();
        names.sort();
        let want = [
            "crate_dead",
            "dead_c",
            "dead_d",
            "dead_m",
            "lib_dead",
            "pub_dead",
            "run",
            "unused_helper",
        ];
        assert_eq!(names, want);
        // Python's output on the same corpus
        let gold: Value =
            serde_json::from_str(include_str!("../tests/golden/rustc-synthetic.json")).unwrap();
        let items: Vec<Value> = gt
            .items
            .iter()
            .map(|((f, l, n), v)| json!([f, l, n, v.reports, v.msg, v.is_macro]))
            .collect();
        assert_eq!(Value::Array(items), gold["items"]);
        assert_eq!(json!(gt.units), gold["units"]);
        assert_eq!(json!(gt.file_class), gold["fclass"]);
        assert_eq!(r.note, gold["note"].as_str().unwrap());
        let classes: BTreeSet<_> = gt.file_class.values().copied().collect();
        assert_eq!(classes, BTreeSet::from(["bin", "lib"]));
    }
}
