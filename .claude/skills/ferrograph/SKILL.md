# /ferrograph

Graph-powered Rust code intelligence. Indexes Rust codebases into a queryable knowledge graph via tree-sitter AST extraction, then exposes CLI and MCP interfaces for search, impact analysis, dead code detection, and raw Datalog queries.

## Usage

```
/ferrograph                              # full pipeline on current directory
/ferrograph <path>                       # full pipeline on a specific Rust project
/ferrograph search "<term>"              # text search over indexed symbols
/ferrograph dead                         # list dead (unreachable) code
/ferrograph dead --file "src/**"         # dead code filtered by file glob
/ferrograph blast "<node_id>"            # impact analysis: what breaks if this changes?
/ferrograph callers "<node_id>"          # reverse call graph (who calls this?)
/ferrograph callers "<node_id>" --depth 3  # transitive callers up to N hops
/ferrograph info "<node_id>"             # full detail on a single node (edges in/out)
/ferrograph modules                      # module containment tree
/ferrograph modules --root "./src/"      # module tree filtered by path prefix
/ferrograph traits "<name>"              # find all implementors of a trait
/ferrograph query "<datalog>"            # raw CozoDB Datalog query
/ferrograph watch                        # watch for file changes, auto-reindex
```

Add `--json` to any subcommand for machine-readable output.

## What ferrograph is for

ferrograph builds a persistent knowledge graph from Rust source code using tree-sitter parsing. No LLM tokens are consumed — extraction is entirely deterministic.

Three things it does that reading code cannot:
1. **Cross-file call graph** — traces function calls across module boundaries, through imports and re-exports, including calls inside macro invocations
2. **Impact analysis** — `blast` shows every node transitively reachable from a change point, answering "what breaks if I touch this?"
3. **Dead code detection** — finds functions unreachable from any entry point (`main`, `pub`, `#[test]`, `#[bench]`)

The graph persists in a `.ferrograph` CozoDB database (SQLite-backed). Once indexed, queries are instant. Use `watch` mode to keep the index fresh during development.

## Node ID format

All node IDs are relative paths with line:column anchors:
```
./src/main.rs#42:1        — function at line 42, column 1
./src/lib.rs#10:5         — item at line 10, column 5
```

Use `search` to find node IDs by name rather than constructing them manually.

## What You Must Do When Invoked

If no path is given, use `.` (current directory). Do not ask the user for a path.

### Step 1 — Check installation

```bash
which ferrograph >/dev/null 2>&1 || { echo "ERROR: ferrograph not found. Install with: cargo install ferrograph"; exit 1; }
ferrograph --version
```

If not installed, tell the user and stop. Do not attempt to install it.

### Step 2 — Detect Rust project

```bash
if [ -f "INPUT_PATH/Cargo.toml" ]; then
    echo "Rust project found: INPUT_PATH"
else
    echo "ERROR: No Cargo.toml found at INPUT_PATH"
    exit 1
fi
```

Replace `INPUT_PATH`. If no Rust project is found, stop and tell the user.

### Step 3 — Index

Check whether a fresh index already exists before re-indexing:

```bash
DB_PATH="INPUT_PATH/.ferrograph"
if [ -f "$DB_PATH" ]; then
    NEWEST_RS=$(find INPUT_PATH -name '*.rs' -newer "$DB_PATH" 2>/dev/null | head -1)
    if [ -n "$NEWEST_RS" ]; then
        echo "Index is stale — re-indexing..."
        ferrograph index INPUT_PATH -o "$DB_PATH" 2>&1
    else
        echo "Index is up to date — skipping reindex"
    fi
else
    ferrograph index INPUT_PATH -o "$DB_PATH" 2>&1
fi
```

Note any warnings from the indexer (ambiguous calls, duplicate names) — mention them in the report but don't treat them as errors.

### Step 4 — Explore the graph

Run these **in parallel**:

```bash
# Overview with node/edge type breakdown
ferrograph status INPUT_PATH

# Dead code (includes symbol names)
ferrograph dead -d "$DB_PATH"

# All traits and their implementors
ferrograph traits "" -d "$DB_PATH" 2>&1 || ferrograph query '?[id, payload] := *nodes[id, "trait", payload]' -d "$DB_PATH"

# Unsafe usage
ferrograph query '?[from, to] := *edges[from, to, "uses_unsafe"]' -d "$DB_PATH"
```

### Step 5 — Report findings

```
## Ferrograph Report: PROJECT_NAME

**Graph**: N nodes, M edges (from status breakdown)

### Key Traits
- TraitName (file.rs#line) — N implementors

### Dead Code (N items)
- function_name (file.rs#line)
- ...
(Note: functions called via dyn Trait may appear dead — see caveat below)

### Unsafe Usage
- N unsafe blocks/functions

### Suggested Explorations
(2-3 interesting findings: high-degree nodes, surprising dead code, complex call chains)
```

After the report, offer to dig deeper:
> "Want me to trace the call graph for any of these, or check the blast radius of a specific function?"

---

## Subcommands

### /ferrograph search "\<term\>"

```bash
DB_PATH=$(find . -name '.ferrograph' -maxdepth 3 | head -1)
ferrograph search "TERM" -d "$DB_PATH"
```

Add `-c` for case-insensitive. Use this to find node IDs before running `blast`, `callers`, or `info`.

### /ferrograph dead

```bash
ferrograph dead -d "$DB_PATH"
# Filter by file:
ferrograph dead -d "$DB_PATH" --file "./src/validate/**"
```

Output includes node ID, type, and symbol name. Always mention the dynamic dispatch caveat.

### /ferrograph blast "\<node_id\>"

```bash
ferrograph blast NODE_ID -d "$DB_PATH"
```

If user gives a name not an ID, run `search` first to find the ID.

### /ferrograph callers "\<node_id\>"

```bash
ferrograph callers NODE_ID -d "$DB_PATH"
ferrograph callers NODE_ID -d "$DB_PATH" --depth 3   # transitive
```

### /ferrograph info "\<node_id\>"

```bash
ferrograph info NODE_ID -d "$DB_PATH"
```

Shows type, payload, and all incoming/outgoing edges.

### /ferrograph modules

```bash
ferrograph modules -d "$DB_PATH"
ferrograph modules -d "$DB_PATH" --root "./src/"
```

### /ferrograph traits "\<name\>"

```bash
ferrograph traits "ValidationRule" -d "$DB_PATH"
```

Substring match — pass an empty string to list all traits (may not be supported; fall back to Datalog if it errors).

### /ferrograph query "\<datalog\>"

```bash
ferrograph query 'DATALOG' -d "$DB_PATH"
```

See Datalog Cookbook below. If the query fails, check:
- Missing `*` prefix (use `*nodes`, not `nodes`)
- Column order: `*nodes[id, type, payload]`, `*edges[from_id, to_id, edge_type]`

### /ferrograph watch

```bash
ferrograph watch INPUT_PATH --output INPUT_PATH/.ferrograph
```

Runs in the foreground — tell the user to run this in a separate terminal.

---

## CozoDB Datalog Cookbook

Three relations underpin everything:

```
*nodes[id, type, payload]          — all code entities
*edges[from_id, to_id, edge_type]  — all relationships  
*dead_functions[id]                — unreachable functions
```

**Node types**: `file`, `module`, `function`, `struct`, `enum`, `trait`, `impl`, `type_alias`, `const`, `static`, `macro`, `crate_root`, `primitive`, `external_type`

**Edge types**: `contains`, `imports`, `calls`, `references`, `implements_trait`, `owns`, `borrows`, `borrows_mut`, `expands_to`, `uses_unsafe`, `lifetime_scope`, `changes_with`

### Common recipes

**Functions in a specific file:**
```datalog
?[id, payload] := *nodes[id, "function", payload], starts_with(id, "./src/lib.rs")
```

**Dead functions with names (prefer `ferrograph dead` instead):**
```datalog
?[id, payload] := *dead_functions[id], *nodes[id, _, payload]
```

**Who calls a specific function:**
```datalog
?[caller, payload] := *edges[caller, "TARGET_NODE_ID", "calls"], *nodes[caller, _, payload]
```

**What does a function call:**
```datalog
?[callee, payload] := *edges["SOURCE_NODE_ID", callee, "calls"], *nodes[callee, _, payload]
```

**Transitive callers (recursive):**
```datalog
reach[x] := *edges[x, "TARGET_NODE_ID", "calls"]
reach[x] := *edges[x, mid, "calls"], reach[mid]
?[id, type, payload] := reach[id], *nodes[id, type, payload]
```

**Trait implementations:**
```datalog
?[impl_id, trait_id, trait_name] := *edges[impl_id, trait_id, "implements_trait"], *nodes[trait_id, "trait", trait_name]
```

**Type references from a module:**
```datalog
?[from, to, to_type] := *edges[from, to, "references"], starts_with(from, "./src/validate/"), *nodes[to, to_type, _]
```

**Unsafe code:**
```datalog
?[from, to] := *edges[from, to, "uses_unsafe"]
```

**Borrow relationships:**
```datalog
?[borrower, borrowed, edge_type] := *edges[borrower, borrowed, edge_type], edge_type in ["borrows", "borrows_mut"]
```

**Items with lifetime parameters:**
```datalog
?[id, type, payload] := *edges[id, id, "lifetime_scope"], *nodes[id, type, payload]
```

**All public functions:**
```datalog
?[id, payload] := *nodes[id, "function", payload], starts_with(payload, "pub::")
```

**All test functions:**
```datalog
?[id, payload] := *nodes[id, "function", payload], starts_with(payload, "test::")
```

---

## Using ferrograph with graphify

For monorepos with both Rust and JS/TS code:

- **ferrograph** handles Rust (AST-based, zero token cost, deep Rust semantics)
- **graphify** handles JS/TS + docs + images (AST + LLM semantic extraction)

Each tool produces independent output — ferrograph in `.ferrograph`, graphify in `graphify-out/`. Cross-reference findings manually: if ferrograph shows a Rust validation rule calling `load_spec_dimensions`, and graphify shows the spec documents that define those dimensions, you can trace the full chain.

---

## Honesty rules

- ferrograph uses static analysis only. Functions called via `dyn Trait` (dynamic dispatch) may appear as dead code. Always mention this caveat.
- The `changes_with` edge type requires the `git` feature and may not be populated.
- Node IDs are positional (line:col). Reindex after significant code changes or IDs may be stale.
