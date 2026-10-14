---
name: lievo
description: "Query a repo's structural code map with lievo (impact, deps, dependents, entities, subsystems). Use when asked what depends on X, what breaks if Y changes, or where a symbol lives in an indexed repo (rs, py, js/jsx/mjs, ts/tsx, go)."
version: 1.0.0
author: Janni Turunen (trail-openers), Hermes Agent
license: AGPL-3.0
platforms: [linux, macos]
metadata:
  hermes:
    tags: [lievo, code-analysis, codebase-map, impact, mcp, tree-sitter]
    related_skills: [hermes-agent-skill-authoring]
---

# lievo

Use lievo to ask **structural** questions about a codebase — what depends on what, what breaks if a file changes, where a function lives — instead of grepping and reading file after file. Lievo keeps a SQLite index of modules / files / functions and their call-import-containment edges, and answers from that graph. It **adds** to the agent's built-in search; it does not replace grep/`search_files`/`read_file`.

Lievo indexes **only** `.rs .py .js .jsx .mjs .ts .tsx .go`. Every other language is invisible to every query. On a mixed repo (e.g. Rails+React) it sees only the supported slice — fall back to built-in search for the rest.

This skill is for **driving the CLI** from the `terminal` tool. (An MCP server, `lievo mcp`, exposes the same graph to other agents over stdio; the MCP surface is in `references/mcp-tools.md`.)

## When to Use
- "What breaks if I change this file/function?" → `query impact` / `query dependents`
- "Where is X implemented / what does X do?" → `query entities` then `query entity`
- "What's in this subsystem / module?" → `query subsystems` / `query children`
- "What's the architecture / where's the complexity?" → `query subsystems`, `query hotspots`, `query flows`
- Orienting to a repo you haven't read yet, before planning edits.

**Don't use for:** literal text search across all files, non-indexed languages, or reading raw file contents (use `search_files` / `read_file`). Lievo has no full-text corpus search — it is a symbol/structure index.

## Prerequisites
- `lievo` on PATH (`lievo --version`). Install: `references/notes.md`.
- A repo registered as a **project** and refreshed. If not, set it up (below). Default project name (same rules as the #27 naming): the checkout dir name if free; if taken, `<owner>/<repo>` when the repo has an identity (origin remote or `identity:` config), else `<dir>-2`, `-3`, …; a second checkout of the same identity (e.g. a git worktree) becomes `<repo>@<dir>`. Pass an explicit name to avoid guessing.
- `lievo doctor` (read-only) reports registration + index state and the one command to fix it. **Run it first** when in doubt.
- **Multi-project caveat:** with more than one project in the DB, omitting `--project <name>` errors (`multiple projects exist`). Almost always pass `--project` explicitly.

## Set up a repo (first time)
```bash
cd /path/to/repo
lievo admin add-repo .            # registers repo + creates project (default name: see Prerequisites)
lievo refresh <project>            # builds the index (add --no-summarize for the fast path, see Summaries)
lievo doctor                       # confirm: registered: true, index: current
```
`add-repo` prints `Registered repository '<repo>' (id: <id>) in project '<project>'`. `refresh` defaults the project to the sole project only when exactly one exists; otherwise pass it.

## Quick Reference (CLI)
All `query` subcommands take `--project <name>`. The global `--format json` goes *before* the subcommand: `lievo --format json query entities ...`.

| Question | Command |
|---|---|
| Is lievo working here? (read-only) | `lievo doctor` (add `--format json`) |
| Register repo + project | `lievo admin add-repo <path> [<project>]` |
| Create / list / delete project | `lievo admin create-project <name>` / `list-projects` / `delete-project <name>` |
| List repos, status, coverage, stats | `lievo admin list-repos` / `status` / `coverage` / `info` |
| Build / update index | `lievo refresh <project>` (`--full`, `--force`, `--no-summarize`, `--no-ignore`) |
| Find symbols (keyword) | `lievo query entities --project P "term"` |
| Find symbols (semantic) | `lievo query entities --semantic --project P "term"` (degrades to keyword if no vector index) |
| Full symbol detail by id | `lievo query entity --project P <entity-id>` |
| What it depends on / what depends on it | `lievo query deps <entity-id>` / `dependents <entity-id>` |
| Both directions | `lievo query relationships <entity-id>` |
| Children of a module/subsystem | `lievo query children <entity-id>` |
| Subsystems / modules / files | `query subsystems` / `modules <subsystem-id>` / `files <module-id>` |
| Blast radius of changed files | `lievo query impact <file> [<file>...]` (repo-relative paths) |
| Complexity / coupling hotspots | `lievo query hotspots --project P` |
| Execution flows (cycles) | `lievo query flows --project P` |
| Detected conventions / docs | `lievo query conventions --project P` / `docs --project P` |
| Re-summarize entities | `lievo summarize <project>` |

Entity IDs are stable strings: `fn-<hash>` for functions, `<project-id>:<repo>:file:<path>` for files, `...:module:<path>` and `...:subsystem:<path>` for containers. Get an ID from `entities`/`subsystems` (JSON `entity_id` / `id`), then reuse it.

## The query loop (recommended)
1. `lievo doctor` — confirm `index: current`. If `never_indexed` or stale, `lievo refresh <project>`.
2. **Orient:** `query subsystems --project P` (top-level map) → `query children <module-id>` to drill.
3. **Locate:** `query entities --project P "<term>"` → grab the `entity_id`.
4. **Interrogate:** `query entity <id>` (metrics/summary) and/or `query deps <id>` / `query dependents <id>`.
5. **Assess change:** `query impact <repo-relative/file>` for the files you plan to touch.

Prefer `--format json` when you will parse the result (pipe to `jq`); human format is for reading.

## Output formats (verified)
- **JSON is NOT uniformly NDJSON.** `subsystems`, `deps`, `dependents`, `relationships`, `entity` emit **NDJSON** (one object per line). `entities` emits **one JSON array**. `children` emits **one object** `{parent, children[], child_count}`. Parse per-command; do not assume line-per-record everywhere.
- Empty `deps`/`dependents`/`relationships` results print a bare `[]` line, not zero lines. A line-per-object parser must accept it.
- On the relationship commands (`deps`, `dependents`, `relationships`) a `# {"unresolved_imports": N, "unresolved_external": N}` line is written to **stdout** before the rows when resolution counts exist. `2>/dev/null` does not remove it. Skip lines starting with `#`.
- Field names are `snake_case`; optional fields are **omitted** (not `null`) in most structs, but `entities`/`entity` carry `summary: null` and `metrics_json: null` when absent. `metrics_json` is a *string* containing JSON — parse it twice.
- `rel_type` ∈ {`calls`, `imports`, `contains`, ...}; `provenance` ∈ {`resolved`, `heuristic`} (heuristic = inferred edge, lower confidence).
- Human layout is **not stable** — scripts must use JSON.

## Summaries — two databases to know
This is the single most important operational distinction. **Summarization is optional and separate from indexing.** A DB can have the full structural index and zero summaries; or summaries and no vector index. Treat `summary` as possibly-null everywhere.

| DB state | `entities[].summary` / `entity.summary` | How you tell | When |
|---|---|---|---|
| **No summaries** | `null` | `lievo admin status` → a `Summary coverage:` header, then `<repo>: 0/478 functions summarized (478 missing)` | No summarizer backend available, or `--no-summarize`, or `summarize: false` |
| **Summaries present** | one-line prose (module/function/file tiers) | the `Summary coverage:` lines show non-zero done counts | A backend (apfel / llama-server / generic OpenAI-compatible) was reachable during `refresh` |

- **Auto-enable:** a manual `lievo refresh` turns summarization **on** when an `apfel` binary is on PATH, or when a summarizer endpoint is configured. apfel (on-device) honors lievo's batch contract, but summarization is **slow** (minutes for hundreds of functions). Use a backend that honors the batch contract (one `#<n>:` line per input item): a model that answers one item well but loops to empty output on the multi-item prompt yields nothing, and summaries are skipped. Use apfel or a contract-honoring server for summaries.
- **Fast path:** `lievo refresh <project> --no-summarize` builds the structural + vector index quickly and skips summaries. Use this for exploration; add summaries later with `lievo summarize <project>` or a full refresh when time permits.
- **Vector index is separate from summaries.** `--no-summarize` still builds the embedding index → `--semantic` works. No vector index → `query entities --semantic` prints `Semantic search unavailable ...` and falls back to keyword (non-fatal). The ~16 MB embedding model downloads once from Hugging Face on the first manual refresh.
- **No summarizer is configured by default** (no `.lievo/config.yaml`, `LIEVO_SUMMARIZER_TOKEN` unset). For a remote/LLM summarizer, configure `.lievo/config.yaml` (`apfel_endpoint`, `summarizer_backend`) — see `references/notes.md`.
- A degraded run (backend unreachable) **never fails**: indexing succeeds, summaries are skipped, and a warning names the backend + endpoint.

## Environment variables (project / config)
- `LIEVO_PROJECT_DIR=/path/to/repo` — pins which repo is resolved (wins over cwd; fixes an MCP server to one repo).
- `LIEVO_NO_REFRESH=1` — disables auto background indexing (run `lievo refresh` manually).
- `LIEVO_MCP_TOOLS=name1,name2` — opt in extra MCP tools (default lists only `lievo_explore`).
- `LIEVO_DB=/path/to/db` — move the SQLite DB off `~/.lievo/`.
- `LIEVO_APFEL_ENDPOINT`, `LIEVO_SUMMARIZER_TOKEN` — summarizer endpoint + bearer token (token is a credential; never commit).
- Per-repo overrides live in `<repo>/.lievo/config.yaml` (`summarize`, `apfel_endpoint`, `summarizer_backend`, `summarizer_model`, `exclude` globs, `preserve_function_entities`, `summarizer_input_char_budget`, `identity`).

## Pitfalls
- **Omitting `--project`** with multiple projects → hard error. Pass it.
- **Assuming NDJSON for every command** — `entities` is an array, `children` is an object (see Output formats).
- **`query modules <subsystem-id>` / `query files <module-id>`** take the *container's* ID, not a name; pass the wrong ID and you get `No modules found.` / empty. Get the ID from `query subsystems` (JSON `id`) first.
- **`summary` can be `null`** even on a healthy index (no summarizer) — don't treat a missing summary as an error; read source via `read_file` or MCP `get_entity`.
- **`--semantic` without a vector index** falls back to keyword with a warning on stderr (not an error).
- **Summarization is slow on-device.** For quick orientation use `--no-summarize`; don't wait on a full summarize pass just to look at structure.
- **Impact paths are repo-relative** (`src/foo.rs`), not absolute and not entity IDs.
- **Mixed-language repos:** lievo sees only the 8 indexed extensions. A symbol in Ruby/Java/etc. won't appear — that's a miss, not a lievo bug.
- **Heuristic edges** (`provenance: heuristic`) are inferred; treat as likely-not-certain. `resolved` edges are verified.
- Index is **incremental**: after editing code, `lievo refresh <project>` updates only what changed (cheap). After upgrading lievo or changing config, use `--force`.
- A moved/renamed checkout may re-key rows by path; `lievo doctor` shows the resolved repo source (`cwd` vs `LIEVO_PROJECT_DIR`).

## Verification
- `lievo doctor` exits 0 and prints `registered: true`, `index: current`, with the resolved repo source correct.
- `lievo admin status` shows entity/relationship counts for your project and the `Summary coverage:` line (0/N = no summaries — expected if you used `--no-summarize`).
- A representative `query entities` returns rows with real paths; `query impact <file>` lists affected modules + downstream dependents.
- JSON parse check: `lievo --format json query subsystems --project P | jq -c .` (NDJSON) vs `... query entities --project P | jq 'length'` (array).

## References
- `references/mcp-tools.md` — the 15 MCP tool names + `lievo_explore` modes (for agents that consume lievo over MCP).
- `references/notes.md` — install, summarizer backends & `.lievo/config.yaml`, data paths & reset, benchmarks.
