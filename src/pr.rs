//! `ferrograph pr`: report what a change introduces (newly dead functions, blast radius).
//!
//! Function identity across base and head is `(file path, name, occurrence index among
//! same-name functions in that file, in source order)`, never the node id, because ids embed
//! line/column and shift when unrelated lines above them move. Limitation: a renamed function,
//! or one moved to another file, looks like one removed plus one added; a same-name function
//! inserted above another in the same file shifts the occurrence index. Pure file renames are
//! followed via git's rename detection. Removed functions are never reported.
//!
//! Changed lines are mapped to functions by re-parsing only the changed head files with
//! tree-sitter (nodes store just their start position, so the graph alone has no end lines).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use tree_sitter::Parser;

use crate::graph::query::Query;
use crate::graph::{unquote_datavalue, Store};
use crate::ops::{
    ChangedFunction, ChangedItems, CouplingHint, NewlyDeadFunction, NodeSummary, PrBlast, PrResult,
};
use crate::pipeline::{run_pipeline, PipelineConfig};

/// Options for [`run`].
#[derive(Debug, Default)]
pub struct PrOptions {
    /// Project root (the directory the graph was indexed from, inside a git repo).
    pub root: PathBuf,
    pub base: String,
    /// Already indexed base graph (skips the temporary worktree).
    pub base_db: Option<PathBuf>,
    /// Head graph path; ignored with `index_head`.
    pub head_db: PathBuf,
    /// Index the head tree into memory instead of reading `head_db`.
    pub index_head: bool,
    pub strict: bool,
    pub entry: Vec<String>,
    pub ignore: Vec<String>,
    pub exclude_low_confidence: bool,
    pub blast_limit: usize,
}

/// One file's changes on the head side.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    /// `(start, count)` of each hunk on the new side; `count == 0` is a pure deletion after `start`.
    pub hunks: Vec<(u32, u32)>,
}

/// Parse `git diff -U0` output into per-file head-side hunks. Non-`.rs`, deleted, binary and
/// quoted-path files are dropped.
#[must_use]
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut out = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut dropped = false;
    let mut in_hunk = false;
    let mut flush = |cur: &mut Option<FileDiff>, dropped: bool| {
        if let Some(f) = cur.take() {
            if !dropped && Path::new(&f.path).extension().is_some_and(|e| e == "rs") {
                out.push(f);
            }
        }
    };
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            flush(&mut cur, dropped);
            cur = Some(FileDiff::default());
            dropped = false;
            in_hunk = false;
        } else if let Some(f) = cur.as_mut() {
            // Header lines only count before the first hunk; `+++ x` can be an added body line.
            if in_hunk && !line.starts_with("@@ ") {
            } else if let Some(p) = line.strip_prefix("rename from ") {
                f.old_path = Some(p.to_string());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                f.path = p.to_string();
            } else if let Some(p) = line.strip_prefix("+++ ") {
                match p.strip_prefix("b/") {
                    Some(p) => f.path = p.split('\t').next().unwrap_or(p).to_string(),
                    // `/dev/null` (deleted) or a quoted path we do not decode.
                    None => dropped = true,
                }
            } else if let Some(rest) = line.strip_prefix("@@ ") {
                in_hunk = true;
                if let Some(h) = rest.split_whitespace().nth(1).and_then(parse_new_side) {
                    f.hunks.push(h);
                }
            }
        }
    }
    flush(&mut cur, dropped);
    out
}

fn parse_new_side(tok: &str) -> Option<(u32, u32)> {
    let (start, count) = tok
        .strip_prefix('+')?
        .split_once(',')
        .unwrap_or((&tok[1..], "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// A `function_item` in a head file (1-based lines).
#[derive(Debug, PartialEq, Eq)]
pub struct FnRange {
    pub name: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
}

/// All `function_item` ranges in `source`, in source order (nested fns included).
///
/// # Errors
/// Fails if the Rust grammar cannot be loaded or parsing yields no tree.
pub fn fn_ranges(source: &str) -> Result<Vec<FnRange>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser.parse(source, None).context("parse failed")?;
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or_default()
                .to_string();
            let line = |p: tree_sitter::Point| u32::try_from(p.row + 1).unwrap_or(u32::MAX);
            out.push(FnRange {
                name,
                start_line: line(node.start_position()),
                start_col: u32::try_from(node.start_position().column + 1).unwrap_or(u32::MAX),
                end_line: line(node.end_position()),
            });
        }
        let mut c = node.walk();
        stack.extend(node.children(&mut c));
    }
    out.sort_by_key(|r| (r.start_line, r.start_col));
    Ok(out)
}

/// Map hunks to the innermost containing function. Returns the indexes into `ranges` that
/// changed (sorted, unique) and inclusive line ranges outside every function.
#[must_use]
pub fn map_hunks(ranges: &[FnRange], hunks: &[(u32, u32)]) -> (Vec<usize>, Vec<[u32; 2]>) {
    let innermost = |from: u32, to: u32| {
        ranges
            .iter()
            .enumerate()
            .filter(|(_, r)| r.start_line <= from && to <= r.end_line)
            .min_by_key(|(_, r)| r.end_line - r.start_line)
            .map(|(i, _)| i)
    };
    let mut fns = BTreeSet::new();
    let mut outside: Vec<u32> = Vec::new();
    for &(start, count) in hunks {
        if count == 0 {
            // Pure deletion after line `start`: inside a fn only if both neighbours are.
            match innermost(start, start + 1) {
                Some(i) => {
                    fns.insert(i);
                }
                None => outside.push(start.max(1)),
            }
            continue;
        }
        for line in start..start + count {
            match innermost(line, line) {
                Some(i) => {
                    fns.insert(i);
                }
                None => outside.push(line),
            }
        }
    }
    outside.sort_unstable();
    outside.dedup();
    let mut merged: Vec<[u32; 2]> = Vec::new();
    for l in outside {
        match merged.last_mut() {
            Some(r) if r[1] + 1 == l => r[1] = l,
            _ => merged.push([l, l]),
        }
    }
    (fns.into_iter().collect(), merged)
}

fn git_cmd(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("failed to run git (is it installed and on PATH?)")
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = git_cmd(root, args)?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_opt(root: &Path, args: &[&str]) -> Option<String> {
    git_cmd(root, args)
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Removes the temporary base worktree on drop, including on error paths.
struct Worktree {
    repo: PathBuf,
    dir: PathBuf,
}

impl Worktree {
    fn add(repo: &Path, rev: &str) -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("ferrograph-pr-{}-{nanos}", std::process::id()));
        let guard = Self {
            repo: repo.to_path_buf(),
            dir,
        };
        active_worktrees().push((guard.repo.clone(), guard.dir.clone()));
        let dir_s = guard.dir.to_string_lossy().into_owned();
        git(
            repo,
            &["worktree", "add", "--detach", "--force", &dir_s, rev],
        )
        .context("failed to check out the base revision into a temporary worktree")?;
        Ok(guard)
    }
}

fn remove_worktree(repo: &Path, dir: &Path) {
    let dir_s = dir.to_string_lossy().into_owned();
    let _ = git_cmd(repo, &["worktree", "remove", "--force", &dir_s]);
    let _ = std::fs::remove_dir_all(dir);
    let _ = git_cmd(repo, &["worktree", "prune"]);
}

impl Drop for Worktree {
    fn drop(&mut self) {
        active_worktrees().retain(|(_, d)| d != &self.dir);
        remove_worktree(&self.repo, &self.dir);
    }
}

/// `(repo, dir)` of the live temporary worktrees, so a signal handler can remove them.
fn active_worktrees() -> std::sync::MutexGuard<'static, Vec<(PathBuf, PathBuf)>> {
    static ACTIVE: std::sync::Mutex<Vec<(PathBuf, PathBuf)>> = std::sync::Mutex::new(Vec::new());
    ACTIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Remove the temp worktrees on SIGINT/SIGTERM; SIGKILL cannot be caught, see
/// [`prune_stale_worktrees`].
fn install_signal_cleanup() {
    // Fails only if a handler is already installed in this process; keep that one.
    let _ = ctrlc::set_handler(|| {
        let active = std::mem::take(&mut *active_worktrees());
        for (repo, dir) in &active {
            remove_worktree(repo, dir);
        }
        std::process::exit(130);
    });
}

fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    // shortcut: no `kill -0` off unix, so such pids count as alive and are never pruned.
    // EPERM (a live process of another user) must count as alive; only ESRCH means gone.
    cfg!(not(unix))
        || Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                o.status.success()
                    || !String::from_utf8_lossy(&o.stderr)
                        .to_lowercase()
                        .contains("no such process")
            })
}

/// Pid in a `<tmp>/ferrograph-pr-<pid>-<nanos>` path, if `path` is one of ours.
fn temp_worktree_pid(path: &Path, tmp: &Path) -> Option<u32> {
    let rest = path.file_name()?.to_str()?.strip_prefix("ferrograph-pr-")?;
    if path.parent()?.canonicalize().ok()?.as_path() != tmp {
        return None;
    }
    rest.split('-').next()?.parse().ok()
}

/// Remove registered `ferrograph-pr-<pid>-*` temp worktrees whose process is gone (killed runs
/// never reach [`Worktree`]'s `Drop`), then `git worktree prune` for those whose directory is gone.
// shortcut: a pid reused by an unrelated live process keeps its stale worktree until that exits.
fn prune_stale_worktrees(repo: &Path) {
    let Some(list) = git_opt(repo, &["worktree", "list", "--porcelain"]) else {
        return;
    };
    let tmp = std::env::temp_dir();
    let tmp = tmp.canonicalize().unwrap_or(tmp);
    for path in list.lines().filter_map(|l| l.strip_prefix("worktree ")) {
        let path = Path::new(path);
        if temp_worktree_pid(path, &tmp).is_some_and(|pid| !pid_alive(pid)) {
            remove_worktree(repo, path);
        }
    }
    let _ = git_cmd(repo, &["worktree", "prune"]);
}

type FnKey = (String, String, usize);

pub(crate) fn split_id(id: &str) -> (String, u32, u32) {
    let (path, pos) = id.rsplit_once('#').unwrap_or((id, "0:0"));
    let (l, c) = pos.split_once(':').unwrap_or(("0", "0"));
    (
        path.strip_prefix("./").unwrap_or(path).to_string(),
        l.parse().unwrap_or(0),
        c.parse().unwrap_or(0),
    )
}

pub(crate) fn bare_name(payload: &str) -> &str {
    let mut p = payload;
    while let Some(rest) = ["pub::", "test::", "bench::"]
        .iter()
        .find_map(|pre| p.strip_prefix(pre))
    {
        p = rest;
    }
    p
}

/// Every function keyed by identity (id kept as value), and the keys that are dead.
struct DeadState {
    all: BTreeMap<FnKey, String>,
    dead: BTreeSet<FnKey>,
}

fn dead_state(
    store: &Store,
    opts: &PrOptions,
    entry: &[String],
    ignore: &[String],
) -> Result<DeadState> {
    let rows = store.run_query(
        &format!(
            "?[id, payload] := *nodes[id, type, payload], type = \"{}\"",
            crate::graph::schema::NodeType::Function
        ),
        BTreeMap::new(),
    )?;
    let mut fns: Vec<(String, u32, u32, String, String)> = rows
        .rows
        .iter()
        .map(|r| {
            let id = unquote_datavalue(&r[0]);
            let payload = r.get(1).map(unquote_datavalue).unwrap_or_default();
            let (path, l, c) = split_id(&id);
            (path, l, c, bare_name(&payload).to_string(), id)
        })
        .collect();
    fns.sort();
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut all = BTreeMap::new();
    let mut by_id: HashMap<String, FnKey> = HashMap::new();
    for (path, _, _, name, id) in fns {
        let n = seen.entry((path.clone(), name.clone())).or_insert(0);
        let key = (path, name, *n);
        *n += 1;
        by_id.insert(id.clone(), key.clone());
        all.insert(key, id);
    }
    let entries = crate::ops::resolve_entry_ids(store, entry)?;
    let pats = ignore
        .iter()
        .map(|p| glob::Pattern::new(p))
        .collect::<Result<Vec<_>, _>>()?;
    let dead = Query::compute_dead_functions_with(store, opts.strict, &entries)?
        .into_iter()
        .filter(|id| !pats.iter().any(|p| crate::ops::path_matches(p, id)))
        .filter_map(|id| by_id.get(&id).cloned())
        .collect();
    Ok(DeadState { all, dead })
}

fn open_graph(path: &Path, what: &str) -> Result<Store> {
    if !path.exists() {
        bail!("{what} graph not found at {}", path.display());
    }
    Store::new_persistent(path)
        .with_context(|| format!("Failed to open graph at {}", path.display()))
}

fn rev(root: &Path, spec: &str) -> Option<String> {
    git_opt(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{spec}^{{commit}}"),
        ],
    )
}

fn open_head(
    opts: &PrOptions,
    root: &Path,
    prefix: &str,
    head: &str,
    warnings: &mut Vec<String>,
) -> Result<Store> {
    if opts.index_head {
        // Index the HEAD commit, not the working tree, so it matches the diff.
        let wt = Worktree::add(root, head)?;
        let s = Store::new_memory()?;
        run_pipeline(&s, &wt.dir.join(prefix), &PipelineConfig::default())?;
        return Ok(s);
    }
    let s = open_graph(&opts.head_db, "Head").map_err(|e| {
        anyhow::anyhow!(
            "{e}. Run 'ferrograph index --output {}' or pass --index-head",
            opts.head_db.display()
        )
    })?;
    match s.get_meta("sources")? {
        Some(fp) if fp != crate::pipeline::source_fingerprint(root).to_string() => bail!(
            "head graph {} is stale (sources changed since it was indexed). \
             Re-run 'ferrograph index' or pass --index-head",
            opts.head_db.display()
        ),
        None => {
            warnings
                .push("head graph has no source fingerprint; cannot check it is current".into());
        }
        _ => {}
    }
    Ok(s)
}

type Changes = (Vec<ChangedFunction>, Vec<ChangedItems>, PrBlast);

fn map_changes(
    opts: &PrOptions,
    root: &Path,
    (prefix, head): (&str, &str),
    head_store: &Store,
    diffs: &[FileDiff],
    warnings: &mut Vec<String>,
) -> Result<Changes> {
    let mut changed_functions = Vec::new();
    let mut changed_without_function = Vec::new();
    let mut union: BTreeMap<String, NodeSummary> = BTreeMap::new();
    let mut truncated = false;
    for d in diffs {
        // Read the HEAD blob, not the working tree: hunk lines are relative to it.
        let Some(source) = git_opt(root, &["show", &format!("{head}:{prefix}{}", d.path)]) else {
            warnings.push(format!("could not read {} at HEAD; skipped", d.path));
            continue;
        };
        let ranges = fn_ranges(&source)?;
        let (fns, outside) = map_hunks(&ranges, &d.hunks);
        if !outside.is_empty() {
            changed_without_function.push(ChangedItems {
                file: d.path.clone(),
                lines: outside,
            });
        }
        for i in fns {
            let r = &ranges[i];
            let id = format!("./{}#{}:{}", d.path, r.start_line, r.start_col);
            let blast = crate::ops::blast_radius(head_store, &id, opts.exclude_low_confidence)?;
            changed_functions.push(ChangedFunction {
                id,
                file: d.path.clone(),
                name: r.name.clone(),
                start_line: r.start_line,
                end_line: r.end_line,
                blast_count: blast.count,
                blast_truncated: blast.truncated,
            });
            truncated |= blast.truncated;
            for n in blast.reachable_nodes {
                union.entry(n.id.clone()).or_insert(n);
            }
        }
    }
    let count = union.len();
    let nodes: Vec<NodeSummary> = union.into_values().take(opts.blast_limit).collect();
    let blast = PrBlast {
        count,
        shown: nodes.len(),
        truncated,
        nodes,
    };

    Ok((changed_functions, changed_without_function, blast))
}

fn newly_dead(head: &DeadState, base: &DeadState, diffs: &[FileDiff]) -> Vec<NewlyDeadFunction> {
    let renames: HashMap<&str, &str> = diffs
        .iter()
        .filter_map(|d| d.old_path.as_deref().map(|o| (o, d.path.as_str())))
        .collect();
    let remap = |k: &FnKey| -> FnKey {
        let path = renames.get(k.0.as_str()).map_or(k.0.as_str(), |p| *p);
        (path.to_string(), k.1.clone(), k.2)
    };
    let base_all: HashSet<FnKey> = base.all.keys().map(remap).collect();
    let base_dead: HashSet<FnKey> = base.dead.iter().map(remap).collect();

    head.dead
        .iter()
        .filter(|k| !base_dead.contains(*k))
        .map(|k| {
            let id = head.all[k].clone();
            NewlyDeadFunction {
                line: split_id(&id).1,
                file: k.0.clone(),
                name: k.1.clone(),
                status: if base_all.contains(k) {
                    "became_dead"
                } else {
                    "added_dead"
                },
                id,
            }
        })
        .collect()
}

fn diff_files(root: &Path, merge_base: &str, head: &str) -> Result<Vec<FileDiff>> {
    let diff_text = git(
        root,
        &[
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "--relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            merge_base,
            head,
            "--",
            "*.rs",
        ],
    )?;
    Ok(parse_diff(&diff_text))
}

/// Compute the [`PrResult`] for `opts.base`..HEAD.
///
/// # Errors
/// Fails if git is unavailable, a ref cannot be resolved (shallow clone, unknown ref), a graph is
/// missing or stale, or analysis fails.
pub fn run(opts: &PrOptions) -> Result<PrResult> {
    let root = opts.root.canonicalize()?;
    if opts.base.starts_with('-') {
        bail!("invalid base ref {:?}", opts.base);
    }
    let prefix = git(&root, &["rev-parse", "--show-prefix"])
        .context("not inside a git repository")?
        .trim()
        .to_string();
    let shallow =
        git_opt(&root, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true");
    let head = rev(&root, "HEAD").context("HEAD does not point to a commit")?;
    let Some(base_sha) = rev(&root, &opts.base) else {
        if shallow {
            bail!(
                "base ref {:?} not found in this shallow clone; fetch full history \
                 (actions/checkout `fetch-depth: 0` or `git fetch --unshallow`)",
                opts.base
            );
        }
        bail!("base ref {:?} not found", opts.base);
    };
    let Some(merge_base) = git_opt(&root, &["merge-base", &base_sha, &head]) else {
        bail!(
            "no merge base between {:?} and HEAD{}",
            opts.base,
            if shallow {
                "; the clone is shallow, fetch full history (`fetch-depth: 0`)"
            } else {
                ""
            }
        );
    };
    prune_stale_worktrees(&root);
    install_signal_cleanup();
    let mut warnings = Vec::new();
    if git_opt(&root, &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty())
    {
        warnings.push(
            "uncommitted changes are ignored: the diff and function ranges come from the HEAD commit; \
a graph given with --db is assumed to match HEAD (--index-head indexes the HEAD commit)"
                .to_string(),
        );
    }

    // Config is read from the HEAD tree for both sides so a config change cannot fake a diff.
    let cfg: crate::config::FileConfig =
        match git_opt(&root, &["show", &format!("HEAD:{prefix}.ferrograph.toml")]) {
            Some(text) => toml::from_str(&text).context("Invalid .ferrograph.toml at HEAD")?,
            None => crate::config::FileConfig::default(),
        };
    let entry: Vec<String> = opts.entry.iter().cloned().chain(cfg.dead.entry).collect();
    let ignore: Vec<String> = opts.ignore.iter().cloned().chain(cfg.dead.ignore).collect();

    let diffs = diff_files(&root, &merge_base, &head)?;

    let head_store = open_head(opts, &root, &prefix, &head, &mut warnings)?;
    let head_state = dead_state(&head_store, opts, &entry, &ignore)?;
    let base_state = if let Some(p) = &opts.base_db {
        warnings.push(format!(
            "using --base-db {}: it is assumed to be indexed from the merge base {merge_base:.8}",
            p.display()
        ));
        dead_state(&open_graph(p, "Base")?, opts, &entry, &ignore)?
    } else {
        let wt = Worktree::add(&root, &merge_base)?;
        let s = Store::new_memory()?;
        run_pipeline(&s, &wt.dir.join(&prefix), &PipelineConfig::default())?;
        dead_state(&s, opts, &entry, &ignore)?
    };

    let newly_dead = newly_dead(&head_state, &base_state, &diffs);
    let (changed_functions, changed_without_function, blast) = map_changes(
        opts,
        &root,
        (&prefix, &head),
        &head_store,
        &diffs,
        &mut warnings,
    )?;

    Ok(PrResult {
        base: opts.base.clone(),
        merge_base,
        head,
        changed_files: diffs.iter().map(|d| d.path.clone()).collect(),
        newly_dead_count: newly_dead.len(),
        newly_dead,
        changed_functions,
        changed_without_function,
        blast,
        coupling: coupling_hints(&head_store, &diffs)?,
        warnings,
    })
}

/// Untouched files coupled (`changes_with`) to changed ones; empty when the graph has no such edges.
fn coupling_hints(store: &Store, diffs: &[FileDiff]) -> Result<Vec<CouplingHint>> {
    let changed: Vec<String> = diffs.iter().map(|d| format!("./{}", d.path)).collect();
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    let mut params = BTreeMap::new();
    params.insert(
        "files".to_string(),
        cozo::DataValue::List(
            changed
                .iter()
                .map(|f| cozo::DataValue::from(f.as_str()))
                .collect(),
        ),
    );
    let rows = store.run_query(
        "?[a, b] := *edges[a, b, \"changes_with\"], a in $files\n\
         ?[a, b] := *edges[b, a, \"changes_with\"], a in $files",
        params,
    )?;
    let mut by_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &rows.rows {
        let (a, b) = (unquote_datavalue(&r[0]), unquote_datavalue(&r[1]));
        if !changed.contains(&b) {
            by_file.entry(a).or_default().insert(b);
        }
    }
    Ok(by_file
        .into_iter()
        .map(|(file, c)| CouplingHint {
            file,
            coupled_with: c.into_iter().collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_new_side_only() {
        let d = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n\
                 @@ -3,2 +3,3 @@ fn x\n+a\n@@ -10 +11 @@\n@@ -20,2 +22,0 @@\n\
                 diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n\
                 diff --git a/gone.rs b/gone.rs\n--- a/gone.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n\
                 diff --git a/x.txt b/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n\
                 diff --git a/img.rs b/img.rs\nBinary files a/img.rs and b/img.rs differ\n";
        let files = parse_diff(d);
        assert_eq!(files.len(), 2, "{files:?}");
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].hunks, vec![(3, 3), (11, 1), (22, 0)]);
        assert_eq!(files[1].path, "new.rs");
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[1].hunks.len(), 0);
    }

    #[test]
    fn body_lines_that_look_like_headers_are_ignored() {
        let d = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1,2 @@\n+++ b/evil.rs\n+rename to x.rs\n";
        let f = parse_diff(d);
        assert_eq!(
            (f.len(), f[0].path.as_str(), f[0].old_path.as_deref()),
            (1, "a.rs", None)
        );
    }

    #[test]
    fn hunk_spanning_two_fns_marks_both() {
        let src = "fn a() {\n    1;\n}\nfn b() {\n    2;\n}\n";
        let r = fn_ranges(src).unwrap();
        let (f, o) = map_hunks(&r, &[(2, 4)]);
        assert_eq!((f, o), (vec![0, 1], vec![]));
    }

    #[test]
    fn maps_to_innermost_fn_and_reports_outside() {
        let src = "use a;\n\nstruct S;\nimpl S {\n    fn outer() {\n        fn inner() {\n            1;\n        }\n        2;\n    }\n}\n";
        let r = fn_ranges(src).unwrap();
        assert_eq!(r.len(), 2);
        let name = |hunks: &[(u32, u32)]| {
            let (f, o) = map_hunks(&r, hunks);
            (f.iter().map(|&i| r[i].name.as_str()).collect::<Vec<_>>(), o)
        };
        assert_eq!(name(&[(7, 1)]).0, ["inner"], "nested fn wins");
        assert_eq!(name(&[(9, 1)]).0, ["outer"], "outer-only line");
        assert_eq!(
            name(&[(1, 1), (3, 1), (4, 1)]),
            (vec![], vec![[1, 1], [3, 4]])
        );
        assert_eq!(name(&[(8, 0)]).0, ["outer"], "deletion inside body");
        assert_eq!(
            name(&[(11, 0)]),
            (vec![], vec![[11, 11]]),
            "deletion between items"
        );
    }

    #[test]
    fn prunes_worktrees_of_dead_pids_only() {
        let git = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .output();
            out.is_ok_and(|o| o.status.success())
        };
        let repo = tempfile::tempdir().unwrap();
        if !git(repo.path(), &["init", "-q"])
            || !git(repo.path(), &["commit", "-q", "--allow-empty", "-m", "x"])
        {
            eprintln!("skipping: git not usable");
            return;
        }
        let mut child = Command::new("git").arg("--version").spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let tmp = std::env::temp_dir();
        let dead = tmp.join(format!("ferrograph-pr-{dead_pid}-{nanos}"));
        let live = tmp.join(format!("ferrograph-pr-{}-{nanos}", std::process::id()));
        let other = tmp.join(format!("unrelated-{nanos}"));
        for d in [&dead, &live, &other] {
            assert!(git(
                repo.path(),
                &["worktree", "add", "--detach", d.to_str().unwrap(), "HEAD"]
            ));
        }
        prune_stale_worktrees(repo.path());
        let (dead_left, live_left, other_left) = (dead.exists(), live.exists(), other.exists());
        let listed = git_opt(repo.path(), &["worktree", "list", "--porcelain"]).unwrap();
        for d in [&live, &other] {
            remove_worktree(repo.path(), d);
        }
        assert!(!dead_left, "dead pid's worktree is removed");
        assert!(
            live_left && other_left,
            "live pid and unrelated worktrees stay"
        );
        assert!(
            !listed.contains(&format!("ferrograph-pr-{dead_pid}-")),
            "{listed}"
        );
    }
}
