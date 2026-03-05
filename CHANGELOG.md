# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No changes yet.

## [1.0.0] - 2026-03-01

### Added

- Initial stable release.
- File discovery and tree-sitter AST extraction (functions, structs, enums, traits, impls, consts, statics, macros, modules).
- `mod`/`use` resolution with Imports edges.
- Call graph construction (same-file and cross-file via imports).
- Dead code detection with entry points: `pub`, `main`, `#[test]`, and `#[bench]`.
- CozoDB graph storage (in-memory and persistent).
- CLI: `index`, `query`, `search`, `status`, `watch`.
- MCP server with tools: `dead_code`, `blast_radius`, `search`, `query`, `reindex`, and related resources.
- Optional git change-coupling analysis (`git` feature).

### Note

- crates.io publish is blocked until the `graph_builder` patch is removed; see README.

[Unreleased]: https://github.com/GarthDB/ferrograph/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/GarthDB/ferrograph/releases/tag/v1.0.0
