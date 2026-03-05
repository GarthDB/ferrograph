# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/GarthDB/ferrograph/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/GarthDB/ferrograph/releases/tag/v1.0.0
