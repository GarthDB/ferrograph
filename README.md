<p align="center">
  <img src="assets/logo-stacked.svg" alt="Ferrograph" width="320" />
</p>

<p align="center">
  <a href="https://crates.io/crates/ferrograph"><img src="https://img.shields.io/crates/v/ferrograph" alt="crates.io"></a>
  <a href="https://github.com/GarthDB/ferrograph/actions/workflows/ci.yml"><img src="https://github.com/GarthDB/ferrograph/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/GarthDB/ferrograph/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/ferrograph" alt="License"></a>
</p>

Graph-powered Rust code intelligence. Indexes Rust codebases into a queryable knowledge graph with CLI and MCP interfaces.

## Status

Implements file discovery, tree-sitter AST extraction (functions, structs, enums, traits, impls, consts, statics, macros, modules), `mod`/`use` resolution with Imports edges, call graph construction (same-file and cross-file via imports; includes calls inside macro invocations such as `format!()` and `println!()`), type reference edges, trait impl edges, macro expansion edges, unsafe usage edges, dead code detection (`pub`, `main`, `#[test]`, and `#[bench]` entry points), CozoDB graph storage, CLI (index, query, search, status, watch), MCP server with 10 tools (`reindex`, `status`, `search`, `node_info`, `dead_code`, `blast_radius`, `callers`, `query`, `trait_implementors`, `module_graph`), and optional git change-coupling analysis. Node IDs are relative to the project root (e.g. `./src/main.rs#10:1`). Ownership/borrowing resolution is stubbed for future rust-analyzer integration.

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

### Setup with Cursor

Add to `.cursor/mcp.json` in your project (or global settings):

```json
{
  "mcpServers": {
    "ferrograph": {
      "command": "ferrograph",
      "args": ["mcp"],
      "env": {
        "FERROGRAPH_DB": "/absolute/path/to/your/project/.ferrograph"
      }
    }
  }
}
```

If building from source instead of installing via `cargo install ferrograph`:

```json
{
  "mcpServers": {
    "ferrograph": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/ferrograph/Cargo.toml", "--", "mcp"],
      "env": {
        "FERROGRAPH_DB": "/absolute/path/to/your/project/.ferrograph"
      }
    }
  }
}
```

### Setup with Claude Desktop

Add to your Claude Desktop config (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "ferrograph": {
      "command": "ferrograph",
      "args": ["mcp"],
      "env": {
        "FERROGRAPH_DB": "/absolute/path/to/your/project/.ferrograph"
      }
    }
  }
}
```

### First use

You can either pre-index your project or let the MCP `reindex` tool bootstrap it:

```bash
# Option 1: Pre-index from the CLI
cd /path/to/your/rust/project
ferrograph index . --output .ferrograph

# Option 2: Skip this step — use the reindex MCP tool from your AI agent
```

The `FERROGRAPH_DB` env var is optional if you run the MCP server from the project root (it defaults to `.ferrograph` in the current directory).

### MCP tools

Node IDs use the format `./path/to/file.rs#line:col` (relative to the project root).

| Tool | Description |
|------|-------------|
| `reindex` | Re-index the project; can bootstrap from scratch (no pre-existing DB needed). |
| `status` | Node/edge counts, DB path, `indexed_at` timestamp. |
| `search` | Text search over node payloads; supports limit/offset pagination. |
| `node_info` | Type, payload, and incoming/outgoing edges for a node ID. |
| `dead_code` | Functions not reachable from entry points; optional `node_type` and `file_glob` filters. |
| `blast_radius` | Transitive impact set via calls, references, and changes_with edges. |
| `callers` | Direct and transitive callers up to a given depth. |
| `query` | Raw Datalog queries (read-only; mutations rejected). |
| `trait_implementors` | Find implementations of a named trait (stub; returns empty with note). |
| `module_graph` | File-to-module containment edges; optional relative path prefix filter (e.g. `./src/`). |

## Graph schema (edge types)

The schema defines 11 edge types; 7 are currently populated:

| Edge type           | v1 populated | Notes |
|---------------------|-------------|-------|
| `contains`          | Yes         | File/module containment. |
| `imports`           | Yes         | From `mod`/`use` resolution. |
| `calls`             | Yes         | Same-file and cross-file (via imports) calls. |
| `references`        | Yes         | Type mentions in fields, params, return types. |
| `implements_trait`  | Yes         | Trait impls (`impl Trait for Type`). |
| `owns`              | No          | Planned. |
| `borrows`           | No          | Planned. |
| `expands_to`        | Yes         | Macro invocation to macro definition. |
| `uses_unsafe`       | Yes         | Unsafe blocks and `unsafe fn`/`unsafe impl`. |
| `lifetime_scope`    | No          | Planned. |
| `changes_with`      | Yes (requires `git` feature) | Git change coupling (optional feature). |

## Publishing

Published to [crates.io](https://crates.io/crates/ferrograph). `Cargo.toml` pins rayon to `>=1.5, <1.11` because cozo's transitive `graph_builder` 0.4.1 is incompatible with rayon 1.11+. This pin does not affect end users.

See [CHANGELOG.md](CHANGELOG.md) for version history.

## License

MIT
