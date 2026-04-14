#!/usr/bin/env sh
# Install the ferrograph Claude Code skill.
# Works whether run from a local clone or piped from GitHub:
#   sh install-claude-skill.sh
#   curl -fsSL https://raw.githubusercontent.com/GarthDB/ferrograph/main/install-claude-skill.sh | sh

set -e

SKILL_DIR="$HOME/.claude/skills/ferrograph"
CLAUDE_MD="$HOME/.claude/CLAUDE.md"
REMOTE_SKILL="https://raw.githubusercontent.com/GarthDB/ferrograph/main/.claude/skills/ferrograph/SKILL.md"

# Only resolve a local repo path when $0 points to an actual file (i.e., we
# were invoked as a script). When piped (curl | sh), $0 is the shell binary
# (e.g. /bin/sh) or a stdin placeholder, not our script, so dirname would
# resolve relative to the wrong directory.
if [ -f "$0" ]; then
  REPO_SKILL="$(cd "$(dirname "$0")" 2>/dev/null && pwd)/.claude/skills/ferrograph/SKILL.md"
else
  REPO_SKILL=""
fi

SKILL_ENTRY='### ferrograph
- **ferrograph** (`~/.claude/skills/ferrograph/SKILL.md`) - Rust code intelligence via knowledge graph. Trigger: `/ferrograph`
When the user types `/ferrograph`, invoke the Skill tool with `skill: "ferrograph"` before doing anything else.'

# ── Step 1: get the skill file ────────────────────────────────────────────────

mkdir -p "$SKILL_DIR"

if [ -n "$REPO_SKILL" ] && [ -f "$REPO_SKILL" ]; then
  cp "$REPO_SKILL" "$SKILL_DIR/SKILL.md"
  echo "✓ Copied skill from local repo"
elif command -v curl >/dev/null 2>&1; then
  curl -fsSL "$REMOTE_SKILL" -o "$SKILL_DIR/SKILL.md"
  echo "✓ Downloaded skill from GitHub"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$SKILL_DIR/SKILL.md" "$REMOTE_SKILL"
  echo "✓ Downloaded skill from GitHub"
else
  echo "Error: cannot fetch skill file — no local repo, curl, or wget found." >&2
  exit 1
fi

# ── Step 2: patch ~/.claude/CLAUDE.md ────────────────────────────────────────

if [ ! -f "$CLAUDE_MD" ]; then
  mkdir -p "$(dirname "$CLAUDE_MD")"
  printf '# Claude Code Rules\n\n## Skills\n\n%s\n' "$SKILL_ENTRY" > "$CLAUDE_MD"
  echo "✓ Created $CLAUDE_MD with ferrograph skill entry"
elif grep -q "skill: \"ferrograph\"" "$CLAUDE_MD" 2>/dev/null; then
  echo "✓ Skill entry already present in $CLAUDE_MD"
else
  # Insert after ## Skills heading if present, else append new section at end
  if grep -q "^## Skills" "$CLAUDE_MD" 2>/dev/null; then
    SKILLS_LINE=$(grep -n "^## Skills" "$CLAUDE_MD" | head -1 | cut -d: -f1)
    TMPFILE="$(mktemp)"
    head -n "$SKILLS_LINE" "$CLAUDE_MD" > "$TMPFILE"
    printf '\n%s\n' "$SKILL_ENTRY" >> "$TMPFILE"
    tail -n +"$((SKILLS_LINE + 1))" "$CLAUDE_MD" >> "$TMPFILE"
    mv "$TMPFILE" "$CLAUDE_MD"
  else
    printf '\n## Skills\n\n%s\n' "$SKILL_ENTRY" >> "$CLAUDE_MD"
  fi
  echo "✓ Added ferrograph skill entry to $CLAUDE_MD"
fi

# ── Done ──────────────────────────────────────────────────────────────────────

echo ""
echo "ferrograph Claude Code skill installed."
echo "  Skill: $SKILL_DIR/SKILL.md"
echo "  Config: $CLAUDE_MD"
echo ""
echo "Use /ferrograph in any Claude Code session to index and explore a Rust project."
echo "Note: 'ferrograph claude install' is coming in a future release (see issue #47)."
