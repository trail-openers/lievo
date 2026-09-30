# lievo — code analysis and knowledge platform

An index and a grep are complements, not substitutes. Adding lievo to an agent that already has file search makes it measurably better at sustained work on a large codebase, and never measurably worse at anything we tested. Lievo is a persistent, semantic index of your codebase — structure, relationships, and subsystems in SQLite — exposed through a CLI and an MCP server. It works alongside your agent's built-in file search and read tools; it does not replace them. External agents like Claude Code and Cursor use lievo for documentation, Q&A, and codebase navigation.

[![CI](https://github.com/trail-openers/lievo/actions/workflows/ci.yml/badge.svg)](https://github.com/trail-openers/lievo/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

## Why Lievo?

Built-in file search is fast and text-based, but it has no model of how the code is structured. Lievo supplies that model:

- **Structure, not strings** — query subsystems, modules, and functions by meaning, not text patterns
- **Relationships built in** — "what depends on this?" is one query, not a manual cross-reference of dozens of files
- **Pre-computed insights** — circular dependencies, complexity hotspots, and coupling metrics detected automatically
- **Incremental and persistent** — knowledge survives across sessions and updates only when code changes

## Evidence

We benchmarked lievo against an agent's built-in tools on an anonymous 11,000-file commercial monorepo (Ruby on Rails backend, React frontend — lievo parses about 20% of the tracked files, so all tasks were scoped to the JavaScript portion). 435 runs: change-impact analysis n=30/arm, bug localization n=39/arm, multi-turn sustained work n=18/arm. Headless agents (Claude Sonnet), one pinned index, lievo build `ff1ade0` (2026-09-24); the benchmarked index was summarized via a generic OpenAI-compatible backend. Arms: baseline (Bash/Read/Grep/Glob, no index); lievo — those four tools plus lievo's MCP server, the shipping configuration; and lievo_only (lievo's MCP server only). Scoring: set-F1 of returned file paths against hand-authored gold derived from git history or hand-derived dependency closures. Comparisons are bootstrap 95% CIs on the difference; "tied" means the interval spans zero.

**Headline — the shipping configuration, on multi-turn sustained work:** built-ins + lievo vs built-ins alone scores F1 0.657 vs 0.550, delta +0.106 [+0.007, +0.206], significant, n=18/arm, at the highest recall of any arm, 0.980 vs baseline 0.958.

| Arm | Multi-turn F1 | Multi-turn resident context | Change-impact F1 |
| --- | --- | --- | --- |
| baseline (built-ins only) | 0.550 | 337,297 tokens | 0.951 |
| lievo (built-ins + lievo, shipping config) | **0.657** (+0.106 [+0.007, +0.206], significant) | −92,667 tokens [−205,049, +20,833] (tied) | 0.926 (−0.025 [−0.054, +0.002], tied) |
| lievo_only (lievo only) | 0.781 (baseline 0.550; +0.232 [+0.110, +0.328], significant) | 129,068 tokens vs baseline 337,297 (−207,353 [−304,133, −119,403], significant) | 0.765 (baseline 0.951; −0.186 [−0.224, −0.150], significant — worse than built-ins alone) |

The lievo_only row is a replacement configuration, not a recommendation: it wins multi-turn F1 and resident context, and it loses change-impact F1.

Across 18 multi-turn sessions the shipping configuration made 114 `lievo_explore` calls plus 477 built-in calls (Read 177, Grep 126, Bash 110, Glob 64), and its lievo call mix is near-identical to the lievo_only arm's: lievo is added to the built-ins rather than substituted for them.

**Where the shipping configuration ties baseline:** change-impact F1 0.926 vs 0.951, −0.025 [−0.054, +0.002] (tied); change-impact tokens −3,161 [−51,401, +47,631] (tied); bug localization F1 0.331 vs 0.371, −0.041 [−0.193, +0.110] (tied) — a replicated null across three runs (n=26, n=39, n=39); bug localization tokens +136,872 [−325,880, +612,437] (tied); multi-turn resident context −92,667 [−205,049, +20,833] (tied). The shipping configuration never loses significantly on any suite.

**Caveats, stated next to the numbers:** single repository, single model (Claude Sonnet), JavaScript portion only; bug localization is a replicated null; the multi-turn gold set is directory-shaped (it favours recall for every arm — the metric that matters there is resident context at equal recall); n=18–39 per arm; these are ballpark figures with stated intervals, not a peer-reviewed study; measured on build `ff1ade0` (2026-09-24) — later builds changed the retrieval and response contract, so these figures are not expected to reproduce on v0.1.0; the raw data is not published.

## Capabilities and limitations

**Languages.** Lievo parses exactly eight extensions: `.rs`, `.py`, `.js`, `.jsx`, `.mjs`, `.ts`, `.tsx`, `.go`. Everything else — Ruby, Java, C#, C/C++, PHP, Swift, Kotlin, and so on — is not indexed and is invisible to every query. **On a mixed-language repository, lievo sees only the supported part.** For example, on a Rails + React monorepo of the kind we benchmarked, lievo parses about 20% of tracked files — the other ~80% is invisible to it.

**Project awareness.** Lievo detects Cargo workspaces, npm workspaces, Python packages, JS/TS source directories, and top-level `src` layouts; framework profiles are matched from the project's dependencies (profiles for languages lievo cannot parse only label the project). JS/TS import resolution honors `tsconfig`/`jsconfig` `baseUrl` plus Webpacker/Shakapacker source roots.

**Summarization (optional).** Backends: `apfel`, `llama-server`, and any OpenAI-compatible endpoint (generic backend; token via `LIEVO_SUMMARIZER_TOKEN`). During a manual `lievo refresh`, summarization turns on automatically when an `apfel` binary is on PATH, or when a remote endpoint is configured — and can be switched off in config. The automatic index started by `lievo mcp` never summarizes. Setup: [Self-hosted Summarization](docs/self-hosted-summarization.md).

**Semantic search (on-device).** Keyword search plus vector search over the index, built on model2vec + usearch, runs entirely on-device. The embedding model (`minishlab/potion-code-16M-v2`, ~16 MB) is downloaded once from Hugging Face on the first manual `lievo refresh`. Storage is local SQLite. During indexing and querying, data leaves the machine only for that one model download and a configured remote summarizer.

**MCP surface.** Only `lievo_explore` is listed to your agent by default; see [MCP integration](#mcp-integration) for the opt-in list.

## Key features

- `lievo refresh [<project>]` — run semantic code analysis and keep entity graph current
- `lievo query` — inspect entities, relationships, subsystems, conventions, metrics, dependencies, and files
- `lievo summarize [<project>]` — re-summarize entities with cached descriptions (alias: `sum`)
- `lievo mcp [<project>]` — start the MCP server over stdio for agent integration
- `lievo doctor [PATH]` — one-screen diagnostic: is lievo registered, indexed, and current here?

## Install

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

**[Quickstart](docs/quickstart.md)** covers install without Rust, registering lievo once with your agent (Claude Code, Codex CLI, VS Code, Cursor, pi), first use, and troubleshooting with `lievo doctor`.

## Getting started

```bash
cd /path/to/repo
lievo refresh        # index the repo you're in
lievo mcp            # start the MCP server for this repo
```

`lievo mcp` with no argument auto-resolves the git repo in the working directory and registers it if needed. An explicit project name still works: `lievo mcp myapp`.

Then connect Claude Code or another MCP client to the server.

## MCP integration

Add lievo to Claude Code with:

```bash
claude mcp add lievo -- lievo mcp
```

> `lievo mcp` auto-registers the repo it is launched in. An explicit project name (`lievo mcp myapp`) keeps the previous per-project behaviour.

Available MCP tools:

`lievo_explore` is the only tool listed to the agent by default; every other tool below is hidden unless you set `LIEVO_MCP_TOOLS` (comma-separated tool names, e.g. `export LIEVO_MCP_TOOLS=get_entity,read_file`) before the server starts — see the [quickstart](docs/quickstart.md).

- `lievo_explore` — the default tool. Tier 1 (default) symbol map, Tier 2 (`include_source=true`) verbatim source, `scope='<dir>'` file listing, `bundle='<dir>'` packed subsystem source — one call, no per-file re-reads.
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

### Admin workflows

| Command | Purpose |
| --- | --- |
| `lievo admin create-project <name>` | Create a new project |
| `lievo admin list-projects` | List all projects |
| `lievo admin delete-project <name>` | Delete a project and all its data |
| `lievo admin add-repo <path> [<project>]` | Register a repository and add to a project |
| `lievo admin list-repos` | List repositories |
| `lievo admin delete-repo <name> <project>` | Delete a repository and its analysis data |
| `lievo admin info` | Show database information and statistics |
| `lievo admin coverage [--project <name>]` | Show per-language extraction coverage metrics |
| `lievo admin selfcheck --repo <path>` | Pinned-repo quality gate: edge correctness, false-0-callers, retrieval probes, payload-bytes floor |

> Admin commands are only available under `lievo admin <subcommand>`. There are no top-level aliases for these commands.

Query commands support human and JSON (`--format json`) output. See [CLI Output Contracts](docs/cli-output-contracts.md) for format details and [CLI Architecture](docs/cli-architecture.md) for design principles.

### Query subcommands

| Command | Purpose |
| --- | --- |
| `lievo query entities --project <name> <query>` | Search entities by keyword |
| `lievo query entity --project <name> <entity-id>` | Show full entity details |
| `lievo query relationships --project <name> <entity-id>` | Show dependencies and dependents |
| `lievo query children --project <name> <entity-id>` | List child entities for a module or subsystem |
| `lievo query flows --project <name>` | Show execution flows |
| `lievo query docs --project <name>` | List project docs |
| `lievo query subsystems --project <name>` | List subsystems with metrics |
| `lievo query modules <subsystem-id>` | List modules in a subsystem |
| `lievo query files <module-id>` | List files in a module |
| `lievo query deps <entity-id>` | Show what an entity depends on |
| `lievo query dependents <entity-id>` | Show what depends on an entity |
| `lievo query impact <file> [<file>...]` | Show impact for changed files |
| `lievo query hotspots --project <name>` | Show complexity and coupling hotspots |
| `lievo query conventions --project <name>` | List detected conventions |

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
