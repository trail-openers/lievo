# Agent Output Contract

## Overview

Lievo provides deterministic, machine-readable output for agents and scripts while maintaining human-friendly defaults for interactive use.

## Output Formats

### Human (default)

- Aligned tables with headers
- Clear section separators
- Optimized for terminal interaction

### JSON (explicit flag)

- **Single values**: One JSON object
- **List-heavy commands**: NDJSON (one JSON object per line)
- Deterministic field ordering
- Null fields preserved (not omitted)

## Format Selection

The output format is selected by the `--format` flag only; there is no TTY detection. `human` is the default.

```bash
# Default: human output
lievo query subsystems

# Machine-readable output for scripts and pipes
lievo query subsystems --format json | jq .
```

## Structured Error Envelope (stderr)

All errors emit to stderr as structured JSON:

```json
{
  "error": {
    "message": "Database locked — is another lievo process running?",
    "kind": "DatabaseLocked"
  },
  "code": "DATABASE_LOCKED",
  "retryable": true
}
```

### Fields

- `error.message`: Human-readable error message
- `error.kind`: Error variant name (camelCase)
- `code`: Uppercase snake_case error code
- `retryable`: Boolean indicating if operation can be retried

### Error Codes

| Code | Retryable | Description |
|------|-----------|-------------|
| DATABASE_LOCKED | Yes | Another process holds database lock |
| ENTITY_NOT_FOUND | No | No entity exists with given ID |
| PROJECT_NOT_FOUND | No | No project exists with given name |
| IO | Yes | Filesystem I/O error |
| ... | ... | (full list in src/error.rs) |

## Deterministic JSON

All JSON output uses `serde_json` with `preserve_order` feature enabled. Field order is fixed across invocations.

### Example: Entity JSON

```json
{
  "id": "entity-id",
  "project_id": "proj-id",
  "repo_id": "repo-id",
  "tier": "module",
  "parent_id": "parent-id",
  "name": "module-name",
  "path": "src/module.rs",
  "language": "Rust",
  "summary": "Module description",
  "summary_commit": "abc123",
  "metrics_json": "{\"complexity_max\": 10}",
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-02T00:00:00Z"
}
```

## NDJSON Format

List-heavy commands output one JSON object per line:

```
{"id": "e1", "name": "module-a", "tier": "module", ...}
{"id": "e2", "name": "module-b", "tier": "module", ...}
```

Commands using NDJSON (list commands):
- `query entities`
- `query subsystems`
- `query modules`
- `query files`
- `query functions`
- `query relationships` — outputs `depends_on` lines followed by `depended_by` lines
- `list insights`
- `list conventions`

### Single vs List Command Schemas

**List commands** (always NDJSON format in JSON mode):
- Output one complete JSON object per line
- Empty result = empty output (no newline)
- All entities have consistent schema with all fields

**Single-value commands** (compact JSON object in JSON mode):
- `query entity <id>` — returns one entity object
- `query children <id>` — returns `{parent, children[], child_count}` object
- `query impact <files>` — returns section-based NDJSON (4 lines with `section` label)

### Relationship Output Structure

The `query relationships <entity_id>` command outputs NDJSON with relationship objects:

```json
{"entity_id": "target-id", "entity_name": "target-mod", "entity_tier": "module", "rel_type": "depends_on", "weight": 1.0}
{"entity_id": "dependent-id", "entity_name": "dependent-mod", "entity_tier": "module", "rel_type": "depended_by", "weight": 1.0}
```

The output contains two sections (separated by newline):
1. **DEPENDS ON** section: relationships where the queried entity depends on other entities
2. **DEPENDED BY** section: relationships where other entities depend on the queried entity

If a section is empty (no relationships), it outputs zero lines for that section.
Note: The `rel_type` field indicates the relationship direction context within the output.

## `lievo doctor` JSON Output

`lievo doctor [PATH] --format json` emits a single JSON object. Null fields are preserved (not omitted). Field set:

| Field | Type | Description |
|---|---|---|
| `version` | string | lievo version (e.g. "0.1.0") |
| `data_dir` | string | The data directory actually used (parent of `db_path`) |
| `db_path` | string | The database file path actually used (honours `LIEVO_DB`) |
| `db_writable` | bool | Whether the data directory is writable (probed via a temp file) |
| `repo` | string \| null | Resolved git repo root, or null when outside a git repo |
| `repo_source` | string \| null | Source that produced `repo`: "LIEVO_PROJECT_DIR" \| "CLAUDE_PROJECT_DIR" \| "cwd" \| null |
| `registered` | bool | Whether the resolved repo is registered in the database |
| `project` | string \| null | Project name for the registered repo, or null |
| `index_state` | string \| null | "never_indexed" \| "current" \| "stale" \| "unborn" (repo has no commits yet — `head_commit` is null) \| null (outside a git repo) |
| `entity_count` | int \| null | Entity count for the registered repo, via `COUNT` of its entities, or null |
| `last_indexed_commit` | string \| null | Stored `last_analyzed_commit` for the repo, or null |
| `head_commit` | string \| null | Current git HEAD of the resolved repo, or null |
| `indexing_in_progress` | bool | Whether a live process holds the per-repo index lock |
| `indexing_elapsed_secs` | int \| null | Seconds the index has been running (null when not running or unknown) |
| `env` | object | The set `LIEVO_*` variables (each only when set): `LIEVO_DB`, `LIEVO_NO_REFRESH`, `LIEVO_MCP_TOOLS`, `LIEVO_PROJECT_DIR` |
| `vector_index_present` | bool | Whether `vectors.usearch` exists on disk (informational; not required for `lievo_explore`) |
| `embedding_model_present` | bool | Whether the embedding model snapshot exists on disk (informational) |
| `problems` | array | Array of `{"what": string, "fix": string}` objects, most severe first |
| `ok` | bool | true when lievo will work here (exit 0); false when it will not (exit 1) |

Outside a git repository, `repo`, `repo_source`, `project`, `index_state`, `entity_count`, `last_indexed_commit`, and `head_commit` are all null; `registered` is false; `problems` contains a single "not in a git repository" entry.

When the database cannot be opened (corrupt file, missing `LIEVO_DB` parent directory, permission denied, locked), `registered`, `project`, `entity_count`, and `last_indexed_commit` are all null and `problems` includes a `database cannot be opened: …` entry that flips `ok` to false (exit 1).

## Testing

See `tests/output_contract_test.rs` for:
- Deterministic field ordering tests
- Structured error envelope schema tests
- NDJSON format validation tests
- Empty result handling

## Migration Notes

- **Default format**: Always `human`, regardless of TTY state
- **Machine-readable output**: Use `--format json` for scripts and pipes
- **Error output**: All stderr now structured JSON
- **Parsing agents**: Should parse stderr error envelopes on non-zero exit