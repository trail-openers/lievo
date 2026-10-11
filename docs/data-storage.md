# Data Storage

Lievo stores project data and analysis artifacts in several locations across the filesystem. This document describes every file and directory lievo creates, their purpose, and whether they can be safely deleted.

## SQLite Database

**`~/.lievo/lievo.db`**

Contains all project data, including:
- Projects and repositories
- Extracted entities (modules, classes, functions, etc.)
- Relationships between entities
- Generated summaries
- Insights and metrics (complexity hotspots, circular dependencies)

Created automatically on first `lievo analyze` or `lievo refresh`.

**Can be safely deleted** — lievo will rebuild from source on the next `lievo refresh`. Rebuilding may take several minutes for large codebases.

## Semantic Search Index

**`~/.lievo/indices/{repo-hash}/ts-index/`**

Tree-sitter and vector search index for entity discovery. Used by `search_entities` with `semantic=true`. Each analysed repository gets its own subdirectory, named by an FNV-1a hash of the repository path.

**Structure:**
- One subdirectory per analyzed repository under `~/.lievo/indices/`
- Each repository subdirectory contains the tree-sitter index and usearch vector embeddings
- Model weights (minishlab/potion-code-16M-v2, ~16MB) are automatically downloaded from Hugging Face on first use and cached locally, by default in `~/.cache/huggingface/hub`
- To use another location (e.g. a read-only default cache), set `HF_HUB_CACHE` to the hub cache directory, or `HF_HOME` to a directory whose `hub/` subdirectory holds the cache. `HF_HUB_CACHE` takes precedence over `HF_HOME`. The download token is read from `HF_HOME/token` (or `~/.cache/huggingface/token` when `HF_HOME` is unset or empty), so `HF_HUB_CACHE` does not change authentication.
- `HF_ENDPOINT` is honoured when set (a non-empty value replaces the Hugging Face URL), as hf-hub does. An empty `HF_ENDPOINT` is treated as unset.
- The Hugging Face token is sent to whatever `HF_ENDPOINT` points at, so use an `https` endpoint.
- `HF_HOME` is honoured for downloads, not only for the token. If `HF_HOME` is set, the model is downloaded again into `$HF_HOME/hub` the first time after upgrading, even if a copy already exists in the default cache. Set `HF_HUB_CACHE` to the old location to keep using it.

Created during `lievo refresh`.

**Network access required**: First-time semantic index builds (or after deleting the cache) require network access to download the embedding model. Subsequent refreshes use the cached model.

**Can be safely deleted** — lievo will rebuild the index on the next `lievo refresh`. If the cache is deleted completely, the model will be re-downloaded from Hugging Face (requires network access). Rebuilding the index is faster than rebuilding the database.

## Per-Repository Configuration

**`<repo-path>/.lievo/config.yaml`**

Optional per-repository configuration. Controls extraction behavior for specific repositories.

**Example:**
```yaml
preserve_function_entities: true
```

Setting `preserve_function_entities: false` disables function-level analysis, which reduces memory usage but removes function-level relationships and execution flow tracking.

**Not created automatically** — create this file manually only if you need project-specific settings to override defaults.

## Debug Log

**`~/.lievo/debug-import-resolution.log`**

Diagnostic log for import resolution debugging. Created only when debug mode is enabled (issue #432).

Contains:
- Analysis run separators and repository names
- Import map samples (first 50 entries)
- Raw import statements for each code unit
- Unresolved imports that contained `::` but had no match

Capped at 1MB. Can be ignored in normal use.

## Disk Layout Summary

Lievo's own data lives under `~/.lievo/`, with one exception: the embedding model weights live in the Hugging Face hub cache (see Semantic Search Index).

| Path | Purpose |
|------|---------|
| `~/.lievo/lievo.db` | SQLite database (entities, relationships, insights) |
| `~/.lievo/indices/{repo-hash}/ts-index/` | Semantic search index (per repository) |
| `~/.cache/huggingface/hub/` (or `HF_HUB_CACHE` / `HF_HOME/hub`) | Embedding model weights, shared with other Hugging Face tools |
| `~/.lievo/debug-import-resolution.log` | Debug log (optional) |
| `<repo>/.lievo/config.yaml` | Per-repository configuration (optional) |

## Resetting Data

To reset lievo's analysis data:

```bash
# Delete database and index (rebuild from source on next refresh)
rm ~/.lievo/lievo.db
rm -rf ~/.lievo/indices/

# Then refresh to rebuild
lievo refresh <project>
```

**Selective reset:**
- Delete only `lievo.db` to clear all analysis data (entities, relationships, insights)
- Delete only `~/.lievo/indices/` to clear semantic search (database remains intact)
- Delete repository-specific `.lievo/` config to reset per-repository settings

## Known limitations

- **Moved-checkout provenance:** a vanished checkout's row is re-keyed to a new path only when the new checkout's origin remote matches the row's stored `git_url`. A `git_url` derived from a repo-controlled `identity:` override can be stored on a row by path-match backfill, and a later origin checkout with that key could then move the row. The `git_url` does not record where its key came from, so this needs a schema change. It is not fixed yet.
