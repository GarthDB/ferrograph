# Ferrograph

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

## License

MIT
