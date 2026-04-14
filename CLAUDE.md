# Ferrograph — Claude Code Rules

## Rust Quality Standards

### Clippy

This project enforces **`clippy::pedantic`** (see `[lints.clippy]` in `Cargo.toml`).

- Run `cargo clippy -- -D warnings` after every meaningful code change. All warnings must be resolved before considering work complete.
- Treat clippy suggestions as requirements, not optional advice.
- When clippy offers a cleaner idiom (e.g., `if let` instead of `match` with one arm, `map_or` instead of `match` on `Option`), prefer the idiomatic form.
- Do not add `#[allow(...)]` attributes unless there is a genuinely unavoidable false positive. If suppression is necessary, leave a comment explaining why.

### Formatting

- Run `cargo fmt` before every commit. Code that does not pass `cargo fmt --check` must not be committed.
- Do not use `#[rustfmt::skip]` unless there is a strong readability reason with an accompanying comment.

### Pre-commit checklist

Run in order before every commit:
1. `cargo fmt`
2. `cargo clippy -- -D warnings`
3. `cargo test`

All three must pass before creating a commit.

## Commit Messages

All commits must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>[optional scope]: <description>
```

Common types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`, `perf`.

Examples:
- `feat(cli): add named subcommands for dead code and blast radius`
- `fix(parser): handle nested impl blocks`
- `docs: add Claude Code integration to README`

## Architecture

### CozoDB schema

Three stored relations underpin the graph:

| Relation | Key | Columns |
|---|---|---|
| `nodes` | `id` (String) | `type` (String), `payload` (String?) |
| `edges` | `from_id, to_id, edge_type` (all String) | — |
| `dead_functions` | `id` (String) | — |

Node IDs are relative to the project root: `./src/main.rs#line:col`

### Pipeline phases

Index runs 12 phases in order: file discovery → AST extraction → primitive nodes → module resolution → call graph → impl→trait edges → type references → owns edges → borrows edges → borrows_mut edges → macro expansion → dead code detection.

Optional: git change-coupling (`changes_with` edges) via `git` feature flag.

### MCP server

10 tools: `reindex`, `status`, `search`, `node_info`, `dead_code`, `blast_radius`, `callers`, `query`, `trait_implementors`, `module_graph`.

The `query` tool is read-only — mutations (`:put`, `:rm`, `:create`, etc.) are blocked at parse time.

## Testing

- Integration tests live in `tests/`
- Unit tests live alongside source in `src/`
- Run `cargo test` to execute all tests
- When adding a new CLI subcommand or MCP tool, add a corresponding integration test

## Using the Claude Code Skill

A Claude Code skill for ferrograph lives at `.claude/skills/ferrograph/SKILL.md`. It covers the full pipeline: indexing, exploring, reporting, all subcommands, and the CozoDB Datalog cookbook.

To install it locally, symlink or copy to your user skill directory:

```bash
# Symlink (stays in sync with repo updates)
mkdir -p ~/.claude/skills/ferrograph
ln -sf "$(pwd)/.claude/skills/ferrograph/SKILL.md" ~/.claude/skills/ferrograph/SKILL.md

# Or copy
cp .claude/skills/ferrograph/SKILL.md ~/.claude/skills/ferrograph/SKILL.md
```

Then add to your `~/.claude/CLAUDE.md`:

```markdown
### ferrograph
- **ferrograph** (`~/.claude/skills/ferrograph/SKILL.md`) - Rust code intelligence via knowledge graph. Trigger: `/ferrograph`
When the user types `/ferrograph`, invoke the Skill tool with `skill: "ferrograph"` before doing anything else.
```
