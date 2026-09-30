# CLI Reference

## Top-level commands

| Command | Purpose |
| --- | --- |
| `lievo refresh [<project>]` | Run semantic code analysis and keep entity graph current |
| `lievo mcp [<project>]` | Start the MCP server over stdio for agent integration |
| `lievo doctor [PATH]` | One-screen diagnostic: is lievo registered, indexed, and current here? |

`lievo summarize [<project>]` re-summarizes entities with cached descriptions (alias: `sum`). `lievo query` subcommands are listed below.

## Admin workflows

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

Query commands support human and JSON (`--format json`) output. See [CLI Output Contracts](cli-output-contracts.md) for format details and [CLI Architecture](cli-architecture.md) for design principles.

## Query subcommands

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
