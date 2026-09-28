# Changelog

All notable changes to lievo will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

### Added

- Prebuilt installers for macOS and Linux (shell script) and Windows (PowerShell) from GitHub Releases — no Rust toolchain required; `cargo install` from git remains as a fallback.
- `lievo mcp` — MCP server over stdio for agent integration. `lievo_explore` is the only tool exposed by default; the full tool set is available by opting in with the `LIEVO_MCP_TOOLS` environment variable.
- Zero-config repository resolution: `lievo mcp` with no arguments finds the git repository it is launched in and registers it automatically, so one user-level registration works in every repo.
- Automatic background indexing: a never-indexed or stale repository is indexed/refreshed in the background when the MCP server starts (core index only — no model downloads). Set `LIEVO_NO_REFRESH=1` to disable.
- `lievo doctor` — one-screen diagnostic that reports registration, index state, environment, and the single action that fixes each problem.
- CLI: `lievo refresh` (index/refresh a repository), `lievo query` (entities, relationships, subsystems, conventions, metrics, dependencies, impact, hotspots, flows), and `lievo admin` (project and repository management).
- Language support: Rust, Python, JavaScript (including TypeScript), and Go.
- Optional features via a manual `lievo refresh`: semantic search with an on-device embedding model, and self-hosted LLM summarization (llama-server, Ollama, apfel, or any compatible backend).
