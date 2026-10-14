# Changelog

All notable changes to lievo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-10-14

### Bug Fixes

- Describe semantic search as embedding-based in MCP tool descriptions (#15)
- Make the integration test hermetic (fixture repo, no summarizer, no model download) (#25) (#20)
- Honour HF_HOME / HF_HUB_CACHE for the embedding-model cache (#36) (#35)

### Documentation

- Benchmark-backed README positioning and capabilities section (#11)
- Make the README reader-first (#13)
- Plain-language first screen; move benchmark detail to docs/benchmarks.md (#16)
- Document project naming rules and moved-checkout behavior (#27) (#41)

### Features

- Persist normalized repository identity and add identity storage methods (#33) (#31)
- Shared identity-aware repo registration for MCP and CLI (#34) (#29)
- Detect a moved or re-cloned checkout by its stored git identity and keep its index (#37)

### Miscellaneous

- Fix the five lens-review findings on the merged README (#21)
- Remove unused hand-written tool descriptions and schemas (#26) (#22)
- *(deps)* Bump the minor-and-patch group across 1 directory with 4 updates (#2)
- *(deps)* Bump indicatif from 0.17.11 to 0.18.4 (#3)
- *(deps)* Bump the minor-and-patch group with 5 updates (#38)
- Publish to crates.io via release-plz (#40) (#44)

## [0.1.0] - 2026-09-29

### Added

- Prebuilt installers for macOS and Linux (shell script) and Windows (PowerShell) from GitHub Releases — no Rust toolchain required; `cargo install` from git remains as a fallback.
- `lievo mcp` — MCP server over stdio for agent integration. `lievo_explore` is the only tool exposed by default; the full tool set is available by opting in with the `LIEVO_MCP_TOOLS` environment variable. `lievo_explore` query mode now matches both file names/paths and symbol names (functions, types) — an exact symbol-name match ranks above a file-name or path match.
- Zero-config repository resolution: `lievo mcp` with no arguments finds the git repository it is launched in and registers it automatically, so one user-level registration works in every repo.
- Automatic background indexing: a never-indexed or stale repository is indexed/refreshed in the background when the MCP server starts (core index only — no model downloads). Set `LIEVO_NO_REFRESH=1` to disable.
- `lievo doctor` — one-screen diagnostic that reports registration, index state, environment, and the single action that fixes each problem.
- CLI: `lievo refresh` (index/refresh a repository), `lievo query` (entities, relationships, subsystems, conventions, metrics, dependencies, impact, hotspots, flows), and `lievo admin` (project and repository management).
- Language support: Rust, Python, JavaScript (including TypeScript), and Go.
- Optional features via a manual `lievo refresh`: semantic search with an on-device embedding model, and self-hosted LLM summarization (llama-server, Ollama, apfel, or any compatible backend).
