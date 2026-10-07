# lievo

**Give your coding agent a map of your codebase.**

[![CI](https://github.com/trail-openers/lievo/actions/workflows/ci.yml/badge.svg)](https://github.com/trail-openers/lievo/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

lievo indexes how your code fits together — modules, functions, and what calls or imports what — and gives that map to your agent (Claude Code, Cursor, Codex and others) over MCP. Instead of grepping and reading file after file to work out how things connect, the agent asks once.

**What it's for**

- "What breaks if I change this?" — one query lists the affected modules and callers.
- Long sessions on a big codebase — the map persists between sessions and updates only what changed.
- Local — the index is a SQLite file on your machine; no account or API key needed. The only download is a one-time ~16 MB embedding model for semantic search.

**Does it help? We measured it.** On long, multi-step work in an 11,000-file codebase, agents that had lievo alongside their normal search tools gave more precise answers than agents without it — fewer irrelevant files, just as many of the right ones (answer quality 0.66 vs 0.55). On working out what a change will affect, and on finding a bug from a description, it made no measurable difference — and it never made results worse. Small study, one codebase, one model: full results and caveats → [docs/benchmarks.md](docs/benchmarks.md).

**When it won't help:** lievo reads Rust, Python, JavaScript/TypeScript and Go only; code in other languages (Ruby, Java, C#, …) is invisible to it.

## What lievo returns

The example below was produced by running lievo at commit `d5c45c6` (2026-09-30) against the lievo repository itself, registered as project `lievo`. Index any repo you have — `lievo mcp` and `lievo admin add-repo` both default the project name to the checkout directory name — and run the same command with your project's name to reproduce it.

```bash
lievo query entities --project lievo "storage"
```

```
ENTITIES
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Name                                 Path                            Tier      Summary
  add_output_dir                       src/storage/sqlite.rs           function  -
  add_output_dir                       src/storage/sqlite_project.rs   function  -
  add_output_dir                       src/storage/sqlite_ref_impl.rs  function  -
  add_repo                             src/storage/sqlite_ref_impl.rs  function  -
  add_repo                             src/storage/sqlite.rs           function  -
  build_entity_search_query            src/storage/queries.rs          function  -
  build_entity_search_query_with_tier  src/storage/queries.rs          function  -
```

Each row is a real entity in the index — a function, file, module, or subsystem — with its path, tier, and (if present) a cached summary. `lievo query entities` is keyword search; add `--semantic` for embedding-based search over the same index. The same surface also answers structural questions:

```bash
lievo query impact --project lievo src/storage/sqlite.rs
```

This lists the modules, subsystems, and functions affected by a change to that file — the cross-referencing an agent would otherwise do file by file.

## Quickstart

```bash
# 1. Install (macOS/Linux; Windows and cargo install: see Install below)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.sh | sh

# 2. Register lievo with your agent — e.g. Claude Code
claude mcp add lievo --scope user -- lievo mcp
# (`--scope` is a Claude Code flag, not a lievo flag)

# 3. Index a repo and query it (from inside the repo)
lievo refresh
lievo query entities --project <name> "storage"
```

`lievo mcp` auto-registers the repo it is launched in and indexes it in the background.

## Why Lievo?

An index and a grep are complements, not substitutes. Adding lievo to an agent that already has file search makes it measurably better at sustained work on a large codebase, and never measurably worse at anything we tested.

Built-in file search is fast and text-based, but it has no model of how the code is structured. Lievo supplies that model, and each of the four capabilities below ships in the current build:

- **Structure, not strings** — query subsystems, modules, and functions by meaning, not text patterns: `query subsystems`, `query modules`, `query entities --semantic`, and the `lievo_explore` symbol map all run against the structural index (`src/query/`, `src/retrieval/`)
- **Relationships built in** — "what depends on this?" is one query, not a manual cross-reference of dozens of files: `query relationships`, `query deps`, `query dependents`, `query impact` read the stored call/import/containment graph (`src/storage/`, `src/query/`)
- **Pre-computed insights** — circular dependencies, complexity and coupling hotspots, god modules, and coverage gaps are detected on every analysis run (`src/analysis/`)
- **Incremental and persistent** — the index lives in SQLite, updates only when the code changes, and survives across sessions and agent restarts (`src/storage/`, `src/analysis/incremental.rs`)

## Capabilities and limitations

**Languages.** Lievo parses exactly eight extensions: `.rs`, `.py`, `.js`, `.jsx`, `.mjs`, `.ts`, `.tsx`, `.go`. Everything else — Ruby, Java, C#, C/C++, PHP, Swift, Kotlin, and so on — is not indexed and is invisible to every query. **On a mixed-language repository, lievo sees only the supported part.** For example, on a Rails + React monorepo of the kind we benchmarked, lievo parses about 20% of tracked files — the other ~80% is invisible to it.

**Project awareness.** Lievo detects Cargo workspaces, npm workspaces, Python packages, JS/TS source directories, and top-level `src` layouts; framework profiles are matched from the project's dependencies (profiles for languages lievo cannot parse only label the project). JS/TS import resolution honors `tsconfig`/`jsconfig` `baseUrl` plus Webpacker/Shakapacker source roots.

**Summarization (optional).** Backends: `apfel`, `llama-server`, and any OpenAI-compatible endpoint (generic backend; token via `LIEVO_SUMMARIZER_TOKEN` — treat as a credential and do not commit it to shell profiles or CI secrets beyond the summarizer endpoint it authorizes). During a manual `lievo refresh`, summarization turns on automatically when an `apfel` binary is on PATH, or when a remote endpoint is configured — and can be switched off in config. The automatic index started by `lievo mcp` never summarizes. If the summarizer is unreachable or times out (300 s per request; HTTP 429 retried 3× with backoff), `lievo refresh` continues with unsummarized entities and reports a warning naming the backend and endpoint — it does not abort. Setup: [Self-hosted Summarization](docs/self-hosted-summarization.md).

**Semantic search (on-device).** Keyword search plus vector search over the index, built on model2vec + usearch, runs entirely on-device. The embedding model (`minishlab/potion-code-16M-v2`, ~16 MB) is downloaded once from Hugging Face on the first manual `lievo refresh`; if the download fails (network unavailable, 300 s timeout), `lievo refresh` reports an error and structural analysis is still saved — vector search is disabled until the next successful refresh. Storage is local SQLite. During indexing and querying, data leaves the machine only for that one model download and a configured remote summarizer.

**MCP surface.** Only `lievo_explore` is listed to your agent by default; see [MCP integration](#mcp-integration) for the opt-in list.

## Install

Installation requires network access (GitHub and crates.io; the embedding model is fetched from Hugging Face on first use).

macOS and Linux (no Rust toolchain required):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.sh | sh
```

Windows (best-effort):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.ps1 | iex"
```

Falling back to Rust, or building from source:

```bash
cargo install --git https://github.com/trail-openers/lievo.git
```

Beyond the commands shown above, lievo also offers `lievo summarize` (re-summarize entities) and `lievo doctor` (one-screen diagnostic). See the [CLI Reference](docs/cli-reference.md) for the full command list.

## MCP integration

Add lievo to Claude Code with:

```bash
claude mcp add lievo --scope user -- lievo mcp
```

> `lievo mcp` auto-registers the repo it is launched in. An explicit project name (`lievo mcp myapp`) keeps the previous per-project behaviour.

Available MCP tools:

`lievo_explore` is the only tool listed to the agent by default; every other tool below is hidden unless you set `LIEVO_MCP_TOOLS` (comma-separated tool names, e.g. `export LIEVO_MCP_TOOLS=get_entity,read_file`) before the server starts — see the [quickstart](docs/quickstart.md).

- `lievo_explore` — the default tool. Tier 1 (default) symbol map, Tier 2 (`include_source=true`) verbatim source, `scope="<dir>"` file listing, `bundle="<dir>"` packed subsystem source — one call, no per-file re-reads.
- `search_entities` — search by name/keyword; set `semantic=true` for vector search
- `get_entity` — full entity details by ID
- `list_relationships` — dependencies and dependents for an entity
- `list_subsystems` — list subsystems with metrics
- `get_function` — function source code and metadata
- `get_conventions` — detected coding conventions
- `get_insights` — circular dependencies, complexity hotspots
- `get_impact` — impact analysis for changed files
- `get_hotspots` — complexity and coupling hotspots
- `get_execution_flows` — execution flow tracking
- `list_project_docs` — list project documentation files
- `read_project_doc` — read a project doc by path
- `read_file` — read source file contents
- `list_directory` — list directory entries

## CLI reference

See [CLI Reference](docs/cli-reference.md) for the full command list. `lievo refresh [<project>]`, `lievo mcp [<project>]`, and `lievo doctor [PATH]` are documented there. For format details see [CLI Output Contracts](docs/cli-output-contracts.md).

## License

Lievo is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0). See [LICENSE](LICENSE) for the full text.

Copyright (C) 2026 Janni Turunen and Trail Openers Oy

Lievo is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, either version 3 of the License.

Lievo is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.

Third-party dependency licences are listed in [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES.md).

## Data Storage

Lievo stores all project data in `~/.lievo/lievo.db` (SQLite) and builds a semantic search index at `~/.lievo/indices/` (per-repo subdirectories). See [Data Storage](docs/data-storage.md) for the full disk layout and how to reset the database or index.

## Notes

- `lievo mcp` exposes the same knowledge graph to MCP clients over stdio.
