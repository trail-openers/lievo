# lievo MCP tool surface

`lievo mcp [<project>]` serves the same knowledge graph as the CLI over stdio. It resolves the repo it is launched in (`LIEVO_PROJECT_DIR`, then `CLAUDE_PROJECT_DIR`, then the working directory), registers it on first use, and indexes in the background. Registration per client: [quickstart.md](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#connect-your-agent).

## Which tools are listed

- **Default:** only `lievo_explore` is listed.
- **Opt in:** set `LIEVO_MCP_TOOLS` to a comma-separated list of tool names in the MCP server's environment before the server starts (the client's server config, e.g. the `env` field). Example: `LIEVO_MCP_TOOLS=get_entity,read_file`.
- **Hidden tools:** calling a tool that is not enabled returns guidance that names `lievo_explore` and the `LIEVO_MCP_TOOLS` variable. It is not an error.
- **Pi:** pi names MCP tools `mcp__<server>__<tool>`, so `lievo_explore` appears as `mcp__lievo__lievo_explore` when lievo is registered under the name `lievo`. Pi's default exposure reaches these tools through codemode rather than declaring them directly; see pi's `docs/mcp.md`.

## The 15 MCP tools

| Tool | Purpose |
|---|---|
| `lievo_explore` | **Default.** One call for structural questions (modes below). |
| `search_entities` | Search by name or keyword. `semantic=true` uses the vector index and falls back to name and path matching when there is none. |
| `get_entity` | Full details for an entity ID. `include_children=true` adds structural children. |
| `get_function` | Function source, signature, and what it calls and is called by. |
| `list_relationships` | Dependencies and dependents of an entity ID. |
| `list_subsystems` | Subsystems with metrics. |
| `get_conventions` | Detected coding conventions. |
| `get_insights` | Circular dependencies and complexity hotspots. |
| `get_impact` | Impact analysis for changed files (`files`, repo-relative paths). |
| `get_hotspots` | Complexity and coupling hotspots. |
| `get_execution_flows` | Execution flows from entry points. |
| `list_project_docs` / `read_project_doc` | List and read project documentation files. |
| `read_file` | Raw source of a **file entity**, by entity ID. See below. |
| `list_directory` | Direct children of a directory, by repo-relative path. |

## `read_file` is not the agent's file read

lievo's `read_file` takes an **entity ID** (for example, `<project-id>:<repo>:file:src/foo.rs`), not a path. Get the ID from `search_entities`. Content is truncated at 32K characters.

To read a file by path, use the agent's built-in file read. To get a file's source through lievo, use `lievo_explore` with `files=["src/foo.rs"]`. That is a repo-relative path, and it works with the default listing.

## `lievo_explore` modes (the default tool)

One call, output capped at 24K characters. Word-matches file names and paths and symbol names. An exact symbol-name match ranks above a file-name or path match.

- **Default (tier 1):** per-symbol map: name, kind, qualified path, signature, one-line summary. No source bodies and no call paths.
- **`include_source=true` (tier 2):** adds line-numbered source for matched symbols.
- **`include_depth=true`:** adds per-file `call_paths` and `blast_radius` (impact). Ignored in scope mode.
- **`scope="<dir>"`:** lists the sorted, repo-relative indexed files under a directory prefix. `scope` alone triggers this mode, so a path-like `query` does not.
- **`files=[...]`:** fetches files by repo-relative path, batched in one call. Overrides `query` and `scope`.
- **`bundle="<dir>"`:** subsystem-bundle mode. Returns line-numbered source for the packed files, the Calls and Imports edges between them, a `not_shown_files` list, and a `completeness` field `{complete, omitted_files, omitted_edges}`.
- **Continuation:** when the cap is hit, the response names what was omitted with `returned`, `total`, and `next` fields. Page with `offset` (scope mode) or the `next` pointer. When `returned == total` with no pointer, the result is complete.

The tool's own description tells the agent to batch related files into one call, not to re-read a file whose source it already has, and not to re-request edges it already has.

## A note on the MCP and CLI split

The CLI (`lievo query ...`) and the MCP tools read the same SQLite graph. The conventions differ:

- MCP entity tools (`get_entity`, `list_relationships`, `get_function`, `read_file`) take **entity IDs**.
- `lievo_explore` `files` takes **repo-relative paths**. `get_impact` takes a `files` array of file paths.
- CLI `query impact` takes **repo-relative paths**, and `query deps`, `query dependents`, and `query entity` take **entity IDs**.

## Disabling automatic indexing

`LIEVO_NO_REFRESH=1` stops background indexing. Run `lievo refresh <project>` yourself. See [quickstart.md](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#first-use).
