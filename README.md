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

# Watch for changes and re-index (requires --output)
cargo run -- watch . --output .ferrograph

# Run MCP server over stdio (for AI agents / IDEs)
cargo run -- mcp
```

## License

MIT
