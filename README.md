# Ferrograph

Graph-powered Rust code intelligence. Indexes Rust codebases into a queryable knowledge graph with CLI and MCP interfaces.

## Status

Implements file discovery, tree-sitter AST extraction, CozoDB graph storage, CLI (index, query, search, status, watch), dead code detection, MCP server (dead_code and blast_radius tools), and optional git coupling and rust-analyzer stubs.

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

# Run Datalog queries
cargo run -- query "?[id, type, payload] := *nodes[id, type, payload]"
cargo run -- query --db .ferrograph "?[id] := *dead_functions[id]"

# Text search over node payloads
cargo run -- search --db .ferrograph "main"

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
