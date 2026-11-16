//! `corpora.toml` loading and corpus preparation (port of `fgacc.prepare_corpus`).

use crate::run::run;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const T: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub name: String,
    pub url: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub path: Option<String>,
    pub index_path: Option<String>,
    pub group: Option<String>,
    #[serde(default)]
    pub heldout: bool,
    pub cohort: Option<String>,
    pub cargo_args: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct File {
    corpus: Vec<Corpus>,
}

pub fn load(path: &Path) -> Result<Vec<Corpus>> {
    let s = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    Ok(toml::from_str::<File>(&s)?.corpus)
}

impl Corpus {
    /// Pooling cohort: `cohort` if set, else held-out / original.
    pub fn cohort(&self) -> &str {
        self.cohort
            .as_deref()
            .unwrap_or(if self.heldout { "held-out" } else { "original" })
    }

    pub fn group(&self) -> &str {
        self.group.as_deref().unwrap_or(&self.name)
    }
}

pub fn git_head(path: &Path) -> Option<String> {
    let o = run(&["git", "rev-parse", "HEAD"], Some(path), &[], T, false).ok()?;
    (o.code == 0).then(|| o.stdout.trim().to_string())
}

pub struct Prepared {
    pub root: PathBuf,
    pub sha: Option<String>,
    pub dirty: bool,
}

/// Materialise a corpus copy under `work` and return its root. Originals are never written.
pub fn prepare(c: &Corpus, work: &Path) -> Result<Prepared> {
    let dest = work.join("corpora").join(c.group());
    if let Some(url) = &c.url {
        if !dest.exists() {
            std::fs::create_dir_all(dest.parent().expect("has parent"))?;
            let d = dest.to_str().context("non-utf8 path")?;
            run(&["git", "init", "-q", d], None, &[], T, true)?;
            run(
                &["git", "remote", "add", "origin", url],
                Some(&dest),
                &[],
                T,
                true,
            )?;
            let r = c.git_ref.as_deref().unwrap_or("HEAD");
            let fetch = ["git", "fetch", "-q", "--depth", "1", "origin", r];
            run(&fetch, Some(&dest), &[], Duration::from_secs(600), true)?;
            run(
                &["git", "checkout", "-q", "FETCH_HEAD"],
                Some(&dest),
                &[],
                T,
                true,
            )?;
        }
        return Ok(Prepared {
            sha: git_head(&dest),
            root: dest,
            dirty: false,
        });
    }
    let raw = c.path.as_deref().context("corpus needs url or path")?;
    let src = std::fs::canonicalize(expand_home(raw))?;
    let sha = git_head(&src);
    let st = run(&["git", "status", "--porcelain"], Some(&src), &[], T, false)?;
    if !dest.exists() {
        copy_tree(&src, &dest)?;
    }
    Ok(Prepared {
        root: dest,
        sha,
        dirty: !st.stdout.trim().is_empty(),
    })
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        _ => PathBuf::from(p),
    }
}

/// Copy a tree keeping symlinks, skipping `target`, `.git`, `node_modules` and `.ferrograph*`.
fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if matches!(n.as_ref(), "target" | ".git" | "node_modules") || n.starts_with(".ferrograph")
        {
            continue;
        }
        let (from, to) = (e.path(), dest.join(&name));
        let ty = e.file_type()?;
        if ty.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
        } else if ty.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repo_corpora_toml() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accuracy/corpora.toml");
        let cs = load(&p).unwrap();
        assert!(cs.len() >= 20);
        assert_eq!(cs[0].name, "clap");
        assert_eq!(cs[0].cohort(), "original");
        assert!(cs.iter().any(|c| c.cohort() == "held-out"));
        assert!(cs.iter().any(|c| c.cargo_args.is_some()));
    }

    #[test]
    fn prepares_path_corpus_without_target_or_git() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(src.join("target")).unwrap();
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::write(src.join("a/x.rs"), "fn x() {}").unwrap();
        std::fs::write(src.join("target/junk"), "").unwrap();
        let c = Corpus {
            name: "t".into(),
            url: None,
            git_ref: None,
            path: Some(src.to_str().unwrap().into()),
            index_path: None,
            group: None,
            heldout: false,
            cohort: None,
            cargo_args: None,
        };
        let p = prepare(&c, &t.path().join("work")).unwrap();
        assert!(p.root.join("a/x.rs").is_file());
        assert!(!p.root.join("target").exists());
    }
}
