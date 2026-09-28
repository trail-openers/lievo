# Query Command Design Patterns

Lievo query commands follow a minimal, repeatable pattern for building entity graph queries. These patterns emphasize separation of concerns and testability.

## Three-Layer Pattern

Every query command follows three layers:

```
CLI arg parsing (lievo.rs)
  → Command handler (commands/query*.rs)
  → Storage query (storage/*.rs)
```

Each layer has a single responsibility:

1. **CLI layer**: Parse arguments, handle errors, exit codes
2. **Command layer**: Validate input, route queries, format results
3. **Storage layer**: Return structured data (entities, relationships)

**Why three layers**: CLI logic should not touch the database. Storage logic should not format output. Command layer orchesrates without mixing concerns.

## Handler Function Pattern

Query handlers follow this signature pattern:

```rust
pub fn command_name(
    storage: &SqliteStorage,
    project_id: Option<&str>,
    // ... other query-specific args
    format: OutputFormat,
) -> Result<()> {
    let results = storage.query_method(...)?;

    match format {
        OutputFormat::Human => {
            let formatted = format_results_human(&results);
            println!("{formatted}");
        }
        OutputFormat::Json => {
            for item in results {
                let json = format_result_json(&item)?;
                println!("{json}");
            }
        }
    }

    Ok(())
}
```

**Key invariants**:
- Returns `Result<()>` for error propagation
- Accepts `format: OutputFormat` (global, not per-flag)
- Prints to stdout via `println!` (no `std::io` wrapper needed)
- Errors bubble up as `LievoError` variants

**Why `format` is a parameter, not a captured global**: Explicit parameter passing makes the contract clear. Each function's call site shows exactly which format it will use.

## Storage Layer Contract

Storage methods return domain types (`Vec<Entity>`, `Relationship`, etc.) — never formatted strings:

```rust
impl SqliteStorage {
    pub fn list_entities(&self, project_id: &str, query: &str) -> Result<Vec<Entity>>;
    pub fn get_entity(&self, id: &str) -> Result<Entity>;
    pub fn get_relationships(&self, entity_id: &str) -> Result<Vec<Relationship>>;
}
```

**Principle**: Storage layer is data-only. No formatting, no CLI considerations. This allows the same storage methods to be used by the library API, MCP server, and CLI without duplication.

## Formatting Separation

All formatting lives in `src/output/*.rs` as pure functions:

```rust
pub fn format_entities_human(entities: &[Entity], tier: EntityTier) -> String { ... }
pub fn format_entity_json(entity: &Entity) -> Result<String> { ... }
```

**Characteristics**:
- Pure functions (no side effects)
- Return `String` or `Result<String>` — never print internally
- Tested with unit tests in the same module
- No database or file I/O

**Why separation**: Formatters can be tested independently of storage. The same formatter can serve CLI and MCP server.

## Error Propagation Pattern

Errors propagate via the `Results<T, LievoError>` type chain:

```
Storage errors (rusqlite::Error) → LievoError::Database
  → Command layer (? operator)
  → CLI main (eprintln! + exit 1)
```

Each layer maps errors to the appropriate domain:

```rust
// Storage layer
Err(rusqlite::Error::QueryReturnedNoRows) => Err(LievoError::EntityNotFound(id))

// Command layer
let entity = storage.get_entity(id)?;

// CLI layer
if let Err(e) = result {
    eprintln!("Error: {e}");
    process::exit(1);
}
```

**No error suppression**: Every error surfaces. The CLI does not swallow or transform errors — they propagate unchanged to stderr.

## Project ID Resolution

Commands with optional `--project` arguments follow a deterministic resolution pattern:

1. If `--project` is provided, use it directly
2. If omitted and exactly one project exists in the database, resolve to that project
3. If omitted and multiple projects exist, return an error

**Why this design**: Optional shorthand for common single-project use cases, not magic auto-detection. Explicit is better than implicit for multi-project setups.

## Impact Analysis Pattern

Impact queries (`lievo query impact`) differ from other queries in structure:

- Accept multiple file paths (repo-relative)
- Return an aggregated impact report
- Format shows affected entities grouped by impact type

This is the only query command that accepts a `Vec<String>` for the primary search term (versus a single query string). All other list/search commands accept a single target identifier.

## Relationship Query Direction

Dependency and dependent queries (`deps`, `dependents`) are the same underlying storage call with direction specified:

```rust
pub fn deps(&self, entity_id: &str) -> Result<Vec<Relationship>> { ... }
pub fn dependents(&self, entity_id: &str) -> Result<Vec<Relationship>> { ... }
```

**Why two commands**: Directional terminology is clearer for users than a generic `relations --direction=forward` flag. Each command is a clear mental model (what I depend on vs what depends on me).

## Discovery Queries

Discovery queries (`flows`, `docs`) operate at the project level and expose structured metadata:

- `flows`: Execution flow graph extracted during analysis
- `docs`: Discovered documentation files from project

These queries do not take entity IDs — they enumerate project-level resources discovered during analysis.