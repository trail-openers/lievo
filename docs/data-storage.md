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
- Model weights (minishlab/potion-code-16M-v2, ~16MB) are automatically downloaded from Hugging Face on first use and cached locally

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

All lievo data lives under `~/.lievo/`:

| Path | Purpose |
|------|---------|
| `~/.lievo/lievo.db` | SQLite database (entities, relationships, insights) |
| `~/.lievo/indices/{repo-hash}/ts-index/` | Semantic search index (per repository) |
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
