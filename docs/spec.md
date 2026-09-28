# Lievo — code analysis and knowledge platform

## Overview

Lievo is a Rust crate and CLI for analyzing codebases, storing structural knowledge in SQLite, and exposing that knowledge through the CLI and MCP server.

It is built for codebase navigation, documentation lookup, and agent integration.

## Architecture

```
source code
  → lievo refresh
  → semantic code analysis
  → entity graph in SQLite
  → lievo query / lievo mcp
```

### Core layers

- **Extraction**: uses tree-sitter for multi-language parsing and model2vec + usearch for semantic code search.
- **Aggregation**: groups code units into entities such as subsystems, modules, and files.
- **Storage**: persists projects, repositories, entities, relationships, insights, and docs in SQLite.
- **Query surface**: exposes structured knowledge through the library API, CLI, and MCP server.

### Project model

- A **project** can span one or more repositories.
- Each repository is linked to a project umbrella.
- Cross-repository relationships are stored in the same database.

## Data model

Lievo stores:

- projects
- repositories
- entities
- relationships
- conventions
- insights
- project docs
- analysis runs

## CLI

CLI commands support human and JSON output formats. See [CLI Architecture](cli-architecture.md) for format selection and [CLI Output Contracts](cli-output-contracts.md) for format details.

### Admin commands

| Command | Purpose |
| --- | --- |
| `lievo admin create-project <name>` | Create a project umbrella |
| `lievo admin list-projects` | List projects |
| `lievo admin delete-project <name>` | Delete a project and its data |
| `lievo admin add-repo <path> [<project>]` | Register a repository |
| `lievo admin link-repo <project> <repo-id>` | Link an existing repository to a project |
| `lievo admin list-repos [<project>]` | List repositories |
| `lievo admin status [project]` | Show analysis status |
| `lievo admin info` | Show database information |
| `lievo admin coverage [--project <name>]` | Show per-language extraction coverage metrics |
| `lievo admin selfcheck --repo <path>` | Pinned-repo quality gate: edge correctness, false-0-callers, retrieval probes, payload-bytes floor |

### Top-level commands

| Command | Purpose |
| --- | --- |
| `lievo refresh [<project>]` | Run analysis and keep entity graph current |
| `lievo query <subcommand>` | Query structured knowledge |
| `lievo mcp [<project>]` | Start the MCP server over stdio |
| `lievo summarize [<project>] [--file <path>]` | Re-summarize entities with cached descriptions (alias: `sum`) |

### Command aliases

| Command | Alias |
| --- | --- |
| `lievo refresh` | `a` |
| `lievo query` | `q` |
| `lievo mcp` | `serve` (hidden backward-compat alias) |

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

See [Query Design Patterns](query-design-patterns.md) for implementation details.

## Operational notes

- `lievo admin create-project <name>` creates a project umbrella.
- `lievo admin add-repo <path> <project>` registers a local git repository with a project.
  If the project does not exist, lievo auto-creates it.
- `lievo refresh [<project>]` accepts either a project name or a registered repo path.
  When you pass a project name, lievo analyzes every repo linked to that project.
  By default it runs incrementally and skips repos already analyzed at the current HEAD.
  Use `--full` to force a full re-analysis and `--no-ignore` to include gitignored files.
- `lievo query entities --project <name> <query>` requires the search term.
- Database location: `~/.lievo/lievo.db`.
- `lievo mcp` only works for single-repo projects.
  If a project has multiple repos, create or choose a project with exactly one linked repo for MCP access.

## Getting started

```bash
lievo admin create-project myapp
lievo admin add-repo /path/to/repo myapp
lievo refresh myapp
lievo query entities --project myapp "authentication"
lievo mcp myapp
```

## MCP server

Lievo exposes the same structured knowledge through stdio MCP.

Add it to Claude Code with:

```bash
claude mcp add lievo -- lievo mcp myapp
```

> `lievo mcp` fails when the chosen project has multiple repositories.
> Use a single-repo project for MCP integration.

Available MCP tools:

- `search_entities`
- `get_entity`
- `list_relationships`
- `list_subsystems`
- `get_conventions`
- `get_insights`
- `get_module_details`
- `list_project_docs`
- `read_project_doc`
- `read_file`
- `list_directory`
- `get_execution_flows`

These tools support documentation lookup, Q&A, and codebase navigation for agents.
