# lievo MCP tool surface

`lievo mcp [<project>]` serves the same knowledge graph over stdio. It auto-registers the repo it is launched in and indexes in the background on first use. By default **only `lievo_explore` is listed to the agent**; every other tool is hidden unless `LIEVO_MCP_TOOLS` (comma-separated names) is set before the server starts.

## The 15 MCP tools

| Tool | Purpose |
|---|---|
| `lievo_explore` | **Primary / default.** One call for structural questions (modes below). |
| `search_entities` | Search by name/keyword; `semantic=true` for vector search (falls back to name/path if no vector index). |
| `get_entity` | Full details by entity ID; `include_children=true` for structural children. |
| `get_function` | Function source + signature + what it calls / what calls it (token-cheaper than `read_file`). |
| `list_relationships` | Dependencies and dependents for an entity. |
| `list_subsystems` | Subsystems with metrics. |
| `get_conventions` | Detected coding conventions. |
| `get_insights` | Circular dependencies, complexity hotspots. |
| `get_impact` | Impact analysis for changed files. |
| `get_hotspots` | Complexity and coupling hotspots. |
| `get_execution_flows` | Execution-flow tracking. |
| `list_project_docs` / `read_project_doc` | List / read project documentation files. |
| `read_file` | Raw source of a file entity (takes an **entity ID**, not a path; truncated at 32K chars). |
| `list_directory` | List directory entries. |

Opt in: `export LIEVO_MCP_TOOLS=get_entity,read_file` before starting `lievo mcp`.

## `lievo_explore` modes (the default tool)
One call, capped at 24K chars of output. Word-matches file names/paths **and** symbol names (an exact symbol-name match ranks above a file-name/path match).

- **Tier 1 (default):** per-symbol map — name, kind, qualified path, signature, one-line summary. No source bodies, no call paths.
- **Tier 2 (`include_source=true`):** adds verbatim line-numbered source for matched symbols.
- **`include_depth=true`:** adds per-file `call_paths` and `blast_radius` (impact opt-in; ignored in scope mode).
- **`scope="<dir>"`** (e.g. `src/retrieval`): switches to *scope-membership listing* — the sorted, repo-relative indexed files under that directory prefix. `scope` is the sole trigger for this mode.
- **`files=[...]`**: fetch files directly by repo-relative path (batched, one call). Short-circuits query word-match and scope.
- **`bundle="<dir>"`**: subsystem-bundle mode — under the 24K cap returns verbatim line-numbered source for the packed files, the intra-scope Calls/Imports edges, a `not_shown_files` list, and a completeness field.
- **Continuation:** when the 24K cap is hit, a structured completeness field names what was omitted with `returned` / `total` / `next` pointers — page the rest; when `returned == total` with no continuation pointer, the set is complete.

Guidance baked into the tool: batch related files into one call instead of one call per file; do not re-read a file whose source you already received this session; do not re-request edges already returned.

## Agent registration (per client)
- Claude Code: `claude mcp add lievo --scope user -- lievo mcp` (uses `CLAUDE_PROJECT_DIR`).
- Codex CLI: `codex mcp add lievo -- lievo mcp` (or pin with `--env LIEVO_PROJECT_DIR=/path/to/repo`).
- VS Code: `code --add-mcp '{"name":"lievo","command":"lievo","args":["mcp"]}'`.
- Cursor (needs the path named explicitly, in `.cursor/mcp.json`): `{"mcpServers":{"lievo":{"command":"lievo","args":["mcp"],"env":{"LIEVO_PROJECT_DIR":"/path/to/repo"}}}}`.
- pi: no native MCP; use the `pi-mcp-adapter` npm package.

Disable auto indexing: `LIEVO_NO_REFRESH=1` (then run `lievo refresh` manually).

## A note on the MCP vs CLI split
The CLI (`lievo query ...`) and the MCP tools read the **same SQLite graph**, so anything you can do with `lievo query` you can reason about via the MCP tools and vice versa. The MCP `read_file` takes an entity **ID** (not a path) — get it from `search_entities`. The CLI `query entity`/`impact` take IDs / repo-relative paths respectively; keep the two conventions straight.
