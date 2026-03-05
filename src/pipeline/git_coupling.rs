//! Phase 10: git change coupling analysis (requires `git` feature).

use std::path::Path;

use anyhow::Result;

use crate::graph::Store;

/// Analyze which files change together (git history) and add coupling edges.
///
/// Walks recent commits and for each commit adds `ChangesWith` edges between
/// pairs of Rust files that were changed together.
///
/// # Errors
/// Fails if git history cannot be read.
pub fn analyze_git_coupling(store: &Store, root: &Path) -> Result<()> {
    #[cfg(feature = "git")]
    {
        analyze_git_coupling_impl(store, root)?;
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, root);
    }
    Ok(())
}

#[cfg(feature = "git")]
const MAX_GIT_COMMITS: usize = 200;

#[cfg(feature = "git")]
fn analyze_git_coupling_impl(store: &Store, root: &Path) -> Result<()> {
    use std::collections::HashSet;

    use crate::graph::schema::{EdgeType, NodeId};

    let root = root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("canonicalize root: {e}"))?;
    let repo = gix::open(&root).map_err(|e| anyhow::anyhow!("open repo: {e}"))?;
    let head = repo
        .head_id()
        .map_err(|e| anyhow::anyhow!("head_id: {e}"))?;
    let mut count = 0usize;
    let mut walk = repo
        .rev_walk([head.detach()])
        .all()
        .map_err(|e| anyhow::anyhow!("rev_walk: {e}"))?;

    while count < MAX_GIT_COMMITS {
        let Some(Ok(info)) = walk.next() else {
            break;
        };
        count += 1;
        let commit = info.object().map_err(|e| anyhow::anyhow!("object: {e}"))?;
        let tree_id = commit
            .tree_id()
            .map_err(|e| anyhow::anyhow!("tree_id: {e}"))?;
        let parent_ids: Vec<_> = info.parent_ids().collect();
        let Some(parent_id) = parent_ids.into_iter().next() else {
            continue;
        };
        let parent_commit = parent_id
            .object()
            .map_err(|e| anyhow::anyhow!("parent object: {e}"))?
            .into_commit();
        let parent_tree_id = parent_commit
            .tree_id()
            .map_err(|e| anyhow::anyhow!("parent tree_id: {e}"))?;
        let parent_tree = repo
            .find_object(parent_tree_id)
            .map_err(|e| anyhow::anyhow!("find parent tree: {e}"))?
            .into_tree();
        let tree = repo
            .find_object(tree_id)
            .map_err(|e| anyhow::anyhow!("find tree: {e}"))?
            .into_tree();
        let diff = repo
            .diff_tree_to_tree(
                Some(&parent_tree),
                Some(&tree),
                Some(gix::diff::Options::default()),
            )
            .map_err(|e| anyhow::anyhow!("diff: {e}"))?;
        let mut rs_files: HashSet<String> = HashSet::new();
        for change in &diff {
            let loc = change.location();
            if loc.ends_with(b".rs") {
                let path_str = String::from_utf8_lossy(loc.as_ref());
                let path = std::path::Path::new(path_str.as_ref());
                let full = root.join(path);
                if let Ok(canon) = full.canonicalize() {
                    let rel = canon
                        .strip_prefix(&root)
                        .unwrap_or_else(|_| canon.as_path());
                    rs_files.insert(format!("./{}", rel.to_string_lossy()));
                } else {
                    rs_files.insert(format!("./{}", path.to_string_lossy()));
                }
            }
        }
        let files: Vec<String> = rs_files.into_iter().collect();
        for (i, a) in files.iter().enumerate() {
            for b in files.iter().skip(i + 1) {
                let from = NodeId(a.clone());
                let to = NodeId(b.clone());
                store.put_edge(&from, &to, &EdgeType::ChangesWith)?;
            }
        }
    }
    Ok(())
}
