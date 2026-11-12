# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [4.1.0](https://github.com/GarthDB/ferrograph/compare/v4.0.4...v4.1.0) - 2026-11-12

### Added

- *(calls)* type receivers bound through tuples, map iteration, Option adapters and fn pointers ([#115](https://github.com/GarthDB/ferrograph/pull/115))
- *(calls)* type receivers through call return types and method chains
- *(calls)* resolve method calls through receiver types

### Fixed

- *(calls)* leave a name renamed to different items in one file unresolved
- *(calls)* resolve qualified calls through use-as aliases of types in other files
- *(calls)* scope function-local use-as aliases and type receivers behind lock/RefCell guards
- *(ast)* index type aliases once per file, stop alias cycles, type empty tuple hints as _
- *(calls)* accept pub(crate) use re-exports when typing module-path receivers ([#91](https://github.com/GarthDB/ferrograph/pull/91))
- *(calls)* accept module-path receiver types only for project modules ([#91](https://github.com/GarthDB/ferrograph/pull/91))
- *(calls)* type receivers whose declared type is a module path ([#91](https://github.com/GarthDB/ferrograph/pull/91))
- *(calls)* keep a child-module match inside the caller's crate
- *(calls)* resolve glob imports, module-file paths, #[path] modules and non-src bin roots to certain edges ([#124](https://github.com/GarthDB/ferrograph/pull/124))
- *(calls)* map iteration yields pairs, hasher is not the value type, Rc::clone is not the inner clone
- *(calls)* resolve bare tuple-variant patterns among the variants of the same file
- *(calls)* prefer the same-file type when several types share a name
- *(calls)* type receivers bound by patterns, for loops and closures ([#115](https://github.com/GarthDB/ferrograph/pull/115))
- *(calls)* alias call behind an unconfirmed leading path segment is low-confidence ([#117](https://github.com/GarthDB/ferrograph/pull/117))
- *(calls)* resolve qualified calls through use-as aliases of a module ([#117](https://github.com/GarthDB/ferrograph/pull/117))
- *(modules)* importing a module itself leaves no imports edge
- *(modules)* an unfound CamelCase import leaf is not evidence about bare fn calls
- *(calls)* only module-level imports make the global fallback low-confidence
- *(modules)* honour #[path] mod declarations when resolving use paths
- *(modules)* resolve use declarations (crate::/self::/pub use/child modules/braces/workspace members)
- *(calls)* library paths and std-qualified types never take a project fn's or type's identity
- *(calls)* a bare call never binds to a method of the same file
- *(calls)* resolve calls through 'use .. as ..' renames
- *(calls)* a lone same-file candidate must match the path owner
- *(calls)* resolve self.m() against the enclosing impl's type first
- *(calls)* names imported from other crates are not matched to same-named local fns
- *(index)* skip unreadable entries (dangling symlinks) instead of aborting the index
- caveat typo; unresolved-import bare fallback is low-confidence; test
- *(dead)* treat #[test_case], #[rstest] and #[wasm_bindgen_test] fns as test roots
- *(dead)* keep pub(crate) as roots until call resolution improves; #91 test ignored
- *(calls)* fall back to the project-wide lookup when a file's use paths do not resolve a bare call
- *(calls)* resolve module paths through re-exports and unknown owners as low-confidence
- *(calls)* resolve qualified path calls whose owner is in another file
- *(dead)* restricted-visibility fns are not roots; attribute-argument names are references

### Other

- *(calls)* describe the new receiver hint forms
- *(calls)* cover map indexing and Rc::clone receivers ([#115](https://github.com/GarthDB/ferrograph/pull/115))
- *(modules)* index items per file once instead of one query per use
- cover imported-fn and std-qualified type receivers
- update the #91 ignore note with the receiver-type numbers
- *(calls)* describe typed receivers in the module docs
- add compiler-ground-truth accuracy harness for dead and pr
- address review feedback on accuracy wording
- state accuracy limits of dead and pr
- *(skill)* state accuracy limits of dead code results

## [4.0.4](https://github.com/GarthDB/ferrograph/compare/v4.0.3...v4.0.4) - 2026-11-10

### Fixed

- *(pipeline)* keep fns referenced from const and static initialisers alive
- *(pipeline)* scan bracket and brace macro bodies for calls
- *(pipeline)* resolve Self::f and Type::f among same-named assoc fns
- *(pipeline)* resolve same-named fns by lexical scope

## [4.0.3](https://github.com/GarthDB/ferrograph/compare/v4.0.2...v4.0.3) - 2026-11-09

### Fixed

- *(pipeline)* emit implements_trait edges for generic traits; cover plain no_mangle and block comments
- *(dead)* treat trait-impl methods and no_mangle/export_name fns as roots, skip comments before #[test]
- *(pipeline)* handle method turbofish in macro args, skip clones for shared names
- *(pipeline)* extract method and turbofish calls

### Other

- cover method and turbofish call edges

## [4.0.2](https://github.com/GarthDB/ferrograph/compare/v4.0.1...v4.0.2) - 2026-11-09

### Fixed

- *(deps)* update rmcp to 3.9.0 for RUSTSEC-2026-0457/0461/0462
- *(deps)* update tree-sitter-rust to 0.24.2 for edition 2024 syntax

### Other

- *(pipeline)* assert exact function names in raw-local test
- Merge branch 'main' into fix/81-tree-sitter-rust-bump
- *(pipeline)* cover unsafe extern blocks and &raw locals

## [4.0.1](https://github.com/GarthDB/ferrograph/compare/v4.0.0...v4.0.1) - 2026-11-09

### Other

- make CI snippet copy-pasteable and note read-only token behaviour
- add CI section linking to ferrograph-action

## [4.0.0](https://github.com/GarthDB/ferrograph/compare/v3.0.0...v4.0.0) - 2026-11-09

### Added

- *(cli)* add --format with sarif and markdown output
- *(pr)* add pr subcommand reporting newly dead functions and blast radius
- *(cli)* exit with code 2 on runtime errors
- *(cli)* add --fail-on-any/--max and documented exit codes
- *(cli)* accept --db on status/index/watch and project dirs everywhere

### Fixed

- *(cli)* treat unsignalable pids as alive, encode SARIF uris, escape more markdown
- *(pr)* clean up stale temporary worktrees
- *(graph)* stop capping blast radius counts at 500
- *(skill)* add required YAML frontmatter
- *(pr)* read function ranges and index head from the HEAD commit, warn on trusted graphs

### Other

- document the pr subcommand
- cover pr subcommand with real git repositories
- clarify --db on index/watch is the output path

## [3.0.0](https://github.com/GarthDB/ferrograph/compare/v2.0.0...v3.0.0) - 2026-11-09

### Added

- *(dead)* entry/ignore config via .ferrograph.toml and --entry/--ignore
- *(graph)* keep ambiguous calls as low-confidence edges

### Fixed

- *(graph)* satisfy clippy pedantic in low-confidence code
- *(pipeline)* treat fns used as values as references, not dead

### Other

- *(release)* build binaries from release-plz workflow; add aarch64 linux and SHA256SUMS

## [2.0.0](https://github.com/GarthDB/ferrograph/compare/v1.5.1...v2.0.0) - 2026-11-05

### Added

- *(mcp)* [**breaking**] track the indexed root and a source fingerprint in the graph
- *(mcp)* reindex a stale graph before answering tool calls

### Fixed

- *(deps)* drop cozo's unused requests feature (rustls 0.21 advisories)

### Other

- satisfy clippy::assert_is_empty (Rust 1.99)

## [1.5.1](https://github.com/GarthDB/ferrograph/compare/v1.5.0...v1.5.1) - 2026-09-15

### Fixed

- upgrade rmcp 0.16 -> 3.4 to fix Copilot CLI handshake crash

### Other

- *(release-plz)* use stable Rust toolchain instead of MSRV pin
- *(audit)* ignore new transitive RUSTSEC advisories

## [1.5.0](https://github.com/GarthDB/ferrograph/compare/v1.4.0...v1.5.0) - 2026-04-15

### Added

- *(cli)* ferrograph claude install/uninstall/status ([#47](https://github.com/GarthDB/ferrograph/pull/47))

### Fixed

- *(claude)* use block-based detection for all CLAUDE.md operations
- *(claude)* replace excessive bools with status string field
- *(installer)* use file check on \$0 instead of tty check for local detection
- *(installer)* always download from GitHub when piped, insert skill under correct heading

### Other

- fix rustfmt formatting in claude tests
- *(claude)* add tests for stale block and stray marker detection
- add install-claude-skill.sh and simplify Claude Code README section
- add Claude Code skill and CLAUDE.md integration

## [1.4.0](https://github.com/GarthDB/ferrograph/compare/v1.3.0...v1.4.0) - 2026-04-14

### Added

- *(cli)* named subcommands, --json flag, richer status, dead code symbols (#41, #42, #43, #44)

### Fixed

- *(ops)* propagate trait_implementors errors, use typed JSON in query results

### Other

- *(cli)* add integration tests for new subcommands and --json flag

## [1.3.0](https://github.com/GarthDB/ferrograph/compare/v1.2.0...v1.3.0) - 2026-04-14

### Added

- *(pipeline)* add BorrowsMut, ExternalType, retain edges to unresolved types
- *(pipeline)* emit owns/borrows edges for function return types ([#39](https://github.com/GarthDB/ferrograph/pull/39))
- *(pipeline)* add primitive type nodes to the graph ([#37](https://github.com/GarthDB/ferrograph/pull/37))

### Fixed

- *(ci)* use is_some_and in borrows tests for clippy map_unwrap_or
- *(ci)* use char 'S' in borrows_mut test for clippy single_char_pattern

### Other

- *(docs)* PR #40 — idempotence, qualified names, return &mut, README

## [1.2.0](https://github.com/GarthDB/ferrograph/compare/v1.1.0...v1.2.0) - 2026-03-09

### Added

- *(pipeline)* populate lifetime_scope edge type ([#31](https://github.com/GarthDB/ferrograph/pull/31))
- *(pipeline)* populate owns and borrows edge types ([#30](https://github.com/GarthDB/ferrograph/pull/30))

## [1.1.0](https://github.com/GarthDB/ferrograph/compare/v1.0.1...v1.1.0) - 2026-03-05

### Added

- *(pipeline)* populate uses_unsafe, implements_trait, references, expands_to edges ([#25](https://github.com/GarthDB/ferrograph/pull/25))

## [1.0.1](https://github.com/GarthDB/ferrograph/compare/v1.0.0...v1.0.1) - 2026-03-05

### Other

- add GitHub Pages workflow for metaball demo ([#19](https://github.com/GarthDB/ferrograph/pull/19))
- add MCP setup instructions for Cursor and Claude Desktop
- add crates.io, CI, and license badges to README

No changes yet.

## [1.0.0] - 2026-03-04

### Added

- Initial stable release.
- File discovery and tree-sitter AST extraction (functions, structs, enums, traits, impls, consts, statics, macros, modules).
- `mod`/`use` resolution with Imports edges.
- Call graph construction (same-file and cross-file via imports).
- Dead code detection with entry points: `pub`, `main`, `#[test]`, and `#[bench]`.
- CozoDB graph storage (in-memory and persistent).
- CLI: `index`, `query`, `search`, `status`, `watch`.
- MCP server with tools: `reindex`, `status`, `search`, `node_info`, `dead_code`, `blast_radius`, `callers`, `query`, `trait_implementors`, `module_graph`.
- Optional git change-coupling analysis (`git` feature).

[Unreleased]: https://github.com/GarthDB/ferrograph/compare/v1.3.0...HEAD
[1.0.0]: https://github.com/GarthDB/ferrograph/releases/tag/v1.0.0
