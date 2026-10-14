---
name: lievo
description: "Query a repo's structural code map with lievo (impact, deps, dependents, entities, subsystems). Use when asked what depends on X, what breaks if Y changes, or where a symbol lives in an indexed repo (rs, py, js/jsx/mjs, ts/tsx, go)."
version: 1.1.0
license: AGPL-3.0
metadata:
  author: Janni Turunen (trail-openers)
---

# lievo

Use lievo to ask **structural** questions about an indexed codebase: what depends on what, what breaks if a file changes, where a function lives. lievo answers from a SQLite graph of modules, files, and functions and their call, import, and containment edges. It complements the agent's built-in file search and file read; it does not replace them.

lievo indexes only `.rs .py .js .jsx .mjs .ts .tsx .go`. Other languages are invisible to every query. On mixed repos, use built-in search for the rest.

## Interfaces

lievo has two interfaces over the same graph:

- **CLI** (`lievo ...`): needs shell access. Commands below.
- **MCP server** (`lievo mcp`): works without shell access. By default it lists one tool, `lievo_explore`. See [references/mcp-tools.md](references/mcp-tools.md).

If the agent is registered for lievo's MCP server, it can use the same steps without shell access. Registration per client: [docs/quickstart.md](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#connect-your-agent).

## When to use
- "What breaks if I change this file or function?" → `query impact`, `query dependents`
- "Where is X implemented?" → `query entities`, then `query entity`
- "What is in this subsystem or module?" → `query subsystems`, `query children`
- Orienting in an unfamiliar repo before planning edits.

**Don't use for** literal text search, non-indexed languages, or reading raw file contents. lievo has no full-text search. Use the agent's file search and file read tools.

## Setup (first time, fast path)
```bash
lievo admin add-repo /path/to/repo          # registers the repo and creates a project
lievo refresh <project> --no-summarize      # structural + vector index, no summaries
lievo doctor                                # expect: registered: true, index: current
```

- Project name: the checkout directory name, or a suffixed or `<owner>/<repo>` name if that is taken. A second checkout of the same repo (e.g. a git worktree) becomes `<repo>@<dir>`. Pass an explicit name to avoid guessing. Details: [quickstart.md#project-names](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#project-names).
- `refresh` picks the project automatically only when exactly one project exists.
- Summaries are optional and slower. Add them as a separate step; see [Summaries](#summaries).

## Query loop (CLI)
1. `lievo doctor`: confirm the index is current. If it is missing or stale, run `lievo refresh <project> --no-summarize`.
2. **Orient:** `lievo --format json query subsystems --project P`, then `query children <id>` to drill down.
3. **Locate:** `lievo --format json query entities --project P "<term>"`, and take the `entity_id`.
4. **Interrogate:** `query entity <id>`, `query deps <id>`, `query dependents <id>`.
5. **Assess change:** `query impact --project P <repo-relative-file>...` for the files you plan to touch.

## Quick reference (CLI)

`--format json` is a global flag. It works before or after the subcommand (`lievo --format json query entities ...` and `lievo query entities ... --format json` both work). Use `--format json` whenever you parse output; human layout is not stable.

`--project` is accepted by `entities`, `subsystems`, `flows`, `docs`, `hotspots`, `conventions`, and `impact`. `deps`, `dependents`, `modules`, and `files` take IDs and have no `--project`. `entity`, `relationships`, and `children` accept `--project` but ignore it, because entity IDs are project-scoped.

| Question | Command |
|---|---|
| Is lievo working here? | `lievo doctor` (`lievo --format json doctor` for scripts) |
| Register repo + project | `lievo admin add-repo <path> [<project>]` |
| Projects | `lievo admin create-project <name>` / `list-projects` / `delete-project <name>` |
| Repos, status, coverage, DB info | `lievo admin list-repos [<project>]` / `status [<project>]` / `coverage` / `info` |
| Build or update the index | `lievo refresh <project>` (`--no-summarize`, `--full`, `--force`, `--no-ignore`) |
| Find symbols (keyword) | `lievo query entities --project P "term"` |
| Find symbols (semantic) | `lievo query entities --semantic --project P "term"` (falls back to keyword with a warning if there is no vector index) |
| Full detail by ID | `lievo query entity <entity-id>` |
| Depends on / depended on by | `lievo query deps <entity-id>` / `query dependents <entity-id>` |
| Both directions | `lievo query relationships <entity-id>` |
| Children of a module or subsystem | `lievo query children <entity-id>` |
| Subsystems / modules / files | `query subsystems --project P` / `query modules <subsystem-id>` / `query files <module-id>` |
| Blast radius of changed files | `lievo query impact --project P <repo-relative-file>...` |
| Complexity / coupling hotspots | `lievo query hotspots --project P [--limit N]` |
| Execution flows | `lievo query flows --project P` |
| Conventions / docs | `lievo query conventions --project P [--category C]` / `query docs --project P` |
| Summaries | `lievo summarize <project> [--file <path>]` |

Entity IDs are stable strings: `fn-<hash>` for functions, `<project-id>:<repo>:file:<path>` for files, and `...:module:<path>` and `...:subsystem:<path>` for containers. Get an ID from `entities` (`entity_id`) or `subsystems` (`id`), then reuse it.

## MCP tool map

**CLI commands need shell access. MCP tools do not.**

By default only `lievo_explore` is listed. The other tools appear only when the server's environment sets `LIEVO_MCP_TOOLS=name1,name2` before `lievo mcp` starts. Set that in the MCP client's server config, not in your shell.

| CLI step | MCP tool | Default listing (`lievo_explore` only) |
|---|---|---|
| `doctor` | none | `lievo_explore` returns an "indexing in progress" response while the index builds |
| `subsystems` | `list_subsystems` | `lievo_explore` with `scope="<dir>"` lists indexed files under a directory |
| `children` / `modules` / `files` | `get_entity` with `include_children=true` | `lievo_explore` with `scope="<dir>"` |
| `entities` (keyword) | `search_entities` | `lievo_explore` with `query="<term>"` |
| `entities --semantic` | `search_entities` with `semantic=true` | none |
| `entity <id>` | `get_entity` | `lievo_explore` with `files=["<repo-relative path>"]` (source by path) |
| `deps` / `dependents` | `list_relationships` | `lievo_explore` with `include_depth=true` (call paths and blast radius; different shape) |
| `impact <files>` | `get_impact` with `files` | `lievo_explore` with `files=[...]` and `include_depth=true` |
| `hotspots` / `conventions` / `flows` | `get_hotspots` / `get_conventions` / `get_execution_flows` | none |

`lievo_explore` takes `query`, `files`, `scope`, `bundle`, `include_source`, `include_depth`, `max_files`, and `offset`. Its modes and continuation contract are in [references/mcp-tools.md](references/mcp-tools.md).

## Output contracts (CLI JSON)

Parse per command. Do not assume one shape everywhere.

- **`entities`**: one JSON array (`[]` when empty).
- **`subsystems`, `modules`, `files`, `hotspots`**: NDJSON, one object per line: `id`, `name`, `tier`, `module_count`, `file_count`, `complexity_max`. The counts are numbers (0 when unknown).
- **`entity`**: one JSON object on one line. Keys are always present; `summary`, `metrics_json`, `path`, `language`, `parent_id`, and `repo_id` are `null` when absent. `metrics_json` is a string containing JSON, so parse it twice.
- **`children`**: one object `{parent, children[], child_count}`. Each child carries `summary` and `metrics_json` in the same nullable form, plus `depends_on` and `depended_by` arrays.
- **`deps`, `dependents`**: NDJSON rows `{entity_id, entity_name, entity_tier, rel_type, weight, provenance}`. An empty result prints one `[]` line.
  - **Header line:** when the repo has recorded resolution counts, the first line is `# {"unresolved_imports": N, "unresolved_external": N}`. It goes to **stdout**, before the rows, so `2>/dev/null` does not remove it. It is absent on indexes without those counts. Strip it:
    ```bash
    lievo --format json query deps <id> | grep -v '^#' | jq -c 'select(type=="object")'
    ```
  - `select(type=="object")` also drops the `[]` line.
- **`relationships`**: NDJSON rows, deps section first, then dependents section. There is no label or separator between them, and each section can be `[]`. There is no header line. If direction matters, use `deps` and `dependents`.
- **`impact`**: NDJSON sections `{"section": <label>, "entities": [{id, name, tier}]}`, with labels `changed_files`, `affected_modules`, `affected_subsystems`, `downstream_dependents`, and `affected_callers`. The last line is `{"section":"resolution","unresolved_internal":N|null,"unresolved_external":N|null}`, where `null` means the index has no recorded counts.
- **`doctor`**: one JSON object. `ok` is `true` exactly when the exit code is 0.
- Field names are `snake_case`. `rel_type` is e.g. `calls`, `imports`, `contains`. `provenance` is `resolved` (verified edge) or `heuristic` (inferred, lower confidence).
- Human output is for reading. Its layout is not stable.

## Summaries

Summarization is optional and separate from indexing. A database can have a full structural index with zero summaries. Treat `summary` as possibly `null` everywhere.

- **Check coverage:** `lievo admin status [<project>]` prints a `Summary coverage:` section with one line per repo, `<repo>: <done>/<total> functions summarized (<missing> missing)`. `0` done means no summaries.
- **Fast path (default):** `lievo refresh <project> --no-summarize` builds the structural and vector indexes and contacts no summarizer. Use it for exploration.
- **Slow path (explicit, later):** `lievo summarize <project>`, or a plain `lievo refresh <project>`. With the default `apfel` backend, summaries turn on when `apfel` is on PATH; with `llama-server` or `generic`, when `apfel_endpoint` is set. It is slow: minutes for a few hundred functions.
- **The vector index is separate.** `--no-summarize` still builds it, so `--semantic` works. The embedding model is downloaded once on the first refresh. If that fails, refresh prints a warning and semantic search is disabled until a later refresh succeeds.
- **A degraded run never fails.** If the backend is unreachable or returns unusable output, indexing still succeeds and summaries are skipped.

Backend requirements, configuration, and the check procedure are in [references/notes.md](references/notes.md#summarizer-backends).

## Configuration and security

- Environment variables: `LIEVO_PROJECT_DIR` (pins the repo), `LIEVO_NO_REFRESH` (disables automatic indexing), `LIEVO_MCP_TOOLS` (extra MCP tools), `LIEVO_DB` (database path), `LIEVO_APFEL_ENDPOINT` and `LIEVO_SUMMARIZER_TOKEN` (summarizer endpoint and bearer token). Full list: [references/notes.md](references/notes.md#configuration).
- **`<repo>/.lievo/config.yaml` is inside the checkout, so it is untrusted for repos you don't control.** A repo-supplied `apfel_endpoint` or `summarizer_backend` can make `lievo refresh` send source code to that endpoint, and send `LIEVO_SUMMARIZER_TOKEN` as a bearer token if it is set. Before refreshing an untrusted repo, read its `.lievo/config.yaml`, or refresh it with `--no-summarize`, which contacts no summarizer.
- Use `https` for any non-loopback summarizer endpoint. lievo does not reject plain `http` to a remote host, but it prints a warning when a summary run sends source to a non-loopback host.
- A repo-supplied `identity:` in `.lievo/config.yaml` is also untrusted. It sets the project's identity key, which affects naming and moved-checkout matching. See [quickstart.md#project-names](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#project-names) and [data-storage.md#known-limitations](https://github.com/trail-openers/lievo/blob/main/docs/data-storage.md#known-limitations).

## Pitfalls
- `query modules <subsystem-id>` and `query files <module-id>` take the container's ID, not a name. Get the ID from `query subsystems` (`id`) first.
- `provenance: heuristic` edges are inferred. Treat them as likely, not certain.
- The index is incremental. After edits, run `lievo refresh <project>`. After upgrading lievo or changing `.lievo/config.yaml`, add `--force`.
- A moved or renamed checkout can re-key rows by path. `lievo doctor` shows the repo source it resolved (`cwd` or `LIEVO_PROJECT_DIR`). If the agent reads the wrong repo, check `LIEVO_PROJECT_DIR`, which wins over the working directory.

## Verification
- `lievo doctor` reports `registered: true` and `index: current`, and the repo source is correct.
- `lievo admin status <project>` shows entity and relationship counts. With `--no-summarize`, `Summary coverage` at `0/N` is expected.
- `lievo --format json query impact --project P <file>` ends with a `"section":"resolution"` line.

## References
- [references/mcp-tools.md](references/mcp-tools.md): the MCP tools, the `lievo_explore` modes, and the file-read tool disambiguation.
- [references/notes.md](references/notes.md): install, summarizer backends and `.lievo/config.yaml`, data paths and reset.
