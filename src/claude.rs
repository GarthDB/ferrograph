//! Claude Code skill management: install, uninstall, status.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::ClaudeAction;

/// The SKILL.md content, embedded at compile time.
const SKILL_CONTENT: &str = include_str!("../.claude/skills/ferrograph/SKILL.md");

/// The block inserted into `~/.claude/CLAUDE.md`.
const SKILL_ENTRY: &str = "\
### ferrograph
- **ferrograph** (`~/.claude/skills/ferrograph/SKILL.md`) - Rust code intelligence via knowledge graph. Trigger: `/ferrograph`
When the user types `/ferrograph`, invoke the Skill tool with `skill: \"ferrograph\"` before doing anything else.";

/// Check whether content contains a `### ferrograph` skill block.
fn has_skill_block(content: &str) -> bool {
    find_skill_block(content).is_some()
}

/// Extract the `### ferrograph` block content (without trailing newline).
fn extract_skill_block(content: &str) -> Option<&str> {
    let (start, end) = find_skill_block(content)?;
    Some(content[start..end].trim_end())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("cannot determine home directory (HOME not set)")
}

fn skill_file_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude/skills/ferrograph/SKILL.md"))
}

fn claude_md_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude/CLAUDE.md"))
}

/// Dispatch the `ferrograph claude` subcommand.
///
/// # Errors
/// Returns an error if any file I/O operation fails.
pub fn run(action: &ClaudeAction, json: bool) -> Result<()> {
    match action {
        ClaudeAction::Install => run_install(),
        ClaudeAction::Uninstall => run_uninstall(),
        ClaudeAction::Status => run_status(json),
    }
}

fn run_install() -> Result<()> {
    let skill_path = skill_file_path()?;
    let claude_md = claude_md_path()?;

    // Step 1: Write SKILL.md
    if let Some(parent) = skill_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&skill_path, SKILL_CONTENT)?;

    // Step 2: Patch CLAUDE.md
    patch_claude_md(&claude_md)?;

    println!("Installed ferrograph skill.");
    println!("  Skill: {}", skill_path.display());
    println!("  Config: {}", claude_md.display());
    Ok(())
}

fn run_uninstall() -> Result<()> {
    let skill_path = skill_file_path()?;
    let claude_md = claude_md_path()?;

    // Step 1: Remove skill file
    if skill_path.exists() {
        fs::remove_file(&skill_path)?;
        if let Some(parent) = skill_path.parent() {
            let _ = fs::remove_dir(parent); // ignore if not empty
        }
        println!("Removed {}", skill_path.display());
    } else {
        println!("Skill file not found (already removed).");
    }

    // Step 2: Remove entry from CLAUDE.md
    if claude_md.exists() {
        let content = fs::read_to_string(&claude_md)?;
        if has_skill_block(&content) {
            let new_content = remove_skill_block(&content);
            fs::write(&claude_md, new_content)?;
            println!("Removed ferrograph entry from {}", claude_md.display());
        } else {
            println!("No ferrograph entry found in CLAUDE.md.");
        }
    }

    println!("ferrograph skill uninstalled.");
    Ok(())
}

/// Status result for `--json` output.
#[derive(Debug, Serialize)]
struct SkillStatus {
    /// `"installed"`, `"outdated"`, or `"not_installed"`
    status: String,
    skill_file: bool,
    claude_md_entry: bool,
    up_to_date: bool,
    skill_path: String,
    claude_md_path: String,
}

fn run_status(json: bool) -> Result<()> {
    let skill_path = skill_file_path()?;
    let claude_md = claude_md_path()?;

    let skill_file = skill_path.exists();
    let skill_up_to_date = skill_file && fs::read_to_string(&skill_path)? == SKILL_CONTENT;

    let claude_md_content = if claude_md.exists() {
        Some(fs::read_to_string(&claude_md)?)
    } else {
        None
    };
    let claude_md_entry = claude_md_content.as_deref().is_some_and(has_skill_block);
    let entry_up_to_date = claude_md_content
        .as_deref()
        .and_then(extract_skill_block)
        .is_some_and(|block| block == SKILL_ENTRY);
    let up_to_date = skill_up_to_date && entry_up_to_date;
    let installed = skill_file && claude_md_entry;

    let status_label = if installed && up_to_date {
        "installed"
    } else if installed {
        "outdated"
    } else {
        "not_installed"
    };

    if json {
        let status = SkillStatus {
            status: status_label.to_string(),
            skill_file,
            claude_md_entry,
            up_to_date,
            skill_path: skill_path.display().to_string(),
            claude_md_path: claude_md.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        let check = |b: bool| if b { "yes" } else { "no" };
        println!("ferrograph Claude Code skill:");
        println!(
            "  Skill file:   {} ({})",
            check(skill_file),
            skill_path.display()
        );
        println!(
            "  CLAUDE.md:    {} ({})",
            check(claude_md_entry),
            claude_md.display()
        );
        println!("  Up to date:   {}", check(up_to_date));
        if !installed {
            println!("\nRun 'ferrograph claude install' to install.");
        }
    }
    Ok(())
}

// ── CLAUDE.md patching ──────────────────────────────────────────────────────

/// Patch `~/.claude/CLAUDE.md` to include the ferrograph skill entry.
fn patch_claude_md(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = format!("# Claude Code Rules\n\n## Skills\n\n{SKILL_ENTRY}\n");
        fs::write(path, content)?;
        return Ok(());
    }

    let content = fs::read_to_string(path)?;

    if has_skill_block(&content) {
        // Already present — replace block for idempotent upgrade.
        let new_content = replace_skill_block(&content);
        fs::write(path, new_content)?;
        return Ok(());
    }

    // Entry not present — insert or append.
    if let Some(pos) = find_skills_heading_end(&content) {
        let new_content = insert_after_position(&content, pos);
        fs::write(path, new_content)?;
    } else {
        let mut new_content = content;
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("\n## Skills\n\n");
        new_content.push_str(SKILL_ENTRY);
        new_content.push('\n');
        fs::write(path, new_content)?;
    }
    Ok(())
}

/// Find the byte offset just past the `## Skills` heading line (including its newline).
fn find_skills_heading_end(content: &str) -> Option<usize> {
    let mut offset = 0;
    for line in content.lines() {
        let line_end = offset + line.len() + 1; // +1 for \n
        if line.trim() == "## Skills" {
            return Some(line_end.min(content.len()));
        }
        offset = line_end;
    }
    None
}

/// Insert the skill entry right after a given byte position.
fn insert_after_position(content: &str, pos: usize) -> String {
    let mut result = String::with_capacity(content.len() + SKILL_ENTRY.len() + 4);
    result.push_str(&content[..pos]);
    result.push('\n');
    result.push_str(SKILL_ENTRY);
    result.push('\n');
    result.push_str(&content[pos..]);
    result
}

/// Find the byte range of the `### ferrograph` block.
/// The block starts at `### ferrograph\n` and ends at the next heading or EOF.
fn find_skill_block(content: &str) -> Option<(usize, usize)> {
    let needle = "### ferrograph\n";
    let start = content.find(needle)?;
    let after = start + needle.len();
    let end = content[after..]
        .find("\n#")
        .map_or(content.len(), |pos| after + pos);
    Some((start, end))
}

/// Replace the existing `### ferrograph` block with the current entry.
fn replace_skill_block(content: &str) -> String {
    if let Some((start, end)) = find_skill_block(content) {
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..start]);
        result.push_str(SKILL_ENTRY);
        result.push('\n');
        result.push_str(&content[end..]);
        result
    } else {
        content.to_string()
    }
}

/// Remove the `### ferrograph` block from content.
fn remove_skill_block(content: &str) -> String {
    if let Some((start, end)) = find_skill_block(content) {
        let mut result = String::with_capacity(content.len());
        // Trim leading blank line before the block if present
        let start = if start > 0 && content.as_bytes()[start - 1] == b'\n' {
            start - 1
        } else {
            start
        };
        result.push_str(&content[..start]);
        result.push_str(&content[end..]);
        result
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_skills_heading_end_present() {
        let content = "# Rules\n\n## Skills\n\nsome stuff\n";
        let pos = find_skills_heading_end(content).unwrap();
        assert_eq!(&content[..pos], "# Rules\n\n## Skills\n");
    }

    #[test]
    fn find_skills_heading_end_absent() {
        let content = "# Rules\n\n## Other\n";
        assert!(find_skills_heading_end(content).is_none());
    }

    #[test]
    fn insert_after_skills_heading() {
        let content = "# Rules\n\n## Skills\n\n### other\n- other skill\n\n## Config\n";
        let pos = find_skills_heading_end(content).unwrap();
        let result = insert_after_position(content, pos);
        assert!(result.contains("## Skills\n\n### ferrograph\n"));
        assert!(result.contains("## Config\n"));
    }

    #[test]
    fn find_skill_block_present() {
        let content = "## Skills\n\n### ferrograph\nline1\nline2\n\n## Other\n";
        let (start, end) = find_skill_block(content).unwrap();
        let block = &content[start..end];
        assert!(block.starts_with("### ferrograph\n"));
        assert!(block.contains("line2"));
        assert!(!block.contains("## Other"));
    }

    #[test]
    fn find_skill_block_at_eof() {
        let content = "## Skills\n\n### ferrograph\nline1\nline2\n";
        let (start, end) = find_skill_block(content).unwrap();
        assert_eq!(end, content.len());
        assert!(content[start..end].starts_with("### ferrograph\n"));
    }

    #[test]
    fn find_skill_block_absent() {
        assert!(find_skill_block("## Skills\n\n### other\n").is_none());
    }

    #[test]
    fn replace_skill_block_idempotent() {
        let content = format!("## Skills\n\n{SKILL_ENTRY}\n\n## Other\n");
        let once = replace_skill_block(&content);
        let twice = replace_skill_block(&once);
        assert_eq!(once, twice, "replace should be idempotent");
    }

    #[test]
    fn remove_skill_block_clean() {
        let content = format!("## Skills\n\n{SKILL_ENTRY}\n\n## Other\nstuff\n");
        let result = remove_skill_block(&content);
        assert!(!result.contains("ferrograph"));
        assert!(result.contains("## Other\nstuff\n"));
    }

    #[test]
    fn remove_skill_block_noop_when_absent() {
        let content = "## Skills\n\n### other\n- stuff\n";
        assert_eq!(remove_skill_block(content), content);
    }

    #[test]
    fn roundtrip_install_uninstall() {
        let original = "# Rules\n\n## Skills\n\n### other\n- stuff\n\n## Config\nfoo\n";
        let pos = find_skills_heading_end(original).unwrap();
        let installed = insert_after_position(original, pos);
        assert!(has_skill_block(&installed));
        let uninstalled = remove_skill_block(&installed);
        assert!(!has_skill_block(&uninstalled));
        assert!(uninstalled.contains("### other\n- stuff\n"));
        assert!(uninstalled.contains("## Config\nfoo\n"));
    }

    #[test]
    fn patch_creates_file_from_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        patch_claude_md(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("## Skills"));
        assert!(has_skill_block(&content));
    }

    #[test]
    fn patch_appends_when_no_skills_heading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        fs::write(&path, "# Rules\n\n## Other\nstuff\n").unwrap();
        patch_claude_md(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("## Skills"));
        assert!(has_skill_block(&content));
        assert!(content.contains("## Other\nstuff\n"));
    }

    #[test]
    fn patch_inserts_under_skills() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        fs::write(&path, "# Rules\n\n## Skills\n\n### other\n\n## Config\n").unwrap();
        patch_claude_md(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(has_skill_block(&content));
        // Entry should be between Skills and other
        let skills_pos = content.find("## Skills").unwrap();
        let entry_pos = content.find("### ferrograph").unwrap();
        let other_pos = content.find("### other").unwrap();
        assert!(entry_pos > skills_pos);
        assert!(entry_pos < other_pos);
    }

    #[test]
    fn patch_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        patch_claude_md(&path).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        patch_claude_md(&path).unwrap();
        let second = fs::read_to_string(&path).unwrap();
        assert_eq!(first, second, "patch should be idempotent");
    }

    #[test]
    fn stale_block_detected_as_not_up_to_date() {
        let stale = "### ferrograph\n- old description from v1.0\n";
        assert!(has_skill_block(stale), "block should be detected");
        let extracted = extract_skill_block(stale).unwrap();
        assert_ne!(extracted, SKILL_ENTRY, "stale block should not match current entry");
    }

    #[test]
    fn stray_marker_outside_block_not_detected() {
        // The marker string appears in prose but there is no ### ferrograph block.
        let content = "## Notes\n\nWe use skill: \"ferrograph\" in our workflow.\n";
        assert!(
            !has_skill_block(content),
            "stray marker without ### ferrograph heading should not be detected as a block"
        );
    }
}
