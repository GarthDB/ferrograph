# Ferrograph

Graph-powered Rust code intelligence. Indexes Rust codebases into a queryable knowledge graph with CLI and MCP interfaces.

## Status

Early stage: file discovery, tree-sitter AST extraction, CozoDB graph storage, and CLI scaffold. Dead code detection, MCP server, and rust-analyzer integration are planned.

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

# Query and search (not yet implemented)
cargo run -- query "?[id, type] := *nodes[id, type, payload]"
cargo run -- status .
```

## License

MIT
