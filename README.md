# lievo — code analysis and knowledge platform

Lievo analyzes codebases with semantic analysis, stores entity graphs in SQLite, and exposes structured knowledge through the CLI and MCP server. External agents like Claude Code and Cursor use lievo tools for documentation, Q&A, and codebase navigation.

## Why Lievo?

LLM agents navigating code with grep and file reads start from scratch every session — scanning for text patterns with no understanding of structure, relationships, or architecture.

Lievo gives agents a **persistent semantic knowledge graph** of your codebase:

- **Structure, not strings** — query subsystems, modules, and functions by meaning, not text patterns
- **Relationships built in** — "what depends on this?" is one query, not a manual cross-reference of dozens of files
- **Pre-computed insights** — circular dependencies, complexity hotspots, and coupling metrics detected automatically
- **Incremental and persistent** — knowledge survives across sessions and updates only when code changes

[![CI](https://github.com/trail-openers/lievo/workflows/CI/badge.svg)](https://github.com/trail-openers/lievo/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

## Key features

- `lievo refresh [<project>]` — run semantic code analysis and keep entity graph current
- `lievo query` — inspect entities, relationships, subsystems, conventions, metrics, dependencies, and files
- `lievo summarize [<project>]` — re-summarize entities with cached descriptions (alias: `sum`)
- `lievo mcp [<project>]` — start the MCP server over stdio for agent integration
- `lievo doctor [PATH]` — one-screen diagnostic: is lievo registered, indexed, and current here?

## Optional features

- **Self-hosted LLM summarization** — entity and subsystem summaries are optional: lievo indexes and retrieves fully without a model, and an absent or unreachable model simply means no summaries rather than a failure. If you want them, point lievo at a local llama-server or Ollama instance — setup, hardware expectations, verification, and troubleshooting in [Self-hosted Summarization](docs/self-hosted-summarization.md).

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

- `lievo_explore` — **PRIMARY TOOL, call FIRST**: Tier 1 (default) symbol map, Tier 2 (`include_source=true`) verbatim source, `scope='<dir>'` file listing, `bundle='<dir>'` packed subsystem source — one call, no per-file re-reads.
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
