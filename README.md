# Ferrograph

<p align="center">
  <img src="assets/logo-wordmark.svg" alt="Ferrograph" width="240" />
</p>

Graph-powered Rust code intelligence. Indexes Rust codebases into a queryable knowledge graph with CLI and MCP interfaces.

## Status

Implements file discovery, tree-sitter AST extraction (functions, structs, enums, traits, impls, consts, statics, macros, modules), `mod`/`use` resolution with Imports edges, call graph construction (same-file and cross-file via imports), dead code detection (`pub`, `main`, and `#[test]` entry points), CozoDB graph storage, CLI (index, query, search, status, watch), MCP server (`dead_code` and `blast_radius` tools), and optional git change-coupling analysis. Trait/ownership resolution is stubbed for future rust-analyzer integration.

## Build

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
```

## Usage

```bash
# Index current directory (in-memory)
cargo run -- index .

# Index to a persistent database
cargo run -- index . --output .ferrograph

# Run Datalog queries (requires a persistent database)
cargo run -- query --db .ferrograph "?[id, type, payload] := *nodes[id, type, payload]"
cargo run -- query --db .ferrograph "?[id] := *dead_functions[id]"

# Text search over node payloads (use -c for case-insensitive)
cargo run -- search --db .ferrograph "main"
cargo run -- search --db .ferrograph -c "greet"

# Show graph stats
cargo run -- status .

# Watch for changes and re-index (--output required)
cargo run -- watch . --output .ferrograph

# Run MCP server over stdio (for AI agents / IDEs).
# The server looks for a graph at FERROGRAPH_DB or .ferrograph in the current directory.
# Set FERROGRAPH_DB to the path of your index to use a specific graph.
cargo run -- mcp
```

**MCP configuration:** Point the MCP server at your graph by either running it from the project root (after `ferrograph index --output .ferrograph`) or setting the `FERROGRAPH_DB` environment variable to the path of your `.ferrograph` (or other) database file.

## Graph schema (edge types)

The schema defines 11 edge types; in v1 only a subset are populated:

| Edge type           | v1 populated | Notes |
|---------------------|-------------|-------|
| `contains`          | Yes         | File/module containment. |
| `imports`           | Yes         | From `mod`/`use` resolution. |
| `calls`             | Yes         | Same-file and cross-file (via imports) calls. |
| `references`        | No          | Planned (e.g. type mentions). |
| `implements_trait`  | No          | Planned (rust-analyzer integration). |
| `owns` / `borrows`  | No          | Planned. |
| `expands_to`        | No          | Macro expansion. |
| `uses_unsafe`       | No          | Planned. |
| `lifetime_scope`    | No          | Planned. |
| `changes_with`      | With `git`  | Git change coupling (optional feature). |

## Publishing

`Cargo.toml` uses a `[patch.crates-io]` for `graph_builder` (transitive via cozo) because crates.io’s graph_builder 0.4.1 has a rayon compatibility bug. The patch is under `patches/graph_builder`. To publish ferrograph to crates.io, remove the patch once upstream [graph_builder](https://github.com/neo4j-labs/graph) releases a fix, or publish a fixed fork and patch by version instead of path. Until then, `cargo publish --dry-run` will fail verification (the packaged crate does not apply the patch).

## License

MIT
